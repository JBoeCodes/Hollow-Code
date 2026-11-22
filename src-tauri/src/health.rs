//! Liveness derived from outcomes, never from configuration.
//!
//! The defect this exists to end: the old Notion importer (removed 2026-11-10) computed "content ready" from the
//! presence of a token and a data-source id and never touched the network, so it
//! reported green for the 18 days its data source sat archived and in the trash.
//! `plaud.rs` defined "connected" as a token file existing, so it reported green
//! for the 9 days recordings sync was dark. Both were true statements about
//! config and false statements about the world.
//!
//! One principle: **a subsystem is live because something recently worked, not
//! because it is configured.** So every subsystem records the outcome of its
//! real attempts here, and every surface that claims health reads this rather
//! than re-deriving it from settings.
//!
//! The store is an append-only `.hollow-health.jsonl` in the vault root,
//! dot-prefixed so the watcher, tree, search, graph and history all ignore it.
//! Writes are best-effort and must never fail a caller: a connector that cannot
//! journal its failure has a worse problem than the journal.
//!
//! This also replaces the failure `eprintln!` sites, which wrote to the stderr
//! of a Finder-launched `.app` — i.e. to nowhere.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Keep the file bounded. Reads take the newest `READ_CAP`; compaction drops
/// anything older once the file exceeds `COMPACT_AT` lines.
const READ_CAP: usize = 400;
const COMPACT_AT: usize = 2_000;

pub const FILE: &str = ".hollow-health.jsonl";

/// What happened on one real attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The subsystem did its job. This is the only thing that makes it live.
    Ok,
    /// It tried and failed. Carries a reason.
    Failed,
    /// It could not try, because it is not set up. Distinct from `Failed`: a
    /// connector the user never configured is not broken.
    Unconfigured,
    /// It declined to try because credentials are missing or expired. The user
    /// can fix this, which is why it is not lumped in with `Failed`.
    NeedsAuth,
    /// It ran, but did less than asked (partial sync, some items skipped).
    Partial,
}

impl Outcome {
    /// Only a full success counts as evidence of liveness.
    #[cfg(test)]
    pub fn is_live(self) -> bool {
        matches!(self, Outcome::Ok)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: DateTime<Utc>,
    /// Stable id: `tracker`, `plaud`, `email`, `calendar`, `teams`, `slack`,
    /// `workflows`, `heartbeat`, `vault-watch`, `usage` (usage-ledger write
    /// failures only, redacted; written from the GUI and from `hollow
    /// team-mcp`, and surfaced as Settings, Usage's `ledgerError`),
    /// `budgets` (an unreadable `budgets.json`), `history` (a vault lease
    /// reclaimed from a Hollow process that exited, Paperclip PR 18).
    pub subsystem: String,
    pub outcome: Outcome,
    /// One line, user-readable. Never a raw error dump.
    pub detail: String,
}

/// What a surface needs to state a subsystem's health honestly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(test)]
pub struct Status {
    pub subsystem: String,
    /// Newest entry of any kind.
    pub last: Option<Entry>,
    /// Newest `Ok`. Absence is the whole point: a subsystem with a token, a
    /// config, and no successful attempt in 18 days is not live.
    pub last_ok: Option<DateTime<Utc>>,
    /// Consecutive non-`Ok` outcomes since the last success.
    pub consecutive_failures: u32,
}

#[cfg(test)]
impl Status {
    /// Live only if the newest outcome succeeded. Deliberately strict: "it
    /// worked once last month" is not a health claim.
    #[cfg(test)]
    pub fn is_live(&self) -> bool {
        self.last
            .as_ref()
            .map(|e| e.outcome.is_live())
            .unwrap_or(false)
    }
}

fn path(vault: &Path) -> PathBuf {
    vault.join(FILE)
}

/// Journal one outcome. Best-effort by design — never propagates an error,
/// because a caller's failure path must not acquire a second failure mode.
pub fn record(vault: &Path, subsystem: &str, outcome: Outcome, detail: impl Into<String>) {
    let entry = Entry {
        at: Utc::now(),
        subsystem: subsystem.to_string(),
        outcome,
        detail: detail.into(),
    };
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    let p = path(vault);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{line}");
    }
    maybe_compact(vault);
}

fn read_all(vault: &Path) -> Vec<Entry> {
    let Ok(f) = std::fs::File::open(path(vault)) else {
        return vec![];
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        // A torn final line from a crash mid-write must not poison the read.
        .filter_map(|l| serde_json::from_str::<Entry>(&l).ok())
        .collect()
}

/// Newest first, bounded.
pub fn recent(vault: &Path) -> Vec<Entry> {
    let mut all = read_all(vault);
    all.reverse();
    all.truncate(READ_CAP);
    all
}

/// Roll up one subsystem. This is what a health surface should call.
#[cfg(test)]
pub fn status(vault: &Path, subsystem: &str) -> Status {
    let all = read_all(vault);
    let mine: Vec<&Entry> = all.iter().filter(|e| e.subsystem == subsystem).collect();
    let last = mine.last().cloned().cloned();
    let last_ok = mine
        .iter()
        .rev()
        .find(|e| e.outcome.is_live())
        .map(|e| e.at);
    let consecutive_failures = mine
        .iter()
        .rev()
        .take_while(|e| !e.outcome.is_live())
        .count() as u32;
    Status {
        subsystem: subsystem.to_string(),
        last,
        last_ok,
        consecutive_failures,
    }
}

/// Every subsystem that has ever reported, newest-activity first.
#[cfg(test)]
pub fn all_statuses(vault: &Path) -> Vec<Status> {
    let all = read_all(vault);
    let mut names: Vec<String> = vec![];
    for e in &all {
        if !names.contains(&e.subsystem) {
            names.push(e.subsystem.clone());
        }
    }
    let mut out: Vec<Status> = names.iter().map(|n| status(vault, n)).collect();
    out.sort_by(|a, b| {
        let ax = a.last.as_ref().map(|e| e.at);
        let bx = b.last.as_ref().map(|e| e.at);
        bx.cmp(&ax)
    });
    out
}

/// Trim the oldest entries once the file grows past `COMPACT_AT`. Rewrites via a
/// temp file so a crash mid-compaction cannot truncate the journal.
fn maybe_compact(vault: &Path) {
    let p = path(vault);
    let Ok(meta) = std::fs::metadata(&p) else {
        return;
    };
    // Cheap guard: only count lines when the file is plausibly large.
    if meta.len() < 64 * 1024 {
        return;
    }
    let all = read_all(vault);
    if all.len() <= COMPACT_AT {
        return;
    }
    let keep = &all[all.len() - READ_CAP..];
    let mut buf = String::new();
    for e in keep {
        if let Ok(l) = serde_json::to_string(e) {
            buf.push_str(&l);
            buf.push('\n');
        }
    }
    let _ = crate::vault::atomic_write(&p, buf.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_configured_subsystem_that_never_succeeded_is_not_live() {
        // The exact defect: configured, tried, failed for days, green.
        let d = vault();
        for _ in 0..5 {
            record(
                d.path(),
                "tracker",
                Outcome::Failed,
                "data source is archived",
            );
        }
        let s = status(d.path(), "tracker");
        assert!(!s.is_live(), "failure must not read as live");
        assert_eq!(s.last_ok, None, "there was never a success to point at");
        assert_eq!(s.consecutive_failures, 5);
        assert_eq!(s.last.unwrap().detail, "data source is archived");
    }

    #[test]
    fn a_success_clears_the_failure_streak_and_a_later_failure_ends_liveness() {
        let d = vault();
        record(d.path(), "plaud", Outcome::Failed, "no recordings pulled");
        record(d.path(), "plaud", Outcome::Ok, "3 recordings");
        let s = status(d.path(), "plaud");
        assert!(s.is_live());
        assert_eq!(s.consecutive_failures, 0);
        assert!(s.last_ok.is_some());

        // Going dark again must flip liveness immediately, while still
        // remembering when it last worked.
        record(d.path(), "plaud", Outcome::Failed, "token expired");
        let s2 = status(d.path(), "plaud");
        assert!(!s2.is_live(), "the newest outcome decides liveness");
        assert!(
            s2.last_ok.is_some(),
            "but the last success is still reportable"
        );
        assert_eq!(s2.consecutive_failures, 1);
    }

    #[test]
    fn unconfigured_and_needs_auth_are_not_failures() {
        let d = vault();
        record(d.path(), "slack", Outcome::Unconfigured, "no token set");
        let s = status(d.path(), "slack");
        assert!(!s.is_live());
        assert_eq!(s.last.unwrap().outcome, Outcome::Unconfigured);
        // Distinguishable so a surface can say "not set up" rather than "broken".
        record(d.path(), "m365", Outcome::NeedsAuth, "sign in again");
        assert_eq!(
            status(d.path(), "m365").last.unwrap().outcome,
            Outcome::NeedsAuth
        );
    }

    #[test]
    fn partial_is_not_live_because_it_did_less_than_asked() {
        let d = vault();
        record(
            d.path(),
            "teams",
            Outcome::Partial,
            "channels skipped, chats synced",
        );
        assert!(!status(d.path(), "teams").is_live());
    }

    #[test]
    fn subsystems_do_not_bleed_into_each_other() {
        let d = vault();
        record(d.path(), "tracker", Outcome::Failed, "boom");
        record(d.path(), "email", Outcome::Ok, "12 messages");
        assert!(!status(d.path(), "tracker").is_live());
        assert!(status(d.path(), "email").is_live());
        assert_eq!(status(d.path(), "never-reported").consecutive_failures, 0);
        assert!(status(d.path(), "never-reported").last.is_none());
    }

    #[test]
    fn recording_never_panics_on_an_unwritable_vault() {
        // A connector's failure path must not acquire a second failure mode.
        record(
            Path::new("/nonexistent/definitely/not/here"),
            "tracker",
            Outcome::Failed,
            "x",
        );
    }

    #[test]
    fn a_torn_line_does_not_poison_the_read() {
        let d = vault();
        record(d.path(), "email", Outcome::Ok, "fine");
        // Simulate a crash mid-write.
        let mut f = OpenOptions::new()
            .append(true)
            .open(d.path().join(FILE))
            .unwrap();
        writeln!(f, "{{\"at\":\"not-a-date\",\"subsy").unwrap();
        drop(f);
        record(d.path(), "email", Outcome::Ok, "still fine");
        let s = status(d.path(), "email");
        assert!(s.is_live());
        assert_eq!(
            recent(d.path()).len(),
            2,
            "the torn line is skipped, not fatal"
        );
    }

    #[test]
    fn recent_is_newest_first_and_all_statuses_ranks_by_activity() {
        let d = vault();
        record(d.path(), "a", Outcome::Ok, "first");
        record(d.path(), "b", Outcome::Ok, "second");
        let r = recent(d.path());
        assert_eq!(r[0].detail, "second", "newest first");
        let all = all_statuses(d.path());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].subsystem, "b", "most recently active first");
    }
}
