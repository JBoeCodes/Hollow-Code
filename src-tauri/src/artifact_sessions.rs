//! Read-only joins between retained file history and Hollow-owned sessions.
//! A filename in a prompt is not evidence of an edit. Only finalized history
//! changes establish the file/run edge; the turn's run id establishes identity.

use crate::types::{AgentSessionDetail, AgentSessionSummary};
use crate::{automated_history, evidence, session_store};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSessionLink {
    pub session: Option<AgentSessionSummary>,
    pub run: automated_history::HistoryRun,
    /// A successful structured write confirms attribution, not correctness.
    pub attribution: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSessionContext {
    pub vault_path: String,
    pub detail: AgentSessionDetail,
    pub permission_mode: Option<String>,
    pub answers: BTreeMap<u32, String>,
    pub receipts: Vec<evidence::EvidenceReceipt>,
}

pub fn list(vault: &Path, path: &str) -> Result<Vec<ArtifactSessionLink>, String> {
    // Validates a relative vault path, including for deleted files. Retention
    // is bounded by history (200 runs / 90 days), not an arbitrary session page.
    let runs = automated_history::list_note_history(vault, path, 200)?;
    let receipts = evidence::list(vault, Some("chat"), None, 500)?;
    runs.into_iter()
        .filter(|run| run.source == "chat")
        .map(|run| {
            let session = session_store::session_for_run(vault, &run.id)?;
            let confirmed = receipts.iter().any(|r| {
                r.run_id == run.id
                    && r.history_run_id == run.id
                    && r.files
                        .iter()
                        .any(|f| f.path == path && f.attribution == "confirmed")
            });
            // Another chat in the same workspace folder changed it, not
            // this run (chat concurrency F1, 2026-10-20).
            let shared = !confirmed
                && run
                    .changes
                    .iter()
                    .any(|change| change.vault_path == path && change.unattributed);
            let attribution = match (confirmed, shared) {
                (true, _) => "confirmed",
                (false, true) => evidence::ATTRIBUTION_SHARED,
                (false, false) => "observed",
            };
            Ok(ArtifactSessionLink {
                session,
                run,
                attribution: attribution.into(),
            })
        })
        .collect()
}

pub fn context(vault: &Path, key: &str) -> Result<ArtifactSessionContext, String> {
    let detail = session_store::read_session(vault, key)?;
    let answers = session_store::assistant_summaries(vault, key)?;
    let receipts = evidence::list(vault, Some("chat"), None, 500)?
        .into_iter()
        .filter(|r| {
            detail
                .turns
                .iter()
                .any(|t| t.run_id.as_deref() == Some(&r.run_id))
        })
        .collect();
    Ok(ArtifactSessionContext {
        permission_mode: session_store::last_permission(vault, key)?,
        vault_path: vault.to_string_lossy().into_owned(),
        detail,
        answers,
        receipts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn links_only_exact_changed_paths_and_run_identity_with_honest_attribution() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        fs::create_dir(vault.join("notes")).unwrap();
        fs::create_dir(vault.join("other")).unwrap();
        fs::write(vault.join("other/plan.md"), "untouched").unwrap();
        let handle = automated_history::begin_run(vault, "chat", None).unwrap();
        let key = "s-20260923-artifact1";
        let ord = session_store::start_turn(
            vault,
            key,
            "codex",
            vault.to_str().unwrap(),
            &handle.id,
            "Same title",
        )
        .unwrap();
        session_store::record_harness_id(vault, key, "codex-thread").unwrap();
        session_store::start_turn(
            vault,
            "s-20260923-artifact2",
            "claude",
            "/elsewhere",
            "unrelated-run",
            "Same title",
        )
        .unwrap();
        fs::write(vault.join("notes/plan.md"), "changed").unwrap();
        let run = automated_history::finish_run(vault, handle, "success").unwrap();
        session_store::append_event(vault, key, ord, "assistant", "Recorded answer").unwrap();
        session_store::finish_turn(vault, key, ord, "success").unwrap();
        let found = list(vault, "notes/plan.md").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session.as_ref().unwrap().key, key);
        assert_eq!(found[0].attribution, "observed");
        assert!(list(vault, "other/plan.md").unwrap().is_empty());
        assert!(list(vault, "../notes/plan.md").is_err());

        let mut collector = evidence::EvidenceCollector::new("chat", None, &run.id);
        collector.set_session_key(key);
        collector.record_tool_use(
            "write",
            "Write",
            &serde_json::json!({ "file_path": vault.join("notes/plan.md").to_str().unwrap() }),
        );
        collector.record_tool_result("write", "written", false);
        collector
            .finish(
                vault,
                "success",
                vec![evidence::ObservedChange {
                    path: "notes/plan.md".into(),
                    change_type: "create".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        assert_eq!(
            list(vault, "notes/plan.md").unwrap()[0].attribution,
            "confirmed"
        );
        session_store::record_turn_permission(vault, key, ord, "plan").unwrap();
        let loaded = context(vault, key).unwrap();
        assert_eq!(loaded.permission_mode.as_deref(), Some("plan"));
        assert_eq!(loaded.answers.get(&1).unwrap(), "Recorded answer");
        assert_eq!(loaded.receipts.len(), 1);
        assert_eq!(
            loaded.detail.summary.harness_session_id.as_deref(),
            Some("codex-thread")
        );
        fs::remove_file(vault.join("notes/plan.md")).unwrap();
        assert_eq!(list(vault, "notes/plan.md").unwrap().len(), 1);
        automated_history::clear_history(vault).unwrap();
        assert!(list(vault, "notes/plan.md").unwrap().is_empty());
    }

    #[test]
    fn older_unlinked_chat_runs_remain_reviewable_and_workflows_are_not_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        for source in ["chat", "workflow"] {
            let handle = automated_history::begin_run(vault, source, Some("legacy-id")).unwrap();
            fs::write(vault.join("plan.md"), source).unwrap();
            automated_history::finish_run(vault, handle, "success").unwrap();
        }
        let found = list(vault, "plan.md").unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].session.is_none());
        assert_eq!(found[0].run.source, "chat");
    }
}
