//! The phone API's task routes (v1.2, overnight 11-07 lane I3; contract
//! `docs/phone-api/tasks.md`, fixtures `docs/phone-api-fixtures/tasks/`).
//!
//! `GET /v1/tasks` lists the vault's tasks the way the Tasks page groups
//! them. The three writes (`POST /v1/tasks`, `/v1/tasks/status`,
//! `/v1/tasks/update`) are the person's own edits made through
//! [`crate::task_broker::dispatch_companion`], the writer a paired companion
//! device's op uses: the same validation, the tracker's rules for a linked
//! task, the compare-and-swap on the file's hash and the short companion
//! write lease. Nothing here deletes, and a ref only ever names a real file
//! under `tasks/` ([`crate::vault_tasks::resolve_task_ref`]).

use super::live::cut_chars;
use super::routes::{self, json, ApiError, Ctx, Reply};
use super::store::{Device, Store};
use crate::task_broker::{self, CompanionRefusal};
use crate::types::Task;
use crate::vault_tasks;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// Rows an open or combined list sends, at most.
const OPEN_MAX: usize = 500;
/// Finished rows a list sends by default, and at most.
const DONE_DEFAULT: usize = 30;
const DONE_MAX: usize = 100;
/// A row's description, at most this many characters.
const DESCRIPTION_MAX: usize = 1_000;
/// A row's checklist items, at most, each at most this many characters.
const SUBTASKS_MAX: usize = 50;
const SUBTASK_TEXT_MAX: usize = 200;
const LABELS_MAX: usize = 20;
/// An idempotency key is this many characters, of letters, digits and `-_.`.
const KEY_MIN: usize = 8;
const KEY_MAX: usize = 64;
/// The fields the update route changes.
const UPDATE_FIELDS: [&str; 4] = ["title", "due", "dueTime", "priority"];
/// The ones of them an empty string or null clears.
const CLEARABLE: [&str; 3] = ["due", "dueTime", "priority"];
/// A recurrence phrase, at most this many characters.
const RECURRENCE_MAX: usize = 60;
/// A ref is at most this many characters (the write routes' own limit).
const REF_MAX: usize = 400;
/// New tasks one iPhone may make in 24 hours: a runaway app can't flood the
/// vault. The Mac's own companion ops stop at 200 files a day too.
const CREATES_PER_DAY: usize = 200;
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// One task as the phone sees it.
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub status: String,
    pub done: bool,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub bucket: &'static str,
    pub days_until_due: Option<i64>,
    pub project: Option<String>,
    pub labels: Vec<String>,
    pub recurrence: Option<String>,
    pub description: Option<String>,
    pub description_truncated: bool,
    pub subtasks: Vec<SubRow>,
    pub subtasks_total: usize,
    pub blocked: bool,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_type: Option<crate::tracker::StateKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker_url: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub tracker_linked: bool,
    /// Compatibility name (the phone contract), true for either a live
    /// tracker link or a legacy Notion import (`notion_id`).
    #[serde(skip_serializing_if = "is_false")]
    pub notion_linked: bool,
    /// Where the task lives, the same words the desktop shows (H4b):
    /// `{kind, text, url, openLabel, ...}` from `sources::label`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<crate::sources::SourceLabel>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubRow {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListBody {
    pub today: String,
    pub tasks: Vec<Row>,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Written {
    pub task: Row,
    pub disposition: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusWritten {
    pub task: Row,
    pub disposition: String,
    pub rolled_forward_to: Option<String>,
}

/// Which tasks a list is after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Open,
    Done,
    All,
}

// ── Reading ──────────────────────────────────────────────────────────────

/// Whether a status means the work is finished or dropped (the Mac's two
/// rules together).
pub(crate) fn is_finished(status: &str) -> bool {
    vault_tasks::is_terminal_status(status) || vault_tasks::is_cancelled_status(status)
}

fn task_finished(task: &Task) -> bool {
    task.tracker
        .as_ref()
        .and_then(|m| m.status_type)
        .map_or_else(|| is_finished(&task.status), |kind| kind.is_closed())
}

/// Whether a task's ref names a file the write routes would accept: none of
/// its parts is hidden (`vault_tasks::resolve_task_ref`'s rule), no backslash,
/// at most [`REF_MAX`] characters, so the list never offers a task the phone
/// could not change.
fn is_addressable(reference: &str) -> bool {
    let rel = reference.strip_prefix("vault:").unwrap_or(reference);
    reference.chars().count() <= REF_MAX
        && !has_control(reference)
        && !rel.contains('\\')
        && !rel.split('/').any(|part| part.starts_with('.'))
}

/// A real folder at `tasks/` (or none yet). A link there would list, and let
/// the phone write to, another part of the vault, so the routes leave it alone.
fn tasks_dir_is_real(vault: &Path) -> bool {
    std::fs::symlink_metadata(vault.join("tasks")).map_or(true, |m| !m.file_type().is_symlink())
}

fn ensure_tasks_dir(vault: &Path) -> Result<(), ApiError> {
    if tasks_dir_is_real(vault) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "The tasks folder is a link, so Hollow leaves it alone on the iPhone.".to_string(),
        ))
    }
}

/// `HH:MM` as the task writer takes it.
fn is_clock(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[3..].iter().all(u8::is_ascii_digit)
        && value[..2].parse::<u32>().is_ok_and(|h| h < 24)
        && value[3..].parse::<u32>().is_ok_and(|m| m < 60)
}

/// A stored due as `(YYYY-MM-DD, HH:MM)`: Notion can store `2026-11-09T14:30`
/// in `due`; Hollow's own tasks keep the time in `due_time`, and when both
/// name a time the task's own `due_time` wins. A due that is no date reads as
/// none, and then so does the time (a time of day needs its day).
pub(crate) fn split_due(
    due: Option<&str>,
    due_time: Option<&str>,
) -> (Option<String>, Option<String>) {
    let due = due.map(str::trim).filter(|d| !d.is_empty());
    let Some((whole, date)) = due.and_then(|d| Some((d, d.get(..10)?))) else {
        return (None, None);
    };
    if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return (None, None);
    }
    let inline = whole
        .get(10..16)
        .and_then(|rest| rest.strip_prefix('T'))
        .filter(|time| is_clock(time));
    let time = due_time
        .map(str::trim)
        .filter(|t| is_clock(t))
        .or(inline)
        .map(str::to_string);
    (Some(date.to_string()), time)
}

/// The Mac's bucket (`commands::enrich_tasks`) for `today`.
fn bucket_of(finished: bool, days: Option<i64>) -> &'static str {
    if finished {
        return "done";
    }
    match days {
        None => "undated",
        Some(d) if d < 0 => "overdue",
        Some(0) => "today",
        Some(d) if d <= 7 => "thisweek",
        Some(_) => "later",
    }
}

fn bucket_rank(bucket: &str) -> u8 {
    match bucket {
        "overdue" => 0,
        "today" => 1,
        "thisweek" => 2,
        "later" => 3,
        "undated" => 4,
        _ => 5,
    }
}

/// `P1` first, `P4` next to last, no priority last (the contract's order).
fn priority_rank(priority: &Option<String>) -> u8 {
    match priority.as_deref() {
        Some("P1") => 0,
        Some("P2") => 1,
        Some("P3") => 2,
        Some(_) => 3,
        None => 4,
    }
}

fn row(task: &Task, today: NaiveDate) -> Row {
    let (due, due_time) = split_due(task.due.as_deref(), task.due_time.as_deref());
    let days = due
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.signed_duration_since(today).num_days());
    let finished = task_finished(task);
    let text = task.description.trim();
    let description = (!text.is_empty()).then(|| cut_chars(text, DESCRIPTION_MAX));
    let subtasks = task
        .subtasks
        .iter()
        .take(SUBTASKS_MAX)
        .map(|s| SubRow {
            text: cut_chars(s.text.trim(), SUBTASK_TEXT_MAX),
            done: s.done,
        })
        .collect();
    Row {
        reference: task.url.clone(),
        title: task.title.clone(),
        status: task.status.clone(),
        done: finished,
        priority: task.priority.clone(),
        due,
        due_time,
        bucket: bucket_of(finished, days),
        days_until_due: days,
        project: task.project_name.clone(),
        labels: task.labels.iter().take(LABELS_MAX).cloned().collect(),
        recurrence: task.recurrence.clone(),
        description_truncated: text.chars().count() > DESCRIPTION_MAX,
        description,
        subtasks,
        subtasks_total: task.subtasks.len(),
        blocked: !task.unresolved_blockers.is_empty(),
        content_hash: task.content_hash.clone(),
        status_type: task.tracker.as_ref().and_then(|m| m.status_type),
        tracker_key: task.tracker.as_ref().and_then(|m| m.key.clone()),
        tracker_url: task.tracker.as_ref().and_then(|m| m.url.clone()),
        tracker_linked: task.tracker.is_some(),
        notion_linked: task.tracker.is_some() || task.notion_id.is_some(),
        source_label: task.source_label.clone(),
    }
}

fn after(a: &Option<String>, b: &Option<String>) -> Ordering {
    // A time sorts before none.
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn modified(vault: &Path, task: &Task) -> Option<SystemTime> {
    let rel = task.url.strip_prefix("vault:")?;
    std::fs::metadata(vault.join(rel)).ok()?.modified().ok()
}

/// The list: open tasks by bucket, due date, time, priority and title;
/// finished ones newest change first; both for `All`, open first. Each task
/// is labeled first (`label`: H4b's `Host::label_tasks`, so the phone shows
/// the desktop's words).
pub(crate) fn list_body(
    vault: &Path,
    which: Which,
    limit: usize,
    today: NaiveDate,
    label: impl FnOnce(&mut [Task]),
) -> ListBody {
    // A link at `tasks/` lists nothing (the write routes refuse it too).
    let mut tasks = if tasks_dir_is_real(vault) {
        vault_tasks::list(vault)
    } else {
        Vec::new()
    };
    label(&mut tasks);
    let (open, finished): (Vec<Task>, Vec<Task>) = tasks
        .into_iter()
        .filter(|t| is_addressable(&t.url))
        .partition(|t| !task_finished(t));
    let total = match which {
        Which::Open => open.len(),
        Which::Done => finished.len(),
        Which::All => open.len() + finished.len(),
    };
    let mut rows: Vec<Row> = Vec::new();
    if which != Which::Done {
        let mut open_rows: Vec<Row> = open.iter().map(|t| row(t, today)).collect();
        open_rows.sort_by(|a, b| {
            bucket_rank(a.bucket)
                .cmp(&bucket_rank(b.bucket))
                .then_with(|| a.due.cmp(&b.due))
                .then_with(|| after(&a.due_time, &b.due_time))
                .then_with(|| priority_rank(&a.priority).cmp(&priority_rank(&b.priority)))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.reference.cmp(&b.reference))
        });
        rows.extend(open_rows);
    }
    if which != Which::Open {
        let mut done_rows: Vec<(Option<SystemTime>, Row)> = finished
            .iter()
            .map(|t| (modified(vault, t), row(t, today)))
            .collect();
        done_rows.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()))
                .then_with(|| a.1.reference.cmp(&b.1.reference))
        });
        rows.extend(done_rows.into_iter().map(|(_, r)| r));
    }
    rows.truncate(limit);
    ListBody {
        today: today.to_string(),
        has_more: total > rows.len(),
        tasks: rows,
        total,
    }
}

pub(crate) async fn list(ctx: &Arc<Ctx>, query: &str) -> Result<Reply, ApiError> {
    let which = match routes::query_param(query, "state").as_deref() {
        None | Some("open") => Which::Open,
        Some("done") => Which::Done,
        Some("all") => Which::All,
        Some(_) => {
            return Err(ApiError::bad_request(
                "state must be open, done or all.".to_string(),
            ))
        }
    };
    let (default, max) = match which {
        Which::Done => (DONE_DEFAULT, DONE_MAX),
        Which::Open | Which::All => (OPEN_MAX, OPEN_MAX),
    };
    let limit = limit_of(query, default, max)?;
    let ctx = ctx.clone();
    let body = routes::blocking(move || {
        Ok(list_body(
            &ctx.vault,
            which,
            limit,
            Local::now().date_naive(),
            |tasks| ctx.host.label_tasks(tasks),
        ))
    })
    .await?;
    Ok(json(200, &body))
}

pub(crate) async fn statuses(ctx: &Arc<Ctx>) -> Result<Reply, ApiError> {
    let root = ctx.vault.clone();
    let body = routes::blocking(move || {
        match crate::tracker_sync::seams::cached_at(&root).and_then(|state| {
            state
                .meta
                .ok_or_else(|| "Tracker metadata is unavailable.".into())
        }) {
            Ok(meta) => {
                let states = meta.states;
                Ok(json!({ "statuses": states, "tracker": true }))
            }
            Err(_) => Ok(json!({ "statuses": [
                {"id":"Not started","name":"Not started","kind":"unstarted"},
                {"id":"In progress","name":"In progress","kind":"started"},
                {"id":"Done","name":"Done","kind":"completed"}
            ], "tracker": false })),
        }
    })
    .await?;
    Ok(json(200, &body))
}

/// `limit`: a whole number from 1 to `max`, `default` when absent.
fn limit_of(query: &str, default: usize, max: usize) -> Result<usize, ApiError> {
    match routes::query_param(query, "limit") {
        None => Ok(default),
        Some(raw) => match raw.trim().parse::<usize>() {
            Ok(n) if (1..=max).contains(&n) => Ok(n),
            _ => Err(ApiError::bad_request(format!(
                "limit must be a whole number from 1 to {max}."
            ))),
        },
    }
}

// ── Writing ──────────────────────────────────────────────────────────────

pub(crate) fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| {
        ApiError::bad_request("The request body isn't the JSON Hollow expects.".to_string())
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateRequest {
    idempotency_key: Option<String>,
    title: Option<String>,
    due: Option<String>,
    due_time: Option<String>,
    priority: Option<String>,
    description: Option<String>,
    labels: Option<Vec<String>>,
    recurrence: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusRequest {
    #[serde(rename = "ref")]
    reference: Option<String>,
    status: Option<String>,
    expected_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateRequest {
    #[serde(rename = "ref")]
    reference: Option<String>,
    expected_hash: Option<String>,
    fields: Option<Map<String, Value>>,
}

pub(crate) fn valid_key(key: &str) -> bool {
    (KEY_MIN..=KEY_MAX).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The ref a request names, trimmed: 400 when missing, absurdly long or not
/// one line of plain text.
fn reference_of(reference: Option<&str>) -> Result<String, ApiError> {
    // Checked as sent: trimming would hide a trailing line break.
    let sent = reference.unwrap_or("");
    let reference = sent.trim();
    if reference.is_empty() || reference.chars().count() > REF_MAX || has_control(sent) {
        return Err(ApiError::bad_request(
            "ref is required: the task's ref as the list gave it.".to_string(),
        ));
    }
    Ok(reference.to_string())
}

/// Whether text holds a control character (C0, DEL, C1) or a Unicode line or
/// paragraph separator: nothing a one-line field may carry.
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// 400 when `value` is not one line of plain text.
pub(crate) fn plain_line(name: &str, value: &str) -> Result<(), ApiError> {
    if has_control(value) {
        return Err(ApiError::bad_request(format!(
            "{name} can't hold control characters or line breaks."
        )));
    }
    Ok(())
}

/// A date as the phone sends it: `YYYY-MM-DD`, ten characters, a real day.
fn canonical_date(value: &str) -> bool {
    let b = value.as_bytes();
    // Digits and two dashes in their places, so no leading space or missing
    // zero gets past chrono's lenient parser.
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// A due date, an optional time of day, or neither: what a create sends and
/// an update's `due` and `dueTime` carry (an empty value is none). 400 for
/// anything the Tasks page would store differently than the phone shows.
fn check_due(due: Option<&str>, due_time: Option<&str>) -> Result<(), ApiError> {
    if let Some(due) = due.filter(|d| !d.is_empty()) {
        if !canonical_date(due) {
            return Err(ApiError::bad_request(
                "due must be a date written YYYY-MM-DD.".to_string(),
            ));
        }
    }
    if let Some(time) = due_time.filter(|t| !t.is_empty()) {
        if !is_clock(time) {
            return Err(ApiError::bad_request(
                "dueTime must be a time of day written HH:MM.".to_string(),
            ));
        }
    }
    Ok(())
}

/// The hash a request sends back: 64 hex characters, lowercased.
fn expected_of(hash: Option<&str>) -> Result<String, ApiError> {
    let hash = hash.map(str::trim).unwrap_or("");
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "expectedHash is required: the task's contentHash as the list gave it.".to_string(),
        ));
    }
    Ok(hash.to_ascii_lowercase())
}

/// A write that did not land, as the contract's error.
fn refusal(error: CompanionRefusal) -> ApiError {
    match error {
        CompanionRefusal::Busy(reason) => {
            eprintln!("[phone] a task write waits: {reason}");
            ApiError::busy("Hollow is busy for a moment. Try again.".to_string())
        }
        CompanionRefusal::Refused(reason)
            if reason.starts_with(vault_tasks::TASK_CONFLICT_ERROR) =>
        {
            ApiError {
                status: 409,
                code: "task_changed",
                message: "This task changed on your Mac after you opened it. Nothing was saved."
                    .to_string(),
                attempts_left: None,
            }
        }
        CompanionRefusal::Refused(reason) if reason.starts_with("No task at ") => {
            ApiError::not_found(reason)
        }
        CompanionRefusal::Refused(reason) => ApiError::bad_request(reason),
    }
}

/// One write through the companion writer, as the person's own edit. The
/// phone is checked again once the writer holds its locks, right before the
/// write (`task_broker::dispatch_companion_gated`): the tracker's edit lock
/// can take up to 20 s, and a phone revoked in that wait writes nothing and
/// gets the 401.
fn write(
    store: &Store,
    vault: &Path,
    device: &Device,
    op_id: &str,
    op: &str,
    args: Value,
) -> Result<Value, ApiError> {
    let revoked = std::cell::Cell::new(None);
    let written = task_broker::dispatch_companion_gated(
        vault,
        &device.device_id,
        &device.name,
        op_id,
        op,
        &args,
        Local::now(),
        || {
            routes::commit_point(store, &device.device_id, "task-write").map_err(|error| {
                let text = error.message.clone();
                revoked.set(Some(error));
                text
            })
        },
    );
    match (written, revoked.take()) {
        (Err(_), Some(error)) => Err(error),
        (written, _) => written.map_err(refusal),
    }
}

/// The task a write named, as the list reads it now. A write that landed is
/// never answered with a 500: when the list does not hold the task (a ref it
/// skips, say) the one file is read instead, and that read carries no blocker
/// structure, so `blocked` reads false there.
pub(crate) fn task_after(vault: &Path, result: &Value) -> Result<Row, ApiError> {
    let reference = result
        .get("ref")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::internal("the write did not name its task".to_string()))?;
    let today = Local::now().date_naive();
    if let Some(task) = vault_tasks::list(vault)
        .into_iter()
        .find(|t| t.url == reference)
    {
        return Ok(row(&task, today));
    }
    let rel = reference.strip_prefix("vault:").unwrap_or(reference);
    vault_tasks::get(vault, rel)
        .map(|task| row(&task, today))
        .ok_or_else(|| ApiError::internal("the task could not be read after the write".to_string()))
}

/// When each iPhone made its tasks in the last 24 hours, oldest first. In
/// memory: a Hollow restart starts the count again (it is a guard against a
/// runaway app, not a quota). The notes route keeps its own lists in the same
/// map, under `notes:<deviceId>` (`notes_api::count_key`).
static CREATED: LazyLock<Mutex<HashMap<String, VecDeque<Instant>>>> = LazyLock::new(Mutex::default);

/// Every device's lapsed times go, and a device left with none is dropped, so
/// the map holds only phones with a task made in the last day (it kept an
/// empty list per device id until a restart).
pub(crate) fn prune_created(all: &mut HashMap<String, VecDeque<Instant>>, now: Instant) {
    all.retain(|_, times| {
        while times
            .front()
            .is_some_and(|made| now.duration_since(*made) >= DAY)
        {
            times.pop_front();
        }
        !times.is_empty()
    });
}

/// Reserve one of this iPhone's `cap` new tasks for the last 24 hours, in one
/// step under the lock (two requests at 199 can't both pass): `Some(slot)`
/// to hand back with [`release_create`] when the write did not make a task,
/// `None` when the day's count is used up.
pub(crate) fn reserve_create(device_id: &str, now: Instant, cap: usize) -> Option<Instant> {
    let mut all = CREATED.lock().unwrap_or_else(PoisonError::into_inner);
    // The sweep reads the real clock, not `now`: a caller's `now` is only for
    // its own device's list (a test may pass one a day ahead, which must not
    // sweep the lists of other phones sharing the map).
    prune_created(&mut all, Instant::now());
    if let Some(times) = all.get_mut(device_id) {
        while times
            .front()
            .is_some_and(|made| now.duration_since(*made) >= DAY)
        {
            times.pop_front();
        }
        if times.is_empty() {
            all.remove(device_id);
        }
    }
    if cap == 0 || all.get(device_id).is_some_and(|times| times.len() >= cap) {
        return None;
    }
    all.entry(device_id.to_string()).or_default().push_back(now);
    Some(now)
}

/// Drop a revoked phone's counts (its tasks and its notes list).
pub(crate) fn forget_device(device_id: &str) {
    let mut all = CREATED.lock().unwrap_or_else(PoisonError::into_inner);
    all.remove(device_id);
    all.remove(&super::notes_api::count_key(device_id));
}

#[cfg(test)]
pub(crate) fn release_create_for_test(device_id: &str, slot: Instant) {
    release_create(device_id, slot);
}

#[cfg(test)]
pub(crate) fn created_has(device_id: &str) -> bool {
    CREATED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(device_id)
}

/// Give back a reservation whose write failed or only repeated an earlier one.
fn release_create(device_id: &str, slot: Instant) {
    let mut all = CREATED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(times) = all.get_mut(device_id) {
        if let Some(at) = times.iter().rposition(|made| *made == slot) {
            times.remove(at);
        }
        if times.is_empty() {
            all.remove(device_id);
        }
    }
}

/// A slot taken from the day's count, handed back when dropped (an error, a
/// `duplicate` answer or a panic included) unless the write made the task.
/// The notes route counts its own files the same way (`notes_api`), under a
/// key of its own in `device_id`.
pub(crate) struct Reserved {
    pub(crate) device_id: String,
    pub(crate) slot: Instant,
    pub(crate) keep: bool,
}

impl Drop for Reserved {
    fn drop(&mut self) {
        if !self.keep {
            release_create(&self.device_id, self.slot);
        }
    }
}

/// Whether a key already made its task (a retry, which is no new file).
fn is_repeat(vault: &Path, op_id: &str) -> bool {
    vault_tasks::fingerprint_for_key(&format!("companion:{op_id}"))
        .is_some_and(|fp| vault_tasks::fingerprint_index(vault).contains_key(&fp))
}

/// A time of day and a date belong together: clearing the time of a due that
/// carries one (Notion stores `2026-11-09T14:30` in `due`) also trims the due
/// to its date, and clearing the date also clears a stored time, so neither
/// comes back later in a row that looks cleared. A task imported from Notion
/// is tied like any other since Notion two-way sync was removed (2026-11-10).
pub(crate) fn tie_time_to_date(task: &Task, changes: &mut Map<String, Value>) {
    let cleared = |changes: &Map<String, Value>, name: &str| {
        changes.get(name).and_then(Value::as_str) == Some("")
    };
    if cleared(changes, "dueTime") && !changes.contains_key("due") {
        // With no `due_time` given, a time comes back only from inside `due`.
        if let (Some(date), Some(_)) = split_due(task.due.as_deref(), None) {
            changes.insert("due".into(), json!(date));
        }
    }
    if cleared(changes, "due")
        && !changes.contains_key("dueTime")
        && task
            .due_time
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
    {
        changes.insert("dueTime".into(), json!(""));
    }
}

fn disposition_of(result: &Value) -> String {
    result
        .get("disposition")
        .and_then(Value::as_str)
        .unwrap_or("applied")
        .to_string()
}

/// A fresh id for a write that has no key of its own.
fn op_id_for(device: &Device) -> String {
    format!(
        "{}.{}",
        device.device_id,
        chrono::Utc::now().timestamp_micros()
    )
}

pub(crate) async fn create(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: CreateRequest = parse_body(body)?;
    let key = request
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    if !valid_key(key) {
        return Err(ApiError::bad_request(
            "idempotencyKey must be 8 to 64 letters, digits, dashes, underscores or dots."
                .to_string(),
        ));
    }
    if let Some(title) = &request.title {
        plain_line("title", title)?;
    }
    check_due(request.due.as_deref(), request.due_time.as_deref())?;
    if let Some(recurrence) = &request.recurrence {
        plain_line("recurrence", recurrence)?;
        if recurrence.chars().count() > RECURRENCE_MAX {
            return Err(ApiError::bad_request(format!(
                "recurrence is at most {RECURRENCE_MAX} characters."
            )));
        }
    }
    for label in request.labels.iter().flatten() {
        plain_line("a label", label)?;
    }
    // Scoped to this iPhone, so two phones never share a key.
    let op_id = format!("{}.{key}", device.device_id);
    let mut args = Map::new();
    args.insert("idempotencyKey".into(), json!(op_id));
    for (name, value) in [
        ("title", &request.title),
        ("due", &request.due),
        ("dueTime", &request.due_time),
        ("priority", &request.priority),
        ("description", &request.description),
        ("recurrence", &request.recurrence),
    ] {
        if let Some(value) = value {
            args.insert(name.into(), json!(value));
        }
    }
    if let Some(labels) = &request.labels {
        args.insert("labels".into(), json!(labels));
    }
    let (vault, device, store) = (ctx.vault.clone(), device.clone(), ctx.store.clone());
    let written = routes::blocking(move || {
        ensure_tasks_dir(&vault)?;
        // One of the day's slots is taken before the write, so two requests
        // at the limit can't both pass. A retry of a key that already made its
        // task is no new file, so the day's count does not stop it.
        let mut reserved = match reserve_create(&device.device_id, Instant::now(), CREATES_PER_DAY)
        {
            Some(slot) => Some(Reserved {
                device_id: device.device_id.clone(),
                slot,
                keep: false,
            }),
            None if is_repeat(&vault, &op_id) => None,
            None => {
                return Err(ApiError::bad_request(format!(
                    "This iPhone has made {CREATES_PER_DAY} tasks in the last 24 hours, the most it may. Try again later."
                )))
            }
        };
        let result = write(&store, &vault, &device, &op_id, "create_task", Value::Object(args))?;
        // The slot stays taken only when this write made a task.
        if disposition_of(&result) != "duplicate" {
            if let Some(reserved) = reserved.as_mut() {
                reserved.keep = true;
            }
        }
        Ok(Written {
            task: task_after(&vault, &result)?,
            disposition: disposition_of(&result),
        })
    })
    .await?;
    Ok(json(200, &written))
}

pub(crate) async fn status(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: StatusRequest = parse_body(body)?;
    let reference = reference_of(request.reference.as_deref())?;
    let expected = expected_of(request.expected_hash.as_deref())?;
    // The characters are checked as sent: trimming would hide a line break.
    plain_line("status", request.status.as_deref().unwrap_or(""))?;
    let status = request.status.as_deref().map(str::trim).unwrap_or("");
    if status.is_empty() {
        return Err(ApiError::bad_request(
            "status is required: Done to complete a task, Not started to reopen it.".to_string(),
        ));
    }
    let args = json!({ "ref": reference, "status": status, "expectedHash": expected });
    let (vault, device, op_id, store) = (
        ctx.vault.clone(),
        device.clone(),
        op_id_for(device),
        ctx.store.clone(),
    );
    let written = routes::blocking(move || {
        ensure_tasks_dir(&vault)?;
        let result = write(&store, &vault, &device, &op_id, "set_status", args)?;
        Ok(StatusWritten {
            task: task_after(&vault, &result)?,
            disposition: disposition_of(&result),
            rolled_forward_to: result
                .get("rolledForwardTo")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
    .await?;
    Ok(json(200, &written))
}

pub(crate) async fn update(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: UpdateRequest = parse_body(body)?;
    let reference = reference_of(request.reference.as_deref())?;
    let expected = expected_of(request.expected_hash.as_deref())?;
    let fields = request
        .fields
        .filter(|f| !f.is_empty())
        .ok_or_else(|| ApiError::bad_request("fields must name at least one field.".to_string()))?;
    let mut changes = Map::new();
    for (name, value) in fields {
        if !UPDATE_FIELDS.contains(&name.as_str()) {
            return Err(ApiError::bad_request(format!(
                "{name} can't be changed from the iPhone; the fields are title, due, dueTime and priority."
            )));
        }
        let value = match value {
            Value::String(text) => Value::String(text),
            // Null clears a date, a time or a priority; a title can't be cleared.
            Value::Null if CLEARABLE.contains(&name.as_str()) => Value::String(String::new()),
            _ => {
                return Err(ApiError::bad_request(format!(
                    "{name} must be a string{}.",
                    if CLEARABLE.contains(&name.as_str()) {
                        " or null"
                    } else {
                        ""
                    }
                )))
            }
        };
        changes.insert(name, value);
    }
    if let Some(title) = changes.get("title").and_then(Value::as_str) {
        plain_line("title", title)?;
    }
    check_due(
        changes.get("due").and_then(Value::as_str),
        changes.get("dueTime").and_then(Value::as_str),
    )?;
    let (vault, device, op_id, store) = (
        ctx.vault.clone(),
        device.clone(),
        op_id_for(device),
        ctx.store.clone(),
    );
    let written = routes::blocking(move || {
        ensure_tasks_dir(&vault)?;
        // The task as it is now tells whether a time and a date must go
        // together (a stale hash is refused below, whatever it says).
        let rel = vault_tasks::resolve_task_ref(&vault, &reference)
            .map_err(|reason| refusal(CompanionRefusal::Refused(reason)))?;
        if let Some(task) = vault_tasks::get(&vault, &rel) {
            tie_time_to_date(&task, &mut changes);
        }
        let args = json!({ "ref": reference, "expectedHash": expected, "fields": changes });
        let result = write(&store, &vault, &device, &op_id, "update_task", args)?;
        Ok(Written {
            task: task_after(&vault, &result)?,
            disposition: disposition_of(&result),
        })
    })
    .await?;
    Ok(json(200, &written))
}
