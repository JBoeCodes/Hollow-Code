//! Waiting on you items as tracker issues (two-Macs T3b, 2026-11-15;
//! decision c5 of `workspaces/main/outputs/hollow-next-build-decisions-2026-11-14.md`).
//!
//! Hollow's Waiting on you list ([`crate::needs_you`]) stays where the
//! person reads and answers; each item that has waited a while also gets a
//! copy in the tracker: one issue, assigned to the signed-in user (the
//! adapter's viewer, never a fixed id), labelled `needs-review`. When the
//! item is answered, dismissed or done in Hollow, the issue gets one comment
//! saying how (host copy, redacted) and moves to its team's completed state.
//!
//! Which items: questions, ask-canvases, review items, results to review
//! and sync conflicts ([`MIRRORED_KINDS`]) that appeared after the current
//! mode began (no backlog) and have waited [`MIRROR_AFTER_MINUTES`].
//! Approval cards (on screen for minutes), failures (notifications), the
//! restart offer and conflict copies stay in Hollow only.
//!
//! Where (T1's rule, [`super::chat_work::destination`]): an item in a
//! workspace that names its tracker project involves the team, so it goes
//! to team ENG in that project with a generic title and a close comment that
//! carries nothing of the item; any other (a learning, a personal decision,
//! a question in quick chats) is the person's and Hollow's, team PER, with
//! the item's own title, redacted.
//!
//! Settings' "Track agent work" gates it as it gates T3a: `off` nothing,
//! `dryRun` one audit row per item ("Dry run: ..."), `on` a link
//! (`work_links`, `linked_by` `waiting`, key `waiting:<item id>`) and its
//! facts for the agent work outbox ([`super::work_outbox`]). The item's id
//! is the idempotency key: its link, a dry run's row or a failed create in
//! the last hour decides it, so a retry, a restart or a re-render never
//! files a second issue; a refusal decides nothing. The close is queued once
//! (`outcome_at`), only for a link Hollow created whose team has a completed
//! state, after a positive signal or two complete passes without the item,
//! and the outbox closes only that issue.

use super::chat_work::{self, Running};
use super::linear;
use super::work::{self, WorkEvent, WorkLink};
use crate::config::Config;
use crate::needs_you::NeedsYouItem;
use crate::tracker::settings::AgentWork;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::Path;

/// The Waiting on you kinds that get a tracker copy.
pub(crate) const MIRRORED_KINDS: [&str; 5] = ["question", "canvas", "review", "result", "conflict"];
/// An item gets its copy once it has waited this long, so one answered
/// while the person is at the screen never makes an issue.
pub(crate) const MIRROR_AFTER_MINUTES: i64 = 10;
/// New copies decided in one pass (a long list trickles in).
pub(crate) const MAX_NEW: usize = 8;
/// The label every copy carries (T1 keeps it in AI and HOL).
pub(crate) const LABEL: &str = "needs-review";
/// Link keys of item copies start with this.
pub(crate) const KEY_PREFIX: &str = "waiting:";

/// A Waiting on you or commitment copy's link key (T3b, T3e): the outbox
/// resends such a create that never landed.
pub(crate) fn is_copy_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX) || key.starts_with(super::commitment_work::KEY_PREFIX)
}

/// The copy's link key: the item's own id.
pub(crate) fn link_key(item_id: &str) -> String {
    format!("{KEY_PREFIX}{item_id}")
}

/// The copy's create key (audit and create journal), from the item's id.
pub(crate) fn create_key(item_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("waiting\n{item_id}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("work-{}", &hex[..32])
}

/// What one reconcile did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Reconciled {
    /// Copies queued (on) or recorded as a dry run.
    pub filed: usize,
    /// Closes queued.
    pub closed: usize,
}

/// Where the current mode's start is kept (sessions store `derived_state`,
/// one row for the vault): `{"mode": "on"|"dryRun"|"off", "since": ...}`.
const SINCE_KEY: &str = "s-hollow-waiting-copies";
const SINCE_KIND: &str = "t3b-since";
/// When each changed result was first seen waiting (`{item id: time}`),
/// beside the mode's start: a changed result's own time is its file's
/// modified time, which a touch of an old file would make look new.
const SEEN_KIND: &str = "t3b-results-seen";
/// A failed copy (its create refused or given up) is tried again after this.
const RETRY_MINUTES: i64 = 60;
/// An item no store says ended closes only when absent on two complete
/// passes at least this far apart.
pub(crate) const ABSENT_SECONDS: i64 = 60;
/// The audit row's key when the mode's start can't be read or kept.
const SINCE_UNKNOWN: &str = "waiting-mode-start-unknown";
/// A copy's create whose recovery says it gave up for good (T3b): its audit
/// row's key suffix; such an item is never filed again.
pub(crate) const GAVE_UP: &str = "-gave-up";

type PerVault<T> =
    once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, T>>>;

/// Which copies a clock or an absence count belongs to.
pub(crate) const WAITING_FAMILY: &str = "waiting";

/// When each link was first seen absent on a complete pass.
pub(crate) type Absences = std::collections::HashMap<String, DateTime<Utc>>;

/// Links absent without a positive signal, per vault and family (Waiting on
/// you copies, commitments), with when that was first seen on a complete
/// pass. Cleared by an incomplete pass, a pass that isn't on, and a mode
/// change.
static ABSENT: PerVault<std::collections::HashMap<&'static str, Absences>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
/// The last mode start this process knew, per vault: used when the store
/// can't be read or written, so a failing store never restarts the clock.
static LAST_SINCE: PerVault<(AgentWork, DateTime<Utc>)> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) fn forget_absent(vault: &Path, family: &'static str) {
    if let Some(families) = ABSENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(vault)
    {
        families.remove(family);
    }
}

/// The family's absences from the last pass, taken out (put the ones still
/// counting back with [`keep_absent`]).
pub(crate) fn take_absent(vault: &Path, family: &'static str) -> Absences {
    ABSENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(vault)
        .and_then(|families| families.remove(family))
        .unwrap_or_default()
}

pub(crate) fn keep_absent(vault: &Path, family: &'static str, absences: Absences) {
    ABSENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(vault.to_path_buf())
        .or_default()
        .insert(family, absences);
}

/// The sync pass's call: read every mirrored kind now waiting and reconcile.
/// Local only (the sessions store and the audit); the outbox sends.
pub(crate) fn reconcile(config: &Config) -> Reconciled {
    let running = chat_work::running_of(config);
    if running.mode == AgentWork::Off {
        forget_absent(&config.vault_path, WAITING_FAMILY);
        // Only the setting itself moves the clock: a pass that reads off
        // because the tracker's scope is missing for a moment keeps it.
        if config.tracker.agent_work == AgentWork::Off {
            mode_since(&config.vault_path, AgentWork::Off, Utc::now());
        }
        return Reconciled::default();
    }
    // Tests never read the real app data folder.
    #[cfg(not(test))]
    let app_data = crate::agent_settings::app_data_dir().ok();
    #[cfg(test)]
    let app_data: Option<std::path::PathBuf> = None;
    let kinds: Vec<String> = MIRRORED_KINDS.iter().map(|k| k.to_string()).collect();
    let (items, unavailable) =
        crate::needs_you::waiting_now(&config.vault_path, app_data.as_deref(), &kinds);
    // Without the app data folder some ask-canvases are missing: never read
    // that as answered.
    let complete = unavailable.is_empty() && app_data.is_some();
    let ledger = work::read_ledger(config);
    let can_close = |link: &WorkLink| ledger.as_ref().is_some_and(|l| closable(l, link));
    reconcile_items(
        &config.vault_path,
        &running,
        &items,
        complete,
        Utc::now(),
        &can_close,
    )
}

/// The copy's issue is known to the ledger and its team has a completed
/// state to close it in.
pub(crate) fn closable(state: &crate::tracker_sync::Ledger, link: &WorkLink) -> bool {
    let Some(id) = link.issue_id.as_deref() else {
        return false;
    };
    let team = state
        .records
        .get(id)
        .and_then(|r| r.issue.as_ref())
        .or_else(|| {
            state
                .creates
                .values()
                .filter_map(|c| c.issue.as_ref())
                .find(|i| i.id == id)
        })
        .map(|i| i.team_id.clone());
    team.is_some_and(|t| super::work_outbox::completed_state(state, &t).is_some())
}

fn mode_name(mode: AgentWork) -> &'static str {
    match mode {
        AgentWork::Off => "off",
        AgentWork::DryRun => "dryRun",
        AgentWork::On => "on",
    }
}

/// When the current mode began, and whether it began just now: kept while
/// the mode stays, reset to `now` when the setting changes (off to dry run,
/// dry run to on), so only items that appeared after it are copied and a
/// backlog never is. A store that can't be read falls back to the last start
/// this process knew; with none, `None` (nothing is filed this pass). A
/// start that can't be saved is kept in memory, so the clock never restarts
/// every pass.
pub(crate) fn mode_since(
    vault: &Path,
    mode: AgentWork,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, bool)> {
    let name = mode_name(mode);
    let mut last = LAST_SINCE.lock().unwrap_or_else(|e| e.into_inner());
    let known = last.get(vault).filter(|(m, _)| *m == mode).map(|(_, t)| *t);
    let stored = match crate::session_store::derived_state(vault, SINCE_KEY, SINCE_KIND) {
        Ok(row) => row.and_then(|row| serde_json::from_str::<serde_json::Value>(&row.payload).ok()),
        Err(_) => return known.map(|t| (t, false)),
    };
    let kept = stored
        .as_ref()
        .filter(|v| v["mode"] == name)
        .and_then(|v| v["since"].as_str())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));
    if let Some(since) = kept {
        last.insert(vault.to_path_buf(), (mode, since));
        return Some((since, false));
    }
    // This process already started this mode but couldn't save it.
    let (since, fresh) = match known {
        Some(t) => (t, false),
        None => (now, true),
    };
    let payload = serde_json::json!({"mode": name, "since": since.to_rfc3339()}).to_string();
    let row = crate::session_store::DerivedRow {
        revision: 1,
        through_ord: 0,
        payload,
    };
    if let Err(error) = crate::session_store::put_derived_state(vault, SINCE_KEY, SINCE_KIND, &row)
    {
        eprintln!("[tracker waiting] mode start not recorded: {error}");
    }
    last.insert(vault.to_path_buf(), (mode, since));
    Some((since, fresh))
}

/// Changed results get their first-seen time instead of their file's: the
/// first time the store is kept, every result already waiting is marked as
/// waiting before any mode began (a backlog). `None` when it can't be read.
fn with_first_seen(
    vault: &Path,
    items: &[NeedsYouItem],
    now: DateTime<Utc>,
) -> Option<Vec<NeedsYouItem>> {
    let row = crate::session_store::derived_state(vault, SINCE_KEY, SEEN_KIND).ok()?;
    let first_time = row.is_none();
    let old: std::collections::BTreeMap<String, String> = row
        .and_then(|r| serde_json::from_str(&r.payload).ok())
        .unwrap_or_default();
    let backlog = "1970-01-01T00:00:00Z".to_string();
    let mut seen = std::collections::BTreeMap::new();
    let out = items
        .iter()
        .map(|item| {
            if item.kind != "result" || item.state != "changed" {
                return item.clone();
            }
            let at = old.get(&item.id).cloned().unwrap_or_else(|| {
                if first_time {
                    backlog.clone()
                } else {
                    now.to_rfc3339()
                }
            });
            seen.insert(item.id.clone(), at.clone());
            NeedsYouItem {
                at: Some(at),
                ..item.clone()
            }
        })
        .collect();
    if first_time || seen != old {
        let row = crate::session_store::DerivedRow {
            revision: 1,
            through_ord: 0,
            payload: serde_json::to_string(&seen).unwrap_or_else(|_| "{}".into()),
        };
        crate::session_store::put_derived_state(vault, SINCE_KEY, SEEN_KIND, &row).ok()?;
    }
    Some(out)
}

/// The current mode's start for one family's pass (shared by T3b and T3e):
/// clears the family's absence count when the pass isn't on, isn't complete
/// or the mode just began. `None` files nothing: off, or the start unknown
/// (one audit row says so).
pub(crate) fn clock(
    vault: &Path,
    running: &Running,
    complete: bool,
    now: DateTime<Utc>,
    family: &'static str,
) -> Option<DateTime<Utc>> {
    let since = mode_since(vault, running.mode, now);
    if running.mode != AgentWork::On || !complete || since.is_none_or(|(_, fresh)| fresh) {
        forget_absent(vault, family);
    }
    if running.mode == AgentWork::Off {
        return None;
    }
    let Some((since, _)) = since else {
        crate::audit::tracker_write(
            vault,
            crate::audit::Write {
                action: "create",
                session_key: Some(SINCE_UNKNOWN),
                asked_by: "hollow".into(),
                request_key: Some(SINCE_UNKNOWN),
                outcome: "skipped",
                detail: Some("Hollow couldn't read when copying Waiting on you items and commitments began, so it files none until it can.".into()),
                ..crate::audit::Write::default()
            },
        );
        return None;
    };
    Some(since)
}

/// File copies of items that entered, and close copies of items that left.
/// `complete` is false when any source could not be read: then nothing is
/// closed (a missing item may only be unread). `can_close` says whether the
/// copy's team has a completed state; without one the close waits, audited.
pub(crate) fn reconcile_items(
    vault: &Path,
    running: &Running,
    items: &[NeedsYouItem],
    complete: bool,
    now: DateTime<Utc>,
    can_close: &dyn Fn(&WorkLink) -> bool,
) -> Reconciled {
    let mut done = Reconciled::default();
    let Some(since) = clock(vault, running, complete, now, WAITING_FAMILY) else {
        return done;
    };
    let items = with_first_seen(vault, items, now).unwrap_or_else(|| {
        // Unreadable: changed results can't show when they appeared.
        items
            .iter()
            .filter(|i| i.kind != "result" || i.state != "changed")
            .cloned()
            .collect()
    });
    let mut new = 0;
    for item in items
        .iter()
        .filter(|i| mirrored(i) && appeared_after(i, since) && waited(i, now))
    {
        if new >= MAX_NEW {
            break;
        }
        let key = link_key(&item.id);
        let create = create_key(&item.id);
        if decided(vault, &key, &create, now) {
            continue;
        }
        if file(vault, running, item, &key, &create) {
            new += 1;
            done.filed += 1;
        }
    }
    if running.mode != AgentWork::On || !complete {
        return done;
    }
    let waiting: HashSet<String> = items.iter().map(|i| link_key(&i.id)).collect();
    let before = take_absent(vault, WAITING_FAMILY);
    let mut still_absent = Absences::new();
    for link in work::open_links_by(vault, work::linked_by::WAITING) {
        // A create not confirmed yet closes on a later pass; one whose fact
        // was let go (tracking turned off) never has an issue to close.
        if !link.created_issue || link.issue_id.is_none() || waiting.contains(&link.session_key) {
            continue;
        }
        let Some(item_id) = link.session_key.strip_prefix(KEY_PREFIX) else {
            continue;
        };
        // Team AI's copies say how it ended in words that carry nothing of
        // the item; HOL's may name the chosen option.
        let shared = !link
            .issue_key
            .as_deref()
            .is_some_and(|k| k.starts_with(&format!("{}-", linear::PERSONAL_TEAM_KEY)));
        // Snoozed, say: not answered yet, so the copy stays open.
        let Some(ended) = answer_of(vault, item_id, shared) else {
            continue;
        };
        let gate = Absence {
            before: &before,
            still: &mut still_absent,
            now,
        };
        if settle(vault, &link, &create_key(item_id), ended, gate, can_close) {
            done.closed += 1;
        }
    }
    keep_absent(vault, WAITING_FAMILY, still_absent);
    done
}

/// One pass's absence count for a family: last pass's, and this one's.
pub(crate) struct Absence<'a> {
    pub before: &'a Absences,
    pub still: &'a mut Absences,
    pub now: DateTime<Utc>,
}

/// Close one copy whose item or commitment ended (shared by T3b and T3e):
/// a positive signal closes now; an absence only on two complete passes at
/// least [`ABSENT_SECONDS`] apart. Without a completed state in the issue's
/// team the close waits, audited under `<create key>-no-completed-state`.
/// Returns whether a close was queued.
pub(crate) fn settle(
    vault: &Path,
    link: &WorkLink,
    create: &str,
    ended: Ended,
    gate: Absence<'_>,
    can_close: &dyn Fn(&WorkLink) -> bool,
) -> bool {
    // No store says so: absent on two complete passes a minute apart.
    if !ended.positive {
        let first = gate
            .before
            .get(&link.session_key)
            .copied()
            .unwrap_or(gate.now);
        if gate.now - first < chrono::Duration::seconds(ABSENT_SECONDS) {
            gate.still.insert(link.session_key.clone(), first);
            return false;
        }
    }
    if !can_close(link) {
        if let Some(first) = gate.before.get(&link.session_key) {
            gate.still.insert(link.session_key.clone(), *first);
        }
        crate::audit::tracker_write(
            vault,
            crate::audit::Write {
                action: "update",
                session_key: Some(&link.session_key),
                asked_by: "hollow".into(),
                issue_id: link.issue_id.as_deref(),
                issue_key: link.issue_key.as_deref(),
                request_key: Some(&format!("{create}-no-completed-state")),
                outcome: "skipped",
                detail: Some("The issue's team has no completed state Hollow can close it in, so the close waits and is tried again.".into()),
                ..crate::audit::Write::default()
            },
        );
        return false;
    }
    close(vault, link, ended.text)
}

fn mirrored(item: &NeedsYouItem) -> bool {
    MIRRORED_KINDS.contains(&item.kind.as_str())
}

fn item_time(item: &NeedsYouItem) -> Option<DateTime<Utc>> {
    item.at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc))
}

/// Appeared once the current mode had begun: an item with no time can't
/// show it did, so it is never copied (it stays in Hollow).
fn appeared_after(item: &NeedsYouItem, since: DateTime<Utc>) -> bool {
    item_time(item).is_some_and(|at| at >= since)
}

/// Waited long enough.
fn waited(item: &NeedsYouItem, now: DateTime<Utc>) -> bool {
    item_time(item).is_some_and(|at| now - at >= chrono::Duration::minutes(MIRROR_AFTER_MINUTES))
}

/// A link (filed, or closed), a dry run's row under the create key, a
/// create the outbox gave up on for good (`<create key>-gave-up`), or a
/// failed create there within the last hour. A refusal (team PER not set
/// up) is audited under its own key and decides nothing, so the item is
/// filed once HOL is; a failed create is tried again after an hour.
pub(crate) fn decided(vault: &Path, key: &str, create: &str, now: DateTime<Utc>) -> bool {
    use crate::audit::kind;
    let gave_up = format!("{create}{GAVE_UP}");
    let failed = crate::audit::rows_of(vault, key, kind::TRACKER_WRITE_FAILED);
    work::link_for(vault, key).is_some()
        || crate::audit::rows_of(vault, key, kind::TRACKER_WRITE)
            .iter()
            .any(|row| row.request_key.as_deref() == Some(create))
        || failed
            .iter()
            .any(|row| row.request_key.as_deref() == Some(gave_up.as_str()))
        || failed
            .iter()
            .filter(|row| row.request_key.as_deref() == Some(create))
            .any(|row| {
                DateTime::parse_from_rfc3339(&row.at).is_ok_and(|at| {
                    now - at.with_timezone(&Utc) < chrono::Duration::minutes(RETRY_MINUTES)
                })
            })
}

/// How the item reads in the issue: a kind, never its words.
fn kind_label(item: &NeedsYouItem) -> &'static str {
    match (item.kind.as_str(), item.category.as_deref()) {
        ("question", Some("confirmation")) => "confirmation",
        ("question", _) => "question",
        ("canvas", _) => "question in a canvas",
        ("review", _) => "review item",
        ("result", _) => "result to review",
        ("conflict", _) => "sync conflict",
        _ => "item",
    }
}

/// The issue's description: host facts only. The item's own text stays in
/// Hollow; the title is the item's own (redacted) in HOL only, a generic
/// "A <kind> waiting on you" in team ENG.
pub(crate) fn description(item: &NeedsYouItem) -> String {
    format!(
        "A {} is waiting on you in Hollow ({}). Answer it there, in Waiting on you; this issue closes on its own when it is answered, dismissed or done.\n\n+++ Agent notes\n\nCreated by Hollow as a copy of a Waiting on you item (Settings, Track agent work). Hollow stays where it is answered; it comments how it ended here and closes this issue. Kind {}; chat session {}.\n\n+++",
        kind_label(item),
        item.workspace_id
            .as_deref()
            .map_or_else(|| "no workspace".to_string(), |id| format!("workspace {id}")),
        item.kind,
        item.session_key.as_deref().unwrap_or("none"),
    )
}

/// Decide one entering item: its copy, filed through [`file_copy`].
fn file(vault: &Path, running: &Running, item: &NeedsYouItem, key: &str, create: &str) -> bool {
    let copy = Copy {
        key,
        create,
        workspace_id: item.workspace_id.as_deref(),
        linked_by: work::linked_by::WAITING,
        own_title: &item.title,
        generic_title: format!("A {} waiting on you", kind_label(item)),
        description: description(item),
        label: LABEL,
        due: None,
        what: format!("waiting {}", kind_label(item)),
        facts: serde_json::json!({"kind": item.kind, "chat": item.session_key}),
    };
    file_copy(vault, running, &copy)
}

/// One copy to file (T3b's Waiting on you item, T3e's commitment).
pub(crate) struct Copy<'a> {
    /// Link key (`waiting:<item id>`, `commitment:<id>`).
    pub key: &'a str,
    pub create: &'a str,
    pub workspace_id: Option<&'a str>,
    pub linked_by: &'static str,
    /// The item's own words: HOL's title only (redacted, at most 200).
    pub own_title: &'a str,
    /// Team AI's title (and HOL's when the own title is empty).
    pub generic_title: String,
    /// Host facts only.
    pub description: String,
    pub label: &'static str,
    /// `YYYY-MM-DD`.
    pub due: Option<String>,
    /// For the dry run's row: "waiting question", "commitment".
    pub what: String,
    /// More audit facts (an object, merged into the row's `after`).
    pub facts: serde_json::Value,
}

/// Decide one entering copy: refused (audited), a dry run (audited) or a
/// link and its create (on). Team by T1's rule ([`chat_work::destination`]);
/// team ENG is shared, so its issue carries only the generic title. Returns
/// whether it was filed or recorded.
pub(crate) fn file_copy(vault: &Path, running: &Running, copy: &Copy<'_>) -> bool {
    let folder = copy
        .workspace_id
        .and_then(|id| crate::workspaces::folder(vault, id).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let provider = if running.linear { linear::PROVIDER } else { "" };
    let place = work::place_in(provider, copy.workspace_id, &folder);
    let decision = chat_work::destination(&place, running);
    let (team_key, project) = match &decision {
        Ok(chat_work::Destination::Ai { project }) => (linear::TEAM_KEY, project.clone()),
        _ => (linear::PERSONAL_TEAM_KEY, String::new()),
    };
    // Team AI is shared: its issue says only what kind of thing waits. Only
    // HOL, the person's own team, gets the item's title (redacted).
    let title = match &decision {
        Ok(chat_work::Destination::Hol) => {
            let own = work::clip(
                crate::claude_runner::redact(copy.own_title.trim()).trim(),
                200,
            );
            if own.is_empty() {
                copy.generic_title.clone()
            } else {
                own
            }
        }
        _ => copy.generic_title.clone(),
    };
    let refused_key = format!("{}-refused", copy.create);
    let audit = |outcome: &str, detail: String, request: &str| {
        let mut after = serde_json::json!({
            "title": title, "team": team_key, "projectId": project, "label": copy.label,
        });
        if let (Some(after), Some(more)) = (after.as_object_mut(), copy.facts.as_object()) {
            after.extend(more.clone());
        }
        if let (Some(after), Some(due)) = (after.as_object_mut(), copy.due.as_ref()) {
            after.insert("due".into(), due.clone().into());
        }
        crate::audit::tracker_write(
            vault,
            crate::audit::Write {
                action: "create",
                session_key: Some(copy.key),
                asked_by: "hollow".into(),
                workspace_id: copy.workspace_id,
                after: Some(after),
                request_key: Some(request),
                outcome,
                detail: Some(detail),
                ..crate::audit::Write::default()
            },
        );
    };
    let dest = match decision {
        Ok(dest) => dest,
        Err(why) => {
            audit("failed", why, &refused_key);
            return false;
        }
    };
    if running.mode == AgentWork::DryRun {
        let due = copy
            .due
            .as_deref()
            .map(|d| format!(", due {d}"))
            .unwrap_or_default();
        audit(
            "skipped",
            format!(
                "Dry run: Hollow would file this {} as an issue in team {team_key}, assigned to you with {}{due}; nothing was sent.",
                copy.what, copy.label
            ),
            copy.create,
        );
        return true;
    }
    let (project_id, team_id) = match dest {
        chat_work::Destination::Ai { project } => (project, None),
        chat_work::Destination::Hol => (String::new(), Some(linear::PERSONAL_TEAM_ID.to_string())),
    };
    let link = WorkLink {
        session_key: copy.key.into(),
        issue_id: None,
        issue_key: None,
        workspace_id: copy.workspace_id.map(str::to_string),
        linked_by: copy.linked_by.into(),
        created_issue: true,
        at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        depth: 1,
        tracked: true,
        outcome_at: None,
        title: None,
    };
    let event = WorkEvent::Create {
        create_key: copy.create.into(),
        project_id,
        title,
        description: copy.description.clone(),
        team_id,
        labels: vec![copy.label.into()],
        assign_viewer: true,
        due: copy.due.clone(),
    };
    match work::insert_link_with(vault, &link, &[event]) {
        Ok(true) => {
            crate::tracker_sync::request_soon();
            true
        }
        Ok(false) => false,
        Err(error) => {
            eprintln!("[tracker copies] copy not recorded: {error}");
            false
        }
    }
}

/// Queue the close once: the answer comment, then Done.
fn close(vault: &Path, link: &WorkLink, answer: String) -> bool {
    let answer = work::clip(crate::claude_runner::redact(&answer).trim(), 300);
    match work::finish_link(vault, &link.session_key, &WorkEvent::Resolved { answer }) {
        Ok(queued) => queued,
        Err(error) => {
            eprintln!("[tracker waiting] close not queued: {error}");
            false
        }
    }
}

const GONE: &str = "No longer waiting in Hollow: answered, dismissed or done.";

/// How a vanished item ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ended {
    /// Host copy for the comment.
    pub text: String,
    /// Its own store says it ended (a question's or review item's status).
    /// Otherwise only its absence does, which must hold on two passes.
    pub positive: bool,
}

pub(crate) fn ended(text: impl Into<String>, positive: bool) -> Option<Ended> {
    Some(Ended {
        text: text.into(),
        positive,
    })
}

/// How a vanished item ended, as host copy; `None` while its store says it
/// is not settled (a snoozed review item). `shared` (a team ENG copy) never
/// names the chosen option.
pub(crate) fn answer_of(vault: &Path, item_id: &str, shared: bool) -> Option<Ended> {
    let (kind, source) = item_id.split_once(':')?;
    match kind {
        "question" => match crate::interactions::get(vault, source) {
            Ok(Some(card)) => match card.status.as_str() {
                "pending" | "stale" => None,
                "answered" if shared => ended("Answered in Hollow.", true),
                "answered" => ended(
                    format!(
                        "Answered in Hollow: {}.",
                        answer_summary(&card.request, card.answer.as_ref())
                    ),
                    true,
                ),
                "dismissed" => ended("Dismissed in Hollow.", true),
                other => ended(format!("Closed in Hollow ({}).", word(other)), true),
            },
            Ok(None) => ended(GONE, false),
            Err(_) => None,
        },
        "review" => match crate::review_queue::list(vault, None) {
            Ok(all) => match all
                .iter()
                .find(|r| r.id == source)
                .map(|r| r.status.as_str())
            {
                Some("pending" | "applying" | "snoozed") => None,
                Some("accepted") => ended("Accepted in Hollow.", true),
                Some("rejected") => ended("Rejected in Hollow.", true),
                Some(other) => ended(format!("Closed in Hollow ({}).", word(other)), true),
                None => ended(GONE, false),
            },
            Err(_) => None,
        },
        "result" => ended("Reviewed in Hollow.", false),
        "conflict" => ended("Resolved in Hollow.", false),
        "canvas" => ended("Answered in its chat in Hollow.", false),
        _ => ended(GONE, false),
    }
}

/// A status word as stored (letters and `_` only).
fn word(status: &str) -> String {
    status
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == '_')
        .take(24)
        .collect()
}

/// A short account of a question's answer: yes or no, the chosen options'
/// labels (the question's own option text), a count of verdicts. Anything
/// the person wrote in their own words is never quoted: it says a written
/// answer stays in Hollow.
pub(crate) fn answer_summary(
    request: &serde_json::Value,
    answer: Option<&serde_json::Value>,
) -> String {
    let Some(answer) = answer else {
        return "an answer".into();
    };
    let mut labels = std::collections::HashMap::new();
    option_labels(request, &mut labels);
    let mut parts: Vec<String> = Vec::new();
    match answer.get("accepted").and_then(|v| v.as_bool()) {
        Some(true) => parts.push("yes".into()),
        Some(false) => parts.push("no".into()),
        None => {}
    }
    let mut chosen: Vec<String> = Vec::new();
    let mut written = false;
    let mut collect = |value: &serde_json::Value| {
        for id in value
            .get("selected")
            .and_then(|s| s.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            chosen.push(labels.get(id).map_or(id, String::as_str).to_string());
        }
        written |= ["other", "text", "note"].iter().any(|k| {
            value
                .get(k)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        });
    };
    collect(answer);
    if let Some(each) = answer.get("answers").and_then(|a| a.as_object()) {
        for value in each.values() {
            collect(value);
        }
    }
    if !chosen.is_empty() {
        parts.push(chosen.join(", "));
    }
    if let Some(verdicts) = answer.get("verdicts").and_then(|v| v.as_object()) {
        if !verdicts.is_empty() {
            parts.push(format!("{} verdicts", verdicts.len()));
        }
    }
    if written {
        parts.push("a written answer (kept in Hollow)".into());
    }
    if parts.is_empty() {
        "an answer".into()
    } else {
        work::clip(&parts.join("; "), 200)
    }
}

/// Every `{id, label}` option in a stored question request.
fn option_labels(value: &serde_json::Value, out: &mut std::collections::HashMap<String, String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let (Some(id), Some(label)) = (
                map.get("id").and_then(|v| v.as_str()),
                map.get("label").and_then(|v| v.as_str()),
            ) {
                out.insert(id.to_string(), label.to_string());
            }
            for child in map.values() {
                option_labels(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                option_labels(child, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "waiting_work_tests.rs"]
mod tests;
