//! Saved turn-feedback command adapters (Paperclip PR 17).
//!
//! Thin, like the other `commands/` adapters: the rules live in
//! `turn_feedback`. Every command touches SQLite (export also writes two
//! files), so each runs on a blocking worker. Refusals are plain strings with
//! the `FEEDBACK_*` prefixes the composer keys on (Tauri rejects with the
//! string itself).

use super::AppState;
use crate::turn_feedback;
use crate::types::{FeedbackExport, TurnFeedback, TurnFeedbackInput};
use std::path::PathBuf;
use tauri::State;

/// Save (or replace) the vote on the turn whose exact run id is given.
#[tauri::command]
pub async fn record_turn_feedback(
    input: TurnFeedbackInput,
    state: State<'_, AppState>,
) -> Result<TurnFeedback, String> {
    let vault = state.config.vault_path.clone();
    let now = chrono::Utc::now();
    crate::vault::spawn_blocking_logged(move || turn_feedback::record(&vault, &input, now))
        .await
        .map_err(|e| e.to_string())?
}

/// Saved votes, newest first: all, one conversation's (`sessionKey`), or the
/// ones on these runs (`runIds`, at most 200). `limit` defaults to 200 and is
/// clamped to 1..=1000.
#[tauri::command]
pub async fn list_feedback(
    session_key: Option<String>,
    run_ids: Option<Vec<String>>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<TurnFeedback>, String> {
    let vault = state.config.vault_path.clone();
    let limit = limit.unwrap_or(turn_feedback::LIST_DEFAULT);
    crate::vault::spawn_blocking_logged(move || {
        turn_feedback::list(
            &vault,
            session_key.as_deref(),
            run_ids.as_deref().unwrap_or(&[]),
            limit,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Write every saved vote, redacted, as JSON and Markdown into `dir` (an
/// absolute folder outside the vault). Never overwrites a file.
#[tauri::command]
pub async fn export_feedback(
    dir: String,
    state: State<'_, AppState>,
) -> Result<FeedbackExport, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        turn_feedback::export(&vault, &PathBuf::from(dir), chrono::Local::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete saved votes (one conversation's with `sessionKey`, else all).
/// Conversations, transcripts and receipts are untouched.
#[tauri::command]
pub async fn clear_feedback(
    session_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        turn_feedback::clear(&vault, session_key.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// File the finished turn `runId` as a pending eval case (NV7): the skill it
/// should have used (`missedSkill`; none: the run record's miss), redacted
/// first, de-duplicated by prompt and workspace. The person's own filing;
/// an agent's goes through `feedback.file_eval_candidate`, held to its own
/// conversation. Accepting is the person's.
#[tauri::command]
pub async fn file_eval_candidate(
    run_id: String,
    missed_skill: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::review_queue::ReviewItem, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::eval_candidates::file(&vault, &run_id, missed_skill.as_deref(), None)
    })
    .await
    .map_err(|e| e.to_string())?
}
