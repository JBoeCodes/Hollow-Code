//! Memory change reports (ME1, 2026-11-18): what one capture run did to
//! memory, written to the vault so a person (or a later check) can read why
//! a learning did or didn't land.
//!
//! A report lists four things: `added` (a new candidate queued for review,
//! or a Home or Org fact routed, or a Know-how note saved with Undo),
//! `updated` and `removed` (a candidate that rewords or takes out a saved
//! memory; capture makes none today, memory maintenance and later rows do),
//! and `skipped`, each with a [`Reason`] code. A near-duplicate is skipped
//! with the memory it resembles, never merged into it: Hollow does not merge
//! two memories on similarity alone (only a person's Save of a proposal that
//! names the note it edits changes a saved memory).
//!
//! Reports are one JSON object per line in
//! `.hollow/memory-reports/<UTC day>.jsonl`, appended through an atomic
//! rewrite of that day's file, kept [`KEEP_DAYS`] days. Statements are capped
//! and any text the memory safety scan flags is withheld (`text` absent), so
//! a refused secret never reaches the report.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::vault_learning::LearningProposalPayload;

/// Where the reports live, relative to the vault root.
pub const REPORTS_DIR: &str = ".hollow/memory-reports";
/// Day files older than this are removed when a new report is written.
pub const KEEP_DAYS: i64 = 30;
/// Report format version.
const SCHEMA: u32 = 1;
/// Longest statement a report keeps.
const MAX_TEXT_CHARS: usize = 300;

static WRITE: Mutex<()> = Mutex::new(());

/// Why a capture left a candidate out. Stable snake_case codes: later rows
/// (NV7's eval candidates) add their own; PX5 added `already_in_profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Paraphrases a saved, pending or rejected learning (the local screen).
    NearDuplicate,
    /// Says exactly a line a `me/` note already holds (PX5, one home per
    /// fact): `similarTo` names the note and the line.
    AlreadyInProfile,
    /// The optional Jev duplicate check judged it a repeat.
    DuplicateCheck,
    /// The exact proposal is already in the queue (same fingerprint).
    AlreadyProposed,
    /// No exact quote from the person's words or a tool result backs it.
    Ungrounded,
    /// A Home or Org fact not stated in the person's own words.
    NotStatedByPerson,
    /// A Home fact from a turn that ran on another machine.
    OtherMachine,
    /// The memory safety scan refused it (its text is withheld).
    Unsafe,
    /// The proposal failed validation (kind, target, empty words).
    Invalid,
    /// Its target refused it (a Home or Org route, a skill lesson); see
    /// `detail`.
    TargetRefused,
    /// Automatic capture keeps one lesson per run; this was not the first.
    OnePerCapture,
    /// It touches a sensitive topic (health, religion, ...) and the person
    /// did not ask Hollow to remember it (ND7); `detail` names the class.
    Sensitive,
    /// Seen once (ND7): a tentative sighting, saved only when a second
    /// message of the person repeats it.
    Tentative,
    /// Queueing it failed; see `detail`.
    WriteFailed,
}

/// One candidate in a report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// The review item, once queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub target_kind: String,
    pub target_path: String,
    /// The statement, capped; absent when the safety scan flags it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The queue status after this run (`pending`, `accepted`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// For a skip: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    /// For a near-duplicate: what it resembles ("a saved learning") and
    /// that memory's words, capped. Kept apart; never merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similar_to: Option<String>,
    /// A target's own refusal or a write error, capped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Change {
    /// A candidate as a report shows it.
    pub fn of(payload: &LearningProposalPayload) -> Change {
        Change {
            target_kind: payload.target_kind.clone(),
            target_path: payload.target_path.clone(),
            text: shown(&payload.statement),
            ..Default::default()
        }
    }
}

/// What one capture run changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeReport {
    pub schema: u32,
    /// `capture` (a chat's background capture) or `exchange` (the
    /// extractor after a turn without the automatic rules).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub at: String,
    pub added: Vec<Change>,
    pub updated: Vec<Change>,
    pub removed: Vec<Change>,
    pub skipped: Vec<Change>,
}

impl ChangeReport {
    pub fn new(kind: &str, run_id: Option<&str>, session: Option<&str>) -> ChangeReport {
        ChangeReport {
            schema: SCHEMA,
            kind: kind.to_string(),
            run_id: run_id.map(str::to_string),
            session: session.map(str::to_string),
            ..Default::default()
        }
    }

    /// A queued or saved candidate, filed by what it does: a proposal that
    /// names a saved note or line rewords it (`updated`) or takes it out
    /// (`removed`); anything else is `added`.
    pub fn queued(&mut self, payload: &LearningProposalPayload, id: &str, status: &str) {
        let mut change = Change::of(payload);
        change.id = Some(id.to_string());
        change.status = Some(status.to_string());
        match &payload.retires {
            Some(retire) if retire.line.trim() == payload.statement.trim() => {
                self.removed.push(change)
            }
            Some(_) => self.updated.push(change),
            None if payload.edits_path.is_some() => self.updated.push(change),
            None => self.added.push(change),
        }
    }

    /// Whether the run had no candidate at all (nothing to report).
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.updated.is_empty()
            && self.removed.is_empty()
            && self.skipped.is_empty()
    }

    /// A fact routed past the queue (a Home line, an Org proposal).
    pub fn routed(&mut self, payload: &LearningProposalPayload, detail: &str) {
        let mut change = Change::of(payload);
        change.detail = Some(cap(detail));
        self.added.push(change);
    }

    pub fn skip(&mut self, payload: &LearningProposalPayload, reason: Reason) {
        self.skip_with(payload, reason, None, None);
    }

    pub fn skip_with(
        &mut self,
        payload: &LearningProposalPayload,
        reason: Reason,
        similar_to: Option<String>,
        detail: Option<&str>,
    ) {
        let mut change = Change::of(payload);
        // Neither an unsafe nor a sensitive statement is written to the file.
        if matches!(reason, Reason::Unsafe | Reason::Sensitive) {
            change.text = None;
        }
        change.reason = Some(reason);
        change.similar_to = similar_to.and_then(|s| shown(&s));
        change.detail = detail.map(cap);
        self.skipped.push(change);
    }

    /// Append this report to today's file. Never fails the capture: the
    /// caller logs an error.
    pub fn write(&mut self, vault: &Path) -> Result<PathBuf, String> {
        self.write_at(vault, Utc::now())
    }

    pub fn write_at(&mut self, vault: &Path, now: DateTime<Utc>) -> Result<PathBuf, String> {
        self.at = now.to_rfc3339();
        let dir = vault.join(REPORTS_DIR);
        let path = dir.join(format!("{}.jsonl", now.format("%Y-%m-%d")));
        let line = serde_json::to_string(self).map_err(|e| format!("encode report: {e}"))?;
        let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {REPORTS_DIR}: {e}"))?;
        let mut body = std::fs::read(&path).unwrap_or_default();
        // A torn last line (a crash mid-write elsewhere) is ended, not joined.
        if !body.is_empty() && !body.ends_with(b"\n") {
            body.push(b'\n');
        }
        body.extend_from_slice(line.as_bytes());
        body.push(b'\n');
        crate::vault::atomic_write(&path, &body)?;
        prune(&dir, now.date_naive());
        Ok(path)
    }
}

/// The reports of one UTC day, oldest first; unreadable lines are skipped.
/// Test-only until a reader (PX5, NV7) lands.
#[cfg(test)]
pub fn read_day(vault: &Path, day: NaiveDate) -> Vec<ChangeReport> {
    let path = vault
        .join(REPORTS_DIR)
        .join(format!("{}.jsonl", day.format("%Y-%m-%d")));
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn prune(dir: &Path, today: NaiveDate) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let oldest = today - Duration::days(KEEP_DAYS);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(day) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".jsonl"))
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if day < oldest {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Text a report may show: capped, and withheld when the memory safety scan
/// flags it (a skipped candidate may never have been scanned).
fn shown(text: &str) -> Option<String> {
    let text = cap(text);
    crate::memory_guard::check(&[("the statement", &text)])
        .ok()
        .map(|_| text)
}

fn cap(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_TEXT_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_TEXT_CHARS - 3).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    fn payload(statement: &str) -> LearningProposalPayload {
        LearningProposalPayload {
            kind: "preference".into(),
            target_kind: "project".into(),
            target_path: "vault:workspaces/x/WORKSPACE.md".into(),
            subject: "s".into(),
            statement: statement.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_report_lands_in_the_day_file_and_reads_back() {
        let dir = tempdir().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0).unwrap();
        let mut report = ChangeReport::new("capture", Some("run-1"), Some("chat-1"));
        assert!(report.is_empty());
        report.queued(
            &payload("Deploys go out on Tuesdays."),
            "review-1",
            "pending",
        );
        report.skip_with(
            &payload("Deploys happen on Tuesday."),
            Reason::NearDuplicate,
            Some("a saved learning: Deploys go out on Tuesdays.".into()),
            None,
        );
        let path = report.write_at(dir.path(), now).unwrap();
        assert!(path.ends_with(".hollow/memory-reports/2026-11-18.jsonl"));
        let mut second = ChangeReport::new("capture", Some("run-2"), None);
        second.write_at(dir.path(), now).unwrap();
        let read = read_day(dir.path(), now.date_naive());
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], report);
        let line = std::fs::read_to_string(&path).unwrap();
        assert!(line.contains("\"reason\":\"near_duplicate\""), "{line}");
    }

    #[test]
    fn unsafe_text_is_withheld_and_long_text_capped() {
        let mut report = ChangeReport::new("capture", None, None);
        // Built, not written out, so no scanner reads a key in this file.
        let token: String = "Zq7Lm".repeat(8).chars().take(36).collect();
        let secret = format!("Deploy with the token ghp_{token} from now on.");
        report.skip(&payload(&secret), Reason::Unsafe);
        report.skip(&payload(&secret), Reason::Ungrounded);
        assert_eq!(report.skipped[0].text, None);
        assert_eq!(report.skipped[1].text, None, "the scan withholds it anyway");
        report.queued(&payload(&"word ".repeat(200)), "r", "pending");
        assert_eq!(
            report.added[0].text.as_ref().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
    }

    #[test]
    fn a_proposal_naming_a_saved_line_is_an_update_or_a_removal() {
        let mut report = ChangeReport::new("capture", None, None);
        let mut reword = payload("Use Outlook for all mail.");
        reword.retires = Some(crate::vault_learning::RetireLine {
            proposal: "p".into(),
            path: "me/Preferences.md".into(),
            line: "Use Gmail.".into(),
            cycle: "stale:2026-10-01".into(),
        });
        report.queued(&reword, "a", "pending");
        let mut removal = reword.clone();
        removal.statement = "Use Gmail.".into();
        report.queued(&removal, "b", "pending");
        let mut edit = payload("New words.");
        edit.edits_path = Some("learnings/x.md".into());
        report.queued(&edit, "c", "pending");
        assert_eq!(report.updated.len(), 2);
        assert_eq!(report.removed.len(), 1);
        assert!(report.added.is_empty());
    }

    #[test]
    fn old_day_files_are_pruned() {
        let dir = tempdir().unwrap();
        let reports = dir.path().join(REPORTS_DIR);
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("2026-09-01.jsonl"), "{}\n").unwrap();
        std::fs::write(reports.join("notes.txt"), "keep").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0).unwrap();
        ChangeReport::new("capture", None, None)
            .write_at(dir.path(), now)
            .unwrap();
        assert!(!reports.join("2026-09-01.jsonl").exists());
        assert!(reports.join("notes.txt").exists());
        assert!(reports.join("2026-11-18.jsonl").exists());
    }
}
