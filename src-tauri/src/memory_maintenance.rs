//! Usage aging is metadata, never deletion or a rewrite of a learned fact.
//! The app-data sidecar is fail-closed and independent of provider memory.
//! New observations receive a full grace period; profile rules never age.
//!
//! Accepted You and Project learnings are lines in `me/` or a workspace's
//! WORKSPACE.md / AGENTS.md (J2). Their sidecar key is `line:<accepting
//! proposal id>`, with that proposal's `learnedPath` and `learnedLine` as
//! the line's identity. A line past its valid-through day, or a Project
//! line whose workspace went quiet, gets a learning proposal to take it out
//! or reword it; the line itself changes only when the person saves that
//! proposal. A line edited by hand since its accept is shown as changed and
//! left alone.

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const STALE_DAYS: i64 = 14;
const ARCHIVE_DAYS: i64 = 30;
const CORE_BUDGET: usize = 16_000;
/// Sidecar key prefix of an accepted line home: `line:<proposal id>`.
const LINE_KEY: &str = "line:";
/// Sidecar key prefix of a workspace's chat days: `workspace:<id>`.
const WORKSPACE_KEY: &str = "workspace:";
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Usage {
    first_seen: String,
    last_used: Option<String>,
    restored_at: Option<String>,
    uses: u64,
    pinned: bool,
    expires_on: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    entries: BTreeMap<String, Usage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// What a click names: the note path, or `line:<proposal id>`.
    pub key: String,
    /// `note` (a Know-how file) or `line` (a You or Project line).
    pub kind: String,
    /// The line, for a line home.
    pub line: Option<String>,
    /// Whether quiet aging applies (profile rules never age).
    pub ages: bool,
    /// The open proposal to take out or reword this line, if one waits.
    pub proposal_id: Option<String>,
    pub path: String,
    pub status: String,
    pub pinned: bool,
    pub uses: u64,
    pub last_used: Option<String>,
    pub expires_on: Option<String>,
    pub expected_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub entries: Vec<Entry>,
    pub core_bytes: usize,
    pub core_budget_bytes: usize,
    pub consolidation_proposal: Option<String>,
}

fn path(vault: &Path) -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?
        .join("memory-usage")
        .join(format!(
            "{}.json",
            crate::session_store::vault_file_stem(vault)
        )))
}

fn load(at: &Path) -> Result<State, String> {
    let state: State =
        crate::state_file::read_json_or(at, "Memory usage", crate::state_file::OnDamage::Leave)?
            .unwrap_or_default();
    for item in state.entries.values() {
        for time in [
            Some(item.first_seen.as_str()),
            item.last_used.as_deref(),
            item.restored_at.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if DateTime::parse_from_rfc3339(time).is_err() {
                return Err("Memory usage has an invalid date; nothing was changed.".into());
            }
        }
        if item
            .expires_on
            .as_deref()
            .is_some_and(|day| parse_day(day).is_err())
        {
            return Err("Memory usage has an invalid expiry; nothing was changed.".into());
        }
    }
    Ok(state)
}

fn save(at: &Path, state: &State) -> Result<(), String> {
    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::vault::atomic_write_private(at, &serde_json::to_vec(state).map_err(|e| e.to_string())?)
}

fn parse_day(day: &str) -> Result<NaiveDate, String> {
    let parsed = NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| "Use an expiry date in YYYY-MM-DD form.".to_string())?;
    if parsed.format("%Y-%m-%d").to_string() != day {
        return Err("Use an expiry date in YYYY-MM-DD form.".into());
    }
    Ok(parsed)
}

/// `now` carries the person's time zone (`Local` in the app): a "valid
/// through" day is a local calendar day, so expiry starts at local midnight,
/// not at UTC midnight. Quiet aging counts whole days in UTC instants.
fn state_of<Tz: TimeZone>(usage: &Usage, now: DateTime<Tz>, ages: bool) -> &'static str {
    let today = now.date_naive();
    let now = now.with_timezone(&Utc);
    let latest = latest(usage).unwrap_or(now);
    if usage
        .expires_on
        .as_deref()
        .and_then(|s| parse_day(s).ok())
        .is_some_and(|day| today > day)
    {
        return "expired";
    }
    if usage.pinned || !ages {
        return "active";
    }
    let days = (now - latest).num_days();
    if days >= ARCHIVE_DAYS {
        "archived"
    } else if days >= STALE_DAYS {
        "stale"
    } else {
        "active"
    }
}

/// The last sign of life: first sighting, last use or restore.
fn latest(usage: &Usage) -> Option<DateTime<Utc>> {
    [
        Some(usage.first_seen.as_str()),
        usage.last_used.as_deref(),
        usage.restored_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
    .map(|t| t.with_timezone(&Utc))
    .max()
}

pub(crate) fn ages(context: &crate::vault_learning::LearningContext) -> bool {
    !matches!(context.target_kind.as_str(), "operator" | "decision")
}

fn fresh(now: DateTime<Utc>) -> Usage {
    Usage {
        first_seen: now.to_rfc3339(),
        ..Usage::default()
    }
}

/// The snapshot a Library click carries: only what the person sets (pin,
/// expiry, restore) and the learning's text. Use counts and last-used times
/// move with every recall, so a turn between opening Library and the click
/// never refuses it.
fn token(usage: &Usage, raw: &str) -> String {
    crate::memory_guard::text_hash(&format!(
        "{}\n{}\n{}\n{}",
        usage.pinned,
        usage.expires_on.as_deref().unwrap_or(""),
        usage.restored_at.as_deref().unwrap_or(""),
        crate::memory_guard::text_hash(raw)
    ))
}

/// A single metadata read per recall. A corrupt sidecar cannot resurrect
/// archived facts. Historical queries can still search every saved note.
pub(crate) fn excluded(
    vault: &Path,
    contexts: &[crate::vault_learning::LearningContext],
) -> BTreeSet<String> {
    let now = Local::now();
    match path(vault).and_then(|at| load(&at)) {
        Ok(state) => contexts
            .iter()
            .filter(|c| {
                state
                    .entries
                    .get(&c.path)
                    .is_some_and(|u| matches!(state_of(u, now, ages(c)), "archived" | "expired"))
            })
            .map(|c| c.path.clone())
            .collect(),
        Err(_) => contexts.iter().map(|c| c.path.clone()).collect(),
    }
}

/// Called only for a turn's selected recall, not a read-only search or
/// opening Library. `observed` is what that recall already listed (path,
/// whether it ages), so nothing is listed twice. Counts selected uses; new
/// notes get a full grace period. Writes only when something changed.
pub(crate) fn record_recall(
    vault: &Path,
    observed: &[(String, bool)],
    items: &[crate::vault_learning::RecallItem],
    workspace: Option<&str>,
) -> Result<(), String> {
    let workspace = workspace.map(str::trim).filter(|w| !w.is_empty());
    if observed.is_empty() && workspace.is_none() {
        return Ok(());
    }
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    record_at(&path(vault)?, observed, items, workspace, Local::now()).map(|_| ())
}

/// Whether it wrote: a turn that recalled nothing and met no new note
/// leaves the sidecar untouched. A turn in a workspace counts one chat day
/// for that workspace's Project lines (its WORKSPACE.md and AGENTS.md are
/// read by every chat there), written at most once per local day.
fn record_at<Tz: TimeZone>(
    at: &Path,
    observed: &[(String, bool)],
    items: &[crate::vault_learning::RecallItem],
    workspace: Option<&str>,
    now: DateTime<Tz>,
) -> Result<bool, String> {
    if observed.is_empty() && workspace.is_none() {
        return Ok(false);
    }
    let utc = now.with_timezone(&Utc);
    let mut state = load(at)?;
    let mut changed = false;
    for (rel, ages) in observed {
        let usage = state.entries.entry(rel.clone()).or_insert_with(|| {
            changed = true;
            fresh(utc)
        });
        if items.iter().any(|i| &i.path == rel)
            && !matches!(state_of(usage, now.clone(), *ages), "archived" | "expired")
        {
            usage.uses = usage.uses.saturating_add(1);
            usage.last_used = Some(utc.to_rfc3339());
            changed = true;
        }
    }
    if let Some(id) = workspace {
        let zone = now.timezone();
        let today = now.date_naive();
        let days = state
            .entries
            .entry(format!("{WORKSPACE_KEY}{id}"))
            .or_insert_with(|| {
                changed = true;
                fresh(utc)
            });
        let seen_today = days
            .last_used
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| t.with_timezone(&zone).date_naive() == today);
        if !seen_today {
            days.uses = days.uses.saturating_add(1);
            days.last_used = Some(utc.to_rfc3339());
            changed = true;
        }
    }
    if changed {
        save(at, &state)?;
    }
    Ok(changed)
}

/// An accepted You or Project learning's line (J2).
struct LineHome {
    /// The accepting proposal: the line's identity with `path` and `line`.
    id: String,
    path: String,
    line: String,
    payload: crate::vault_learning::LearningProposalPayload,
}

impl LineHome {
    fn key(&self) -> String {
        format!("{LINE_KEY}{}", self.id)
    }
    /// Project lines age with their workspace's quiet; profile rules never do.
    fn ages(&self) -> bool {
        self.payload.target_kind == "workspace"
    }
    fn workspace(&self) -> Option<&str> {
        crate::memory_homes::project_target(&self.path).map(|(id, _)| id)
    }
}

struct Lines {
    homes: Vec<LineHome>,
    /// The waiting maintenance proposal per line (by its owner's id).
    open: BTreeMap<String, String>,
    /// Every (line owner, cycle) a maintenance proposal was ever made for.
    proposed: BTreeSet<(String, String)>,
}

/// Accepted line homes from the review queue. A line a saved maintenance
/// proposal already took out or reworded belongs to that proposal now.
fn lines(vault: &Path) -> Result<Lines, String> {
    let all = crate::vault_learning::list_proposals(vault)?;
    let mut retired = BTreeSet::new();
    let mut open = BTreeMap::new();
    let mut proposed = BTreeSet::new();
    for p in &all {
        if let Some(r) = &p.payload.retires {
            proposed.insert((r.proposal.clone(), r.cycle.clone()));
            match p.status.as_str() {
                "accepted" => {
                    retired.insert(r.proposal.clone());
                }
                "pending" | "snoozed" | "applying" => {
                    open.insert(r.proposal.clone(), p.id.clone());
                }
                _ => {}
            }
        }
    }
    let homes = all
        .into_iter()
        .filter(|p| p.status == "accepted" && !retired.contains(&p.id))
        .filter_map(|p| {
            Some(LineHome {
                path: p.payload.learned_path.clone()?,
                line: p.payload.learned_line.clone()?,
                id: p.id,
                payload: p.payload,
            })
        })
        .collect();
    Ok(Lines {
        homes,
        open,
        proposed,
    })
}

struct LineState {
    status: &'static str,
    intact: bool,
    /// The line's own metadata (what the person set).
    own: Usage,
    /// With its workspace's chat days folded in.
    seen: Usage,
    /// What a maintenance proposal for this state is keyed by.
    cycle: Option<String>,
}

fn line_state<Tz: TimeZone>(
    vault: &Path,
    state: &State,
    home: &LineHome,
    now: DateTime<Tz>,
) -> LineState {
    let raw = crate::vault::safe_join(vault, &home.path)
        .ok()
        .and_then(|full| std::fs::read_to_string(full).ok());
    let intact = raw
        .as_deref()
        .is_some_and(|raw| crate::memory_homes::has_line(raw, &home.line));
    let own = state
        .entries
        .get(&home.key())
        .cloned()
        .unwrap_or_else(|| fresh(now.with_timezone(&Utc)));
    let mut seen = own.clone();
    if let Some(days) = home
        .workspace()
        .and_then(|id| state.entries.get(&format!("{WORKSPACE_KEY}{id}")))
    {
        seen.uses = days.uses;
        let at = |t: &Option<String>| {
            t.as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        };
        if at(&days.last_used) > at(&own.last_used) {
            seen.last_used = days.last_used.clone();
        }
    }
    let status = if !intact {
        "changed"
    } else {
        // A line is still in its home and still read, so it is never
        // archived: past the quiet stretch it stays stale.
        match state_of(&seen, now, home.ages()) {
            "archived" => "stale",
            other => other,
        }
    };
    let cycle = match status {
        "expired" => own.expires_on.as_ref().map(|day| format!("expired:{day}")),
        "stale" => latest(&seen).map(|t| format!("stale:{}", t.to_rfc3339())),
        _ => None,
    };
    LineState {
        status,
        intact,
        own,
        seen,
        cycle,
    }
}

/// The text a line click's snapshot covers: where it is, what it says and
/// whether it is still there as saved.
fn line_raw(home: &LineHome, intact: bool) -> String {
    format!("{}\n{}\n{intact}", home.path, home.line)
}

fn line_entries<Tz: TimeZone>(
    vault: &Path,
    state: &State,
    lines: &Lines,
    now: DateTime<Tz>,
) -> Vec<Entry> {
    lines
        .homes
        .iter()
        .map(|home| {
            let at = line_state(vault, state, home, now.clone());
            Entry {
                key: home.key(),
                kind: "line".into(),
                line: Some(home.line.clone()),
                ages: home.ages(),
                proposal_id: lines.open.get(&home.id).cloned(),
                path: home.path.clone(),
                status: at.status.into(),
                pinned: at.own.pinned,
                uses: at.seen.uses,
                last_used: at.seen.last_used.clone(),
                expires_on: at.own.expires_on.clone(),
                expected_hash: token(
                    state.entries.get(&home.key()).unwrap_or(&Usage::default()),
                    &line_raw(home, at.intact),
                ),
            }
        })
        .collect()
}

/// Library's Learnings view, after filing what maintenance found: a line's
/// first sighting starts its grace period, and an intact line that passed
/// its valid-through day or went quiet gets one proposal per lapse (a
/// rejected one is not asked again until the line lapses anew), and a saved
/// learning that repeats a `me/` line gets a retire card (PX5). Proposals
/// only: no line or note is changed here.
pub fn refresh(vault: &Path) -> Result<View, String> {
    {
        let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let at = path(vault)?;
        let mut state = load(&at)?;
        let lines = lines(vault)?;
        let now = Local::now();
        let mut seen_new = false;
        for home in &lines.homes {
            if let std::collections::btree_map::Entry::Vacant(e) = state.entries.entry(home.key()) {
                e.insert(fresh(now.with_timezone(&Utc)));
                seen_new = true;
            }
        }
        if seen_new {
            save(&at, &state)?;
        }
        for home in &lines.homes {
            if lines.open.contains_key(&home.id) {
                continue;
            }
            let at = line_state(vault, &state, home, now);
            let Some(cycle) = at.cycle.filter(|_| at.intact) else {
                continue;
            };
            if lines.proposed.contains(&(home.id.clone(), cycle.clone())) {
                continue;
            }
            if let Err(error) = propose(vault, home, at.status, &at.own, cycle) {
                eprintln!("[memory] a maintenance proposal could not be filed: {error}");
            }
        }
    }
    // One home per fact (PX5): a card for each learning that repeats a `me/`
    // line; only its Save retires the learning.
    crate::vault_learning::offer_profile_copies(vault);
    view(vault)
}

fn propose(
    vault: &Path,
    home: &LineHome,
    status: &str,
    own: &Usage,
    cycle: String,
) -> Result<(), String> {
    let why = match status {
        "expired" => format!(
            "Valid through {} has passed.",
            own.expires_on.as_deref().unwrap_or("its date")
        ),
        _ => format!("No chat in this workspace for {STALE_DAYS} days or more."),
    };
    let subject: String = format!("Remove or update: {}", home.payload.subject)
        .chars()
        .take(200)
        .collect();
    crate::vault_learning::propose_retire(
        vault,
        crate::vault_learning::LearningProposalPayload {
            kind: home.payload.kind.clone(),
            target_kind: home.payload.target_kind.clone(),
            target_path: home.payload.target_path.clone(),
            subject,
            statement: home.line.clone(),
            rationale: format!(
                "{why} Save takes this line out of {}. Edit the words first to replace it instead. Reject keeps it as it is.",
                home.path
            ),
            source_run_id: home.payload.source_run_id.clone(),
            retires: Some(crate::vault_learning::RetireLine {
                proposal: home.id.clone(),
                path: home.path.clone(),
                line: home.line.clone(),
                cycle,
            }),
            ..Default::default()
        },
    )
    .map(|_| ())
}

pub fn view(vault: &Path) -> Result<View, String> {
    let state = load(&path(vault)?)?;
    let now = Local::now();
    let entries = crate::vault_learning::list_context(vault, true)
        .into_iter()
        .map(|c| {
            let usage = state
                .entries
                .get(&c.path)
                .cloned()
                .unwrap_or_else(|| fresh(now.with_timezone(&Utc)));
            let raw = std::fs::read_to_string(crate::vault::safe_join(vault, &c.path)?)
                .map_err(|e| e.to_string())?;
            Ok(Entry {
                key: c.path.clone(),
                kind: "note".into(),
                line: None,
                ages: ages(&c),
                proposal_id: None,
                path: c.path.clone(),
                status: state_of(&usage, now, ages(&c)).into(),
                pinned: usage.pinned,
                uses: usage.uses,
                last_used: usage.last_used.clone(),
                expires_on: usage.expires_on.clone(),
                expected_hash: token(
                    state.entries.get(&c.path).unwrap_or(&Usage::default()),
                    &raw,
                ),
            })
        })
        .collect::<Result<Vec<Entry>, String>>()?;
    let mut entries = entries;
    entries.extend(line_entries(vault, &state, &lines(vault)?, now));
    let core_bytes = [
        "Identity.md",
        "Communication.md",
        "Anti-patterns.md",
        "Guardrails.md",
        "Decisions.md",
    ]
    .iter()
    .map(|name| match std::fs::read(vault.join("me").join(name)) {
        Ok(bytes) => Ok(bytes.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("Profile size could not be read: {e}")),
    })
    .collect::<Result<Vec<_>, String>>()?
    .into_iter()
    .sum::<usize>();
    Ok(View { entries, core_bytes, core_budget_bytes:CORE_BUDGET, consolidation_proposal:(core_bytes > CORE_BUDGET).then(||
        "Proposed consolidation: review repeated profile facts, keep one fact in one home, and move task procedures into a skill. Review the source files before saving any change. Nothing has been shortened or removed.".into()) })
}

/// The person's pin, restore or valid-through click on `key`: a saved
/// note's path, or `line:<proposal id>` for an accepted You or Project line.
/// Only the sidecar changes; a line's own text never does here.
pub fn edit(
    vault: &Path,
    key: &str,
    expected_hash: &str,
    action: &str,
    date: Option<&str>,
) -> Result<View, String> {
    edit_held(vault, key, expected_hash, action, date)?;
    refresh(vault)
}

fn edit_held(
    vault: &Path,
    key: &str,
    expected_hash: &str,
    action: &str,
    date: Option<&str>,
) -> Result<(), String> {
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    // A line's open maintenance proposal, and the line, for after the edit.
    let mut line: Option<(LineHome, Option<String>)> = None;
    let (rel, raw) = match key.strip_prefix(LINE_KEY) {
        Some(id) => {
            let mut found = lines(vault)?;
            let at = found
                .homes
                .iter()
                .position(|h| h.id == id)
                .ok_or("Saved learning not found.")?;
            let home = found.homes.swap_remove(at);
            let intact = crate::vault::safe_join(vault, &home.path)
                .ok()
                .and_then(|full| std::fs::read_to_string(full).ok())
                .is_some_and(|raw| crate::memory_homes::has_line(&raw, &home.line));
            if !intact {
                return Err(
                    "This line was changed by hand since it was saved; Hollow leaves it alone."
                        .into(),
                );
            }
            let raw = line_raw(&home, intact);
            let rel = home.path.clone();
            line = Some((home, found.open.remove(id)));
            (rel, raw)
        }
        None => {
            if !crate::vault_learning::list_context(vault, true)
                .iter()
                .any(|c| c.path == key)
            {
                return Err("Saved learning not found.".into());
            }
            let full = crate::vault::safe_join(vault, key)?;
            (
                key.to_string(),
                std::fs::read_to_string(full).map_err(|e| e.to_string())?,
            )
        }
    };
    let at = path(vault)?;
    let mut state = load(&at)?;
    let now = Utc::now();
    // An unseen entry's snapshot is stable across read times; its grace
    // period starts only on the first real observation or explicit action.
    if token(state.entries.get(key).unwrap_or(&Usage::default()), &raw) != expected_hash {
        return Err(
            "This learning changed. Refresh and review it again; nothing was changed.".into(),
        );
    }
    let usage = state
        .entries
        .entry(key.into())
        .or_insert_with(|| fresh(now));
    let before = serde_json::to_string(usage).map_err(|e| e.to_string())?;
    match action {
        "pin" => usage.pinned = true,
        "unpin" => usage.pinned = false,
        "restore" => {
            usage.restored_at = Some(now.to_rfc3339());
            usage.expires_on = None;
        }
        "expiry" => {
            // A blank date field clears the expiry.
            let day = date.map(str::trim).filter(|d| !d.is_empty());
            if let Some(day) = day {
                parse_day(day)?;
            }
            usage.expires_on = day.map(str::to_string);
        }
        _ => return Err("Unknown memory maintenance action.".into()),
    }
    let after = serde_json::to_string(usage).map_err(|e| e.to_string())?;
    crate::memory_guard::check(&[("Memory lifecycle", &after)]).map_err(|e| e.message())?;
    crate::memory_guard::ready(vault)?;
    save(&at, &state)?;
    crate::memory_guard::record_or_warn(
        vault,
        crate::memory_guard::NewEntry {
            actor: crate::memory_guard::PERSON.into(),
            action: Some(crate::memory_guard::Action::Edited),
            subject: key.into(),
            path: Some(rel),
            before: Some(before),
            after: Some(after),
            note: Some(format!("Memory lifecycle: {action}")),
            ..Default::default()
        },
    );
    // A line the person kept (pinned, restored or re-dated so it is no
    // longer due) closes the waiting proposal to take it out; the line is
    // untouched either way.
    if let Some((home, Some(open))) = line {
        let due = line_state(vault, &state, &home, Local::now()).cycle;
        if due.is_none() {
            let _ = crate::review_queue::supersede(
                vault,
                &open,
                "Superseded: kept in Library Learnings (pinned, restored or re-dated).",
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// A temporary vault the note walk reads: `walk_notes` skips a
    /// dot-prefixed root like `tempfile::tempdir()`'s `.tmpXXXX`.
    fn tmp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("hollow-memmaint-")
            .tempdir()
            .unwrap()
    }

    fn observed(v: &Path) -> Vec<(String, bool)> {
        crate::vault_learning::list_context(v, true)
            .iter()
            .map(|c| (c.path.clone(), ages(c)))
            .collect()
    }

    #[test]
    fn valid_through_ends_at_local_midnight_not_utc_midnight() {
        let mut usage = fresh(now());
        usage.expires_on = Some("2026-11-08".into());
        // New York is UTC-5 in November.
        let ny = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        let at = |utc: &str| {
            DateTime::parse_from_rfc3339(utc)
                .unwrap()
                .with_timezone(&ny)
        };
        // 7 PM and 11:59 PM on the 8th in New York are past midnight UTC.
        assert_eq!(state_of(&usage, at("2026-11-09T00:30:00Z"), true), "active");
        assert_eq!(state_of(&usage, at("2026-11-09T04:59:00Z"), true), "active");
        // 12:01 AM on the 9th in New York.
        assert_eq!(
            state_of(&usage, at("2026-11-09T05:01:00Z"), true),
            "expired"
        );
        // East of UTC the local day starts earlier: Tokyo, UTC+9.
        let tokyo = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        let early = DateTime::parse_from_rfc3339("2026-11-08T14:59:00Z")
            .unwrap()
            .with_timezone(&tokyo);
        assert_eq!(state_of(&usage, early, true), "active");
        let late = DateTime::parse_from_rfc3339("2026-11-08T15:01:00Z")
            .unwrap()
            .with_timezone(&tokyo);
        assert_eq!(state_of(&usage, late, true), "expired");
    }

    #[test]
    fn a_quiet_turn_writes_nothing_and_a_recall_never_stales_a_click() {
        let tmp = tmp();
        let v = tmp.path();
        fixture(v);
        let at = path(v).unwrap();
        let seen = observed(v);
        assert_eq!(seen.len(), 1);
        assert!(
            record_at(&at, &seen, &[], None, Utc::now()).unwrap(),
            "first sight"
        );
        let bytes = std::fs::read(&at).unwrap();
        assert!(!record_at(&at, &seen, &[], None, Utc::now()).unwrap());
        assert_eq!(
            std::fs::read(&at).unwrap(),
            bytes,
            "no write when unchanged"
        );
        let snapshot = view(v).unwrap().entries.remove(0);
        let recalled = crate::vault_learning::recall_with(v, "CSV reports", &[], None);
        assert_eq!(recalled.items.len(), 1);
        assert_eq!(recalled.observed, seen, "recall hands over what it listed");
        assert!(record_at(&at, &recalled.observed, &recalled.items, None, Utc::now()).unwrap());
        assert_eq!(view(v).unwrap().entries[0].uses, 1);
        let after = edit(v, &snapshot.path, &snapshot.expected_hash, "pin", None).unwrap();
        assert!(
            after.entries[0].pinned,
            "a recall since Library opened is no conflict"
        );
    }

    #[test]
    fn a_blank_valid_through_clears_the_expiry() {
        let tmp = tmp();
        let v = tmp.path();
        fixture(v);
        let first = view(v).unwrap().entries.remove(0);
        let dated = edit(
            v,
            &first.path,
            &first.expected_hash,
            "expiry",
            Some("2099-01-31"),
        )
        .unwrap();
        assert_eq!(dated.entries[0].expires_on.as_deref(), Some("2099-01-31"));
        let cleared = edit(
            v,
            &first.path,
            &dated.entries[0].expected_hash,
            "expiry",
            Some(""),
        )
        .unwrap();
        assert_eq!(cleared.entries[0].expires_on, None);
        assert!(edit(
            v,
            &first.path,
            &cleared.entries[0].expected_hash,
            "expiry",
            Some("2026-13-01")
        )
        .is_err());
    }

    #[test]
    fn quiet_usage_stales_then_archives_without_deleting_and_pin_keeps_it() {
        let start = now();
        let mut usage = fresh(start);
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(13), true),
            "active"
        );
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(14), true),
            "stale"
        );
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(30), true),
            "archived"
        );
        usage.pinned = true;
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(300), true),
            "active"
        );
        usage.expires_on = Some("2026-11-08".into());
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(1), true),
            "expired"
        );
        usage.expires_on = None;
        usage.pinned = false;
        usage.restored_at = Some((start + chrono::Duration::days(40)).to_rfc3339());
        assert_eq!(
            state_of(&usage, start + chrono::Duration::days(41), true),
            "active"
        );
        assert_eq!(
            state_of(&fresh(start), start + chrono::Duration::days(300), false),
            "active"
        );
    }

    #[test]
    fn damaged_usage_and_invalid_dates_are_left_byte_exact() {
        let tmp = tmp();
        let at = tmp.path().join("usage.json");
        let bytes = b"{unfinished";
        std::fs::write(&at, bytes).unwrap();
        assert!(load(&at).is_err());
        assert_eq!(std::fs::read(&at).unwrap(), bytes);
        assert!(parse_day("2026-02-30").is_err());
        assert!(parse_day("2026-1-1").is_err());
        let mut state = State::default();
        state.entries.insert("learnings/a.md".into(), fresh(now()));
        save(&at, &state).unwrap();
        assert!(load(&at).is_ok());
    }

    fn fixture(vault: &Path) -> String {
        std::fs::create_dir_all(vault.join("learnings")).unwrap();
        let raw="---\ntype: learned-context\ntitle: CSV delimiter\ntarget_kind: goal\ntarget_path: goals/report.md\nsubject: CSV reports\ntrust: approved\nproposal_id: review-fixture\nlearned_at: 2026-10-01T00:00:00Z\n---\n\nCSV reports preserve their declared delimiter.\n";
        std::fs::write(vault.join("learnings/csv.md"), raw).unwrap();
        raw.into()
    }

    #[test]
    fn a_cloned_fact_ages_but_stays_searchable_and_restore_keeps_both_files() {
        let source = tmp();
        let original = fixture(source.path());
        let clone = tmp();
        std::fs::create_dir_all(clone.path().join("learnings")).unwrap();
        std::fs::copy(
            source.path().join("learnings/csv.md"),
            clone.path().join("learnings/csv.md"),
        )
        .unwrap();
        let v = clone.path();
        let at = path(v).unwrap();
        let contexts = observed(v);
        assert_eq!(contexts.len(), 1, "the walk sees the temporary vault");
        record_at(
            &at,
            &contexts,
            &[],
            None,
            Utc::now() - chrono::Duration::days(45),
        )
        .unwrap();
        assert_eq!(view(v).unwrap().entries[0].status, "archived");
        assert!(
            crate::vault_learning::recall_with(v, "CSV reports", &[], None)
                .items
                .is_empty()
        );
        let history = crate::vault_learning::recall_with(v, "CSV reports", &[], Some("history"));
        assert_eq!(history.items.len(), 1);
        assert!(history.prompt_block.contains("historical use only"));
        record_at(&at, &contexts, &history.items, None, Utc::now()).unwrap();
        assert_eq!(
            view(v).unwrap().entries[0].status,
            "archived",
            "search does not restore it"
        );
        let current = view(v).unwrap().entries.remove(0);
        edit(v, &current.path, &current.expected_hash, "restore", None).unwrap();
        assert_eq!(view(v).unwrap().entries[0].status, "active");
        assert_eq!(
            crate::vault_learning::recall(v, "CSV reports", &[])
                .items
                .len(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(v.join("learnings/csv.md")).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("learnings/csv.md")).unwrap(),
            original
        );
    }

    #[test]
    fn first_click_is_stable_but_a_later_edit_refuses_the_old_snapshot() {
        let tmp = tmp();
        let v = tmp.path();
        let original = fixture(v);
        let first = view(v).unwrap().entries.remove(0);
        assert_eq!(
            first.expected_hash,
            view(v).unwrap().entries[0].expected_hash
        );
        let after = edit(v, &first.path, &first.expected_hash, "pin", None).unwrap();
        assert!(after.entries[0].pinned);
        let bytes = std::fs::read(path(v).unwrap()).unwrap();
        assert!(edit(v, &first.path, &first.expected_hash, "unpin", None)
            .unwrap_err()
            .contains("changed"));
        assert_eq!(std::fs::read(path(v).unwrap()).unwrap(), bytes);
        let current = &after.entries[0];
        let expired = (Local::now().date_naive() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let after = edit(
            v,
            &current.path,
            &current.expected_hash,
            "expiry",
            Some(&expired),
        )
        .unwrap();
        assert_eq!(after.entries[0].status, "expired");
        assert_eq!(
            std::fs::read_to_string(v.join("learnings/csv.md")).unwrap(),
            original
        );
        let current = &after.entries[0];
        std::fs::write(
            v.join("learnings/csv.md"),
            original.replace("delimiter.", "separator."),
        )
        .unwrap();
        assert!(edit(v, &current.path, &current.expected_hash, "restore", None).is_err());
    }

    /// A vault with one profile note and one workspace (J2's line homes).
    fn line_vault() -> tempfile::TempDir {
        let dir = tmp();
        let v = dir.path();
        std::fs::create_dir_all(v.join("me")).unwrap();
        std::fs::write(
            v.join("me/Guardrails.md"),
            "# Guardrails\n\n- Never delete outside the vault.\n",
        )
        .unwrap();
        std::fs::create_dir_all(v.join("workspaces/w1")).unwrap();
        std::fs::write(
            v.join("workspaces/w1/WORKSPACE.md"),
            "# W1\n\n## Decisions\n\n- Keep it local.\n\n## History\n\n- Started.\n",
        )
        .unwrap();
        dir
    }

    fn accept_line(v: &Path, kind: &str, target: &str, statement: &str) -> String {
        use crate::vault_learning::*;
        let pending = submit_proposal(
            v,
            LearningProposalPayload {
                kind: "preference".into(),
                target_kind: kind.into(),
                target_path: target.into(),
                subject: "subject".into(),
                statement: statement.into(),
                rationale: "Said so.".into(),
                evidence_excerpt: "the person said so".into(),
                source_run_id: Some("run-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let saved = resolve_proposal(v, &pending.id, "accept", None).unwrap();
        assert!(saved.payload.learned_line.is_some(), "a line home");
        saved.id
    }

    fn retires(v: &Path) -> Vec<crate::vault_learning::LearningProposal> {
        crate::vault_learning::list_proposals(v)
            .unwrap()
            .into_iter()
            .filter(|p| p.payload.retires.is_some())
            .collect()
    }

    fn entry(view: &View, key: &str) -> Option<Entry> {
        view.entries.iter().find(|e| e.key == key).cloned()
    }

    fn yesterday() -> String {
        (Local::now().date_naive() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn a_profile_line_past_its_date_is_proposed_and_changes_only_when_saved() {
        let tmp = line_vault();
        let v = tmp.path();
        let id = accept_line(v, "operator", "me/Guardrails.md", "Use Outlook, not Gmail.");
        let key = format!("line:{id}");
        let first = entry(&refresh(v).unwrap(), &key).expect("the line is listed");
        assert_eq!(
            (first.kind.as_str(), first.status.as_str()),
            ("line", "active")
        );
        assert!(!first.ages, "profile rules never age");
        let saved = std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap();
        assert!(saved.contains("- Use Outlook, not Gmail.\n"));
        let dated = edit(v, &key, &first.expected_hash, "expiry", Some(&yesterday())).unwrap();
        let now = entry(&dated, &key).unwrap();
        assert_eq!(now.status, "expired");
        let open = retires(v);
        assert_eq!(open.len(), 1);
        assert_eq!(now.proposal_id.as_deref(), Some(open[0].id.as_str()));
        assert_eq!(open[0].status, "pending");
        assert_eq!(open[0].payload.statement, "Use Outlook, not Gmail.");
        assert_eq!(open[0].payload.retires.as_ref().unwrap().proposal, id);
        assert_eq!(
            std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
            saved,
            "filing the proposal changes no line"
        );
        refresh(v).unwrap();
        assert_eq!(retires(v).len(), 1, "one proposal per lapse");

        let batch =
            crate::vault_learning::resolve_batch(v, &[open[0].id.clone()], "accept", None).unwrap();
        assert!(batch.items[0].ok, "{:?}", batch.items[0].error);
        let after = std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap();
        assert_eq!(after, "# Guardrails\n\n- Never delete outside the vault.\n");
        assert!(
            entry(&refresh(v).unwrap(), &key).is_none(),
            "the line is gone"
        );
        crate::vault_learning::undo_batch(v, batch.batch_id.as_deref().unwrap()).unwrap();
        assert_eq!(
            std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
            saved,
            "Undo puts the line back"
        );
        assert_eq!(retires(v)[0].status, "pending");
    }

    #[test]
    fn a_line_edited_by_hand_is_shown_changed_and_left_alone() {
        let tmp = line_vault();
        let v = tmp.path();
        let id = accept_line(v, "operator", "me/Guardrails.md", "Use Outlook, not Gmail.");
        let key = format!("line:{id}");
        let first = entry(&refresh(v).unwrap(), &key).unwrap();
        edit(v, &key, &first.expected_hash, "expiry", Some(&yesterday())).unwrap();
        let open = retires(v).remove(0);
        let by_hand =
            "# Guardrails\n\n- Never delete outside the vault.\n- Use Outlook for work mail.\n";
        std::fs::write(v.join("me/Guardrails.md"), by_hand).unwrap();
        let now = entry(&refresh(v).unwrap(), &key).unwrap();
        assert_eq!(now.status, "changed");
        assert!(edit(v, &key, &now.expected_hash, "pin", None)
            .unwrap_err()
            .contains("by hand"));
        let refused =
            crate::vault_learning::resolve_proposal(v, &open.id, "accept", None).unwrap_err();
        assert!(refused.contains("by hand"), "{refused}");
        assert_eq!(
            std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
            by_hand,
            "the hand edit stays byte-exact"
        );
    }

    #[test]
    fn a_quiet_project_line_is_proposed_and_a_reworded_save_replaces_it_in_place() {
        let tmp = line_vault();
        let v = tmp.path();
        let id = accept_line(
            v,
            "workspace",
            "workspaces/w1/WORKSPACE.md",
            "Ship on Fridays.",
        );
        let key = format!("line:{id}");
        let at = path(v).unwrap();
        // Seen 20 days ago, with the workspace's last chat the same day.
        let then = Utc::now() - chrono::Duration::days(20);
        let mut state = load(&at).unwrap();
        state.entries.insert(key.clone(), fresh(then));
        save(&at, &state).unwrap();
        assert!(record_at(&at, &[], &[], Some("w1"), then).unwrap());
        assert!(
            !record_at(&at, &[], &[], Some("w1"), then).unwrap(),
            "one chat day is written once"
        );
        let stale = entry(&refresh(v).unwrap(), &key).unwrap();
        assert_eq!(stale.status, "stale", "a line still read never archives");
        assert!(stale.ages);
        assert_eq!(stale.uses, 1);
        let open = retires(v).remove(0);
        assert!(open.payload.rationale.contains("No chat in this workspace"));
        let saved = crate::vault_learning::resolve_proposal(
            v,
            &open.id,
            "accept",
            Some("Ship on Mondays."),
        )
        .unwrap();
        assert_eq!(
            saved.payload.learned_line.as_deref(),
            Some("Ship on Mondays.")
        );
        assert_eq!(
            std::fs::read_to_string(v.join("workspaces/w1/WORKSPACE.md")).unwrap(),
            "# W1\n\n## Decisions\n\n- Keep it local.\n- Ship on Mondays.\n\n## History\n\n- Started.\n"
        );
        let view = refresh(v).unwrap();
        assert!(entry(&view, &key).is_none(), "the old line is retired");
        let reworded = entry(&view, &format!("line:{}", open.id)).expect("the new line");
        assert_eq!(reworded.status, "active", "a new grace period");
        // A chat in the workspace today keeps its lines active.
        assert!(record_at(&at, &[], &[], Some("w1"), Local::now()).unwrap());
        assert_eq!(
            entry(&refresh(v).unwrap(), &format!("line:{}", open.id))
                .unwrap()
                .status,
            "active"
        );
    }

    fn guardrails(v: &Path) -> String {
        std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap()
    }

    /// A saved Guardrails line past its date, and its waiting proposal.
    fn due_line(v: &Path, statement: &str) -> crate::vault_learning::LearningProposal {
        let id = accept_line(v, "operator", "me/Guardrails.md", statement);
        let key = format!("line:{id}");
        let first = entry(&refresh(v).unwrap(), &key).unwrap();
        edit(v, &key, &first.expected_hash, "expiry", Some(&yesterday())).unwrap();
        retires(v).remove(0)
    }

    /// As if the app stopped after a Save changed its line but before the
    /// queue recorded it (or that queue write failed): the item is left
    /// `applying` from long ago, which the queue's next read recovers.
    fn interrupt(v: &Path, id: &str) {
        let at = v.join(crate::review_queue::FILE_NAME);
        let mut queue: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&at).unwrap()).unwrap();
        let item = queue["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|item| item["id"] == id)
            .unwrap();
        item["status"] = "applying".into();
        item["updatedAt"] = "2020-01-01T00:00:00Z".into();
        std::fs::write(&at, serde_json::to_vec_pretty(&queue).unwrap()).unwrap();
    }

    /// PR #1239 review: case is meaning. A Save that only changes the case
    /// rewords the line in place; it never takes the line out.
    #[test]
    fn a_case_only_rewording_keeps_the_line_reworded() {
        let tmp = line_vault();
        let v = tmp.path();
        let open = due_line(v, "Use redis.");
        let done = crate::vault_learning::resolve_batch_edited(
            v,
            std::slice::from_ref(&open.id),
            "accept",
            None,
            Some("Use  Redis. "),
        )
        .unwrap();
        assert!(done.items[0].ok, "{:?}", done.items[0].error);
        assert_eq!(
            guardrails(v),
            "# Guardrails\n\n- Never delete outside the vault.\n- Use Redis.\n"
        );
        let saved = retires(v).remove(0);
        assert_eq!(saved.payload.learned_line.as_deref(), Some("Use Redis."));
    }

    /// PR #1239 review: a row's Save with edited words goes as a one-item
    /// batch (what Learnings sends), so its Undo snackbar and "Undo last
    /// batch" put the original words back in place.
    #[test]
    fn a_reworded_row_save_is_undone_through_its_batch() {
        use crate::vault_learning::{resolve_batch_edited, undo_batch};
        let tmp = line_vault();
        let v = tmp.path();
        let open = due_line(v, "Use Outlook, not Gmail.");
        let saved = guardrails(v);
        let one = std::slice::from_ref(&open.id);
        // Edited words go only with the Save of one learning.
        let refused = resolve_batch_edited(v, one, "reject", None, Some("New words.")).unwrap_err();
        assert!(refused.contains("Save of one learning"), "{refused}");
        let two = [
            open.id.clone(),
            open.payload.retires.clone().unwrap().proposal,
        ];
        let refused =
            resolve_batch_edited(v, &two, "accept", None, Some("New words.")).unwrap_err();
        assert!(refused.contains("Save of one learning"), "{refused}");
        assert_eq!(guardrails(v), saved, "a refused request changes nothing");

        let done = resolve_batch_edited(v, one, "accept", None, Some("Use Outlook for all mail."))
            .unwrap();
        assert!(done.items[0].ok, "{:?}", done.items[0].error);
        assert_eq!(
            guardrails(v),
            "# Guardrails\n\n- Never delete outside the vault.\n- Use Outlook for all mail.\n"
        );
        let batch = done.batch_id.expect("a row's Save has an Undo");
        undo_batch(v, &batch).unwrap();
        assert_eq!(guardrails(v), saved, "Undo puts the old words back");
        assert_eq!(retires(v)[0].status, "pending");
    }

    /// PR #1239 review: a Save interrupted after it changed the line keeps
    /// its claim as a receipt. Saving again finishes it (before, the retry
    /// called the missing original a hand edit), and Undo restores the line.
    #[test]
    fn an_interrupted_retire_finishes_on_retry_and_undo_restores_the_line() {
        use crate::vault_learning::{resolve_batch, resolve_batch_edited, undo_batch};
        for edited in [None, Some("Use Outlook for all mail.")] {
            let tmp = line_vault();
            let v = tmp.path();
            let open = due_line(v, "Use Outlook, not Gmail.");
            let saved = guardrails(v);
            let one = std::slice::from_ref(&open.id);
            let first = resolve_batch_edited(v, one, "accept", None, edited).unwrap();
            assert!(first.items[0].ok, "{:?}", first.items[0].error);
            let changed = guardrails(v);
            assert!(!changed.contains("Gmail"));

            interrupt(v, &open.id);
            let recovered = retires(v).remove(0);
            assert_eq!(recovered.status, "pending", "{edited:?}");
            assert!(recovered.error.unwrap().contains("interrupted"));
            if edited.is_none() {
                // The line is already out: new words can't go in its place.
                let other =
                    resolve_batch_edited(v, one, "accept", None, Some("Other words.")).unwrap();
                let why = other.items[0].error.clone().unwrap_or_default();
                assert!(why.contains("took the line out"), "{why}");
                assert_eq!(retires(v)[0].status, "pending");
            }

            // The retry the row's Save sends: the words the claim kept.
            let retry = resolve_batch(v, one, "accept", None).unwrap();
            assert!(retry.items[0].ok, "{edited:?}: {:?}", retry.items[0].error);
            assert_eq!(guardrails(v), changed, "nothing is written twice");
            let done = retires(v).remove(0);
            assert_eq!(done.status, "accepted");
            assert_eq!(done.payload.learned_line.as_deref(), edited);

            undo_batch(v, retry.batch_id.as_deref().unwrap()).unwrap();
            assert_eq!(guardrails(v), saved, "{edited:?}: Undo puts the line back");
            assert_eq!(retires(v)[0].status, "pending");
        }
    }

    /// PR #1239 delta review: after an interrupted rewording (A to B), a
    /// retry with other words (C, or A, which takes the line out) is refused
    /// and leaves the receipt as it was. Overwriting it would let a second
    /// interruption strand the line at B, matching neither receipt, so no
    /// Save or Undo could get past it. A retry with B finishes, Undo puts A
    /// back, and a fresh Save can then reword.
    #[test]
    fn an_interrupted_rewording_refuses_other_words_until_it_is_finished() {
        use crate::vault_learning::{resolve_batch_edited, undo_batch};
        let tmp = line_vault();
        let v = tmp.path();
        let original = "Use Outlook, not Gmail.";
        let intended = "Use Outlook for all mail.";
        let open = due_line(v, original);
        let saved = guardrails(v);
        let one = std::slice::from_ref(&open.id);
        let first = resolve_batch_edited(v, one, "accept", None, Some(intended)).unwrap();
        assert!(first.items[0].ok, "{:?}", first.items[0].error);
        let changed = guardrails(v);
        assert!(changed.contains(intended));

        interrupt(v, &open.id);
        let receipt = retires(v).remove(0);
        assert_eq!(receipt.status, "pending");
        assert_eq!(receipt.payload.learned_line.as_deref(), Some(intended));
        let kept = serde_json::to_value(&receipt.payload).unwrap();
        for other in ["Use Outlook and Proton.", original] {
            let refused = resolve_batch_edited(v, one, "accept", None, Some(other)).unwrap();
            let why = refused.items[0].error.clone().unwrap_or_default();
            assert!(why.contains("Finish the earlier Save first"), "{why}");
            assert!(why.contains(intended), "{why}");
            assert_eq!(guardrails(v), changed, "{other}: the note is untouched");
            let after = retires(v).remove(0);
            assert_eq!(after.status, "pending");
            assert_eq!(
                serde_json::to_value(&after.payload).unwrap(),
                kept,
                "{other}: the receipt is unchanged"
            );
        }

        let retry = resolve_batch_edited(v, one, "accept", None, Some(intended)).unwrap();
        assert!(retry.items[0].ok, "{:?}", retry.items[0].error);
        assert_eq!(guardrails(v), changed, "nothing is written twice");
        assert_eq!(retires(v)[0].status, "accepted");
        undo_batch(v, retry.batch_id.as_deref().unwrap()).unwrap();
        assert_eq!(guardrails(v), saved, "Undo puts the original back");
        assert_eq!(retires(v)[0].status, "pending");

        let fresh = "Use Outlook and Proton.";
        let reworded = resolve_batch_edited(v, one, "accept", None, Some(fresh)).unwrap();
        assert!(reworded.items[0].ok, "{:?}", reworded.items[0].error);
        assert_eq!(
            guardrails(v),
            format!("# Guardrails\n\n- Never delete outside the vault.\n- {fresh}\n")
        );
        assert_eq!(retires(v)[0].payload.learned_line.as_deref(), Some(fresh));
    }

    #[test]
    fn keeping_a_due_line_closes_its_waiting_proposal() {
        let tmp = line_vault();
        let v = tmp.path();
        let id = accept_line(v, "operator", "me/Guardrails.md", "Use Outlook, not Gmail.");
        let key = format!("line:{id}");
        let first = entry(&refresh(v).unwrap(), &key).unwrap();
        let dated = edit(v, &key, &first.expected_hash, "expiry", Some(&yesterday())).unwrap();
        let due = entry(&dated, &key).unwrap();
        assert!(due.proposal_id.is_some());
        let kept = edit(v, &key, &due.expected_hash, "restore", None).unwrap();
        let now = entry(&kept, &key).unwrap();
        assert_eq!(
            (now.status.as_str(), now.proposal_id.as_deref()),
            ("active", None)
        );
        assert_eq!(retires(v)[0].status, "rejected", "superseded, not deleted");
        assert!(std::fs::read_to_string(v.join("me/Guardrails.md"))
            .unwrap()
            .contains("- Use Outlook, not Gmail.\n"));
    }

    #[test]
    fn an_agent_cannot_aim_a_proposal_at_a_saved_line() {
        let tmp = line_vault();
        let v = tmp.path();
        let id = accept_line(v, "operator", "me/Guardrails.md", "Use Outlook, not Gmail.");
        let sneaky = crate::vault_learning::submit_proposal(
            v,
            crate::vault_learning::LearningProposalPayload {
                kind: "preference".into(),
                target_kind: "operator".into(),
                target_path: "me/Guardrails.md".into(),
                subject: "s".into(),
                statement: "Use Outlook, not Gmail.".into(),
                retires: Some(crate::vault_learning::RetireLine {
                    proposal: id,
                    path: "me/Guardrails.md".into(),
                    line: "Use Outlook, not Gmail.".into(),
                    cycle: "expired:2000-01-01".into(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            sneaky.payload.retires.is_none(),
            "cleared on a chat's submit"
        );
    }

    #[test]
    fn core_growth_makes_a_proposal_without_shortening_the_profile() {
        let tmp = tmp();
        let v = tmp.path();
        std::fs::create_dir_all(v.join("me")).unwrap();
        let original = "A profile fact.\n".repeat(1500);
        std::fs::write(v.join("me/Guardrails.md"), &original).unwrap();
        let view = view(v).unwrap();
        assert_eq!(view.core_bytes, original.len());
        assert!(view.consolidation_proposal.is_some());
        assert_eq!(
            std::fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
            original
        );
    }
}
