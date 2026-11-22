//! Shared, approval-gated review queue for autonomous suggestions.
//!
//! Learning and commitment detectors may *propose* durable records, but they
//! never write those records directly. This versioned store is deliberately a
//! single atomically-rewritten JSON document instead of JSONL: resolving an
//! item is a state transition, and concurrent detector/UI writes must not lose
//! one another. Every read-modify-write is serialized by [`QUEUE_LOCK`].

use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Hash-watched during automated runs ([`crate::run_guard`]); every write
/// goes through `run_guard::write_host_state`.
pub(crate) const FILE_NAME: &str = ".review-queue.json";
const V1_BACKUP_FILE_NAME: &str = ".review-queue.v1.json";
/// A `rejected` item whose error starts with this was retired by Hollow
/// because its subject changed again — not rejected by the user. Such an item
/// is reopened (not sticky) when the same fingerprint recurs.
pub const SUPERSEDED_PREFIX: &str = "Superseded:";
const SCHEMA_VERSION: u32 = 2;
const KEEP: usize = 1_000;
/// Superseded items kept per subject (kind + `source_path`). Each distinct
/// hand edit of an automation supersedes the previous review, so an Obsidian
/// session autosaving a workflow every few seconds would otherwise add
/// hundreds of rows and push other kinds' sticky accept/reject decisions out
/// of the [`KEEP`] window. A pruned pair that recurs simply gets a new item.
const SUPERSEDED_KEEP_PER_SUBJECT: usize = 5;
const APPLYING_RECOVERY_MINUTES: i64 = 10;

static QUEUE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub fingerprint: String,
    pub payload: Value,
    #[serde(default = "default_risk")]
    pub risk: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub snoozed_until: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewReviewItem {
    pub kind: String,
    pub fingerprint: String,
    pub payload: Value,
    pub risk: String,
    pub source: String,
    pub source_path: Option<String>,
}

/// Result of [`enqueue_exclusive`]: the subject's single live item, and
/// whether this call opened it (created or reopened it as pending).
#[derive(Debug, Clone)]
pub struct ExclusiveOutcome {
    pub item: ReviewItem,
    pub opened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewQueueFile {
    schema_version: u32,
    items: Vec<ReviewItem>,
}

impl Default for ReviewQueueFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

fn path(vault: &Path) -> PathBuf {
    vault.join(FILE_NAME)
}

fn v1_backup_path(vault: &Path) -> PathBuf {
    vault.join(V1_BACKUP_FILE_NAME)
}

fn default_risk() -> String {
    "local_write".into()
}

fn default_source() -> String {
    "chat".into()
}

const MESSAGING_RETIRED: &str =
    "Messaging proposals are retired; Hollow supports local task and Calendar review only.";

/// Legacy messaging records remain on disk as recovery evidence, but never
/// enter the live review surface or any approval transition. Classify by all
/// routing metadata so changing a kind alone cannot reactivate an old send.
pub fn is_retired_messaging_item(item: &ReviewItem) -> bool {
    retired_messaging_metadata(
        &item.kind,
        &item.risk,
        &item.source,
        item.source_path.as_deref(),
    ) || retired_message_payload(&item.payload)
}

fn retired_message_payload(payload: &Value) -> bool {
    ["sourcePath", "sourceVaultPath", "replyToVaultPath"]
        .iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .any(is_retired_message_path)
}

pub fn is_retired_message_path(path: &str) -> bool {
    let path = path.trim().replace('\\', "/").to_ascii_lowercase();
    let path = path
        .strip_prefix("vault:")
        .unwrap_or(&path)
        .trim_start_matches('/');
    matches!(
        path.split('/')
            .find(|part| !part.is_empty() && *part != "."),
        Some("emails" | "teams" | "slack")
    )
}

/// Known retired message links in historical notification/audit text. Ordinary
/// local task prose is not classified merely for mentioning an email.
pub fn has_retired_message_reference(text: &str) -> bool {
    let text = text.replace('\\', "/").to_ascii_lowercase();
    [
        "emails/",
        "teams/",
        "slack/",
        "mailto:",
        "outlook.office.com/mail",
        "outlook.office365.com/mail",
        "teams.microsoft.com/l/chat",
        "slack.com/archives/",
        "drafts/heartbeat/reply-",
        "drafts/heartbeat/reply_",
        "drafts/heartbeat/email-",
        "drafts/heartbeat/email_",
        "drafts/heartbeat/teams-",
        "drafts/heartbeat/slack-",
        "heartbeat_send",
        "reply_draft",
        "meeting_followup",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn retired_messaging_metadata(
    kind: &str,
    risk: &str,
    source: &str,
    source_path: Option<&str>,
) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "heartbeat_send" | "reply_draft" | "meeting_followup"
    ) || risk.trim().eq_ignore_ascii_case("external_send")
        || matches!(
            source.trim().to_ascii_lowercase().as_str(),
            "inbox" | "email" | "teams" | "slack"
        )
        || source_path.is_some_and(is_retired_message_path)
}

fn require_active_item(item: &ReviewItem) -> Result<(), String> {
    if is_retired_messaging_item(item) {
        Err(MESSAGING_RETIRED.into())
    } else {
        Ok(())
    }
}

struct LoadedQueue {
    file: ReviewQueueFile,
    legacy_bytes: Option<Vec<u8>>,
    dirty: bool,
}

fn read_unlocked(vault: &Path) -> Result<LoadedQueue, String> {
    let path = path(vault);
    if !path.exists() {
        return Ok(LoadedQueue {
            file: ReviewQueueFile::default(),
            legacy_bytes: None,
            dirty: false,
        });
    }
    let raw = fs::read(&path).map_err(|e| format!("read review queue: {e}"))?;
    let mut file: ReviewQueueFile =
        serde_json::from_slice(&raw).map_err(|e| format!("parse review queue: {e}"))?;
    if !matches!(file.schema_version, 1 | SCHEMA_VERSION) {
        return Err(format!(
            "unsupported review queue schema {} (expected 1 or {})",
            file.schema_version, SCHEMA_VERSION,
        ));
    }
    let legacy_bytes = (file.schema_version == 1).then_some(raw);
    file.schema_version = SCHEMA_VERSION;

    // A process can die after claiming an item but before applying it. Recover
    // stale claims rather than leaving an invisible permanent limbo state.
    let mut dirty = false;
    let now = Utc::now();
    let cutoff = Utc::now() - Duration::minutes(APPLYING_RECOVERY_MINUTES);
    for item in &mut file.items {
        // Do not wake, retry, or rewrite retired sends, including interrupted
        // claims. Their historical payload and lifecycle remain recoverable.
        if is_retired_messaging_item(item) {
            continue;
        }
        if item.status == "applying" {
            let stale = DateTime::parse_from_rfc3339(&item.updated_at)
                .map(|d| d.with_timezone(&Utc) < cutoff)
                .unwrap_or(true);
            if stale {
                item.status = "pending".into();
                item.error =
                    Some("Recovered an interrupted approval; nothing was assumed applied.".into());
                item.updated_at = now.to_rfc3339();
                dirty = true;
            }
        }

        if item.status == "snoozed" {
            let expired = item
                .snoozed_until
                .as_deref()
                .and_then(|until| DateTime::parse_from_rfc3339(until).ok())
                .map(|until| until.with_timezone(&Utc) <= now)
                .unwrap_or(true);
            if expired {
                let invalid = item
                    .snoozed_until
                    .as_deref()
                    .is_some_and(|until| DateTime::parse_from_rfc3339(until).is_err());
                item.snoozed_until = None;
                item.status = "pending".into();
                item.updated_at = now.to_rfc3339();
                if invalid {
                    item.error = Some("Recovered an invalid snooze timestamp.".into());
                }
                dirty = true;
            }
        }
    }
    Ok(LoadedQueue {
        file,
        legacy_bytes,
        dirty,
    })
}

fn write_unlocked(
    vault: &Path,
    file: &ReviewQueueFile,
    legacy_bytes: Option<&[u8]>,
) -> Result<(), String> {
    fs::create_dir_all(vault).map_err(|e| format!("create vault: {e}"))?;
    if let Some(bytes) = legacy_bytes {
        let backup = v1_backup_path(vault);
        if !backup.exists() {
            crate::vault::atomic_write(&backup, bytes)
                .map_err(|e| format!("back up review queue v1: {e}"))?;
        }
    }
    let body = serde_json::to_vec_pretty(file).map_err(|e| format!("encode review queue: {e}"))?;
    // A host write: attributed so an in-flight automated run is not flagged
    // for Hollow's own queue updates (and a direct agent edit still is).
    crate::run_guard::write_host_state(vault, FILE_NAME, &body)
        .map_err(|e| format!("write review queue: {e}"))
}

fn with_file<T>(
    vault: &Path,
    f: impl FnOnce(&mut ReviewQueueFile) -> Result<(T, bool), String>,
) -> Result<T, String> {
    let _guard = QUEUE_LOCK
        .lock()
        .map_err(|_| "review queue lock poisoned".to_string())?;
    let mut loaded = read_unlocked(vault)?;
    let (value, changed) = f(&mut loaded.file)?;
    if changed || loaded.dirty {
        write_unlocked(vault, &loaded.file, loaded.legacy_bytes.as_deref())?;
    }
    Ok(value)
}

/// Return newest-first queue items, optionally restricted by proposal kind.
pub fn list(vault: &Path, kind: Option<&str>) -> Result<Vec<ReviewItem>, String> {
    with_file(vault, |file| {
        let mut items: Vec<_> = file
            .items
            .iter()
            .filter(|item| !is_retired_messaging_item(item))
            .filter(|item| kind.map(|k| item.kind == k).unwrap_or(true))
            .cloned()
            .collect();
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok((items, false))
    })
}

/// Validate and canonicalize a proposal: the retired-messaging check, the
/// kind/risk/source allowlists, the source-path trim and the fingerprint
/// lowercase. Shared by [`enqueue`] and [`enqueue_exclusive`].
fn normalize_new(input: NewReviewItem) -> Result<NewReviewItem, String> {
    if retired_messaging_metadata(
        &input.kind,
        &input.risk,
        &input.source,
        input.source_path.as_deref(),
    ) || retired_message_payload(&input.payload)
    {
        return Err(MESSAGING_RETIRED.into());
    }
    let kind = input.kind.trim().to_lowercase();
    if !matches!(
        kind.as_str(),
        "learning" | "skill-learning"
            | "commitment"
            | "project_activation"
            | "goal"
            | "heartbeat_task"
            | "meeting_task"
            | "workflow_permissions"
            | "workflow_failure_streak"
            | "budget_pause"
            // Paperclip PR 12: hollow_tasks propose_tasks (source chat or
            // workflow), accepted through `task_broker::accept_agent_task`.
            | "agent_task"
            // Master plan #19: the gardener's one live card, applied through
            // `gardener::resolve_review`.
            | "gardener"
            // Master plan F3: an extracted decision or person fact, accepted
            // only by the person through F4's records writer.
            | "record"
            // Master plan #3: "Is 'Ana' Ana Ruiz?", answered only by the
            // person (`people_dossier::resolve`).
            | "person_alias"
            // A companion device and the Mac both changed one task field
            // (`companion_inbox`): only the person picks which value stays.
            | "companion_conflict"
            // A companion op sent more than 14 days ago: only the person
            // applies or discards it (`companion_inbox::accept_old_op`).
            | "companion_old_op"
            // ME2: a stale memory or home file, offered once to recheck
            // (`context_stale`); Rechecked or Dismiss only closes it.
            | "memory_recheck"
            // NV7: a chat that missed a skill, filed as an eval case; only a
            // reviewer-written expectation accepts it (`eval_candidates`).
            | "eval_candidate"
    ) {
        return Err(format!("unsupported review item kind: {}", input.kind));
    }
    let risk = input.risk.trim().to_lowercase();
    if risk != "local_write" {
        return Err(format!("unsupported review item risk: {}", input.risk));
    }
    let source = input.source.trim().to_lowercase();
    if !matches!(
        source.as_str(),
        "activation"
            | "heartbeat"
            | "meeting"
            | "chat"
            | "agent"
            | "workflow"
            | "budget"
            | "gardener"
            | "companion"
            | "host-capture"
            | "private-memory"
            // Memory maintenance (J5): a proposal to take out or reword a
            // saved line that passed its date or went quiet.
            | "memory"
    ) {
        return Err(format!("unsupported review item source: {}", input.source));
    }
    // Budget pauses (Paperclip PR 8a) come only from the budgets layer, and
    // the budget source carries nothing else.
    if (kind == "budget_pause") != (source == "budget") {
        return Err(format!(
            "review item kind {kind} can't come from source {source}"
        ));
    }
    if (kind == "gardener") != (source == "gardener") {
        return Err(format!(
            "review item kind {kind} can't come from source {source}"
        ));
    }
    if kind == crate::eval_candidates::KIND && source != "chat" {
        return Err(format!(
            "review item kind {kind} can't come from source {source}"
        ));
    }
    let companion_kind = matches!(kind.as_str(), "companion_conflict" | "companion_old_op");
    if companion_kind != (source == "companion") {
        return Err(format!(
            "review item kind {kind} can't come from source {source}"
        ));
    }
    let source_path = input
        .source_path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let fingerprint = input.fingerprint.trim().to_lowercase();
    if fingerprint.is_empty() {
        return Err("review item fingerprint is required".into());
    }
    Ok(NewReviewItem {
        kind,
        fingerprint,
        payload: input.payload,
        risk,
        source,
        source_path,
    })
}

fn new_pending(input: NewReviewItem) -> ReviewItem {
    let now = Utc::now().to_rfc3339();
    ReviewItem {
        id: make_id(&input.kind, &input.fingerprint, &now),
        kind: input.kind,
        status: "pending".into(),
        created_at: now.clone(),
        updated_at: now,
        fingerprint: input.fingerprint,
        payload: input.payload,
        risk: input.risk,
        source: input.source,
        source_path: input.source_path,
        snoozed_until: None,
        error: None,
    }
}

/// Append an item, then prune. Preserve every unresolved item: prune the
/// oldest terminal items first; if more than KEEP are still pending, keep them
/// all rather than silently dropping an approval request.
fn push_pruned(file: &mut ReviewQueueFile, item: ReviewItem) {
    file.items.push(item);
    if file.items.len() > KEEP {
        file.items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let mut excess = file.items.len().saturating_sub(KEEP);
        file.items.retain(|candidate| {
            if excess > 0
                && !is_retired_messaging_item(candidate)
                && matches!(candidate.status.as_str(), "accepted" | "rejected")
            {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// Enqueue a suggestion unless its fingerprint has ever been seen. Terminal
/// records intentionally participate in deduplication: rejecting or resolving
/// a suggestion is a durable decision, not permission to nag again.
pub fn enqueue(vault: &Path, input: NewReviewItem) -> Result<ReviewItem, String> {
    let input = normalize_new(input)?;
    with_file(vault, move |file| {
        if let Some(existing) = file
            .items
            .iter()
            .find(|item| item.kind == input.kind && item.fingerprint == input.fingerprint)
            .cloned()
        {
            require_active_item(&existing)?;
            return Ok((existing, false));
        }
        let item = new_pending(input);
        push_pruned(file, item.clone());
        Ok((item, true))
    })
}

fn is_live(item: &ReviewItem) -> bool {
    matches!(item.status.as_str(), "pending" | "snoozed")
}

pub(crate) fn is_superseded(item: &ReviewItem) -> bool {
    item.status == "rejected"
        && item
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with(SUPERSEDED_PREFIX))
}

fn retire(item: &mut ReviewItem, reason: &str, now: &str) {
    item.status = "rejected".into();
    item.snoozed_until = None;
    item.error = Some(truncate_error(reason));
    item.updated_at = now.to_string();
}

/// A supersede reason must start with [`SUPERSEDED_PREFIX`]: that prefix is
/// what tells Hollow's retirement apart from the user's own rejection.
fn require_supersede_reason(reason: &str) -> Result<(), String> {
    if reason.starts_with(SUPERSEDED_PREFIX) {
        Ok(())
    } else {
        Err(format!(
            "a supersede reason must start with {SUPERSEDED_PREFIX:?}"
        ))
    }
}

/// Supersede every live (pending/snoozed) item of `kind` that `pred`
/// accepts, inside an open [`with_file`] transaction. Retired messaging
/// records are never touched. Returns how many were retired. The shared core
/// of [`retire_live`] and [`close_subject`] (cross-PR helper `retire_where`).
fn retire_where_locked(
    file: &mut ReviewQueueFile,
    kind: &str,
    pred: impl Fn(&ReviewItem) -> bool,
    reason: &str,
    now: &str,
) -> usize {
    let mut retired = 0;
    for item in file.items.iter_mut() {
        if item.kind == kind && is_live(item) && !is_retired_messaging_item(item) && pred(&*item) {
            retire(item, reason, now);
            retired += 1;
        }
    }
    retired
}

/// Payload key [`close_subject`] stamps on a subject's resolved items. An
/// accepted or user-rejected item without it still speaks for its subject
/// (Dismiss and Pause are sticky); once stamped, the subject is closed and the
/// next occurrence opens a new item.
pub(crate) const CLOSED_AT_KEY: &str = "closedAt";

/// Whether [`close_subject`] has closed this item.
pub(crate) fn has_closed_at(item: &ReviewItem) -> bool {
    item.payload
        .get(CLOSED_AT_KEY)
        .is_some_and(|value| !value.is_null())
}

/// Stamp `closedAt` into the payload. A non-object payload (never written by
/// the subject-scoped producers, but possible on disk) is wrapped rather than
/// left unclosable, so one odd record can never block its subject forever.
/// `updated_at` is deliberately left alone: on an `applying` item it drives
/// the stale-claim recovery window.
fn stamp_closed(item: &mut ReviewItem, now: &str) {
    if let Some(map) = item.payload.as_object_mut() {
        map.insert(CLOSED_AT_KEY.to_string(), Value::String(now.to_string()));
        return;
    }
    let previous = std::mem::take(&mut item.payload);
    let mut map = serde_json::Map::new();
    map.insert("previousPayload".to_string(), previous);
    map.insert(CLOSED_AT_KEY.to_string(), Value::String(now.to_string()));
    item.payload = Value::Object(map);
}

/// Drop all but the newest [`SUPERSEDED_KEEP_PER_SUBJECT`] superseded items
/// of one subject. Never touches live items, accepted items or the user's own
/// rejections. Returns whether anything was removed.
fn prune_superseded(file: &mut ReviewQueueFile, kind: &str, source_path: Option<&str>) -> bool {
    let mut superseded: Vec<(String, String)> = file
        .items
        .iter()
        .filter(|item| {
            item.kind == kind && item.source_path.as_deref() == source_path && is_superseded(item)
        })
        .map(|item| (item.updated_at.clone(), item.id.clone()))
        .collect();
    if superseded.len() <= SUPERSEDED_KEEP_PER_SUBJECT {
        return false;
    }
    // Oldest first; ties broken by id so the choice is deterministic.
    superseded.sort();
    let excess = superseded.len() - SUPERSEDED_KEEP_PER_SUBJECT;
    let doomed: std::collections::HashSet<String> = superseded
        .into_iter()
        .take(excess)
        .map(|(_, id)| id)
        .collect();
    file.items.retain(|item| {
        !(item.kind == kind
            && item.source_path.as_deref() == source_path
            && is_superseded(item)
            && doomed.contains(&item.id))
    });
    true
}

/// Keep exactly ONE live item per subject (`kind` + `source_path`), for
/// reviews whose subject can change again while the review is open (workflow
/// permission changes). In one queue transaction:
/// 1. every OTHER pending/snoozed item of the same subject is superseded
///    (`rejected`, error = `reason`, which must start with
///    [`SUPERSEDED_PREFIX`]);
/// 2. an item with the same fingerprint that is pending/snoozed/applying is
///    returned (its payload refreshed unless it is mid-application);
///    an accepted or superseded one is reopened as pending with the refreshed
///    payload; one the USER rejected is returned unchanged (sticky);
/// 3. otherwise a new pending item is created.
///
/// The queue is written only when something changed.
pub fn enqueue_exclusive(
    vault: &Path,
    input: NewReviewItem,
    reason: &str,
) -> Result<ExclusiveOutcome, String> {
    if !reason.starts_with(SUPERSEDED_PREFIX) {
        return Err(format!(
            "a supersede reason must start with {SUPERSEDED_PREFIX:?}"
        ));
    }
    let input = normalize_new(input)?;
    with_file(vault, move |file| {
        let now = Utc::now().to_rfc3339();
        let mut changed = false;
        for item in file.items.iter_mut() {
            if item.kind == input.kind
                && item.source_path == input.source_path
                && item.fingerprint != input.fingerprint
                && is_live(item)
                && !is_retired_messaging_item(item)
            {
                retire(item, reason, &now);
                changed = true;
            }
        }
        let kind = input.kind.clone();
        let source_path = input.source_path.clone();
        let position = file
            .items
            .iter()
            .position(|item| item.kind == input.kind && item.fingerprint == input.fingerprint);
        let outcome = if let Some(index) = position {
            let existing = &mut file.items[index];
            require_active_item(existing)?;
            let opened = match existing.status.as_str() {
                "pending" | "snoozed" => {
                    if existing.payload != input.payload {
                        existing.payload = input.payload.clone();
                        existing.updated_at = now.clone();
                        changed = true;
                    }
                    false
                }
                // Mid-application: leave it exactly as claimed.
                "applying" => false,
                "accepted" => true,
                _ if is_superseded(existing) => true,
                // Rejected by the user: a durable decision for this exact pair.
                _ => false,
            };
            if opened {
                existing.status = "pending".into();
                existing.error = None;
                existing.snoozed_until = None;
                existing.payload = input.payload.clone();
                existing.updated_at = now;
                changed = true;
            }
            ExclusiveOutcome {
                item: existing.clone(),
                opened,
            }
        } else {
            let item = new_pending(input);
            push_pruned(file, item.clone());
            changed = true;
            ExclusiveOutcome { item, opened: true }
        };
        if prune_superseded(file, &kind, source_path.as_deref()) {
            changed = true;
        }
        Ok((outcome, changed))
    })
}

/// Supersede every live (pending/snoozed) item of `kind` for `source_path`.
/// Returns how many were retired; the queue is written only when any were.
pub fn retire_live(
    vault: &Path,
    kind: &str,
    source_path: &str,
    reason: &str,
) -> Result<usize, String> {
    require_supersede_reason(reason)?;
    with_file(vault, |file| {
        let now = Utc::now().to_rfc3339();
        let retired = retire_where_locked(
            file,
            kind,
            |item| item.source_path.as_deref() == Some(source_path),
            reason,
            &now,
        );
        let pruned = retired > 0 && prune_superseded(file, kind, Some(source_path));
        Ok((retired, retired > 0 || pruned))
    })
}

/// Enqueue a subject-scoped item (`kind` + `source_path`) unless the subject
/// already has an OPEN item, as `is_open` decides (Paperclip PR 9's failure
/// streak: "open" includes accepted and user-dismissed items until
/// [`close_subject`] stamps them). In one queue transaction:
/// 1. an open item of the same subject is returned, `false`;
/// 2. otherwise an item with the same fingerprint (in any state) is returned,
///    `false`: like [`enqueue`], a fingerprint is never reused;
/// 3. otherwise a new pending item is created, `true`.
///
/// Unlike [`enqueue_exclusive`] it never reopens an accepted item, and unlike
/// [`enqueue`] its dedupe is per subject, so it does not depend on which rows
/// a producer can still see (a workflow's run log drains at 50). The subject
/// needs a `source_path`. Written through [`with_file`], so an automated run
/// in flight never has this host write flagged.
pub fn enqueue_unless_open(
    vault: &Path,
    input: NewReviewItem,
    is_open: impl Fn(&ReviewItem) -> bool,
) -> Result<(ReviewItem, bool), String> {
    let input = normalize_new(input)?;
    if input.source_path.is_none() {
        return Err("a subject-scoped review item needs a source path".into());
    }
    with_file(vault, move |file| {
        if let Some(open) = file.items.iter().find(|item| {
            item.kind == input.kind
                && item.source_path == input.source_path
                && !is_retired_messaging_item(item)
                && is_open(item)
        }) {
            // (value, changed): the value is (item, created).
            return Ok(((open.clone(), false), false));
        }
        if let Some(existing) = file
            .items
            .iter()
            .find(|item| item.kind == input.kind && item.fingerprint == input.fingerprint)
            .cloned()
        {
            require_active_item(&existing)?;
            return Ok(((existing, false), false));
        }
        let item = new_pending(input);
        push_pruned(file, item.clone());
        Ok(((item, true), true))
    })
}

/// Close a subject (`kind` + `source_path`) because the condition it reviewed
/// is over (Paperclip PR 9: the automation succeeded again, or was deleted or
/// renamed). In one queue transaction:
/// - live (pending/snoozed) items are superseded with `reason`, which must
///   start with [`SUPERSEDED_PREFIX`];
/// - accepted, user-rejected and `applying` items without a `closedAt` get
///   `payload.closedAt = now` ([`CLOSED_AT_KEY`]). An applying item keeps the
///   stamp through its `accept`/`fail`, which preserve the payload, so an
///   approval landing mid-close still leaves the subject closed.
///
/// Returns how many items changed; the queue is written only when any did
/// (or superseded rows were pruned). Written through [`with_file`].
pub fn close_subject(
    vault: &Path,
    kind: &str,
    source_path: &str,
    reason: &str,
) -> Result<usize, String> {
    require_supersede_reason(reason)?;
    let kind = kind.trim().to_lowercase();
    let source_path = source_path.trim().to_string();
    with_file(vault, |file| {
        let now = Utc::now().to_rfc3339();
        let in_subject = |item: &ReviewItem| {
            item.kind == kind && item.source_path.as_deref() == Some(source_path.as_str())
        };
        let retired = retire_where_locked(file, &kind, in_subject, reason, &now);
        let mut stamped = 0;
        for item in file.items.iter_mut() {
            if in_subject(&*item)
                && matches!(item.status.as_str(), "applying" | "accepted" | "rejected")
                && !is_superseded(item)
                && !is_retired_messaging_item(item)
                && !has_closed_at(item)
            {
                stamp_closed(item, &now);
                stamped += 1;
            }
        }
        let pruned = retired > 0 && prune_superseded(file, &kind, Some(source_path.as_str()));
        let changed = retired + stamped;
        Ok((changed, changed > 0 || pruned))
    })
}

/// Supersede ONE live (pending/snoozed) item by id (PR 8's budget pauses: a
/// pause that lapsed at its window reset, or was cleared from Settings).
/// `reason` must start with [`SUPERSEDED_PREFIX`]. A missing id or an item
/// that is no longer live is a no-op (`Ok(None)`); so is more than one item
/// with the id (the queue does not enforce unique ids, so an ambiguous id is
/// never acted on). Superseded items of the same subject (kind +
/// `source_path`) are pruned to the newest five: every budget pause has no
/// `source_path`, so at most five superseded budget items are kept across all
/// budgets, which is fine because they are history and per-episode
/// fingerprints never need them for deduplication.
pub fn supersede(vault: &Path, id: &str, reason: &str) -> Result<Option<ReviewItem>, String> {
    require_supersede_reason(reason)?;
    with_file(vault, |file| {
        if file.items.iter().filter(|item| item.id == id).count() != 1 {
            return Ok((None, false));
        }
        let Some(item) = file.items.iter_mut().find(|item| item.id == id) else {
            return Ok((None, false));
        };
        if !is_live(item) || is_retired_messaging_item(item) {
            return Ok((None, false));
        }
        let now = Utc::now().to_rfc3339();
        retire(item, reason, &now);
        let retired = item.clone();
        prune_superseded(file, &retired.kind, retired.source_path.as_deref());
        Ok((Some(retired), true))
    })
}

/// What a card shows when the item it was read from changed before the
/// person's decision reached Hollow (ME1): nothing is written.
pub const CHANGED_SINCE_READ: &str =
    "This changed since you opened it (another window or a capture changed it). Nothing was saved; review it again.";

/// An item's version as a reader saw it (ME1): a short SHA-256 over its
/// status, last change, snooze and payload. Computed, never stored, so the
/// queue file needs no migration; any change a card could miss (new words,
/// a failed Save putting its wording back, a settle and an Undo) moves it.
/// A card sends it back with the person's edit and [`claim_at`] refuses the
/// edit when the item moved on.
pub fn version_of(item: &ReviewItem) -> String {
    let mut h = Sha256::new();
    for part in [
        item.id.as_str(),
        item.status.as_str(),
        item.updated_at.as_str(),
        item.snoozed_until.as_deref().unwrap_or(""),
    ] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    // serde_json's map is ordered (no `preserve_order`), so the same payload
    // always encodes the same way.
    h.update(serde_json::to_vec(&item.payload).unwrap_or_default());
    let digest = format!("{:x}", h.finalize());
    format!("v-{}", &digest[..16])
}

/// A review item with the [`version_of`] its reader saw, for the cards
/// (`list_review_inbox`). The queue file never stores it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedItem {
    #[serde(flatten)]
    pub item: ReviewItem,
    pub version: String,
}

impl From<ReviewItem> for VersionedItem {
    fn from(item: ReviewItem) -> Self {
        let version = version_of(&item);
        VersionedItem { item, version }
    }
}

/// Claim a pending item before applying its durable write. An optional edited
/// payload is persisted with the claim, making the exact user-approved value
/// recoverable if the subsequent write fails.
pub fn claim(vault: &Path, id: &str, payload: Option<Value>) -> Result<ReviewItem, String> {
    claim_at(vault, id, None, payload)
}

/// [`claim`], as a compare-and-swap when `expected_version` is given: the
/// item must still be the one that version names ([`version_of`]), checked
/// under the queue lock, else [`CHANGED_SINCE_READ`] and nothing changes.
pub fn claim_at(
    vault: &Path,
    id: &str,
    expected_version: Option<&str>,
    payload: Option<Value>,
) -> Result<ReviewItem, String> {
    with_file(vault, |file| {
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if expected_version.is_some_and(|v| v.trim() != version_of(item)) {
            return Err(CHANGED_SINCE_READ.into());
        }
        if item.status != "pending" {
            return Err(format!("review item {id} is {}", item.status));
        }
        if let Some(payload) = payload {
            if retired_message_payload(&payload) {
                return Err(MESSAGING_RETIRED.into());
            }
            item.payload = payload;
        }
        item.status = "applying".into();
        item.updated_at = Utc::now().to_rfc3339();
        item.error = None;
        Ok((item.clone(), true))
    })
}

pub fn accept(vault: &Path, id: &str) -> Result<ReviewItem, String> {
    finish(vault, id, "accepted", None, None)
}

pub fn reject(vault: &Path, id: &str) -> Result<ReviewItem, String> {
    reject_with(vault, id, None)
}

/// [`reject`], replacing the payload in the same write when one is given
/// (bulk learning review stamps its batch and the optional reason).
pub fn reject_with(vault: &Path, id: &str, payload: Option<Value>) -> Result<ReviewItem, String> {
    with_file(vault, |file| {
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if !matches!(item.status.as_str(), "pending" | "snoozed") {
            return Err(format!("review item {id} is {}", item.status));
        }
        if let Some(payload) = payload {
            if retired_message_payload(&payload) {
                return Err(MESSAGING_RETIRED.into());
            }
            item.payload = payload;
        }
        item.status = "rejected".into();
        item.updated_at = Utc::now().to_rfc3339();
        item.snoozed_until = None;
        item.error = None;
        Ok((item.clone(), true))
    })
}

/// Return a resolved item to pending with `payload` (bulk learning review's
/// Undo). A compare-and-swap: the item must still be exactly `expected` (its
/// status and payload as the caller read them), so a second Undo, or one
/// racing another change, fails instead of reopening twice. Only an
/// accepted item or the user's own rejection reopens; a superseded one is
/// Hollow's retirement, not a decision to take back.
pub fn reopen(vault: &Path, expected: &ReviewItem, payload: Value) -> Result<ReviewItem, String> {
    let id = expected.id.as_str();
    with_file(vault, |file| {
        if file.items.iter().filter(|item| item.id == id).count() != 1 {
            return Err(format!("review item not found or ambiguous: {id}"));
        }
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if !matches!(item.status.as_str(), "accepted" | "rejected") || is_superseded(item) {
            return Err(format!(
                "review item {id} is {}, not a decision to undo",
                item.status
            ));
        }
        if item.status != expected.status || item.payload != expected.payload {
            return Err(format!("review item {id} changed since it was read"));
        }
        if retired_message_payload(&payload) {
            return Err(MESSAGING_RETIRED.into());
        }
        item.payload = payload;
        item.status = "pending".into();
        item.updated_at = Utc::now().to_rfc3339();
        item.snoozed_until = None;
        item.error = None;
        Ok((item.clone(), true))
    })
}

/// Kinds whose Reject does more than set the status (a budget pause's Keep
/// paused holds the pause; an old companion op is answered `refused` to the
/// device), so a single Reject of one has no Undo.
const REJECT_ACTS: [&str; 2] = [
    crate::budgets::REVIEW_KIND,
    crate::companion_inbox::OLD_OP_KIND,
];

/// Kinds with one live card per subject (`source_path`): a newer card for
/// the subject replaced the rejected one. Other kinds share a `source_path`
/// between siblings (a meeting's extracted proposals), which never block.
const SUBJECT_KINDS: [&str; 3] = [
    crate::workflow_permissions::REVIEW_KIND,
    crate::workflow_hygiene::STREAK_KIND,
    crate::gardener::REVIEW_KIND,
];

/// Undo of one Reject (P0-10, the snackbar's Undo): the same item back to
/// pending with its payload, through [`reopen`]'s compare-and-swap, so a
/// second Undo fails. Only a rejection of a kind whose Reject only set the
/// status reopens.
pub fn undo_reject(
    vault: &Path,
    id: &str,
    expected_updated_at: &str,
) -> Result<ReviewItem, String> {
    let items = list(vault, None)?;
    let item = items
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .ok_or_else(|| format!("review item not found: {id}"))?;
    if item.status != "rejected" {
        return Err(format!(
            "review item {id} is {}, not a rejection to undo",
            item.status
        ));
    }
    if REJECT_ACTS.contains(&item.kind.as_str()) {
        return Err(format!("a {} rejection can't be undone here", item.kind));
    }
    // Only the rejection this Undo's snackbar saw: a later one (reopened
    // and rejected again meanwhile) is a decision it never offered to undo.
    if item.updated_at != expected_updated_at {
        return Err(format!("review item {id} changed since it was rejected"));
    }
    // A subject closed meanwhile (an automation that succeeded again) or
    // with a newer live card stays as it is: never two live cards for one
    // subject, nor one for a subject that no longer needs it.
    if has_closed_at(&item) {
        return Err(format!("review item {id}'s subject was closed since"));
    }
    if SUBJECT_KINDS.contains(&item.kind.as_str())
        && item.source_path.is_some()
        && items.iter().any(|other| {
            other.id != item.id
                && other.kind == item.kind
                && other.source_path == item.source_path
                && matches!(other.status.as_str(), "pending" | "snoozed" | "applying")
        })
    {
        return Err(format!(
            "a newer card for the same {} is already waiting",
            item.kind
        ));
    }
    let payload = item.payload.clone();
    reopen(vault, &item, payload)
}

/// Take back a [`reopen`]: the item returns to `original` (its status and
/// payload before the reopen) if it is still exactly `reopened`. Bulk
/// learning review's Undo uses it when the note it then removes can't be
/// removed, so the item never sits pending beside its still-recalled note.
pub fn restore(
    vault: &Path,
    reopened: &ReviewItem,
    original: &ReviewItem,
) -> Result<ReviewItem, String> {
    let id = reopened.id.as_str();
    with_file(vault, |file| {
        if file.items.iter().filter(|item| item.id == id).count() != 1 {
            return Err(format!("review item not found or ambiguous: {id}"));
        }
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if item.status != reopened.status || item.payload != reopened.payload {
            return Err(format!("review item {id} changed since it was reopened"));
        }
        item.status = original.status.clone();
        item.payload = original.payload.clone();
        item.updated_at = Utc::now().to_rfc3339();
        item.error = None;
        Ok((item.clone(), true))
    })
}

/// Hide a pending item until a future instant without resolving it.
pub fn snooze(vault: &Path, id: &str, until: &str) -> Result<ReviewItem, String> {
    let until = DateTime::parse_from_rfc3339(until)
        .map_err(|_| "snooze deadline must be an RFC 3339 timestamp".to_string())?
        .with_timezone(&Utc);
    if until <= Utc::now() {
        return Err("snooze deadline must be in the future".into());
    }
    with_file(vault, |file| {
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if item.status != "pending" {
            return Err(format!("review item {id} is {}", item.status));
        }
        item.status = "snoozed".into();
        item.snoozed_until = Some(until.to_rfc3339());
        item.updated_at = Utc::now().to_rfc3339();
        item.error = None;
        Ok((item.clone(), true))
    })
}

/// Wake now (P1-13): a snoozed item back to pending before its time, for the
/// Snoozed list's Wake now and a snooze's Undo. Anything else is refused.
pub fn wake(vault: &Path, id: &str) -> Result<ReviewItem, String> {
    with_file(vault, |file| {
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        if item.status != "snoozed" {
            return Err(format!("review item {id} is {}, not snoozed", item.status));
        }
        item.status = "pending".into();
        item.snoozed_until = None;
        item.updated_at = Utc::now().to_rfc3339();
        Ok((item.clone(), true))
    })
}

/// Return a failed application to pending with the error visible. The durable
/// target write is responsible for atomicity; callers invoke this on *any*
/// error after [`claim`].
pub fn fail(vault: &Path, id: &str, error: &str) -> Result<ReviewItem, String> {
    finish(vault, id, "pending", Some(error), None)
}

/// Close a claimed item as rejected with `reason` visible: its target refused
/// it for good (a companion old op the task broker refused), so returning it
/// to pending would only offer a retry that can't work.
pub fn refuse(vault: &Path, id: &str, reason: &str) -> Result<ReviewItem, String> {
    finish(vault, id, "rejected", Some(reason), None)
}

/// [`fail`], replacing the payload in the same write (a failed learning
/// accept drops the note path and batch it stamped with the claim, keeping
/// the edited wording).
pub fn fail_with(
    vault: &Path,
    id: &str,
    error: &str,
    payload: Value,
) -> Result<ReviewItem, String> {
    finish(vault, id, "pending", Some(error), Some(payload))
}

fn finish(
    vault: &Path,
    id: &str,
    status: &str,
    error: Option<&str>,
    payload: Option<Value>,
) -> Result<ReviewItem, String> {
    with_file(vault, |file| {
        let item = file
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("review item not found: {id}"))?;
        require_active_item(item)?;
        if item.status != "applying" {
            return Err(format!(
                "review item {id} is {}, expected applying",
                item.status
            ));
        }
        if let Some(payload) = payload {
            if retired_message_payload(&payload) {
                return Err(MESSAGING_RETIRED.into());
            }
            item.payload = payload;
        }
        item.status = status.into();
        item.updated_at = Utc::now().to_rfc3339();
        item.error = error.map(truncate_error);
        Ok((item.clone(), true))
    })
}

fn truncate_error(error: &str) -> String {
    error.chars().take(500).collect()
}

fn make_id(kind: &str, fingerprint: &str, now: &str) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(fingerprint.as_bytes());
    h.update([0]);
    h.update(now.as_bytes());
    let digest = format!("{:x}", h.finalize());
    format!("review-{}", &digest[..20])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn proposal(fp: &str) -> NewReviewItem {
        NewReviewItem {
            kind: "learning".into(),
            fingerprint: fp.into(),
            payload: json!({ "statement": "Use Outlook" }),
            risk: "local_write".into(),
            source: "chat".into(),
            source_path: None,
        }
    }

    #[test]
    fn round_trip_and_sticky_dedup() {
        let dir = tempdir().unwrap();
        let first = enqueue(dir.path(), proposal("abc")).unwrap();
        let duplicate = enqueue(dir.path(), proposal("ABC")).unwrap();
        assert_eq!(first.id, duplicate.id);
        let claimed = claim(dir.path(), &first.id, None).unwrap();
        assert_eq!(claimed.status, "applying");
        let accepted = accept(dir.path(), &first.id).unwrap();
        assert_eq!(accepted.status, "accepted");
        assert_eq!(enqueue(dir.path(), proposal("abc")).unwrap().id, first.id);
        assert_eq!(list(dir.path(), Some("learning")).unwrap().len(), 1);
    }

    /// ME1: a claim made at a version is refused once the item moved on
    /// (here a failed Save put other words back), and nothing changes.
    #[test]
    fn claim_at_refuses_a_version_the_item_moved_past() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("versioned")).unwrap();
        let read = version_of(&item);
        assert_eq!(read, version_of(&list(dir.path(), None).unwrap()[0]));
        claim(dir.path(), &item.id, None).unwrap();
        let mut other = item.payload.clone();
        other["statement"] = Value::String("Other words.".into());
        fail_with(dir.path(), &item.id, "write failed", other).unwrap();
        let before = list(dir.path(), None).unwrap();
        let refused = claim_at(dir.path(), &item.id, Some(&read), None).unwrap_err();
        assert_eq!(refused, CHANGED_SINCE_READ);
        assert_eq!(list(dir.path(), None).unwrap(), before, "nothing changed");
        let now = version_of(&before[0]);
        assert_ne!(now, read);
        let claimed = claim_at(dir.path(), &item.id, Some(&now), None).unwrap();
        assert_eq!(claimed.status, "applying");
    }

    #[test]
    fn reject_is_terminal_without_claim() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("reject-me")).unwrap();
        let rejected = reject(dir.path(), &item.id).unwrap();
        assert_eq!(rejected.status, "rejected");
        assert!(claim(dir.path(), &item.id, None).is_err());
    }

    /// P1-13: Wake now (and a snooze's Undo) puts a snoozed item back
    /// pending at once; anything not snoozed is refused.
    #[test]
    fn wake_returns_a_snoozed_item_to_pending_once() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("wake-me")).unwrap();
        assert!(wake(dir.path(), &item.id).is_err(), "a pending item");
        let until = (Utc::now() + Duration::hours(10)).to_rfc3339();
        snooze(dir.path(), &item.id, &until).unwrap();
        let back = wake(dir.path(), &item.id).unwrap();
        assert_eq!(back.status, "pending");
        assert_eq!(back.snoozed_until, None);
        assert_eq!(back.payload, item.payload);
        assert!(wake(dir.path(), &item.id).is_err(), "a second wake");
        assert!(wake(dir.path(), "missing").is_err());
    }

    /// P0-10: one Reject has an Undo, which puts the same item back pending,
    /// once.
    #[test]
    fn undo_reject_reopens_the_same_item_once() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("undo-me")).unwrap();
        let rejected = reject(dir.path(), &item.id).unwrap();
        let back = undo_reject(dir.path(), &item.id, &rejected.updated_at).unwrap();
        assert_eq!(back.id, item.id);
        assert_eq!(back.status, "pending");
        assert_eq!(back.payload, item.payload);
        assert!(
            undo_reject(dir.path(), &item.id, &rejected.updated_at).is_err(),
            "a second Undo"
        );
    }

    /// An old snackbar's Undo never takes back a later rejection of the same
    /// item (review finding: the swap compared the item with itself).
    #[test]
    fn undo_reject_refuses_a_rejection_it_did_not_see() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("stale-undo")).unwrap();
        reject(dir.path(), &item.id).unwrap();
        let refused = undo_reject(dir.path(), &item.id, "2000-01-01T00:00:00Z").unwrap_err();
        assert!(refused.contains("changed"), "{refused}");
    }

    /// Undo never makes a second live card for one subject, nor reopens one
    /// whose subject was closed meanwhile (review finding).
    #[test]
    fn undo_reject_refuses_a_closed_or_replaced_subject() {
        let dir = tempdir().unwrap();
        let first = enqueue(dir.path(), permission_review("p10-a", "first")).unwrap();
        let rejected = reject(dir.path(), &first.id).unwrap();
        enqueue(dir.path(), permission_review("p10-b", "second")).unwrap();
        let refused = undo_reject(dir.path(), &first.id, &rejected.updated_at).unwrap_err();
        assert!(refused.contains("newer"), "{refused}");

        // Sibling cards from one source (a meeting's extracted proposals)
        // aren't one subject: rejecting one and undoing it works (delta
        // review finding).
        let mut one = proposal("meeting-1");
        one.source_path = Some("meetings/standup.md".into());
        let one = enqueue(dir.path(), one).unwrap();
        let mut two = proposal("meeting-2");
        two.source_path = Some("meetings/standup.md".into());
        enqueue(dir.path(), two).unwrap();
        let rejected = reject(dir.path(), &one.id).unwrap();
        assert_eq!(
            undo_reject(dir.path(), &one.id, &rejected.updated_at)
                .unwrap()
                .status,
            "pending"
        );

        let mut closed = proposal("closed");
        closed.payload = json!({ "statement": "Use Outlook", "closedAt": "2026-10-23T06:00:00Z" });
        let closed = enqueue(dir.path(), closed).unwrap();
        let rejected = reject(dir.path(), &closed.id).unwrap();
        let refused = undo_reject(dir.path(), &closed.id, &rejected.updated_at).unwrap_err();
        assert!(refused.contains("closed"), "{refused}");
    }

    /// Only a rejection reopens, and only of a kind whose Reject did nothing
    /// but set the status (a budget pause's Keep paused holds the pause).
    #[test]
    fn undo_reject_refuses_a_pending_item_and_a_budget_pause() {
        let dir = tempdir().unwrap();
        let pending = enqueue(dir.path(), proposal("still-pending")).unwrap();
        assert!(undo_reject(dir.path(), &pending.id, &pending.updated_at).is_err());
        let pause = enqueue(
            dir.path(),
            NewReviewItem {
                kind: "budget_pause".into(),
                fingerprint: "budget-pause:b1:1".into(),
                payload: json!({ "budgetId": "b1" }),
                risk: "local_write".into(),
                source: "budget".into(),
                source_path: None,
            },
        )
        .unwrap();
        let rejected = reject(dir.path(), &pause.id).unwrap();
        let refused = undo_reject(dir.path(), &pause.id, &rejected.updated_at).unwrap_err();
        assert!(refused.contains("can't be undone"), "{refused}");
    }

    #[test]
    fn failed_apply_returns_to_pending_with_edited_payload() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("edit-me")).unwrap();
        let claimed = claim(
            dir.path(),
            &item.id,
            Some(json!({ "statement": "Always use Outlook" })),
        )
        .unwrap();
        assert_eq!(claimed.payload["statement"], "Always use Outlook");
        let retry = fail(dir.path(), &item.id, "disk full").unwrap();
        assert_eq!(retry.status, "pending");
        assert_eq!(retry.error.as_deref(), Some("disk full"));
    }

    #[test]
    fn rejects_unknown_kind_and_corrupt_schema() {
        let dir = tempdir().unwrap();
        let bad = NewReviewItem {
            kind: "send".into(),
            fingerprint: "x".into(),
            payload: json!({}),
            risk: "local_write".into(),
            source: "chat".into(),
            source_path: None,
        };
        assert!(enqueue(dir.path(), bad).is_err());
        fs::write(path(dir.path()), r#"{"schemaVersion":99,"items":[]}"#).unwrap();
        assert!(list(dir.path(), None).unwrap_err().contains("unsupported"));
    }

    #[test]
    fn migrates_v1_on_first_mutation_and_keeps_exact_backup() {
        let dir = tempdir().unwrap();
        let v1 = br#"{
  "schemaVersion": 1,
  "items": [{
    "id": "review-old",
    "kind": "learning",
    "status": "pending",
    "createdAt": "2026-01-01T00:00:00Z",
    "updatedAt": "2026-01-01T00:00:00Z",
    "fingerprint": "old",
    "payload": {"statement": "Old lesson"}
  }]
}"#;
        fs::write(path(dir.path()), v1).unwrap();

        let listed = list(dir.path(), None).unwrap();
        assert_eq!(listed[0].risk, "local_write");
        assert_eq!(listed[0].source, "chat");
        assert!(!v1_backup_path(dir.path()).exists());

        enqueue(dir.path(), proposal("new")).unwrap();
        assert_eq!(fs::read(v1_backup_path(dir.path())).unwrap(), v1);
        let migrated: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(migrated.items.len(), 2);
    }

    #[test]
    fn supports_planned_kinds_and_wakes_expired_snoozes() {
        let dir = tempdir().unwrap();
        let item = enqueue(
            dir.path(),
            NewReviewItem {
                kind: "meeting_task".into(),
                fingerprint: "follow-up".into(),
                payload: json!({ "title": "Prepare the next review" }),
                risk: "local_write".into(),
                source: "meeting".into(),
                source_path: Some("Calendar/meeting.md".into()),
            },
        )
        .unwrap();
        let until = (Utc::now() + Duration::hours(1)).to_rfc3339();
        assert_eq!(
            snooze(dir.path(), &item.id, &until).unwrap().status,
            "snoozed"
        );

        let mut file: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        file.items[0].snoozed_until = Some("2020-01-01T00:00:00Z".into());
        fs::write(path(dir.path()), serde_json::to_vec_pretty(&file).unwrap()).unwrap();

        let woke = list(dir.path(), None).unwrap();
        assert_eq!(woke[0].status, "pending");
        assert_eq!(woke[0].snoozed_until, None);
        let persisted: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(persisted.items[0].status, "pending");
    }

    #[test]
    fn agent_task_kind_is_allowed_from_chat_and_workflow_runs() {
        let dir = tempdir().unwrap();
        for (source, fingerprint) in [("chat", "agent-task:a"), ("workflow", "agent-task:b")] {
            let item = enqueue(
                dir.path(),
                NewReviewItem {
                    kind: "agent_task".into(),
                    fingerprint: fingerprint.into(),
                    payload: json!({ "title": "Draft the rollout checklist", "runId": "run-1" }),
                    risk: "local_write".into(),
                    source: source.into(),
                    source_path: None,
                },
            )
            .unwrap();
            assert_eq!(
                (item.kind.as_str(), item.status.as_str()),
                ("agent_task", "pending")
            );
        }
        let unknown = NewReviewItem {
            kind: "agent_send".into(),
            ..proposal("unknown-kind")
        };
        assert!(enqueue(dir.path(), unknown)
            .unwrap_err()
            .contains("unsupported review item kind"));
        let budget_source = NewReviewItem {
            kind: "agent_task".into(),
            source: "budget".into(),
            ..proposal("wrong-source")
        };
        assert!(
            enqueue(dir.path(), budget_source).is_err(),
            "the budget source carries only budget pauses"
        );
    }

    #[test]
    fn rejects_messaging_routes_before_creating_a_queue() {
        let dir = tempdir().unwrap();
        let variants = [
            ("heartbeat_send", "local_write", "heartbeat", None),
            ("reply_draft", "local_write", "chat", None),
            ("meeting_followup", "local_write", "meeting", None),
            ("learning", "external_send", "chat", None),
            ("learning", "local_write", "inbox", None),
            ("learning", "local_write", "Email", None),
            ("learning", "local_write", "Teams", None),
            ("learning", "local_write", "Slack", None),
            (
                "heartbeat_task",
                "local_write",
                "heartbeat",
                Some("emails/inbox/a.md"),
            ),
            (
                "meeting_task",
                "local_write",
                "meeting",
                Some("vault:Teams/dms/a.md"),
            ),
            (
                "commitment",
                "local_write",
                "chat",
                Some("slack/channel/a.md"),
            ),
        ];
        for (kind, risk, source, source_path) in variants {
            let input = NewReviewItem {
                kind: kind.into(),
                risk: risk.into(),
                source: source.into(),
                source_path: source_path.map(str::to_string),
                ..proposal("retired")
            };
            assert!(enqueue(dir.path(), input).unwrap_err().contains("retired"));
        }
        for key in ["sourcePath", "sourceVaultPath", "replyToVaultPath"] {
            let input = NewReviewItem {
                payload: json!({ (key): "emails/inbox/a.md" }),
                ..proposal("payload-route")
            };
            assert!(enqueue(dir.path(), input).unwrap_err().contains("retired"));
        }
        assert!(!path(dir.path()).exists());
    }

    #[test]
    fn edited_local_proposal_cannot_switch_to_a_message_source() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("local-edit")).unwrap();
        let original = fs::read(path(dir.path())).unwrap();
        assert!(claim(
            dir.path(),
            &item.id,
            Some(json!({
                "sourceVaultPath": "teams/dms/a.md"
            }))
        )
        .unwrap_err()
        .contains("retired"));
        assert_eq!(fs::read(path(dir.path())).unwrap(), original);
    }

    #[test]
    fn dedup_never_returns_a_retired_record_to_a_live_caller() {
        let dir = tempdir().unwrap();
        enqueue(dir.path(), proposal("collision")).unwrap();
        let mut file: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        file.items[0].source_path = Some("./emails/inbox/a.md".into());
        let original = serde_json::to_vec_pretty(&file).unwrap();
        fs::write(path(dir.path()), &original).unwrap();
        assert!(enqueue(dir.path(), proposal("collision"))
            .unwrap_err()
            .contains("retired"));
        assert!(list(dir.path(), None).unwrap().is_empty());
        assert_eq!(fs::read(path(dir.path())).unwrap(), original);
    }

    #[test]
    fn historical_messaging_stays_hidden_inert_and_byte_preserved() {
        let dir = tempdir().unwrap();
        let local = enqueue(dir.path(), proposal("local")).unwrap();
        let mut file: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        let statuses = ["pending", "snoozed", "applying", "accepted", "rejected"];
        for (n, status) in statuses.iter().enumerate() {
            file.items.push(ReviewItem {
                id: format!("old-{n}"),
                kind: [
                    "heartbeat_send",
                    "reply_draft",
                    "meeting_followup",
                    "learning",
                    "commitment",
                ][n]
                    .into(),
                status: (*status).into(),
                updated_at: "2020-01-01T00:00:00Z".into(),
                risk: if n == 3 {
                    "external_send"
                } else {
                    "local_write"
                }
                .into(),
                source: if n == 4 { "inbox" } else { "chat" }.into(),
                payload: json!({ "body": "Historical payload must remain recoverable." }),
                snoozed_until: Some("2020-01-01T00:00:00Z".into()),
                ..local.clone()
            });
        }
        let original = serde_json::to_vec_pretty(&file).unwrap();
        fs::write(path(dir.path()), &original).unwrap();
        assert_eq!(list(dir.path(), None).unwrap(), vec![local.clone()]);
        assert!(list(dir.path(), Some("heartbeat_send")).unwrap().is_empty());
        let until = (Utc::now() + Duration::hours(1)).to_rfc3339();
        for n in 0..statuses.len() {
            let id = format!("old-{n}");
            assert!(claim(dir.path(), &id, Some(json!({ "body": "changed" })))
                .unwrap_err()
                .contains("retired"));
            assert!(snooze(dir.path(), &id, &until)
                .unwrap_err()
                .contains("retired"));
            assert!(reject(dir.path(), &id).unwrap_err().contains("retired"));
            assert!(accept(dir.path(), &id).unwrap_err().contains("retired"));
            assert!(fail(dir.path(), &id, "retry")
                .unwrap_err()
                .contains("retired"));
        }
        assert_eq!(fs::read(path(dir.path())).unwrap(), original);
        // A legitimate local write still works and preserves old item values.
        claim(dir.path(), &local.id, None).unwrap();
        accept(dir.path(), &local.id).unwrap();
        let persisted: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(persisted.items[1..], file.items[1..]);
    }

    fn permission_review(fp: &str, summary: &str) -> NewReviewItem {
        NewReviewItem {
            kind: "workflow_permissions".into(),
            fingerprint: fp.into(),
            payload: json!({ "slug": "x", "summary": summary }),
            risk: "local_write".into(),
            source: "workflow".into(),
            source_path: Some("workflows/x.md".into()),
        }
    }

    const SUPERSEDE: &str = "Superseded: changed again.";

    fn by_fp(vault: &Path, fp: &str) -> ReviewItem {
        list(vault, None)
            .unwrap()
            .into_iter()
            .find(|item| item.fingerprint == fp)
            .unwrap()
    }

    fn live_count(vault: &Path) -> usize {
        list(vault, Some("workflow_permissions"))
            .unwrap()
            .iter()
            .filter(|item| matches!(item.status.as_str(), "pending" | "snoozed"))
            .count()
    }

    #[test]
    fn enqueue_exclusive_keeps_one_live_item_per_subject() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        // 1. A second fingerprint for the same subject supersedes the first.
        let first = enqueue_exclusive(v, permission_review("a", "first"), SUPERSEDE).unwrap();
        assert!(first.opened);
        let second = enqueue_exclusive(v, permission_review("b", "second"), SUPERSEDE).unwrap();
        assert!(second.opened);
        let a = by_fp(v, "a");
        assert_eq!(a.status, "rejected");
        assert!(a.error.as_deref().unwrap().starts_with(SUPERSEDED_PREFIX));
        assert_eq!(by_fp(v, "b").status, "pending");
        assert_eq!(live_count(v), 1);

        // 2. Re-enqueueing the first reopens it and supersedes the second.
        let reopened = enqueue_exclusive(v, permission_review("A", "first"), SUPERSEDE).unwrap();
        assert!(reopened.opened);
        assert_eq!(reopened.item.id, first.item.id);
        assert_eq!(reopened.item.status, "pending");
        assert_eq!(reopened.item.error, None);
        assert_eq!(by_fp(v, "b").status, "rejected");
        assert_eq!(live_count(v), 1);

        // 3. Same fingerprint, new payload: refreshed in place, not reopened.
        let refreshed =
            enqueue_exclusive(v, permission_review("a", "first, again"), SUPERSEDE).unwrap();
        assert!(!refreshed.opened);
        assert_eq!(refreshed.item.id, first.item.id);
        assert_eq!(refreshed.item.payload["summary"], "first, again");

        // 4. An accepted fingerprint reopens when the same pair recurs.
        claim(v, &first.item.id, None).unwrap();
        accept(v, &first.item.id).unwrap();
        let recurring = enqueue_exclusive(v, permission_review("a", "first"), SUPERSEDE).unwrap();
        assert!(recurring.opened);
        assert_eq!(recurring.item.status, "pending");

        // 5. A user rejection is sticky for that exact pair.
        reject(v, &first.item.id).unwrap();
        let sticky = enqueue_exclusive(v, permission_review("a", "first"), SUPERSEDE).unwrap();
        assert!(!sticky.opened);
        assert_eq!(sticky.item.status, "rejected");
        assert_eq!(sticky.item.error, None);

        // 6. An item mid-application is left exactly as claimed.
        let c = enqueue_exclusive(v, permission_review("c", "third"), SUPERSEDE).unwrap();
        claim(v, &c.item.id, None).unwrap();
        let applying = enqueue_exclusive(v, permission_review("c", "changed"), SUPERSEDE).unwrap();
        assert!(!applying.opened);
        assert_eq!(applying.item.status, "applying");
        assert_eq!(applying.item.payload["summary"], "third");
        fail(v, &c.item.id, "retry").unwrap();

        // 7. retire_live touches only live items of that kind and path.
        let other_path = NewReviewItem {
            source_path: Some("workflows/y.md".into()),
            ..permission_review("y", "other workflow")
        };
        enqueue_exclusive(v, other_path, SUPERSEDE).unwrap();
        let learning = enqueue(
            v,
            NewReviewItem {
                source_path: Some("workflows/x.md".into()),
                ..proposal("learning-on-the-same-path")
            },
        )
        .unwrap();
        assert!(retire_live(
            v,
            "workflow_permissions",
            "workflows/x.md",
            "not a supersede"
        )
        .is_err());
        assert_eq!(
            retire_live(v, "workflow_permissions", "workflows/x.md", SUPERSEDE).unwrap(),
            1
        );
        assert_eq!(by_fp(v, "c").status, "rejected");
        assert_eq!(by_fp(v, "y").status, "pending");
        assert_eq!(by_fp(v, &learning.fingerprint).status, "pending");
        assert_eq!(
            retire_live(v, "workflow_permissions", "workflows/x.md", SUPERSEDE).unwrap(),
            0
        );

        // 8. The new kind and source are allowed; unknown ones still are not.
        assert!(enqueue_exclusive(
            v,
            NewReviewItem {
                kind: "send".into(),
                ..permission_review("z", "z")
            },
            SUPERSEDE
        )
        .is_err());
        assert!(enqueue_exclusive(
            v,
            NewReviewItem {
                source: "robot".into(),
                ..permission_review("z", "z")
            },
            SUPERSEDE
        )
        .is_err());
        assert!(enqueue_exclusive(v, permission_review("z", "z"), "no prefix").is_err());

        // 9. Plain enqueue keeps its sticky dedup.
        let plain = enqueue(v, proposal("plain")).unwrap();
        claim(v, &plain.id, None).unwrap();
        accept(v, &plain.id).unwrap();
        assert_eq!(enqueue(v, proposal("plain")).unwrap().status, "accepted");
    }

    #[test]
    fn superseded_items_are_capped_per_subject_and_never_evict_decisions() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        // A decision of another kind, and the user's own rejection of one
        // permission pair: neither may ever be pruned by autosave churn.
        let plain = enqueue(v, proposal("keep-me")).unwrap();
        reject(v, &plain.id).unwrap();
        let rejected = enqueue_exclusive(v, permission_review("user-no", "no"), SUPERSEDE).unwrap();
        reject(v, &rejected.item.id).unwrap();
        // Forty distinct autosaved states of the same automation.
        for n in 0..40 {
            enqueue_exclusive(
                v,
                permission_review(&format!("save-{n}"), "edit"),
                SUPERSEDE,
            )
            .unwrap();
        }
        let items = list(v, Some("workflow_permissions")).unwrap();
        let superseded: Vec<_> = items.iter().filter(|i| is_superseded(i)).collect();
        assert_eq!(superseded.len(), SUPERSEDED_KEEP_PER_SUBJECT, "{items:#?}");
        assert_eq!(live_count(v), 1);
        assert_eq!(by_fp(v, "save-39").status, "pending");
        // The newest superseded states are the ones kept.
        assert!(superseded.iter().any(|i| i.fingerprint == "save-38"));
        assert!(!items.iter().any(|i| i.fingerprint == "save-0"));
        assert_eq!(
            by_fp(v, "user-no").status,
            "rejected",
            "the user's rejection is kept"
        );
        assert_eq!(
            by_fp(v, "keep-me").status,
            "rejected",
            "other kinds are untouched"
        );

        // A→B→A still leaves one live item, whether A was kept or pruned.
        enqueue_exclusive(v, permission_review("save-38", "edit"), SUPERSEDE).unwrap();
        assert_eq!(by_fp(v, "save-38").status, "pending");
        enqueue_exclusive(v, permission_review("save-0", "edit"), SUPERSEDE).unwrap();
        assert_eq!(by_fp(v, "save-0").status, "pending");
        assert_eq!(live_count(v), 1);
        // retire_live prunes too.
        retire_live(v, "workflow_permissions", "workflows/x.md", SUPERSEDE).unwrap();
        let superseded = list(v, Some("workflow_permissions"))
            .unwrap()
            .into_iter()
            .filter(is_superseded)
            .count();
        assert_eq!(superseded, SUPERSEDED_KEEP_PER_SUBJECT);
        assert_eq!(live_count(v), 0);
    }

    fn streak_review(fp: &str, path: &str) -> NewReviewItem {
        NewReviewItem {
            kind: "workflow_failure_streak".into(),
            fingerprint: fp.into(),
            payload: json!({ "slug": "x", "count": 3 }),
            risk: "local_write".into(),
            source: "workflow".into(),
            source_path: Some(path.into()),
        }
    }

    /// The failure streak's real openness rule, not a copy, so these tests
    /// cannot drift from what `record_outcome` passes.
    fn open_streak(item: &ReviewItem) -> bool {
        crate::workflow_hygiene::is_open_streak(item)
    }

    const STREAK: &str = "workflow_failure_streak";
    const X: &str = "workflows/x.md";
    const CLOSE: &str = "Superseded: the automation succeeded again.";

    #[test]
    fn enqueue_unless_open_and_close_subject() {
        let dir = tempdir().unwrap();
        let v = dir.path();

        // The first call creates; a second fingerprint for the same subject
        // returns the open item and creates nothing.
        let (a, created) = enqueue_unless_open(v, streak_review("A", X), open_streak).unwrap();
        assert!(created);
        assert_eq!(a.fingerprint, "a", "fingerprints are normalized");
        let (same, created) = enqueue_unless_open(v, streak_review("b", X), open_streak).unwrap();
        assert!(!created);
        assert_eq!(same.id, a.id);
        assert!(!list(v, Some(STREAK))
            .unwrap()
            .iter()
            .any(|i| i.fingerprint == "b"));
        // Subjects are per source path.
        let (y, created) =
            enqueue_unless_open(v, streak_review("c", "workflows/y.md"), open_streak).unwrap();
        assert!(created);

        // The reason must start with Superseded:, and nothing is written.
        assert!(close_subject(v, STREAK, X, "the automation succeeded").is_err());
        assert_eq!(by_fp(v, "a").status, "pending");

        // A live item is superseded; the other subject is untouched.
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 1);
        let a = by_fp(v, "a");
        assert!(is_superseded(&a));
        assert_eq!(a.error.as_deref(), Some(CLOSE));
        assert_eq!(by_fp(v, "c").status, "pending");
        assert_eq!(by_fp(v, "c").id, y.id);

        // A user rejection stays the user's (no error), but gets closedAt.
        let (d, created) = enqueue_unless_open(v, streak_review("d", X), open_streak).unwrap();
        assert!(created);
        reject(v, &d.id).unwrap();
        let (still_d, created) =
            enqueue_unless_open(v, streak_review("d2", X), open_streak).unwrap();
        assert!(!created, "a dismissal is sticky until the subject closes");
        assert_eq!(still_d.id, d.id);
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 1);
        let d = by_fp(v, "d");
        assert_eq!((d.status.as_str(), d.error.as_deref()), ("rejected", None));
        assert!(has_closed_at(&d));

        // An item mid-application is stamped and keeps the stamp through its
        // accept, so a success landing during a Pause still closes the subject.
        let (e, created) = enqueue_unless_open(v, streak_review("e", X), open_streak).unwrap();
        assert!(created);
        claim(v, &e.id, None).unwrap();
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 1);
        let accepted = accept(v, &e.id).unwrap();
        assert_eq!(accepted.status, "accepted");
        assert!(has_closed_at(&accepted));
        let (f, created) = enqueue_unless_open(v, streak_review("f", X), open_streak).unwrap();
        assert!(
            created,
            "a closed accepted item no longer blocks its subject"
        );

        // An accepted item without closedAt is still open.
        claim(v, &f.id, None).unwrap();
        accept(v, &f.id).unwrap();
        let (still_f, created) =
            enqueue_unless_open(v, streak_review("g", X), open_streak).unwrap();
        assert!(!created);
        assert_eq!(still_f.id, f.id);

        // Closing again stamps only what is unstamped; a closed subject is a
        // no-op. A known fingerprint is never reused for a new item.
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 1);
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 0);
        let (old_a, created) = enqueue_unless_open(v, streak_review("a", X), open_streak).unwrap();
        assert!(!created);
        assert_eq!(old_a.id, by_fp(v, "a").id);

        // Other kinds on the same path are untouched.
        let permission =
            enqueue_exclusive(v, permission_review("perm", "edit"), SUPERSEDE).unwrap();
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 0);
        assert_eq!(by_fp(v, "perm").id, permission.item.id);
        assert_eq!(by_fp(v, "perm").status, "pending");
    }

    #[test]
    fn subject_scoped_enqueue_needs_a_path_and_a_known_kind() {
        let dir = tempdir().unwrap();
        let mut pathless = streak_review("p", X);
        pathless.source_path = Some("   ".into());
        assert!(enqueue_unless_open(dir.path(), pathless, open_streak).is_err());
        let mut unknown = streak_review("q", X);
        unknown.kind = "workflow_streak".into();
        assert!(enqueue_unless_open(dir.path(), unknown, open_streak).is_err());
        assert!(!path(dir.path()).exists(), "nothing was written");
    }

    #[test]
    fn close_subject_wraps_a_non_object_payload_rather_than_leaving_it_open() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let (item, _) = enqueue_unless_open(v, streak_review("odd", X), open_streak).unwrap();
        reject(v, &item.id).unwrap();
        let mut file: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(v)).unwrap()).unwrap();
        file.items[0].payload = json!("legacy text");
        fs::write(path(v), serde_json::to_vec_pretty(&file).unwrap()).unwrap();
        assert_eq!(close_subject(v, STREAK, X, CLOSE).unwrap(), 1);
        let closed = by_fp(v, "odd");
        assert!(has_closed_at(&closed));
        assert_eq!(closed.payload["previousPayload"], "legacy text");
        assert!(!open_streak(&closed));
    }

    #[test]
    fn interrupted_local_approval_still_recovers_for_review() {
        let dir = tempdir().unwrap();
        let item = enqueue(dir.path(), proposal("interrupted")).unwrap();
        claim(dir.path(), &item.id, None).unwrap();
        let mut file: ReviewQueueFile =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        file.items[0].updated_at = "2020-01-01T00:00:00Z".into();
        fs::write(path(dir.path()), serde_json::to_vec_pretty(&file).unwrap()).unwrap();
        let recovered = list(dir.path(), None).unwrap();
        assert_eq!(recovered[0].status, "pending");
        assert!(recovered[0]
            .error
            .as_deref()
            .unwrap()
            .contains("nothing was assumed applied"));
    }

    // Paperclip PR 8 plan test 50.
    #[test]
    fn budget_pause_items_are_allowed_and_supersede_only_touches_live_items() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let pause = |fp: &str| NewReviewItem {
            kind: "budget_pause".into(),
            fingerprint: fp.into(),
            payload: json!({ "budgetId": "b1" }),
            risk: "local_write".into(),
            source: "budget".into(),
            source_path: None,
        };
        let open = enqueue(v, pause("budget-pause:b1:1")).unwrap();
        assert_eq!(open.status, "pending");
        assert!(
            supersede(v, &open.id, "not superseded").is_err(),
            "the prefix is required"
        );
        let retired = supersede(v, &open.id, "Superseded: the day reset.")
            .unwrap()
            .expect("a live item is superseded");
        assert_eq!(retired.status, "rejected");
        assert!(retired
            .error
            .as_deref()
            .unwrap()
            .starts_with(SUPERSEDED_PREFIX));
        assert!(supersede(v, &open.id, "Superseded: again.")
            .unwrap()
            .is_none());

        let done = enqueue(v, pause("budget-pause:b1:2")).unwrap();
        claim(v, &done.id, None).unwrap();
        accept(v, &done.id).unwrap();
        assert!(supersede(v, &done.id, "Superseded: late.")
            .unwrap()
            .is_none());
        let items = list(v, Some("budget_pause")).unwrap();
        assert_eq!(
            items.iter().find(|i| i.id == done.id).unwrap().status,
            "accepted",
            "an accepted item is never touched"
        );
        assert!(supersede(v, "review-missing", "Superseded: x.")
            .unwrap()
            .is_none());
        // The budget source is not a free pass for other kinds' rules.
        let mut wrong = pause("x");
        wrong.risk = "external_send".into();
        assert!(enqueue(v, wrong).is_err());
        // Kind and source pair: another kind can't borrow the budget source,
        // and a budget pause can't come from anywhere else.
        let mut borrowed = pause("y");
        borrowed.kind = "heartbeat_task".into();
        assert!(enqueue(v, borrowed).is_err());
        let mut elsewhere = pause("z");
        elsewhere.source = "heartbeat".into();
        assert!(enqueue(v, elsewhere).is_err());
    }
}
