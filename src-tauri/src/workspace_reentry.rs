//! A small, deterministic return-to-work snapshot. Prose never proves completion.
use crate::{
    session_store,
    types::AgentSessionSummary,
    workspaces::{self, Workspace},
};
use chrono::Utc;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReentry {
    pub as_of: String,
    pub next_step: Option<String>,
    pub open_items: Vec<String>,
    pub open_item_count: Option<usize>,
    pub last_run: Option<LastRun>,
    pub gaps: Vec<String>,
    /// The brief looks stale (overnight F3): `WORKSPACE.md` unchanged for
    /// [`STALE_BRIEF_DAYS`]+ days while conversations here were active since.
    /// File-level: Hollow keeps no per-section history of the brief.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub brief_stale: Option<BriefStale>,
}

/// How stale a brief looks (overnight F3).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BriefStale {
    /// Whole days since `WORKSPACE.md` last changed.
    pub days: i64,
    /// Conversations here active since then.
    pub conversations: usize,
}

/// A brief unchanged this long while work went on is worth a refresh.
pub const STALE_BRIEF_DAYS: i64 = 7;
/// A session's activity is stamped when its turn ends, after the turn wrote
/// the brief: activity this soon after the change is the same work.
const STALE_GRACE_MINUTES: i64 = 60;

/// `Some` when the brief changed `STALE_BRIEF_DAYS`+ days before `now` and
/// some conversation was active more than an hour after that change (the
/// turn that wrote the brief ends a little later, and isn't "work since").
pub fn brief_stale<'a>(
    brief_changed: chrono::DateTime<Utc>,
    activity: impl IntoIterator<Item = &'a str>,
    now: chrono::DateTime<Utc>,
) -> Option<BriefStale> {
    let days = (now - brief_changed).num_days();
    if days < STALE_BRIEF_DAYS {
        return None;
    }
    let conversations = activity
        .into_iter()
        .filter(|a| {
            chrono::DateTime::parse_from_rfc3339(a).is_ok_and(|t| {
                t.with_timezone(&Utc)
                    > brief_changed + chrono::Duration::minutes(STALE_GRACE_MINUTES)
            })
        })
        .count();
    (conversations > 0).then_some(BriefStale {
        days,
        conversations,
    })
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ReentryLastRun"))]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub session_key: String,
    pub status: Option<String>,
    pub finished_at: Option<String>,
    /// None means no matching finalized receipt, not zero failures.
    pub failed_checks: Option<usize>,
    pub check_count: Option<usize>,
    pub unknown_checks: Option<usize>,
    pub verification: Option<String>,
}

/// A bounded plain-text excerpt of the brief's explicit Next step section.
/// Ignore fenced examples and stop at the next H1/H2 heading.
pub(crate) fn next_step(text: &str) -> Option<String> {
    let mut found = false;
    let mut fence: Option<char> = None;
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            let marker = line.chars().next().unwrap();
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            if found {
                break;
            }
            found = heading
                .trim()
                .trim_end_matches('#')
                .trim()
                .eq_ignore_ascii_case("next step");
            continue;
        }
        if found && line.starts_with("# ") {
            break;
        }
        if found && !line.is_empty() {
            lines.push(line);
        }
    }
    let value = lines.join(" ");
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(480).collect())
    }
}

pub fn read(
    root: &Path,
    workspace: &Workspace,
    sessions: &[AgentSessionSummary],
) -> WorkspaceReentry {
    let mut result = WorkspaceReentry {
        as_of: Utc::now().to_rfc3339(),
        next_step: None,
        open_items: vec![],
        open_item_count: None,
        last_run: None,
        gaps: vec![],
        brief_stale: None,
    };
    let brief_rel = format!("{}/WORKSPACE.md", workspace.path);
    if workspace.kind.as_deref() != Some("quick") {
        let changed = crate::vault::safe_join(root, &brief_rel)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<Utc>::from);
        result.brief_stale = changed.and_then(|c| {
            brief_stale(
                c,
                sessions.iter().map(|s| s.last_activity_at.as_str()),
                Utc::now(),
            )
        });
    }
    match workspaces::read_file(root, &brief_rel) {
        Ok(note) => result.next_step = next_step(&note.content),
        Err(_) => result
            .gaps
            .push("The workspace brief could not be read.".into()),
    }
    match workspaces::read_file(root, &format!("{}/PLAN.md", workspace.path)) {
        Ok(note) => {
            let items = crate::vault_tasks::open_checklist_items(&note.content);
            result.open_item_count = Some(items.len());
            result.open_items = items
                .into_iter()
                .take(3)
                .map(|s| s.chars().take(240).collect())
                .collect();
        }
        Err(_) => result
            .gaps
            .push("The plan could not be read; unfinished work is unknown.".into()),
    }
    // list_sessions already sorts by activity, but keep the contract explicit.
    if let Some(session) = sessions
        .iter()
        .max_by_key(|s| chrono::DateTime::parse_from_rfc3339(&s.last_activity_at).ok())
    {
        match session_store::latest_turn(root, &session.key) {
            Ok(Some(turn)) => {
                let receipt = if turn.ended_at.is_some() {
                    turn.run_id
                        .as_deref()
                        .and_then(|id| {
                            crate::evidence::find_by_history_run(root, id)
                                .ok()
                                .flatten()
                        })
                        .filter(|r| {
                            r.source == "chat"
                                && r.session_key.as_deref() == Some(session.key.as_str())
                                && Some(r.status.as_str()) == turn.status.as_deref()
                                && !r.finished_at.is_empty()
                        })
                } else {
                    None
                };
                if turn.ended_at.is_some() && receipt.is_none() {
                    result
                        .gaps
                        .push("Hollow has no finished record of the last run.".into());
                }
                result.last_run = Some(LastRun {
                    session_key: session.key.clone(),
                    status: turn.status,
                    finished_at: turn.ended_at,
                    failed_checks: receipt
                        .as_ref()
                        .map(|r| r.checks.iter().filter(|c| c.outcome == "failed").count()),
                    check_count: receipt.as_ref().map(|r| r.checks.len()),
                    unknown_checks: receipt.as_ref().map(|r| {
                        r.checks
                            .iter()
                            .filter(|c| !matches!(c.outcome.as_str(), "passed" | "failed"))
                            .count()
                    }),
                    verification: receipt.map(|r| r.verification_state),
                });
            }
            Ok(None) => result
                .gaps
                .push("No run record is available for the last conversation.".into()),
            Err(_) => result
                .gaps
                .push("The last run record could not be read.".into()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_brief_is_stale_only_when_old_and_work_went_on_after_it() {
        let at = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&Utc)
        };
        let now = at("2026-10-10T12:00:00Z");
        let changed = at("2026-10-01T12:00:00Z");
        let later = ["2026-10-08T09:00:00Z", "2026-09-20T09:00:00Z"];
        assert_eq!(
            brief_stale(changed, later, now),
            Some(BriefStale {
                days: 9,
                conversations: 1
            })
        );
        assert_eq!(
            brief_stale(changed, ["2026-10-01T12:00:40Z"], now),
            None,
            "the turn that wrote the brief ended 40 s later: not work since"
        );
        assert_eq!(
            brief_stale(at("2026-10-05T12:00:00Z"), later, now),
            None,
            "under 7 days"
        );
        assert_eq!(
            brief_stale(changed, ["2026-09-30T09:00:00Z"], now),
            None,
            "nothing happened since"
        );
    }
    #[test]
    fn explicit_next_step_ignores_examples_and_other_sections() {
        assert_eq!(next_step("```md\n## Next step\nWrong\n```\n## Next step\nCheck the brief.\n\nKeep sources.\n## Decisions\nDone"), Some("Check the brief. Keep sources.".into()));
        assert_eq!(next_step("# Brief\nEverything succeeded."), None);
    }
    #[test]
    fn missing_plan_is_unknown_and_output_existence_is_not_success() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces::create(root.path(), "Review", "Make a brief").unwrap();
        std::fs::remove_file(Path::new(&w.cwd).join("PLAN.md")).unwrap();
        std::fs::write(Path::new(&w.cwd).join("outputs/draft.md"), "Done!").unwrap();
        let d = workspaces::read(root.path(), &w.id).unwrap();
        assert!(d.reentry.open_item_count.is_none());
        assert!(d.reentry.last_run.is_none());
        assert!(!d.reentry.gaps.is_empty());
        assert!(d
            .files
            .iter()
            .find(|f| f.kind == "output")
            .unwrap()
            .modified
            .is_some());
    }
    #[test]
    fn interrupted_run_and_open_plan_are_not_completion() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces::create(root.path(), "Review", "Make a brief").unwrap();
        std::fs::write(
            Path::new(&w.cwd).join("PLAN.md"),
            "- [x] Draft\n- [ ] Verify\n```\n- [ ] example\n```\n",
        )
        .unwrap();
        let key = "s-20260927-abcd0001";
        let first =
            session_store::start_turn(root.path(), key, "codex", &w.cwd, "run-1", "First").unwrap();
        session_store::finish_turn(root.path(), key, first, "success").unwrap();
        let last = session_store::start_turn(root.path(), key, "codex", &w.cwd, "run-2", "Second")
            .unwrap();
        session_store::finish_turn(root.path(), key, last, "interrupted").unwrap();
        let d = workspaces::read(root.path(), &w.id).unwrap();
        assert_eq!(d.reentry.open_item_count, Some(1));
        assert_eq!(d.reentry.open_items, vec!["Verify"]);
        let run = d.reentry.last_run.unwrap();
        assert_eq!(run.status.as_deref(), Some("interrupted"));
        assert_eq!(run.failed_checks, None);
        assert!(d
            .reentry
            .gaps
            .iter()
            .any(|s| s.contains("no finished record")));
    }
    #[test]
    fn receipt_must_match_the_finalized_run_and_session() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces::create(root.path(), "Review", "Make a brief").unwrap();
        let key = "s-20260927-abcd0002";
        let ord =
            session_store::start_turn(root.path(), key, "codex", &w.cwd, "run-match", "Check")
                .unwrap();
        session_store::finish_turn(root.path(), key, ord, "success").unwrap();
        let mut receipt = crate::evidence::EvidenceReceipt {
            history_run_id: "run-match".into(),
            run_id: "run-match".into(),
            source: "chat".into(),
            session_key: Some("s-20260927-dead0001".into()),
            status: "success".into(),
            finished_at: Utc::now().to_rfc3339(),
            ..Default::default()
        };
        let write = |r: &crate::evidence::EvidenceReceipt| {
            std::fs::write(
                root.path().join(crate::evidence::FILE_NAME),
                serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"receipts":[r]})).unwrap(),
            )
            .unwrap()
        };
        write(&receipt);
        let unknown = workspaces::read(root.path(), &w.id)
            .unwrap()
            .reentry
            .last_run
            .unwrap();
        assert_eq!(unknown.check_count, None);
        receipt.session_key = Some(key.into());
        write(&receipt);
        let matched = workspaces::read(root.path(), &w.id)
            .unwrap()
            .reentry
            .last_run
            .unwrap();
        assert_eq!(matched.check_count, Some(0));
        assert_eq!(matched.failed_checks, Some(0));
    }
}
