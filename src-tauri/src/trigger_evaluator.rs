//! Triggered agents — the PURE fire/dedup decision for event-driven workflows.
//!
//! A workflow's `triggers:` frontmatter is a comma-list of tokens (same shape as
//! `allowed_tools`), each `kind` or `kind:arg`:
//!   - `file-changed:<glob>`    — a vault-relative file matching the glob changed.
//!   - `task-overdue`           — a task newly entered the overdue set (edge).
//!   - `task-unblocked`         — a task with a non-empty `blocked_by` newly
//!     has every blocker resolved (edge; Paperclip PR 11b). The first
//!     observation seeds silently; an id that leaves the set is forgotten, so
//!     a task re-blocked and unblocked again fires again.
//!
//!   - `tracker-issue[:<workspace>]`, `pr-merged[:<workspace>]`,
//!     `recording-ended` (COO rebuild F2): event triggers. They parse here so
//!     a queued fire's token still reads as listed at claim, but the
//!     evaluator never decides them: `event_triggers` does (dedup by event
//!     id, hourly cap, off until turned on) and queues the row itself.
//!
//! Unknown/malformed tokens parse to `None` and are ignored (forward-compatible),
//! as do the retired `email-from:` and `meeting-in:` (Outlook was removed on
//! 2026-09-26; calendar now comes from the source connected in Settings, Connectors).
//!
//! [`TriggerEvaluator::evaluate`] decides from a parsed [`TriggerSpec`], the live
//! facts the scanner gathered ([`TriggerFacts`]), and the per-(workflow,trigger)
//! watermark ([`TriggerWatermark`]) — no IO, no clock read (`now` is in facts), so
//! the dedup/edge/window logic is fully offline-testable. The caller
//! (`workflows::enqueue_due`) commits the advanced watermark once the fire is
//! enqueued on the run queue (Paperclip PR 7), so an overlapping tick can't
//! double-fire; the queued row carries the fire until the pump claims it.
//!
//! [`TriggerEvaluator::evaluate_all_detailed`] (Paperclip PR 7) also reports
//! which tokens fired and the facts behind each ([`FiredToken`]: the freshly
//! overdue or unblocked task ids, the newest matching mtime),
//! and [`fired_key`] turns a fire into the run queue's idempotency key. The
//! queue records those facts on a trigger row so a claim can tell whether a
//! fire is still true; [`merge_fired`] unions them when a re-fire coalesces
//! into a row that is still queued.
//!
//! Port of hollow-mac's `TriggerSpec` + `TriggerEvaluator`.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// One parsed `triggers:` token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerSpec {
    FileChanged(String),
    TaskOverdue,
    /// Paperclip PR 11b: any argument is ignored.
    TaskUnblocked,
    /// F2 event triggers (decided in `event_triggers`): a new issue in team
    /// AI, for one workspace or any turned on.
    TrackerIssue(Option<String>),
    /// A merged PR in a workspace's connected repository.
    PrMerged(Option<String>),
    /// A recording or calendar meeting ending.
    RecordingEnded,
}

impl TriggerSpec {
    /// The canonical token string (round-trips through `parse`). Used as the
    /// watermark key so on-disk state is stable across re-parses.
    pub fn token(&self) -> String {
        match self {
            TriggerSpec::FileChanged(g) => format!("file-changed:{g}"),
            TriggerSpec::TaskOverdue => "task-overdue".to_string(),
            TriggerSpec::TaskUnblocked => "task-unblocked".to_string(),
            TriggerSpec::TrackerIssue(None) => "tracker-issue".to_string(),
            TriggerSpec::TrackerIssue(Some(w)) => format!("tracker-issue:{w}"),
            TriggerSpec::PrMerged(None) => "pr-merged".to_string(),
            TriggerSpec::PrMerged(Some(w)) => format!("pr-merged:{w}"),
            TriggerSpec::RecordingEnded => "recording-ended".to_string(),
        }
    }

    /// An event trigger (F2): `event_triggers` decides and queues it; the
    /// polled evaluation below never does.
    pub fn is_event(&self) -> bool {
        matches!(
            self,
            TriggerSpec::TrackerIssue(_) | TriggerSpec::PrMerged(_) | TriggerSpec::RecordingEnded
        )
    }

    /// [`Self::parse_all`] without the event triggers: what the scheduler's
    /// watermark evaluation reads.
    pub fn parse_polled(tokens: &[String]) -> Vec<TriggerSpec> {
        Self::parse_all(tokens)
            .into_iter()
            .filter(|spec| !spec.is_event())
            .collect()
    }

    /// Parse one token. `kind` or `kind:arg`; case-insensitive kind. `None` for
    /// unknown kinds, empty args, or malformed durations.
    pub fn parse(raw: &str) -> Option<TriggerSpec> {
        let t = raw.trim();
        if t.is_empty() {
            return None;
        }
        // Split on the FIRST colon only (globs/substrings may contain colons).
        if let Some(colon) = t.find(':') {
            let kind = t[..colon].trim().to_lowercase();
            let arg = t[colon + 1..].trim();
            return match kind.as_str() {
                "file-changed" => {
                    (!arg.is_empty()).then(|| TriggerSpec::FileChanged(arg.to_string()))
                }
                "task-overdue" => Some(TriggerSpec::TaskOverdue), // arg ignored
                "task-unblocked" => Some(TriggerSpec::TaskUnblocked), // arg ignored
                "tracker-issue" => workspace_arg(arg).map(TriggerSpec::TrackerIssue),
                "pr-merged" => workspace_arg(arg).map(TriggerSpec::PrMerged),
                "recording-ended" => Some(TriggerSpec::RecordingEnded), // arg ignored
                _ => None,
            };
        }
        match t.to_lowercase().as_str() {
            "task-overdue" => Some(TriggerSpec::TaskOverdue),
            "task-unblocked" => Some(TriggerSpec::TaskUnblocked),
            "tracker-issue" => Some(TriggerSpec::TrackerIssue(None)),
            "pr-merged" => Some(TriggerSpec::PrMerged(None)),
            "recording-ended" => Some(TriggerSpec::RecordingEnded),
            _ => None,
        }
    }

    /// Parse all tokens, dropping any that don't parse (lenient).
    pub fn parse_all(tokens: &[String]) -> Vec<TriggerSpec> {
        tokens
            .iter()
            .filter_map(|t| TriggerSpec::parse(t))
            .collect()
    }
}

/// An event trigger's workspace argument: `Some(None)` (any workspace) for
/// an empty one, `Some(Some(id))` for a workspace id (letters, digits, `-`,
/// `_`), and `None` (the token is ignored) for anything else, so a typo
/// never widens a trigger to every workspace.
fn workspace_arg(arg: &str) -> Option<Option<String>> {
    if arg.is_empty() {
        return Some(None);
    }
    let ok = arg.len() <= 128
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| Some(arg.to_string()))
}

/// Match a vault-relative `path` against a `glob` supporting `*` (one segment),
/// `**` (any depth), and `?` (one char). Anchored full-string match.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    match glob_regex(glob) {
        Some(re) => re.is_match(path),
        None => glob == path,
    }
}

fn glob_regex(glob: &str) -> Option<Regex> {
    let mut pattern = String::from("^");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    pattern.push_str(".*"); // `**` → any depth (incl. `/`)
                    i += 2;
                    if i < chars.len() && chars[i] == '/' {
                        i += 1; // swallow `**/`
                    }
                    continue;
                } else {
                    pattern.push_str("[^/]*"); // `*` → one segment
                }
            }
            '?' => pattern.push_str("[^/]"),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    pattern.push('$');
    Regex::new(&pattern).ok()
}

// ── Facts ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub mtime: DateTime<Utc>,
}

/// The live facts the scanner gathers each tick; the evaluator decides from these.
#[derive(Debug, Clone)]
pub struct TriggerFacts {
    pub now_utc: DateTime<Utc>,
    pub changed_files: Vec<ChangedFile>,
    pub overdue_task_ids: HashSet<String>,
    /// `vault:tasks/…` URLs of open (not terminal, not cancelled) tasks whose
    /// non-empty `blocked_by` has every blocker resolved.
    pub unblocked_task_ids: HashSet<String>,
}

impl TriggerFacts {
    #[cfg(test)]
    pub fn new(now_utc: DateTime<Utc>) -> Self {
        TriggerFacts {
            now_utc,
            changed_files: Vec::new(),
            overdue_task_ids: HashSet::new(),
            unblocked_task_ids: HashSet::new(),
        }
    }
}

/// Per-(workflow,trigger-token) watermark, persisted in `.workflow-triggers.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerWatermark {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_max_mtime: Option<String>,
    #[serde(default)]
    pub last_overdue_ids: Vec<String>,
    /// The unblocked set last observed for `task-unblocked`. `None` means
    /// never observed: the first evaluation seeds it silently instead of
    /// firing for every task already unblocked. Omitted when `None`, so an
    /// older watermark file loads and saves unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_unblocked_ids: Option<Vec<String>>,
}

/// Fire (with a reason + advanced watermark) or skip (watermark may still
/// advance). A skip's `new_state` is committed as is; a fire's once its run
/// queue row is enqueued (Paperclip PR 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerVerdict {
    Fire {
        reason: String,
        new_state: TriggerWatermark,
    },
    Skip {
        new_state: TriggerWatermark,
    },
}

impl TriggerVerdict {
    pub fn new_state(&self) -> &TriggerWatermark {
        match self {
            TriggerVerdict::Fire { new_state, .. } | TriggerVerdict::Skip { new_state } => {
                new_state
            }
        }
    }
    pub fn fired(&self) -> bool {
        matches!(self, TriggerVerdict::Fire { .. })
    }
}

/// One token that fired, with the facts that made it fire. The run queue
/// stores these in a trigger row's payload (`{"fired": [...]}`) and re-checks
/// them at claim: a `task-overdue` fire once none of its fresh ids is still overdue,
/// a `task-unblocked` fire once none of its ids is still unblocked (re-blocked,
/// finished or cancelled). A `file-changed` fire (a level trigger) never
/// lapses on its facts. Task ids are `vault:tasks/…` URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FiredToken {
    /// The canonical token ([`TriggerSpec::token`]).
    pub token: String,
    /// The ids that newly entered the overdue set, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overdue_ids: Vec<String>,
    /// The ids that newly entered the unblocked set, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unblocked_ids: Vec<String>,
    /// The newest mtime among the matching changed files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mtime: Option<DateTime<Utc>>,
    /// An event trigger's event ids (F2: `event_triggers::TriggerEvent::id`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ids: Vec<String>,
}

impl FiredToken {
    /// Fold a later fire of the same token into this one.
    fn absorb(&mut self, other: &FiredToken) {
        for id in &other.overdue_ids {
            if !self.overdue_ids.contains(id) {
                self.overdue_ids.push(id.clone());
            }
        }
        self.overdue_ids.sort();
        for id in &other.unblocked_ids {
            if !self.unblocked_ids.contains(id) {
                self.unblocked_ids.push(id.clone());
            }
        }
        self.unblocked_ids.sort();
        for id in &other.event_ids {
            if !self.event_ids.contains(id) {
                self.event_ids.push(id.clone());
            }
        }
        self.max_mtime = match (self.max_mtime, other.max_mtime) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
}

/// Everything one evaluation of a workflow's triggers decided: whether it
/// fires and why, every token's advanced watermark, and the tokens that fired
/// with their facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerOutcome {
    pub fire: bool,
    pub reason: String,
    pub next: HashMap<String, TriggerWatermark>,
    pub fired: Vec<FiredToken>,
}

pub struct TriggerEvaluator;

impl TriggerEvaluator {
    /// Decide for ONE spec against its watermark. Pure. Production goes
    /// through [`Self::evaluate_all_detailed`]; the unit tests pin single
    /// specs through this view.
    #[cfg(test)]
    pub fn evaluate(
        spec: &TriggerSpec,
        facts: &TriggerFacts,
        state: &TriggerWatermark,
    ) -> TriggerVerdict {
        Self::evaluate_with_facts(spec, facts, state).0
    }

    /// [`Self::evaluate`], plus the facts behind a fire: which
    /// freshly overdue or unblocked task ids, or the newest matching mtime.
    /// `None` on a skip.
    fn evaluate_with_facts(
        spec: &TriggerSpec,
        facts: &TriggerFacts,
        state: &TriggerWatermark,
    ) -> (TriggerVerdict, Option<FiredToken>) {
        match spec {
            // Decided by `event_triggers`, never here.
            TriggerSpec::TrackerIssue(_)
            | TriggerSpec::PrMerged(_)
            | TriggerSpec::RecordingEnded => (
                TriggerVerdict::Skip {
                    new_state: state.clone(),
                },
                None,
            ),
            TriggerSpec::FileChanged(glob) => {
                let mtimes: Vec<DateTime<Utc>> = facts
                    .changed_files
                    .iter()
                    .filter(|c| glob_matches(glob, &c.path))
                    .map(|c| c.mtime)
                    .collect();
                let verdict = level_fire(
                    &mtimes,
                    state,
                    facts.now_utc,
                    &format!("file changed: {glob}"),
                );
                let fired = verdict.fired().then(|| FiredToken {
                    token: spec.token(),
                    max_mtime: mtimes.iter().copied().max(),
                    ..FiredToken::default()
                });
                (verdict, fired)
            }
            TriggerSpec::TaskOverdue => {
                let prev: HashSet<String> = state.last_overdue_ids.iter().cloned().collect();
                let fresh: Vec<String> = facts
                    .overdue_task_ids
                    .iter()
                    .filter(|id| !prev.contains(*id))
                    .cloned()
                    .collect();
                let mut s = state.clone();
                let mut ids: Vec<String> = facts.overdue_task_ids.iter().cloned().collect();
                ids.sort();
                s.last_overdue_ids = ids;
                if fresh.is_empty() {
                    return (TriggerVerdict::Skip { new_state: s }, None);
                }
                let mut fresh_sorted = fresh;
                fresh_sorted.sort();
                s.last_fired_at = Some(facts.now_utc.to_rfc3339());
                let fired = FiredToken {
                    token: spec.token(),
                    overdue_ids: fresh_sorted.clone(),
                    ..FiredToken::default()
                };
                (
                    TriggerVerdict::Fire {
                        reason: format!("task overdue: {}", fresh_sorted.join(", ")),
                        new_state: s,
                    },
                    Some(fired),
                )
            }
            TriggerSpec::TaskUnblocked => {
                let mut ids: Vec<String> = facts.unblocked_task_ids.iter().cloned().collect();
                ids.sort();
                let mut s = state.clone();
                let Some(prev) = state.last_unblocked_ids.as_ref() else {
                    // First observation: seed silently (no burst for every
                    // task that was already unblocked).
                    s.last_unblocked_ids = Some(ids);
                    return (TriggerVerdict::Skip { new_state: s }, None);
                };
                let prev: HashSet<&String> = prev.iter().collect();
                let fresh: Vec<String> = ids
                    .iter()
                    .filter(|id| !prev.contains(id))
                    .cloned()
                    .collect();
                // Storing the current set forgets the ids that left it, so a
                // re-blocked task that unblocks again fires again.
                s.last_unblocked_ids = Some(ids);
                if fresh.is_empty() {
                    return (TriggerVerdict::Skip { new_state: s }, None);
                }
                s.last_fired_at = Some(facts.now_utc.to_rfc3339());
                let fired = FiredToken {
                    token: spec.token(),
                    unblocked_ids: fresh.clone(),
                    ..FiredToken::default()
                };
                (
                    TriggerVerdict::Fire {
                        reason: format!("task unblocked: {}", fresh.join(", ")),
                        new_state: s,
                    },
                    Some(fired),
                )
            }
        }
    }

    /// Decide across ALL of a workflow's specs. Fires if ANY spec fires; threads
    /// the advanced per-token watermarks back into the combined state map (keyed
    /// by `spec.token()`). A thin view of [`Self::evaluate_all_detailed`].
    pub fn evaluate_all(
        specs: &[TriggerSpec],
        facts: &TriggerFacts,
        state: &HashMap<String, TriggerWatermark>,
    ) -> (bool, String, HashMap<String, TriggerWatermark>) {
        let outcome = Self::evaluate_all_detailed(specs, facts, state);
        (outcome.fire, outcome.reason, outcome.next)
    }

    /// [`Self::evaluate_all`] plus the tokens that fired and the facts behind
    /// each ([`FiredToken`]). The run queue (Paperclip PR 7) records these on a
    /// trigger row at enqueue, so a claim can tell whether the fire is still
    /// true (the task is still overdue or unblocked) instead
    /// of running a stale fire late.
    pub fn evaluate_all_detailed(
        specs: &[TriggerSpec],
        facts: &TriggerFacts,
        state: &HashMap<String, TriggerWatermark>,
    ) -> TriggerOutcome {
        let mut next = state.clone();
        let mut reasons: Vec<String> = Vec::new();
        let mut fired: Vec<FiredToken> = Vec::new();
        for spec in specs.iter().filter(|spec| !spec.is_event()) {
            let token = spec.token();
            let prior = state.get(&token).cloned().unwrap_or_default();
            let (verdict, facts_fired) = Self::evaluate_with_facts(spec, facts, &prior);
            if let TriggerVerdict::Fire { reason, .. } = &verdict {
                reasons.push(reason.clone());
            }
            if let Some(token_facts) = facts_fired {
                fired.push(token_facts);
            }
            next.insert(token, verdict.new_state().clone());
        }
        TriggerOutcome {
            fire: !reasons.is_empty(),
            reason: reasons.join("; "),
            next,
            fired,
        }
    }
}

/// The idempotency key of one trigger fire: `trigger:{slug}:{32 hex}`, the
/// first 32 hex characters of the SHA-256 of the fired tokens' advanced
/// watermarks (a `BTreeMap`, so the JSON is ordered and stable). Two identical
/// evaluations give the same key. A watermark's `last_fired_at` is the
/// evaluation time, so a genuinely new fire gets a new key; repeats of a fire
/// that is still queued are absorbed by coalescing, not by the key.
pub fn fired_key(slug: &str, outcome: &TriggerOutcome) -> String {
    use sha2::{Digest, Sha256};
    let fired: BTreeMap<&str, Option<&TriggerWatermark>> = outcome
        .fired
        .iter()
        .map(|token| (token.token.as_str(), outcome.next.get(&token.token)))
        .collect();
    let bytes = serde_json::to_vec(&fired).unwrap_or_default();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    format!("trigger:{slug}:{}", &digest[..32])
}

/// Union two sets of fire facts by token: overdue and unblocked ids deduplicated and sorted, the newest mtime
/// kept. Used
/// when a re-fire coalesces into a trigger row that is still queued.
pub fn merge_fired(existing: &[FiredToken], incoming: &[FiredToken]) -> Vec<FiredToken> {
    let mut merged: Vec<FiredToken> = existing.to_vec();
    for token in incoming {
        match merged.iter_mut().find(|t| t.token == token.token) {
            Some(target) => target.absorb(token),
            None => merged.push(token.clone()),
        }
    }
    merged
}

/// Level (mtime) helper: fire when the max relevant mtime is newer than the
/// watermark, then advance the watermark (never regress).
fn level_fire(
    mtimes: &[DateTime<Utc>],
    state: &TriggerWatermark,
    now: DateTime<Utc>,
    reason: &str,
) -> TriggerVerdict {
    let Some(max_m) = mtimes.iter().copied().max() else {
        return TriggerVerdict::Skip {
            new_state: state.clone(),
        };
    };
    let last = state.last_max_mtime.as_deref().and_then(parse_rfc3339);
    let mut s = state.clone();
    let new_watermark = last.map(|l| l.max(max_m)).unwrap_or(max_m);
    s.last_max_mtime = Some(new_watermark.to_rfc3339());
    if let Some(last) = last {
        if max_m <= last {
            return TriggerVerdict::Skip { new_state: s }; // nothing newer
        }
    }
    s.last_fired_at = Some(now.to_rfc3339());
    TriggerVerdict::Fire {
        reason: reason.to_string(),
        new_state: s,
    }
}

/// Parse an RFC3339 instant (accepts `Z` and `+00:00`, with/without fractional).
pub fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs_from_epoch: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs_from_epoch, 0).unwrap()
    }
    const NOW_TS: i64 = 1_800_000_000;
    fn now() -> DateTime<Utc> {
        at(NOW_TS)
    }

    // ── parse / token round-trip ──
    #[test]
    fn parse_grammar_and_token_roundtrip() {
        assert_eq!(
            TriggerSpec::parse("task-overdue"),
            Some(TriggerSpec::TaskOverdue)
        );
        assert_eq!(
            TriggerSpec::parse("file-changed:projects/**/*.md"),
            Some(TriggerSpec::FileChanged("projects/**/*.md".into()))
        );
        assert_eq!(TriggerSpec::parse("email-from:boss@acme.com"), None);
        assert_eq!(
            TriggerSpec::parse("meeting-in:30m"),
            None,
            "retired with Outlook"
        );
        assert_eq!(
            TriggerSpec::parse("task-unblocked"),
            Some(TriggerSpec::TaskUnblocked)
        );
        assert_eq!(
            TriggerSpec::parse(" Task-Unblocked:anything "),
            Some(TriggerSpec::TaskUnblocked),
            "case-insensitive, argument ignored"
        );
        assert_eq!(TriggerSpec::TaskUnblocked.token(), "task-unblocked");
        assert_eq!(
            TriggerSpec::parse(&TriggerSpec::TaskUnblocked.token()),
            Some(TriggerSpec::TaskUnblocked)
        );
        // malformed / unknown → None
        assert_eq!(TriggerSpec::parse("file-changed:"), None);
        assert_eq!(TriggerSpec::parse("nope"), None);
        // token round-trips
        assert_eq!(
            TriggerSpec::FileChanged("x.md".into()).token(),
            "file-changed:x.md"
        );
    }

    /// F2: the event tokens parse and round-trip, a bad workspace argument
    /// is ignored (never widened to every workspace), and the polled
    /// evaluation leaves them alone (no watermark, never a fire).
    #[test]
    fn event_tokens_parse_and_are_never_polled() {
        for (raw, spec) in [
            ("tracker-issue", TriggerSpec::TrackerIssue(None)),
            (
                "tracker-issue:ws-1",
                TriggerSpec::TrackerIssue(Some("ws-1".into())),
            ),
            (
                "pr-merged:repo_ws",
                TriggerSpec::PrMerged(Some("repo_ws".into())),
            ),
            ("pr-merged", TriggerSpec::PrMerged(None)),
            ("recording-ended", TriggerSpec::RecordingEnded),
        ] {
            assert_eq!(TriggerSpec::parse(raw), Some(spec.clone()), "{raw}");
            assert_eq!(TriggerSpec::parse(&spec.token()), Some(spec.clone()));
            assert!(spec.is_event());
        }
        assert_eq!(TriggerSpec::parse("tracker-issue:../x"), None);
        assert_eq!(TriggerSpec::parse("pr-merged:a b"), None);
        let tokens = vec!["tracker-issue:ws-1".to_string(), "task-overdue".to_string()];
        assert_eq!(
            TriggerSpec::parse_polled(&tokens),
            vec![TriggerSpec::TaskOverdue]
        );
        let outcome = TriggerEvaluator::evaluate_all_detailed(
            &TriggerSpec::parse_all(&tokens[..1]),
            &TriggerFacts::new(now()),
            &HashMap::new(),
        );
        assert!(!outcome.fire);
        assert!(outcome.next.is_empty(), "no watermark for an event token");
    }

    #[test]
    fn glob_matches_segments_and_depth() {
        assert!(glob_matches("projects/*.md", "projects/a.md"));
        assert!(!glob_matches("projects/*.md", "projects/sub/a.md")); // * is one segment
        assert!(glob_matches("projects/**/*.md", "projects/sub/a.md")); // ** any depth
        assert!(glob_matches("**/*.md", "deep/nest/x.md"));
        assert!(!glob_matches("projects/*.md", "other/a.md"));
    }

    // ── file-changed (level on mtime) ──
    #[test]
    fn file_changed_level_dedups_until_mtime_advances() {
        let facts = TriggerFacts {
            changed_files: vec![
                ChangedFile {
                    path: "projects/a.md".into(),
                    mtime: at(NOW_TS - 10),
                },
                ChangedFile {
                    path: "notes/b.md".into(),
                    mtime: now(),
                },
            ],
            ..TriggerFacts::new(now())
        };
        let spec = TriggerSpec::FileChanged("projects/*.md".into());
        let v1 = TriggerEvaluator::evaluate(&spec, &facts, &TriggerWatermark::default());
        assert!(v1.fired());
        let v2 = TriggerEvaluator::evaluate(&spec, &facts, v1.new_state());
        assert!(!v2.fired()); // nothing newer
        let facts3 = TriggerFacts {
            changed_files: vec![ChangedFile {
                path: "projects/a.md".into(),
                mtime: at(NOW_TS + 5),
            }],
            ..TriggerFacts::new(now())
        };
        let v3 = TriggerEvaluator::evaluate(&spec, &facts3, v2.new_state());
        assert!(v3.fired());
        let non_match = TriggerFacts {
            changed_files: vec![ChangedFile {
                path: "other/x.md".into(),
                mtime: at(NOW_TS + 99),
            }],
            ..TriggerFacts::new(now())
        };
        assert!(!TriggerEvaluator::evaluate(&spec, &non_match, v3.new_state()).fired());
    }

    #[test]
    fn retired_message_triggers_are_ignored() {
        let tokens = vec![
            "email-from:boss@acme.com".into(),
            "meeting-in:30m".into(),
            "task-overdue".into(),
        ];
        assert_eq!(
            TriggerSpec::parse_all(&tokens),
            vec![TriggerSpec::TaskOverdue]
        );
    }

    // ── task-overdue (edge) ──
    #[test]
    fn task_overdue_fires_on_new_id_only() {
        let mut f1 = TriggerFacts::new(now());
        f1.overdue_task_ids = ["t1".to_string()].into_iter().collect();
        let v1 = TriggerEvaluator::evaluate(
            &TriggerSpec::TaskOverdue,
            &f1,
            &TriggerWatermark::default(),
        );
        assert!(v1.fired());
        let v2 = TriggerEvaluator::evaluate(&TriggerSpec::TaskOverdue, &f1, v1.new_state());
        assert!(!v2.fired()); // same set
        let mut f3 = TriggerFacts::new(now());
        f3.overdue_task_ids = ["t1".to_string(), "t2".to_string()].into_iter().collect();
        let v3 = TriggerEvaluator::evaluate(&TriggerSpec::TaskOverdue, &f3, v2.new_state());
        assert!(v3.fired());
        // id leaving doesn't re-fire; re-entry does.
        let mut f4 = TriggerFacts::new(now());
        f4.overdue_task_ids = ["t1".to_string()].into_iter().collect();
        let v4 = TriggerEvaluator::evaluate(&TriggerSpec::TaskOverdue, &f4, v3.new_state());
        assert!(!v4.fired());
        let v5 = TriggerEvaluator::evaluate(&TriggerSpec::TaskOverdue, &f3, v4.new_state());
        assert!(v5.fired()); // t2 re-entered
    }

    // ── task-unblocked (edge, seeded silently; Paperclip PR 11b) ──
    fn unblocked(ids: &[&str]) -> TriggerFacts {
        let mut facts = TriggerFacts::new(now());
        facts.unblocked_task_ids = ids.iter().map(|id| id.to_string()).collect();
        facts
    }

    #[test]
    fn task_unblocked_seeds_silently_then_fires_on_entry_and_refires_after_reblock() {
        let spec = TriggerSpec::TaskUnblocked;
        let a = "vault:tasks/a.md";
        let b = "vault:tasks/b.md";
        // First observation: no burst, but the set is seeded.
        let seed =
            TriggerEvaluator::evaluate(&spec, &unblocked(&[a]), &TriggerWatermark::default());
        assert!(!seed.fired());
        assert_eq!(
            seed.new_state().last_unblocked_ids,
            Some(vec![a.to_string()])
        );
        // Same set: nothing.
        let same = TriggerEvaluator::evaluate(&spec, &unblocked(&[a]), seed.new_state());
        assert!(!same.fired());
        // b enters: fires for b only.
        let (fire, facts) =
            TriggerEvaluator::evaluate_with_facts(&spec, &unblocked(&[b, a]), same.new_state());
        match &fire {
            TriggerVerdict::Fire { reason, new_state } => {
                assert_eq!(reason, "task unblocked: vault:tasks/b.md");
                assert_eq!(
                    new_state.last_unblocked_ids,
                    Some(vec![a.to_string(), b.to_string()])
                );
                assert!(new_state.last_fired_at.is_some());
            }
            other => panic!("expected a fire, got {other:?}"),
        }
        assert_eq!(
            facts,
            Some(FiredToken {
                token: "task-unblocked".into(),
                unblocked_ids: vec![b.to_string()],
                ..FiredToken::default()
            })
        );
        // b is re-blocked: it leaves the set (forgotten), no fire.
        let reblocked = TriggerEvaluator::evaluate(&spec, &unblocked(&[a]), fire.new_state());
        assert!(!reblocked.fired());
        assert_eq!(
            reblocked.new_state().last_unblocked_ids,
            Some(vec![a.to_string()])
        );
        // b unblocks again: fires again.
        let again = TriggerEvaluator::evaluate(&spec, &unblocked(&[a, b]), reblocked.new_state());
        assert!(again.fired());

        // A never-observed watermark with an empty set seeds Some([]), and
        // the first task to unblock afterwards fires.
        let empty =
            TriggerEvaluator::evaluate(&spec, &unblocked(&[]), &TriggerWatermark::default());
        assert!(!empty.fired());
        assert_eq!(empty.new_state().last_unblocked_ids, Some(Vec::new()));
        assert!(TriggerEvaluator::evaluate(&spec, &unblocked(&[a]), empty.new_state()).fired());

        // The watermark JSON omits the key until it is observed.
        let json = serde_json::to_value(TriggerWatermark::default()).unwrap();
        assert!(json.get("lastUnblockedIds").is_none(), "{json}");
        let json = serde_json::to_value(empty.new_state()).unwrap();
        assert_eq!(json["lastUnblockedIds"], serde_json::json!([]));
    }

    // ── evaluate_all ──
    #[test]
    fn evaluate_all_fires_if_any_and_threads_watermarks() {
        let mut facts = TriggerFacts::new(now());
        facts.changed_files = vec![ChangedFile {
            path: "x.md".into(),
            mtime: now(),
        }];
        facts.overdue_task_ids = ["t1".to_string()].into_iter().collect();
        let specs = vec![
            TriggerSpec::TaskOverdue,
            TriggerSpec::FileChanged("x.md".into()),
        ];
        let r = TriggerEvaluator::evaluate_all(&specs, &facts, &HashMap::new());
        assert!(r.0);
        assert!(r.2.contains_key("task-overdue"));
        assert!(r.2.contains_key("file-changed:x.md"));
        let r2 = TriggerEvaluator::evaluate_all(&specs, &facts, &r.2);
        assert!(!r2.0); // both dedup
    }

    // ── evaluate_all_detailed (the queue's fire facts) ──
    #[test]
    fn evaluate_all_detailed_reports_the_tokens_and_facts_that_fired() {
        let mut facts = TriggerFacts::new(now());
        facts.overdue_task_ids = ["t2".to_string(), "t1".to_string()].into_iter().collect();
        let specs = vec![
            TriggerSpec::TaskOverdue,
            TriggerSpec::FileChanged("projects/*.md".into()),
        ];
        // Only the overdue spec fires: nothing changed.
        let only = TriggerEvaluator::evaluate_all_detailed(&specs, &facts, &HashMap::new());
        assert!(only.fire);
        assert_eq!(
            only.fired,
            vec![FiredToken {
                token: "task-overdue".into(),
                overdue_ids: vec!["t1".into(), "t2".into()],
                ..FiredToken::default()
            }]
        );
        let (fire, reason, next) = TriggerEvaluator::evaluate_all(&specs, &facts, &HashMap::new());
        assert_eq!((fire, reason.as_str()), (only.fire, only.reason.as_str()));
        assert_eq!(next, only.next);
        assert_eq!(
            fired_key("wf", &only),
            fired_key("wf", &only.clone()),
            "the key is stable for the same watermarks"
        );
        assert!(fired_key("wf", &only).starts_with("trigger:wf:"));
        assert_eq!(fired_key("wf", &only).len(), "trigger:wf:".len() + 32);

        // A change carries its mtime.
        facts.changed_files = vec![ChangedFile {
            path: "projects/a.md".into(),
            mtime: at(NOW_TS - 5),
        }];
        let all = TriggerEvaluator::evaluate_all_detailed(&specs, &facts, &only.next);
        let tokens: Vec<&str> = all.fired.iter().map(|t| t.token.as_str()).collect();
        assert_eq!(tokens, vec!["file-changed:projects/*.md"]);
        assert_eq!(all.fired[0].max_mtime, Some(at(NOW_TS - 5)));
        assert_ne!(fired_key("wf", &all), fired_key("wf", &only));
        assert_ne!(fired_key("wf", &all), fired_key("other", &all));

        // The facts round-trip through the queue payload's JSON.
        let json = serde_json::to_value(&all.fired).unwrap();
        let back: Vec<FiredToken> = serde_json::from_value(json).unwrap();
        assert_eq!(back, all.fired);
    }

    #[test]
    fn merged_fire_facts_union_ids_and_the_newest_mtime() {
        let a = vec![FiredToken {
            token: "task-overdue".into(),
            overdue_ids: vec!["t2".into()],
            ..FiredToken::default()
        }];
        let b = vec![
            FiredToken {
                token: "task-overdue".into(),
                overdue_ids: vec!["t1".into(), "t2".into()],
                ..FiredToken::default()
            },
            FiredToken {
                token: "task-unblocked".into(),
                unblocked_ids: vec!["vault:tasks/z.md".into(), "vault:tasks/a.md".into()],
                ..FiredToken::default()
            },
            FiredToken {
                token: "file-changed:x.md".into(),
                max_mtime: Some(at(NOW_TS)),
                ..FiredToken::default()
            },
        ];
        let merged = merge_fired(&a, &b);
        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged[0].overdue_ids,
            vec!["t1".to_string(), "t2".to_string()]
        );
        assert_eq!(merged[1].token, "task-unblocked");
        assert_eq!(merged[2].max_mtime, Some(at(NOW_TS)));
        // Unblocked ids are unioned and sorted when a re-fire coalesces.
        let unblocked_again = merge_fired(
            &merged,
            &[FiredToken {
                token: "task-unblocked".into(),
                unblocked_ids: vec!["vault:tasks/m.md".into(), "vault:tasks/a.md".into()],
                ..FiredToken::default()
            }],
        );
        assert_eq!(
            unblocked_again[1].unblocked_ids,
            vec![
                "vault:tasks/a.md".to_string(),
                "vault:tasks/m.md".into(),
                "vault:tasks/z.md".into()
            ]
        );
        let again = merge_fired(
            &merged,
            &[FiredToken {
                token: "file-changed:x.md".into(),
                max_mtime: Some(at(NOW_TS - 100)),
                ..FiredToken::default()
            }],
        );
        assert_eq!(
            again[2].max_mtime,
            Some(at(NOW_TS)),
            "an older mtime never regresses"
        );
    }
}
