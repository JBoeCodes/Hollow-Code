//! Event triggers (COO rebuild F2): start work when something happens, not
//! on a clock. Three provider-neutral kinds, each event with a stable id:
//!
//! | kind | event | id | workspace |
//! |---|---|---|---|
//! | `tracker-issue` | a new issue in team AI, from the tracker sync's ledger (an id it never had, created after the trigger was turned on) | the provider's issue id | the workspace whose `PROJECT.md` names the issue's tracker project |
//! | `pr-merged` | a merged PR in a workspace's connected repository (its build binding's repository, read with `gh pr list`) | `owner/repo#N` | the workspaces bound to that repository |
//! | `recording-ended` | a Plaud recording listed, or a calendar meeting's end | `recording:<id>`, `meeting:<event id>@<end>` | none |
//!
//! **Off until turned on**, per kind and, for the first two, per workspace
//! (`set`); turning one on stamps `since`, and an event from before it never
//! fires. Recordings carry only a day, so the first clean Plaud read after
//! the kind is turned on records every listed one as seen without firing
//! ([`prime`]); until then no recording fires. An evicted seen entry raises
//! a per-kind high-water mark (`evicted_through`), so nothing at or before
//! it is decided again.
//! **Deduplicated** by event id in a persisted seen-set, the newest
//! [`SEEN_MAX`] per kind, so a restart never fires one twice. **Capped**: at
//! most `capPerHour` fires per kind in a rolling hour (default
//! [`DEFAULT_CAP`]); an event over the cap is recorded and dropped, never
//! queued for later. Only a fire that something is bound to counts, and a
//! merged PR bound to two projects counts once.
//!
//! **What runs** is the person's choice ([`bindings`]):
//! - an automation whose `triggers:` names the kind (`tracker-issue:<ws>`,
//!   `pr-merged:<ws>`, a bare kind for every workspace turned on, or
//!   `recording-ended`): a run-queue trigger row keyed by the event, whose
//!   `{{trigger_reason}}` quotes it. It runs like any trigger fire: the
//!   pump's budgets and CFO checks, and automations never create in the
//!   tracker.
//! - the default binding, "Wake the lead for new issues" (a project's menu,
//!   its own switch apart from the project's trigger, and only for issues
//!   created after it was turned on): a short unattended turn in that
//!   project's lead conversation quoting the issue. It is Hollow's own turn (`host_asker` [`ASKED_BY_EVENT`]), so it
//!   queues behind an open question card and never closes one, and the pump's
//!   CFO budget check holds it like any unattended turn. A lead that is busy
//!   keeps the wake pending (several issues join one turn), and so does a
//!   passing failure; a wake pending [`PENDING_HOURS`], or whose lead wake
//!   was turned off, is dropped and logged. A lead in a build workspace is
//!   never woken (unattended work never runs on a person's branch).
//!
//! Storage: `<support>/EventTriggers/<vault>.json` (app support, outside the
//! vault: History never diffs it). The live pass runs each minute from the
//! queue's owner ([`start`]); the tracker ledger is read when it changed,
//! `gh` and the recordings and calendar every [`SLOW_PASS_SECS`].

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Events remembered per kind (the seen-set's bound).
pub(crate) const SEEN_MAX: usize = 500;
/// Fires per kind in a rolling hour unless the person set another cap.
pub(crate) const DEFAULT_CAP: u32 = 6;
/// The highest cap the setting takes.
pub(crate) const MAX_CAP: u32 = 60;
/// A lead wake waiting on a busy lead longer than this is dropped.
pub(crate) const PENDING_HOURS: i64 = 6;
/// Issues one lead wake quotes, at most (the rest are counted).
const WAKE_QUOTED: usize = 8;
/// Seconds between `gh`, recordings and calendar reads.
pub(crate) const SLOW_PASS_SECS: i64 = 300;
/// A meeting that ended longer ago than this is never read as an event.
const MEETING_LOOKBACK_HOURS: i64 = 6;
/// The tag of the lead's woken turn (`ChatSendRequest::host_asker`).
pub(crate) const ASKED_BY_EVENT: &str = "hollow:event";
/// The one tracker team whose issues fire (team Engineering).
pub(crate) const TEAM: &str = crate::tracker::settings::CONFIGURE_TEAM.0;

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One kind of event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum EventKind {
    TrackerIssue,
    PrMerged,
    RecordingEnded,
}

impl EventKind {
    pub(crate) const ALL: [EventKind; 3] = [
        EventKind::TrackerIssue,
        EventKind::PrMerged,
        EventKind::RecordingEnded,
    ];

    pub(crate) fn token(self) -> &'static str {
        match self {
            EventKind::TrackerIssue => "tracker-issue",
            EventKind::PrMerged => "pr-merged",
            EventKind::RecordingEnded => "recording-ended",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            EventKind::TrackerIssue => "New issue in team AI",
            EventKind::PrMerged => "Merged PR in a connected repo",
            EventKind::RecordingEnded => "Recording or meeting ends",
        }
    }

    /// Turned on per workspace too (the first two kinds).
    pub(crate) fn per_workspace(self) -> bool {
        self != EventKind::RecordingEnded
    }

    pub(crate) fn parse(raw: &str) -> Option<EventKind> {
        Self::ALL
            .into_iter()
            .find(|k| k.token() == raw.trim().to_ascii_lowercase())
    }
}

/// One event, provider-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TriggerEvent {
    /// [`EventKind::token`].
    pub kind: String,
    /// Stable across reads and restarts.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// When it happened (RFC 3339): created, merged, ended.
    pub at: String,
    pub title: String,
    /// The issue key (`ENG-37`) or the PR (`owner/repo#12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The issue's mirror note in the vault (opens its detail in Hollow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

impl TriggerEvent {
    fn kind(&self) -> Option<EventKind> {
        EventKind::parse(&self.kind)
    }

    /// One line for a run's reason and the log.
    pub(crate) fn line(&self) -> String {
        let key = self
            .key
            .as_deref()
            .map(|k| format!("{k} "))
            .unwrap_or_default();
        let what = match self.kind() {
            Some(EventKind::TrackerIssue) => "New issue",
            Some(EventKind::PrMerged) => "PR merged",
            _ if self.id.starts_with("meeting:") => "Meeting ended",
            _ => "Recording ended",
        };
        let mut line = format!("{what}: {key}{}", clip(&self.title, 160));
        if let Some(url) = &self.url {
            line.push_str(&format!(" ({url})"));
        }
        line
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KindState {
    #[serde(default)]
    pub on: bool,
    /// When it was turned on (RFC 3339); None while off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_per_hour: Option<u32>,
    /// Recordings: a pass since it was turned on read Plaud and recorded
    /// every listed recording as seen ([`prime`]).
    #[serde(default)]
    pub primed: bool,
    /// The newest event time evicted from the seen-set: an event at or
    /// before it is never decided again, so an evicted id can't fire twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evicted_through: Option<String>,
}

/// One fire counted against the hourly cap: when, and its cap key (a merged
/// PR's `owner/repo#N`, so one PR bound to two projects counts once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FireStamp {
    pub at: String,
    pub key: String,
}

/// One event the pass decided about, newest last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Seen {
    pub id: String,
    /// When Hollow decided (RFC 3339).
    pub at: String,
    /// `fired`, `capped`, `unbound`, `before` (listed before it was on).
    pub outcome: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// What it started: automation slugs and `lead:<workspace>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub started: Vec<String>,
    /// When the event happened (its `at`): the high-water mark on eviction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_at: Option<String>,
}

/// A lead wake not yet queued (the lead was busy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingWake {
    pub workspace_id: String,
    pub event: TriggerEvent,
    pub since: String,
}

/// Everything saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct State {
    #[serde(default)]
    pub kinds: BTreeMap<String, KindState>,
    /// Kind token -> workspace id -> since.
    #[serde(default)]
    pub workspaces: BTreeMap<String, BTreeMap<String, String>>,
    /// Workspace id -> since: "Wake the lead for new issues".
    #[serde(default)]
    pub lead_wakes: BTreeMap<String, String>,
    #[serde(default)]
    pub seen: BTreeMap<String, Vec<Seen>>,
    /// Kind token -> fires in the last hour.
    #[serde(default)]
    pub fired: BTreeMap<String, Vec<FireStamp>>,
    #[serde(default)]
    pub pending: Vec<PendingWake>,
}

impl State {
    fn kind(&self, kind: EventKind) -> KindState {
        self.kinds.get(kind.token()).cloned().unwrap_or_default()
    }

    pub(crate) fn is_on(&self, kind: EventKind) -> bool {
        self.kind(kind).on
    }

    pub(crate) fn cap(&self, kind: EventKind) -> u32 {
        self.kind(kind)
            .cap_per_hour
            .unwrap_or(DEFAULT_CAP)
            .clamp(1, MAX_CAP)
    }

    fn workspace_since(&self, kind: EventKind, ws: &str) -> Option<DateTime<Utc>> {
        self.workspaces
            .get(kind.token())
            .and_then(|m| m.get(ws))
            .and_then(|at| parse(at))
    }

    fn was_seen(&self, kind: EventKind, id: &str) -> bool {
        self.seen
            .get(kind.token())
            .is_some_and(|all| all.iter().any(|s| s.id == id))
    }

    fn lead_since(&self, ws: &str) -> Option<DateTime<Utc>> {
        self.lead_wakes.get(ws).and_then(|at| parse(at))
    }

    /// Remember a decided event. The oldest past [`SEEN_MAX`] are evicted,
    /// and the newest event time among them (its decision time when it has
    /// none: missing an old event beats firing one twice), never later than
    /// the eviction itself, raises `evicted_through`.
    fn remember(&mut self, kind: EventKind, seen: Seen) {
        let now = parse(&seen.at).unwrap_or_else(Utc::now);
        let all = self.seen.entry(kind.token().to_string()).or_default();
        all.push(seen);
        let extra = all.len().saturating_sub(SEEN_MAX);
        let evicted = all
            .drain(..extra)
            .filter_map(|s| {
                s.event_at
                    .as_deref()
                    .and_then(parse)
                    .or_else(|| parse(&s.at))
            })
            .map(|at| at.min(now))
            .max();
        if let Some(newest) = evicted {
            let entry = self.kinds.entry(kind.token().to_string()).or_default();
            if entry
                .evicted_through
                .as_deref()
                .and_then(parse)
                .is_none_or(|old| newest > old)
            {
                entry.evicted_through = Some(newest.to_rfc3339());
            }
        }
    }

    /// The cap keys fired in the hour before `now`, each once.
    fn fired_keys(&self, kind: EventKind, now: DateTime<Utc>) -> Vec<&str> {
        let mut keys: Vec<&str> = self
            .fired
            .get(kind.token())
            .into_iter()
            .flatten()
            .filter(|f| parse(&f.at).is_some_and(|at| at > now - Duration::hours(1)))
            .map(|f| f.key.as_str())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    pub(crate) fn fired_last_hour(&self, kind: EventKind, now: DateTime<Utc>) -> u32 {
        self.fired_keys(kind, now).len() as u32
    }

    /// Turn a kind on or off. On stamps `since` (kept when it already was
    /// on); off forgets it, so turning it on again never fires older events.
    pub(crate) fn set_kind(&mut self, kind: EventKind, on: bool, now: DateTime<Utc>) {
        let entry = self.kinds.entry(kind.token().to_string()).or_default();
        if on && !entry.on {
            entry.on = true;
            entry.since = Some(now.to_rfc3339());
            entry.primed = false;
        } else if !on {
            entry.on = false;
            entry.since = None;
            entry.primed = false;
            // Its wakes go now, so off then on before the next pass never
            // delivers an issue from before.
            if kind == EventKind::TrackerIssue {
                self.pending.clear();
            }
        }
    }

    /// Turn one workspace on or off for a per-workspace kind; turning one on
    /// turns the kind on too.
    pub(crate) fn set_workspace(
        &mut self,
        kind: EventKind,
        ws: &str,
        on: bool,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        if !kind.per_workspace() {
            return Err(format!(
                "{} isn't turned on per project. Nothing changed.",
                kind.label()
            ));
        }
        let map = self.workspaces.entry(kind.token().to_string()).or_default();
        if on {
            map.entry(ws.to_string())
                .or_insert_with(|| now.to_rfc3339());
            self.set_kind(kind, true, now);
        } else {
            // The project's lead wake is its own switch: untouched.
            map.remove(ws);
        }
        Ok(())
    }

    /// A project's "Wake the lead for new issues": on turns the kind on;
    /// off forgets the wakes still pending for it.
    pub(crate) fn set_lead_wake(&mut self, ws: &str, on: bool, now: DateTime<Utc>) {
        if on {
            self.lead_wakes
                .entry(ws.to_string())
                .or_insert_with(|| now.to_rfc3339());
            self.set_kind(EventKind::TrackerIssue, true, now);
        } else {
            self.lead_wakes.remove(ws);
            self.pending.retain(|p| p.workspace_id != ws);
        }
    }

    pub(crate) fn set_cap(&mut self, kind: EventKind, cap: u32) {
        self.kinds
            .entry(kind.token().to_string())
            .or_default()
            .cap_per_hour = Some(cap.clamp(1, MAX_CAP));
    }
}

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn clip(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}...", one.chars().take(max).collect::<String>())
    }
}

fn path(vault: &Path) -> Option<PathBuf> {
    let stem = crate::session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(
            crate::test_temp::process_dir("hollow-test-event-triggers-")
                .join(format!("{stem}.json")),
        )
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|r| r.join("EventTriggers").join(format!("{stem}.json")))
    }
}

pub(crate) fn load(vault: &Path) -> State {
    path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(vault: &Path, state: &State) -> Result<(), String> {
    let p = path(vault).ok_or("No app data folder.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)
}

/// Read, change and save the state under the lock.
pub(crate) fn update<T>(
    vault: &Path,
    change: impl FnOnce(&mut State) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load(vault);
    let out = change(&mut state)?;
    save(vault, &state)?;
    Ok(out)
}

/// The pure decision for one kind's events: which fire now. Each event
/// decided is remembered (fired, capped or unbound), so it is never decided
/// again; one that doesn't qualify yet (off, its workspace off, from before
/// it was turned on, outside team AI) is left alone. `bound` says whether
/// anything would run for it.
pub(crate) fn decide(
    state: &mut State,
    kind: EventKind,
    events: &[TriggerEvent],
    now: DateTime<Utc>,
    bound: &dyn Fn(&TriggerEvent) -> Vec<String>,
) -> Vec<(TriggerEvent, Vec<String>)> {
    let ks = state.kind(kind);
    let Some(kind_since) = ks.on.then(|| ks.since.as_deref().and_then(parse)).flatten() else {
        return Vec::new();
    };
    let at_now = now.to_rfc3339();
    let mut ordered: Vec<&TriggerEvent> =
        events.iter().filter(|e| e.kind() == Some(kind)).collect();
    ordered.sort_by(|a, b| (parse(&a.at), &a.id).cmp(&(parse(&b.at), &b.id)));
    let cap = state.cap(kind);
    let mut out = Vec::new();
    for e in ordered {
        if state.was_seen(kind, &e.id) {
            continue;
        }
        if kind == EventKind::TrackerIssue && e.team_id.as_deref() != Some(TEAM) {
            continue;
        }
        // Recordings fire only once a successful read primed them.
        if kind == EventKind::RecordingEnded && !ks.primed && e.id.starts_with("recording:") {
            continue;
        }
        let mut since = kind_since;
        if kind.per_workspace() {
            // The project's trigger or (new issues) its lead wake; each
            // binding checks its own time again ([`bindings`]).
            let Some(ws_since) = e.workspace_id.as_deref().and_then(|ws| {
                let lead = (kind == EventKind::TrackerIssue)
                    .then(|| state.lead_since(ws))
                    .flatten();
                match (state.workspace_since(kind, ws), lead) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            }) else {
                continue;
            };
            since = since.max(ws_since);
        }
        let Some(at) = parse(&e.at) else {
            continue;
        };
        if state
            .kind(kind)
            .evicted_through
            .as_deref()
            .and_then(parse)
            .is_some_and(|hw| at <= hw)
        {
            continue;
        }
        let old = if kind == EventKind::RecordingEnded && !e.id.starts_with("meeting:") {
            // A recording has only its day.
            at.date_naive() < since.date_naive()
        } else {
            at < since
        };
        if old {
            continue;
        }
        let started = bound(e);
        let cap_key = match kind {
            EventKind::PrMerged => e.key.clone().unwrap_or_else(|| e.id.clone()),
            _ => e.id.clone(),
        };
        let keys = state.fired_keys(kind, now);
        let counted = keys.contains(&cap_key.as_str());
        let outcome = if started.is_empty() {
            "unbound"
        } else if !counted && keys.len() as u32 >= cap {
            "capped"
        } else {
            "fired"
        };
        state.remember(
            kind,
            Seen {
                id: e.id.clone(),
                at: at_now.clone(),
                outcome: outcome.into(),
                title: clip(&e.title, 160),
                workspace_id: e.workspace_id.clone(),
                started: if outcome == "fired" {
                    started.clone()
                } else {
                    Vec::new()
                },
                event_at: Some(e.at.clone()),
            },
        );
        if outcome == "fired" {
            state
                .fired
                .entry(kind.token().to_string())
                .or_default()
                .push(FireStamp {
                    at: at_now.clone(),
                    key: cap_key,
                });
            out.push((e.clone(), started));
        }
    }
    let hour_ago = now - Duration::hours(1);
    if let Some(all) = state.fired.get_mut(kind.token()) {
        all.retain(|f| parse(&f.at).is_some_and(|at| at > hour_ago));
    }
    out
}

/// Record every listed recording as seen without firing, once a pass read
/// Plaud successfully after the kind was turned on (a failed or skipped
/// read leaves it unprimed, so nothing listed before ever fires). Meetings
/// have an exact end and need no priming.
pub(crate) fn prime(state: &mut State, events: &[TriggerEvent], now: DateTime<Utc>) {
    let kind = EventKind::RecordingEnded;
    let ks = state.kind(kind);
    if !ks.on || ks.primed {
        return;
    }
    for e in events
        .iter()
        .filter(|e| e.kind() == Some(kind) && e.id.starts_with("recording:"))
    {
        if !state.was_seen(kind, &e.id) {
            state.remember(
                kind,
                Seen {
                    id: e.id.clone(),
                    at: now.to_rfc3339(),
                    outcome: "before".into(),
                    title: clip(&e.title, 160),
                    workspace_id: None,
                    started: Vec::new(),
                    event_at: Some(e.at.clone()),
                },
            );
        }
    }
    if let Some(k) = state.kinds.get_mut(kind.token()) {
        k.primed = true;
    }
}

/// One automation an event starts: its slug, title and the matching token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bound {
    pub slug: String,
    pub title: String,
    pub token: String,
}

/// The automations bound to `e`: those whose `triggers:` names its kind for
/// its workspace, or the bare kind.
pub(crate) fn workflow_bindings(
    workflows: &[crate::types::Workflow],
    e: &TriggerEvent,
) -> Vec<Bound> {
    use crate::trigger_evaluator::TriggerSpec;
    let ws = e.workspace_id.as_deref();
    let kind = e.kind();
    workflows
        .iter()
        // G2: an automation an end rule stopped (or is due to stop) takes no
        // events either.
        .filter(|w| {
            w.ended_note.is_none() && !crate::vault_workflows::end_passed(w, chrono::Utc::now())
        })
        .filter_map(|w| {
            TriggerSpec::parse_all(&w.triggers)
                .into_iter()
                .find(|spec| match (spec, kind) {
                    (TriggerSpec::TrackerIssue(only), Some(EventKind::TrackerIssue))
                    | (TriggerSpec::PrMerged(only), Some(EventKind::PrMerged)) => {
                        only.as_deref().is_none_or(|o| Some(o) == ws)
                    }
                    (TriggerSpec::RecordingEnded, Some(EventKind::RecordingEnded)) => true,
                    _ => false,
                })
                .map(|spec| Bound {
                    slug: w.slug.clone(),
                    title: w.title.clone(),
                    token: spec.token(),
                })
        })
        .collect()
}

/// Everything `e` starts: its automations (for a per-project kind, only
/// while the project's trigger is on and the event came after it was
/// turned on) and, for a new issue created after the project's lead wake
/// was turned on, `lead:<workspace>`.
pub(crate) fn bindings(
    state: &State,
    workflows: &[crate::types::Workflow],
    e: &TriggerEvent,
) -> Vec<String> {
    let Some(kind) = e.kind() else {
        return Vec::new();
    };
    let at = parse(&e.at);
    let after = |since: Option<DateTime<Utc>>| since.zip(at).is_some_and(|(s, at)| at >= s);
    let ws = e.workspace_id.as_deref();
    let project_on =
        !kind.per_workspace() || ws.is_some_and(|ws| after(state.workspace_since(kind, ws)));
    let mut out: Vec<String> = if project_on {
        workflow_bindings(workflows, e)
            .into_iter()
            .map(|b| b.slug)
            .collect()
    } else {
        Vec::new()
    };
    if kind == EventKind::TrackerIssue {
        if let Some(ws) = ws.filter(|ws| after(state.lead_since(ws))) {
            out.push(format!("lead:{ws}"));
        }
    }
    out
}

/// Queue what fired: a trigger row per bound automation (keyed by the event,
/// so a repeat is a duplicate) and a pending lead wake. Returns the queued
/// row ids.
pub(crate) fn dispatch(
    vault: &Path,
    state: &mut State,
    fired: &[(TriggerEvent, Vec<String>)],
    workflows: &[crate::types::Workflow],
    now: DateTime<Utc>,
) -> Vec<i64> {
    use crate::run_queue::{self, NewRow, RunSource};
    use crate::trigger_evaluator::FiredToken;
    let mut rows = Vec::new();
    for (e, started) in fired {
        for b in workflow_bindings(workflows, e)
            .into_iter()
            .filter(|b| started.contains(&b.slug))
        {
            let token = FiredToken {
                token: b.token.clone(),
                event_ids: vec![e.id.clone()],
                ..FiredToken::default()
            };
            let row = NewRow::new(RunSource::Trigger, Some(&b.slug), &b.title, &e.line())
                .with_origin(crate::work_origin::Request::scheduled_workflow())
                .with_payload(run_queue::trigger_payload(&[token]))
                .with_key(format!("event:{}:{}:{}", b.slug, e.kind, e.id));
            match run_queue::enqueue(vault, row, now) {
                Ok(enqueued) => rows.push(enqueued.id()),
                Err(error) => eprintln!(
                    "[event triggers] could not queue {} for {}: {error}",
                    b.slug, e.id
                ),
            }
        }
        if let Some(ws) = started.iter().find_map(|s| s.strip_prefix("lead:")) {
            state.pending.push(PendingWake {
                workspace_id: ws.to_string(),
                event: e.clone(),
                since: now.to_rfc3339(),
            });
        }
    }
    rows
}

/// The message a woken lead reads.
pub(crate) fn wake_prompt(events: &[&TriggerEvent]) -> String {
    let mut text = if events.len() == 1 {
        "Hollow event: a new issue in team AI for this project.\n".to_string()
    } else {
        format!(
            "Hollow event: {} new issues in team AI for this project.\n",
            events.len()
        )
    };
    for e in events.iter().take(WAKE_QUOTED) {
        text.push_str(&format!(
            "\n- {}: {}",
            e.key.as_deref().unwrap_or("Issue"),
            clip(&e.title, 200)
        ));
        if let Some(note) = &e.note {
            text.push_str(&format!("\n  Detail in Hollow: {note}"));
        }
        if let Some(url) = &e.url {
            text.push_str(&format!("\n  Link: {url}"));
        }
    }
    if events.len() > WAKE_QUOTED {
        text.push_str(&format!("\n- and {} more", events.len() - WAKE_QUOTED));
    }
    text.push_str("\n\nRead the issue, decide whether it needs work now, and start a subagent for it if so. Say in a line or two what you decided. Don't create tracker issues from this turn.");
    text
}

/// Why a lead wake didn't queue: worth another try next pass, or never.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WakeError {
    /// The lead is busy, or a read or lease failed: kept for the next pass.
    Retry(String),
    /// It can't ever be sent (no project or lead, a build workspace, a
    /// moved conversation, an unsafe folder): dropped and logged.
    Never(String),
}

impl WakeError {
    /// How `chat::accept_turn`'s refusal is read: the folder or lead
    /// conversation checks are permanent; the rest (already queued, a
    /// lease, a store read) pass.
    fn from_accept(error: String) -> WakeError {
        let permanent = error.starts_with("CHAT_CONTEXT_RECOVERED")
            || error.starts_with("UNSAFE_WORKING_DIRECTORY")
            || error.contains("lead conversation");
        if permanent {
            WakeError::Never(error)
        } else {
            WakeError::Retry(error)
        }
    }
}

/// The lead turn for workspace `ws` quoting `events`, or why there is none.
pub(crate) fn lead_request(
    vault: &Path,
    ws: &str,
    events: &[&TriggerEvent],
) -> Result<crate::chat::ChatSendRequest, WakeError> {
    let never = |text: &str| WakeError::Never(text.to_string());
    let workspace = crate::workspaces::list(vault)
        .map_err(WakeError::Retry)?
        .into_iter()
        .find(|w| w.id == ws)
        .ok_or_else(|| never("the project is gone"))?;
    if workspace.build.is_some() {
        return Err(never("a build workspace's lead is never woken unattended"));
    }
    let key = crate::chat_chain::lead_current(vault, ws)
        .ok_or_else(|| never("the project has no lead"))?;
    let session = crate::session_store::session_by_key(vault, &key).map_err(WakeError::Retry)?;
    let (cwd, agent, harness) = match &session {
        Some(s) => (
            PathBuf::from(&s.cwd),
            crate::agent::AgentKind::from_opt(&s.agent),
            s.harness_session_id.clone(),
        ),
        None => (PathBuf::from(&workspace.cwd), None, None),
    };
    if crate::build_workspaces::workspace_for_worktree(vault, &cwd).is_some() {
        return Err(never("a build workspace's lead is never woken unattended"));
    }
    let (agent, model) = match agent {
        Some(agent) => (
            agent,
            crate::wake_me::wake_model(vault, &key, agent.as_str()),
        ),
        None => crate::agent_settings::resolve(None, None).map_err(WakeError::Retry)?,
    };
    let last = crate::session_store::last_permission(vault, &key)
        .ok()
        .flatten();
    Ok(crate::chat::ChatSendRequest {
        prompt: wake_prompt(events),
        session_id: harness,
        session_key: Some(key),
        cwd,
        model,
        permission_mode: Some(crate::main_agent::wake_mode(last.as_deref())),
        agent,
        client_turn_id: None,
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: true,
        typed: false,
        host_asker: Some(ASKED_BY_EVENT),
        canned: None,
        worker: false,
    })
}

/// Queue each workspace's pending lead wakes as one turn. A wake whose
/// kind or lead wake is off now, or one pending over [`PENDING_HOURS`] (each
/// by its own age), is dropped; a busy lead or a passing failure keeps the
/// rest for the next pass; a permanent refusal drops them. Every drop is
/// logged on the event's seen entry. Returns the queued row ids.
pub(crate) fn deliver_pending(vault: &Path, state: &mut State, now: DateTime<Utc>) -> Vec<i64> {
    let mut queued = Vec::new();
    let mut by_ws: BTreeMap<String, Vec<PendingWake>> = BTreeMap::new();
    let mut dropped: Vec<(String, String)> = Vec::new();
    let kind_on = state.is_on(EventKind::TrackerIssue);
    for p in std::mem::take(&mut state.pending) {
        let why = if !kind_on || !state.lead_wakes.contains_key(&p.workspace_id) {
            Some("lead wake turned off".to_string())
        } else if parse(&p.since).is_none_or(|at| now - at > Duration::hours(PENDING_HOURS)) {
            Some(format!("waited over {PENDING_HOURS} hours for the lead"))
        } else {
            None
        };
        match why {
            Some(why) => dropped.push((p.event.id, why)),
            None => by_ws.entry(p.workspace_id.clone()).or_default().push(p),
        }
    }
    for (ws, waits) in by_ws {
        let events: Vec<&TriggerEvent> = waits.iter().map(|p| &p.event).collect();
        let result = lead_request(vault, &ws, &events).and_then(|req| {
            crate::chat::accept_turn(vault, req, now).map_err(WakeError::from_accept)
        });
        match result {
            Ok(accepted) => queued.push(accepted.queue_id),
            Err(WakeError::Retry(error)) => {
                if error != crate::chat::CHAT_ALREADY_QUEUED {
                    eprintln!("[event triggers] lead wake for {ws} waits: {error}");
                }
                state.pending.extend(waits);
            }
            Err(WakeError::Never(error)) => {
                eprintln!("[event triggers] lead wake for {ws} dropped: {error}");
                dropped.extend(waits.into_iter().map(|p| (p.event.id, error.clone())));
            }
        }
    }
    for (id, why) in dropped {
        if let Some(seen) = state
            .seen
            .get_mut(EventKind::TrackerIssue.token())
            .and_then(|all| all.iter_mut().rev().find(|s| s.id == id))
        {
            seen.outcome = format!("lead not woken: {}", clip(&why, 120));
        }
    }
    queued
}

// ── Sources ─────────────────────────────────────────────────────────────

/// New-issue events from the tracker ledger's issues: each issue of the
/// configured team, mapped to the workspace whose `PROJECT.md` names its
/// project. Only team AI's ever fire ([`decide`]).
pub(crate) fn issue_events(
    issues: &[(crate::tracker::TrackerIssue, String)],
    tracked: &[crate::tracker::status::Tracked],
) -> Vec<TriggerEvent> {
    issues
        .iter()
        .filter(|(i, _)| !i.trashed && i.archived_at.is_none())
        .map(|(i, note)| TriggerEvent {
            kind: EventKind::TrackerIssue.token().into(),
            id: i.id.clone(),
            workspace_id: i.project_id.as_deref().and_then(|p| {
                tracked
                    .iter()
                    .find(|t| t.project_id == p)
                    .map(|t| t.workspace_id.clone())
            }),
            at: i.created_at.clone(),
            title: i.title.clone(),
            key: Some(i.key.clone()).filter(|k| !k.is_empty()),
            url: Some(i.url.clone()).filter(|u| !u.is_empty()),
            note: Some(note.clone()).filter(|n| !n.is_empty()),
            team_id: Some(i.team_id.clone()),
        })
        .collect()
}

/// The ledger's issues and their mirror notes, when the tracker is set up.
fn ledger_issues(
    config: &crate::config::Config,
) -> Option<Vec<(crate::tracker::TrackerIssue, String)>> {
    let provider = config.tracker.provider.as_deref()?;
    let scope = config.tracker.team_id.as_deref()?;
    let ledger = crate::tracker_sync::load(&config.vault_path, provider, scope).ok()?;
    Some(
        ledger
            .records
            .into_values()
            .filter(|r| !r.remote_missing)
            .filter_map(|r| r.issue.map(|i| (i, r.path)))
            .collect(),
    )
}

/// `gh pr list --state merged` JSON into events for each bound workspace.
pub(crate) fn pr_events(json: &str, workspaces: &[String]) -> Vec<TriggerEvent> {
    let Ok(serde_json::Value::Array(prs)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pr in prs {
        let (Some(number), Some(url), Some(merged)) = (
            pr.get("number").and_then(|n| n.as_u64()),
            pr.get("url").and_then(|u| u.as_str()),
            pr.get("mergedAt").and_then(|m| m.as_str()),
        ) else {
            continue;
        };
        // https://github.com/<owner>/<repo>/pull/<n>
        let parts: Vec<&str> = url.trim_end_matches('/').rsplit('/').collect();
        let (Some(repo), Some(owner)) = (parts.get(2), parts.get(3)) else {
            continue;
        };
        let id = format!("{owner}/{repo}#{number}");
        for ws in workspaces {
            out.push(TriggerEvent {
                kind: EventKind::PrMerged.token().into(),
                id: format!("{id}@{ws}"),
                workspace_id: Some(ws.clone()),
                at: merged.to_string(),
                title: pr
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                key: Some(id.clone()),
                url: Some(url.to_string()),
                note: None,
                team_id: None,
            });
        }
    }
    out
}

/// Recording rows into events (`date` is a day only).
pub(crate) fn recording_events(rows: &[crate::recordings::RecordingRow]) -> Vec<TriggerEvent> {
    rows.iter()
        .filter_map(|r| {
            let day = chrono::NaiveDate::parse_from_str(r.date.as_deref()?, "%Y-%m-%d").ok()?;
            Some(TriggerEvent {
                kind: EventKind::RecordingEnded.token().into(),
                id: format!("recording:{}", r.id),
                workspace_id: None,
                at: day.and_hms_opt(12, 0, 0)?.and_utc().to_rfc3339(),
                title: r.name.clone(),
                key: None,
                url: None,
                note: None,
                team_id: None,
            })
        })
        .collect()
}

/// Calendar events (the Agenda's view) that ended by `now`, in the last
/// [`MEETING_LOOKBACK_HOURS`]: not cancelled, not all day.
pub(crate) fn meeting_events(view: &serde_json::Value, now: DateTime<Utc>) -> Vec<TriggerEvent> {
    let text = |e: &serde_json::Value, k: &str| {
        e.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let flag = |e: &serde_json::Value, k: &str| e.get(k).and_then(|v| v.as_bool()) == Some(true);
    view.get("events")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter(|e| !flag(e, "cancelled") && !flag(e, "isCancelled") && !flag(e, "allDay"))
        .filter_map(|e| {
            let id = text(e, "id")?;
            let end_text = text(e, "end")?;
            let end = parse(&end_text)?;
            if end > now || now - end > Duration::hours(MEETING_LOOKBACK_HOURS) {
                return None;
            }
            Some(TriggerEvent {
                kind: EventKind::RecordingEnded.token().into(),
                id: format!("meeting:{id}@{end_text}"),
                workspace_id: None,
                at: end.to_rfc3339(),
                title: text(e, "subject")
                    .or_else(|| text(e, "title"))
                    .unwrap_or_default(),
                key: None,
                url: None,
                note: None,
                team_id: None,
            })
        })
        .collect()
}

/// Workspace id -> its connected repository (the build binding's repo).
fn connected_repos(vault: &Path) -> BTreeMap<String, String> {
    crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| w.build.map(|b| (w.id, b.repo)))
        .collect()
}

async fn merged_prs(repo: &Path) -> Option<String> {
    let gh = crate::build_changes::gh_binary()?;
    let mut cmd = tokio::process::Command::new(gh);
    cmd.current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .args([
            "pr",
            "list",
            "--state",
            "merged",
            "--limit",
            "20",
            "--json",
            "number,title,url,mergedAt",
        ]);
    let out = crate::process_group::run_to_completion(
        &mut cmd,
        None,
        std::time::Duration::from_secs(20),
        None,
    )
    .await
    .ok()?;
    if out.timed_out || !out.status.is_some_and(|s| s.success()) {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── Status (Settings, the editor, the agent action) ─────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceStatus {
    pub id: String,
    pub title: String,
    pub since: String,
    pub lead_wake: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KindStatus {
    pub kind: String,
    pub label: String,
    pub on: bool,
    pub since: Option<String>,
    pub cap_per_hour: u32,
    pub fired_last_hour: u32,
    pub per_workspace: bool,
    pub workspaces: Vec<WorkspaceStatus>,
    /// Automations bound to this kind.
    pub automations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentEvent {
    pub kind: String,
    #[serde(flatten)]
    pub seen: Seen,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventTriggerStatus {
    pub kinds: Vec<KindStatus>,
    /// Workspaces whose lead wakes for new issues.
    pub lead_wakes: Vec<String>,
    /// Workspaces tracked in the tracker (a lead wake needs one).
    pub tracked: Vec<String>,
    /// Workspaces with a connected repository.
    pub connected: Vec<String>,
    /// The newest decided events, newest first (at most 20).
    pub recent: Vec<RecentEvent>,
    pub pending_wakes: usize,
}

pub(crate) fn status(config: &crate::config::Config, now: DateTime<Utc>) -> EventTriggerStatus {
    let vault = &config.vault_path;
    let state = load(vault);
    let titles: BTreeMap<String, String> = crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .map(|w| (w.id, w.title))
        .collect();
    let workflows = crate::vault_workflows::list(vault);
    let kinds = EventKind::ALL
        .into_iter()
        .map(|kind| {
            let ks = state.kind(kind);
            let workspaces = state
                .workspaces
                .get(kind.token())
                .into_iter()
                .flatten()
                .map(|(id, since)| WorkspaceStatus {
                    id: id.clone(),
                    title: titles.get(id).cloned().unwrap_or_else(|| id.clone()),
                    since: since.clone(),
                    lead_wake: kind == EventKind::TrackerIssue && state.lead_wakes.contains_key(id),
                })
                .collect();
            let automations = workflows
                .iter()
                .filter(|w| {
                    crate::trigger_evaluator::TriggerSpec::parse_all(&w.triggers)
                        .iter()
                        .any(|s| s.token().split(':').next() == Some(kind.token()))
                })
                .map(|w| w.slug.clone())
                .collect();
            KindStatus {
                kind: kind.token().into(),
                label: kind.label().into(),
                on: ks.on,
                since: ks.since.clone(),
                cap_per_hour: state.cap(kind),
                fired_last_hour: state.fired_last_hour(kind, now),
                per_workspace: kind.per_workspace(),
                workspaces,
                automations,
            }
        })
        .collect();
    let mut recent: Vec<RecentEvent> = state
        .seen
        .iter()
        .flat_map(|(kind, all)| {
            all.iter().map(|s| RecentEvent {
                kind: kind.clone(),
                seen: s.clone(),
            })
        })
        .collect();
    recent.sort_by(|a, b| b.seen.at.cmp(&a.seen.at));
    recent.truncate(20);
    let tracked = config
        .tracker
        .provider
        .as_deref()
        .map(|p| crate::tracker::status::tracked_workspaces(vault, p))
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.workspace_id)
        .collect();
    EventTriggerStatus {
        kinds,
        lead_wakes: state.lead_wakes.keys().cloned().collect(),
        tracked,
        connected: connected_repos(vault).into_keys().collect(),
        recent,
        pending_wakes: state.pending.len(),
    }
}

/// What `set_event_trigger` changes; every field but `kind` is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetTrigger {
    pub kind: String,
    #[serde(default)]
    pub on: Option<bool>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub lead_wake: Option<bool>,
    #[serde(default)]
    pub cap_per_hour: Option<u32>,
}

/// Apply a [`SetTrigger`]: the kind on or off; with `workspaceId`, that
/// workspace (turning it on turns the kind on); `leadWake` (team AI issues
/// only, a tracked project that isn't a build workspace); `capPerHour`.
pub(crate) fn set(
    config: &crate::config::Config,
    input: &SetTrigger,
    now: DateTime<Utc>,
) -> Result<EventTriggerStatus, String> {
    let vault = &config.vault_path;
    let kind = EventKind::parse(&input.kind).ok_or_else(|| {
        format!(
            "Unknown trigger kind {:?}: use tracker-issue, pr-merged or recording-ended. Nothing changed.",
            input.kind
        )
    })?;
    let ws = input
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty());
    if let Some(ws) = ws {
        let all = crate::workspaces::list(vault)?;
        let found = all
            .iter()
            .find(|w| w.id == ws)
            .ok_or_else(|| format!("No project {ws}. Nothing changed."))?;
        if input.lead_wake == Some(true) {
            if kind != EventKind::TrackerIssue {
                return Err("Only new issues wake a lead. Nothing changed.".into());
            }
            if found.build.is_some() {
                return Err(
                    "A build workspace's lead is never woken unattended. Nothing changed.".into(),
                );
            }
            let provider = config.tracker.provider.as_deref().unwrap_or("");
            if !crate::tracker::status::tracked_workspaces(vault, provider)
                .iter()
                .any(|t| t.workspace_id == ws)
            {
                return Err("This project isn't tracked in the task tracker yet (Track in, from the project's tracker menu). Nothing changed.".into());
            }
        }
        if kind == EventKind::PrMerged && input.on == Some(true) && found.build.is_none() {
            return Err("This project has no connected repository. Nothing changed.".into());
        }
    } else if input.lead_wake.is_some() {
        return Err("Name the project whose lead to wake (workspaceId). Nothing changed.".into());
    }
    update(vault, |state| {
        if let Some(cap) = input.cap_per_hour {
            state.set_cap(kind, cap);
        }
        // The lead wake and the project's trigger (what its automations use)
        // are separate switches: one never turns the other off.
        match (ws, input.on, input.lead_wake) {
            (Some(ws), on, Some(lead)) => {
                state.set_lead_wake(ws, lead, now);
                if let Some(on) = on {
                    state.set_workspace(kind, ws, on, now)?;
                }
            }
            (Some(ws), Some(on), None) => state.set_workspace(kind, ws, on, now)?,
            (None, Some(on), _) => state.set_kind(kind, on, now),
            _ => {}
        }
        Ok(())
    })?;
    Ok(status(config, now))
}

// ── The live pass ───────────────────────────────────────────────────────

/// The pass's slow-source clock and the ledger's last change, in memory.
#[derive(Default)]
struct Clock {
    slow_at: Option<DateTime<Utc>>,
    ledger_mtime: Option<std::time::SystemTime>,
}

/// One pass: read what each kind that is on needs, decide, queue, deliver.
async fn tick(config: &crate::config::Config, clock: &mut Clock) {
    let vault = config.vault_path.clone();
    let now = Utc::now();
    let state = load(&vault);
    let any_on = EventKind::ALL.into_iter().any(|k| state.is_on(k));
    if !any_on && state.pending.is_empty() {
        return;
    }
    let mut events: Vec<TriggerEvent> = Vec::new();
    let mut recordings_read: Option<Vec<TriggerEvent>> = None;
    if state.is_on(EventKind::TrackerIssue) {
        let ledger = vault.join(crate::tracker_sync::STATE);
        let mtime = std::fs::metadata(&ledger).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime != clock.ledger_mtime {
            clock.ledger_mtime = mtime;
            let cfg = config.clone();
            let found = tauri::async_runtime::spawn_blocking(move || {
                let issues = ledger_issues(&cfg).unwrap_or_default();
                let tracked = cfg
                    .tracker
                    .provider
                    .as_deref()
                    .map(|p| crate::tracker::status::tracked_workspaces(&cfg.vault_path, p))
                    .unwrap_or_default();
                issue_events(&issues, &tracked)
            })
            .await
            .unwrap_or_default();
            events.extend(found);
        }
    }
    let slow = clock
        .slow_at
        .is_none_or(|at| now - at >= Duration::seconds(SLOW_PASS_SECS));
    if slow {
        clock.slow_at = Some(now);
        if state.is_on(EventKind::PrMerged) {
            let on: Vec<String> = state
                .workspaces
                .get(EventKind::PrMerged.token())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (ws, repo) in connected_repos(&vault) {
                if on.contains(&ws) {
                    by_repo.entry(repo).or_default().push(ws);
                }
            }
            for (repo, wss) in by_repo {
                if let Some(json) = merged_prs(Path::new(&repo)).await {
                    events.extend(pr_events(&json, &wss));
                }
            }
        }
        if state.is_on(EventKind::RecordingEnded) {
            if crate::plaud::is_connected() {
                // Only a clean read primes (a failed one can list nothing).
                if let Ok(list) =
                    crate::recordings::list(&vault, 2, crate::state_file::OnDamage::Leave, false)
                        .await
                {
                    if list.state == "ok" && list.error.is_none() {
                        let read = recording_events(&list.recordings);
                        events.extend(read.iter().cloned());
                        recordings_read = Some(read);
                    }
                }
            }
            let from = (now - Duration::hours(MEETING_LOOKBACK_HOURS)).to_rfc3339();
            let view =
                crate::agenda::events_with(&from, &now.to_rfc3339(), crate::agenda::Reuse::Kept)
                    .await;
            events.extend(meeting_events(&view, now));
        }
    }
    let accepting = crate::run_pump::accepting().is_ok();
    let v = vault.clone();
    let queued = tauri::async_runtime::spawn_blocking(move || {
        // M4: only automations whose owner Mac is this one take events.
        let workflows = crate::runs_on::owned_here(&v, crate::vault_workflows::list(&v));
        update(&v, |state| {
            let mut queued = Vec::new();
            if let Some(read) = &recordings_read {
                prime(state, read, now);
            }
            for kind in EventKind::ALL {
                let snapshot = state.clone();
                let bound = |e: &TriggerEvent| bindings(&snapshot, &workflows, e);
                let fired = decide(state, kind, &events, now, &bound);
                queued.extend(dispatch(&v, state, &fired, &workflows, now));
            }
            if accepting {
                queued.extend(deliver_pending(&v, state, now));
            }
            Ok(queued)
        })
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    if !queued.is_empty() {
        crate::run_queue::wake();
    }
}

/// Start the minute pass (the queue's owner only).
pub(crate) fn start(config: std::sync::Arc<crate::config::Config>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        let mut clock = Clock::default();
        loop {
            tick(&config, &mut clock).await;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
#[path = "event_triggers_tests.rs"]
mod tests;
