//! Activity Ledger — a chronological feed over local heartbeat, computer, and
//! workflow trails. Retired messaging audit rows remain on disk as recovery
//! evidence but are never exposed in the live feed.
//!
//! Distinct from the **ActivityDock** (live, in-flight backend tasks) and the
//! **NotificationCenter** (workflow-completion pops) — this is the durable
//! after-the-fact record.
//!
//! PURE assembly over existing on-disk formats — no new schema, no network, no
//! LLM. The per-source `LedgerEntry` parse adapters are unit-testable against
//! fixture lines; `read` does the file reads + merge. Port of hollow-mac's
//! `LedgerEntry` (adapters) + `ActivityLedgerReader`.

use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub const HEARTBEAT_LOG: &str = ".heartbeat-log.jsonl";
pub const COMPUTER_LOG: &str = ".computer-actions.jsonl";

/// Which subsystem produced a row. Serializes to the lowercase tag the UI keys
/// its source chips off (`heartbeat` / `workflow` / `computer` / `m365`).
pub const SOURCE_HEARTBEAT: &str = "heartbeat";
pub const SOURCE_WORKFLOW: &str = "workflow";
pub const SOURCE_COMPUTER: &str = "computer";
pub const SOURCE_M365: &str = "m365";

/// One row of the activity ledger. `at` is the source's raw RFC3339-UTC stamp
/// (these sort lexically = chronologically). `ok` is the success/failure axis
/// the UI tints on.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub at: String,
    /// One of the `SOURCE_*` constants.
    pub source: String,
    pub title: String,
    pub detail: String,
    pub ok: bool,
    /// Deep-link hint — a workflow slug today, `None` otherwise.
    pub deep_link: Option<String>,
}

impl LedgerEntry {
    /// Map a `.heartbeat-log.jsonl` line (keys: at/reason/status/observations/
    /// surfaced/proposedTasks/actionsTaken/error). Retired message rows are hidden.
    pub fn from_heartbeat(v: &Value, index: usize) -> Option<LedgerEntry> {
        let sends = v.get("proposedSends");
        if sends.and_then(Value::as_u64).is_some_and(|count| count > 0)
            || sends
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
            || crate::review_queue::has_retired_message_reference(&v.to_string())
        {
            return None;
        }
        let at = v.get("at").and_then(|x| x.as_str())?.to_string();
        let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("ok");
        let reason = v
            .get("reason")
            .and_then(|x| x.as_str())
            .unwrap_or("heartbeat");
        let surfaced = v.get("surfaced").and_then(|x| x.as_u64()).unwrap_or(0);
        let tasks = v.get("proposedTasks").and_then(|x| x.as_u64()).unwrap_or(0);
        let mut detail = format!("{surfaced} surfaced · {tasks} tasks proposed");
        if let Some(obs) = v.get("observations").and_then(|x| x.as_str()) {
            if !obs.is_empty() {
                detail = obs.to_string();
            }
        }
        if status != "ok" {
            if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
                if !err.is_empty() {
                    detail = err.to_string();
                }
            }
        }
        Some(LedgerEntry {
            id: format!("heartbeat-{at}-{index}"),
            at,
            source: SOURCE_HEARTBEAT.into(),
            title: reason.to_string(),
            detail,
            ok: status == "ok",
            deep_link: None,
        })
    }

    /// Map a `.computer-actions.jsonl` / `.m365-actions.jsonl` line (keys: ts/
    /// tool/args/ok/outcome). `None` when `ts` is missing.
    pub fn from_audit(v: &Value, source: &str, index: usize) -> Option<LedgerEntry> {
        if source == SOURCE_M365
            || crate::review_queue::has_retired_message_reference(&v.to_string())
        {
            return None;
        }
        let ts = v.get("ts").and_then(|x| x.as_str())?.to_string();
        let tool = v.get("tool").and_then(|x| x.as_str()).unwrap_or("(tool)");
        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(true);
        let outcome = v
            .get("outcome")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        Some(LedgerEntry {
            id: format!("{source}-{ts}-{index}"),
            at: ts,
            source: source.to_string(),
            title: tool.to_string(),
            detail: outcome,
            ok,
            deep_link: None,
        })
    }

    /// Map a workflow `RunLogEntry` (a `## Run log` row) + its slug. A
    /// `deferred` row is informational (a waiting run started once the vault
    /// was free, and its own terminal row follows), so it is not a failure.
    /// `skipped` still is, because no run happened, with two benign
    /// exceptions (Paperclip PR 9b, `vault_workflows::is_benign_skip_note`):
    /// a quiet skip (`no changes since last run`) and an inactive-workspace
    /// skip (`workspace inactive: …`), which skipped because there was
    /// nothing to do. Refusals (`not run: …`) and misfires stay failures.
    pub fn from_run_log(e: &crate::types::RunLogEntry, slug: &str, index: usize) -> LedgerEntry {
        let ok = matches!(e.status.as_str(), "success" | "ok" | "deferred")
            || (e.status == crate::vault_workflows::SKIPPED_STATUS
                && crate::vault_workflows::is_benign_skip_note(&e.note));
        let detail = if e.note.is_empty() {
            format!("{}s", e.duration_sec)
        } else {
            format!("{} ({}s)", e.note, e.duration_sec)
        };
        LedgerEntry {
            id: format!("workflow-{slug}-{}-{index}", e.at),
            at: e.at.clone(),
            source: SOURCE_WORKFLOW.into(),
            title: format!("{slug} · {}", e.status),
            detail,
            ok,
            deep_link: Some(slug.to_string()),
        }
    }
}

/// Merge groups newest-first (lexical RFC3339-UTC descending = chronological),
/// capping at `limit`. Pure.
pub fn merge(groups: Vec<Vec<LedgerEntry>>, limit: usize) -> Vec<LedgerEntry> {
    let mut all: Vec<LedgerEntry> = groups
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.source != SOURCE_M365
                && !crate::review_queue::has_retired_message_reference(&entry.title)
                && !crate::review_queue::has_retired_message_reference(&entry.detail)
                && !entry
                    .deep_link
                    .as_deref()
                    .is_some_and(crate::review_queue::has_retired_message_reference)
        })
        .collect();
    all.sort_by(|a, b| b.at.cmp(&a.at));
    all.truncate(limit);
    all
}

/// Read a JSONL file line-by-line, decode each as a `Value`, and map through
/// `f(value, index)`. A missing file / unparseable line is skipped.
fn read_jsonl<F>(path: &Path, mut f: F) -> Vec<LedgerEntry>
where
    F: FnMut(&Value, usize) -> Option<LedgerEntry>,
{
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, line) in raw.split('\n').enumerate() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(s) else {
            continue;
        };
        if let Some(entry) = f(&v, i) {
            out.push(entry);
        }
    }
    out
}

/// Read + merge local sources from a vault root, newest-first, capped at
/// `limit`. Reads the workflow run logs via `vault_workflows`.
pub fn read(vault: &Path, limit: usize) -> Vec<LedgerEntry> {
    let mut groups: Vec<Vec<LedgerEntry>> = Vec::new();
    groups.push(read_jsonl(
        &vault.join(HEARTBEAT_LOG),
        LedgerEntry::from_heartbeat,
    ));
    groups.push(read_jsonl(&vault.join(COMPUTER_LOG), |v, i| {
        LedgerEntry::from_audit(v, SOURCE_COMPUTER, i)
    }));

    // Workflow run logs — read each workflow's `## Run log` via the repository.
    let mut wf: Vec<LedgerEntry> = Vec::new();
    for w in crate::vault_workflows::list(vault) {
        if let Ok((_, _, entries)) = crate::vault_workflows::read_full(vault, &w.slug) {
            for (i, e) in entries.iter().enumerate() {
                wf.push(LedgerEntry::from_run_log(e, &w.slug, i));
            }
        }
    }
    groups.push(wf);

    merge(groups, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RunLogEntry;
    use serde_json::json;

    #[test]
    fn from_heartbeat_maps_ok_error_and_bare() {
        let ok = json!({
            "at": "2026-06-19T10:00:00+00:00", "reason": "morning beat",
            "status": "ok", "surfaced": 3, "proposedTasks": 1,
            "observations": "Prepare the next meeting.",
        });
        let e = LedgerEntry::from_heartbeat(&ok, 0).unwrap();
        assert_eq!(e.source, SOURCE_HEARTBEAT);
        assert_eq!(e.title, "morning beat");
        assert_eq!(e.detail, "Prepare the next meeting."); // observations win over counts
        assert!(e.ok);

        let err = json!({
            "at": "2026-06-19T11:00:00+00:00", "reason": "beat",
            "status": "error", "error": "timed out",
        });
        let e2 = LedgerEntry::from_heartbeat(&err, 1).unwrap();
        assert!(!e2.ok);
        assert_eq!(e2.detail, "timed out");

        let bare = json!({ "at": "2026-06-19T09:00:00+00:00", "status": "ok" });
        assert_eq!(
            LedgerEntry::from_heartbeat(&bare, 2).unwrap().detail,
            "0 surfaced · 0 tasks proposed"
        );
        assert!(LedgerEntry::from_heartbeat(&json!({ "status": "ok" }), 3).is_none());
    }

    #[test]
    fn from_audit_maps_action_line() {
        let v = json!({
            "ts": "2026-06-19T12:00:00+00:00", "tool": "run_applescript",
            "ok": false, "outcome": "permission denied",
        });
        let e = LedgerEntry::from_audit(&v, SOURCE_COMPUTER, 0).unwrap();
        assert_eq!(e.source, SOURCE_COMPUTER);
        assert_eq!(e.title, "run_applescript");
        assert_eq!(e.detail, "permission denied");
        assert!(!e.ok);
        assert!(LedgerEntry::from_audit(&json!({ "tool": "x" }), SOURCE_M365, 0).is_none());
    }

    #[test]
    fn retired_sends_and_draft_links_never_reappear_in_activity() {
        for retired in [
            json!({"at": "2026-06-19T12:00:00Z", "proposedSends": 1}),
            json!({"at": "2026-06-19T12:00:00Z", "proposedSends": [{"body": "old draft"}]}),
            json!({"at": "2026-06-19T12:00:00Z", "observations": "See vault:drafts/heartbeat/reply-old.md"}),
            json!({"at": "2026-06-19T12:00:00Z", "actionsTaken": ["Reviewed teams/dms/old.md"]}),
        ] {
            assert!(LedgerEntry::from_heartbeat(&retired, 0).is_none());
        }
        let legacy_audit = json!({"ts": "2026-06-19T12:00:00Z", "tool": "send_email", "ok": true});
        assert!(LedgerEntry::from_audit(&legacy_audit, SOURCE_M365, 0).is_none());
        let local = LedgerEntry::from_heartbeat(
            &json!({
                "at": "2026-06-19T12:00:00Z", "proposedSends": 0,
                "proposedTasks": 2, "surfaced": 1,
            }),
            0,
        )
        .unwrap();
        assert_eq!(local.detail, "1 surfaced · 2 tasks proposed");
        let linked = LedgerEntry {
            deep_link: Some("vault:emails/inbox/old.md".into()),
            ..local.clone()
        };
        assert_eq!(merge(vec![vec![linked, local.clone()]], 50), vec![local]);
    }

    #[test]
    fn from_run_log_maps_and_carries_slug_deep_link() {
        let e = LedgerEntry::from_run_log(
            &RunLogEntry {
                at: "2026-06-19T08:00:00+00:00".into(),
                status: "success".into(),
                duration_sec: 12,
                note: "brief written".into(),
                run_id: Some("run-123".into()),
                error_tail: None,
                exit_kind: None,
            },
            "morning-brief",
            0,
        );
        assert_eq!(e.source, SOURCE_WORKFLOW);
        assert_eq!(e.title, "morning-brief · success");
        assert!(e.ok);
        assert_eq!(e.deep_link.as_deref(), Some("morning-brief"));
        assert!(e.detail.contains("brief written") && e.detail.contains("12s"));

        let fail = LedgerEntry::from_run_log(
            &RunLogEntry {
                at: "x".into(),
                status: "error".into(),
                duration_sec: 1,
                note: String::new(),
                run_id: None,
                error_tail: None,
                exit_kind: None,
            },
            "w",
            1,
        );
        assert!(!fail.ok);
    }

    #[test]
    fn deferred_run_log_rows_are_informational() {
        let row = |status: &str| RunLogEntry {
            at: "2026-09-25T09:00:00+00:00".into(),
            status: status.into(),
            duration_sec: 0,
            note: "waited for another run".into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        };
        let deferred = LedgerEntry::from_run_log(&row("deferred"), "close-my-day", 0);
        assert!(
            deferred.ok,
            "a waiting run that then started is not a failure"
        );
        assert_eq!(deferred.title, "close-my-day · deferred");
        assert!(deferred.detail.contains("waited for another run"));
        let skipped = LedgerEntry::from_run_log(&row("skipped"), "close-my-day", 1);
        assert!(!skipped.ok, "a skipped occurrence is still flagged");
    }

    #[test]
    fn benign_skips_are_not_failures() {
        let skipped = |note: &str| RunLogEntry {
            at: "2026-09-25T09:00:00+00:00".into(),
            status: "skipped".into(),
            duration_sec: 0,
            note: note.into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        };
        let now = chrono::Utc::now();
        let quiet = crate::vault_workflows::IdleSkip::Quiet.entry(now);
        let inactive = crate::vault_workflows::IdleSkip::Inactive {
            workspace: "Launch plan".into(),
            status: "done".into(),
        }
        .entry(now);
        let coalesced = crate::vault_workflows::IdleSkip::Quiet.coalesced_note(3);
        for row in [quiet, inactive, skipped(&coalesced)] {
            assert!(
                LedgerEntry::from_run_log(&row, "brief", 0).ok,
                "{}",
                row.note
            );
        }
        for note in [
            "not run: automation settings changed; review them in Automations → Permission changes",
            crate::vault_workflows::MISSED_WHILE_CLOSED_NOTE,
            "not run: budget paused (all usage)",
            "not run: skipped from the run queue",
        ] {
            assert!(
                !LedgerEntry::from_run_log(&skipped(note), "brief", 0).ok,
                "{note}"
            );
        }
    }

    #[test]
    fn merge_sorts_newest_first_and_caps() {
        let a = vec![LedgerEntry {
            id: "1".into(),
            at: "2026-06-19T08:00:00+00:00".into(),
            source: SOURCE_HEARTBEAT.into(),
            title: "a".into(),
            detail: String::new(),
            ok: true,
            deep_link: None,
        }];
        let b = vec![
            LedgerEntry {
                id: "2".into(),
                at: "2026-06-19T12:00:00+00:00".into(),
                source: SOURCE_COMPUTER.into(),
                title: "b".into(),
                detail: String::new(),
                ok: true,
                deep_link: None,
            },
            LedgerEntry {
                id: "3".into(),
                at: "2026-06-19T10:00:00+00:00".into(),
                source: SOURCE_WORKFLOW.into(),
                title: "c".into(),
                detail: String::new(),
                ok: true,
                deep_link: None,
            },
        ];
        let merged = merge(vec![a.clone(), b.clone()], 10);
        assert_eq!(
            merged.iter().map(|e| e.title.clone()).collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
        assert_eq!(merge(vec![a, b], 2).len(), 2);
    }

    #[test]
    fn read_merges_jsonl_trails_from_a_vault() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::write(
            v.join(HEARTBEAT_LOG),
            "{\"at\":\"2026-06-19T08:00:00+00:00\",\"reason\":\"beat\",\"status\":\"ok\",\"observations\":\"all quiet\"}\n",
        )
        .unwrap();
        std::fs::write(
            v.join(COMPUTER_LOG),
            "{\"ts\":\"2026-06-19T12:00:00+00:00\",\"tool\":\"open_url\",\"ok\":true,\"outcome\":\"opened\"}\n\ngarbage line\n",
        )
        .unwrap();
        let historical_m365 = "{\"ts\":\"2026-06-19T13:00:00+00:00\",\"tool\":\"send_email\",\"ok\":true,\"outcome\":\"sent\"}\n";
        std::fs::write(v.join(".m365-actions.jsonl"), historical_m365).unwrap();
        let rows = read(v, 50);
        assert_eq!(rows.len(), 2); // garbage line skipped
        assert_eq!(rows[0].title, "open_url"); // 12:00 newest
        assert_eq!(rows[1].title, "beat");
        assert_eq!(
            std::fs::read_to_string(v.join(".m365-actions.jsonl")).unwrap(),
            historical_m365
        );
    }
}
