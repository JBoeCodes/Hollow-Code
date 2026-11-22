//! Run-queue command adapters (Paperclip PR 7).
//!
//! Thin, like the other `commands/` adapters: rows and their rules live in
//! `run_queue`. Queue reads and writes touch SQLite (with a busy timeout), so
//! both commands run on a blocking worker, never on the main thread.

use super::AppState;
use crate::run_queue::{self, CancelOutcome, RunSource};
use crate::types::RunQueueItem;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;
use tauri::{AppHandle, State};

/// Terminal rows returned by default after the live ones.
const DEFAULT_RECENT: u32 = 20;
const MAX_RECENT: u32 = 100;

/// Emit one `run-queue-event` (see `run_pump::emit`).
pub(crate) fn emit_queue_event(
    app: &impl crate::core_host::CoreHost,
    kind: &str,
    item: RunQueueItem,
) {
    crate::run_pump::emit(app, kind, item);
}

/// Live rows (running, claimed, then queued in claim order, each queued row
/// with its position), then the newest `recent` finished rows (default 20,
/// at most 100).
#[tauri::command]
pub async fn list_run_queue(
    recent: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<RunQueueItem>, String> {
    let vault = state.config.vault_path.clone();
    let recent = recent.unwrap_or(DEFAULT_RECENT).min(MAX_RECENT) as usize;
    crate::vault::spawn_blocking_logged(move || {
        let live = run_queue::live_rows(&vault).unwrap_or_default();
        let facts = crate::run_pump::QueueFacts::read(&vault, &live);
        let now = Utc::now();
        run_queue::list(&vault, recent).map(|rows| {
            rows.iter()
                .map(|(row, position)| {
                    crate::run_pump::item_with_reason(&vault, row, *position, &facts, now)
                })
                .collect()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cancel one row, by status: a queued cron row is marked to be skipped (its
/// occurrence is still bookkept at claim), any other queued row ends
/// `cancelled` (a waiting Run now resolves as cancelled), a claimed row is
/// marked so the pump stops it, and a running workflow or chat turn is
/// stopped through its normal stop path (a chat turn's keyed stop slot,
/// Paperclip PR 7b). A queued chat turn also gets its `cancelled{queued}` and
/// `done` chat events. The heartbeat cannot be stopped once it starts.
#[tauri::command]
pub async fn cancel_queued_run(
    app: AppHandle,
    id: i64,
    state: State<'_, AppState>,
) -> Result<RunQueueItem, String> {
    // Only the queue owner may change rows: Run now's waiter lives in the
    // owner's memory, so a row ended from a second process (a dev build
    // beside the installed app) would leave that `run_workflow` hanging.
    crate::run_pump::accepting()?;
    let vault = state.config.vault_path.clone();
    let (outcome, item, chat_events) = crate::vault::spawn_blocking_logged(move || {
        cancel_row(
            &vault,
            id,
            Utc::now(),
            &|slug: &str| crate::workflows::cancel(slug),
            &stop_chat_row(id),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    for event in chat_events {
        crate::chat::emit_event(&app, event);
    }
    let kind = match outcome {
        CancelOutcome::AlreadyFinished => None,
        _ => Some("cancelled"),
    };
    if let Some(kind) = kind {
        emit_queue_event(&app, kind, item.clone());
    }
    run_queue::wake();
    Ok(item)
}

/// Stop a running chat turn only while it is still row `id`'s turn, checked
/// under the stop slot's lock (`chat::cancel_running_row`). A Stop clicked in
/// Mission Control or the Subagents tab lands after a confirm; by then the
/// row's turn may have ended and the same key's next turn taken the slot,
/// and a stop by key alone would end that one (overnight 11-13).
pub(crate) fn stop_chat_row(id: i64) -> impl Fn(&str) -> bool {
    move |key: &str| crate::chat::cancel_running_row(key, id)
}

/// The sync, AppHandle-free body of [`cancel_queued_run`] (tests drive it).
/// `stop_workflow` and `stop_chat` are `workflows::cancel` and
/// [`stop_chat_row`] in production. Returns the outcome, the row as it
/// reads after the cancel, and the chat events to emit: a queued chat turn
/// taken out of the queue (the composer's Edit and Discard) ends in the
/// webview like any turn, `cancelled{queued}` then `done`, built from the
/// row read BEFORE the cancel, since ending it clears its payload (and with
/// it the `clientTurnId`) (Paperclip PR 7b).
pub(crate) fn cancel_row(
    vault: &Path,
    id: i64,
    now: DateTime<Utc>,
    stop_workflow: &dyn Fn(&str),
    stop_chat: &dyn Fn(&str) -> bool,
) -> Result<(CancelOutcome, RunQueueItem, Vec<Value>), String> {
    let before = run_queue::get(vault, id)?;
    let outcome = run_queue::cancel(vault, id, now)?;
    if let CancelOutcome::Running { source, source_id } = &outcome {
        match source {
            source if source.is_workflow() => {
                if let Some(slug) = source_id {
                    stop_workflow(slug);
                }
            }
            RunSource::Heartbeat => {
                return Err("The heartbeat cannot be stopped once it starts.".to_string())
            }
            // A running chat turn stops through its keyed stop slot; the turn
            // then ends itself (`cancelled`, then the pump's `done`).
            RunSource::Chat => {
                if !source_id.as_deref().is_some_and(stop_chat) {
                    return Err(
                        "That chat turn is finishing; it cannot be stopped now.".to_string()
                    );
                }
            }
            _ => return Err("This run cannot be stopped from the queue.".to_string()),
        }
    }
    // Paperclip PR 9b: a cancelled retry ends the failure it was deferring,
    // which counts toward the streak now (no banner: the user's own doing).
    if let (CancelOutcome::Cancelled, Some(row)) = (&outcome, &before) {
        if row.source == RunSource::Retry {
            if let Some(slug) = row.source_id.as_deref() {
                crate::workflow_retry::resolve_skipped_retry(
                    vault,
                    slug,
                    &row.payload,
                    crate::workflow_retry::RetrySkipReason::UserCancel,
                );
            }
        }
    }
    let chat_events = match (&outcome, &before) {
        (CancelOutcome::Cancelled, Some(row)) if row.source == RunSource::Chat => {
            // P1a: a cancelled resume gives its claim back.
            crate::chat_pause::release_for_row(vault, row, false);
            crate::chat::queued_end_events(row, &crate::chat::QueuedEnd::Cancelled)
        }
        _ => Vec::new(),
    };
    let row = run_queue::get(vault, id)?.ok_or_else(|| format!("There is no queued run {id}."))?;
    Ok((outcome, run_queue::to_item(&row, None), chat_events))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::RowStatus;
    use std::cell::RefCell;

    fn chat_vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn queue_chat(vault: &Path) -> crate::chat::AcceptedTurn {
        let request = crate::chat::ChatSendRequest {
            prompt: "Hello queue".into(),
            session_id: None,
            session_key: None,
            cwd: vault.to_path_buf(),
            model: None,
            permission_mode: None,
            agent: crate::agent::AgentKind::Claude,
            client_turn_id: Some("ct-queue-api".into()),
            app_depth: 0,
            ultracode: false,
            canvases: None,
            continued_from: Vec::new(),
            unattended: false,
            typed: false,
            host_asker: None,
            canned: None,
            worker: false,
        };
        crate::chat::accept_turn(vault, request, Utc::now()).unwrap()
    }

    fn no_workflow(_: &str) {
        panic!("no workflow is stopped here");
    }

    /// Edit and Discard on a queued chat turn: the row ends `cancelled` and
    /// the turn gets `cancelled{queued}` + `done`, carrying its nonce.
    #[test]
    fn a_queued_chat_row_is_cancelled_with_its_end_events() {
        let dir = chat_vault();
        let v = dir.path();
        let accepted = queue_chat(v);
        let never = |_: &str| -> bool { panic!("a queued turn has no stop slot to signal") };
        let (outcome, item, events) =
            cancel_row(v, accepted.queue_id, Utc::now(), &no_workflow, &never).unwrap();
        assert_eq!(outcome, CancelOutcome::Cancelled);
        assert_eq!(item.id, accepted.queue_id);
        assert_eq!(
            run_queue::get(v, accepted.queue_id)
                .unwrap()
                .unwrap()
                .status,
            RowStatus::Cancelled
        );
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0]["type"], "cancelled");
        assert_eq!(events[0]["queued"], true);
        assert_eq!(events[0]["clientTurnId"], "ct-queue-api");
        assert_eq!(events[1]["type"], "done");
        assert_eq!(events[1]["clientTurnId"], "ct-queue-api");
    }

    /// Stop on a running chat row from the Settings queue signals that
    /// session's keyed stop slot, and says so when the turn is finishing.
    #[test]
    fn a_running_chat_row_stops_through_its_keyed_slot() {
        let dir = chat_vault();
        let v = dir.path();
        let accepted = queue_chat(v);
        run_queue::claim(v, accepted.queue_id, Utc::now())
            .unwrap()
            .unwrap();
        run_queue::mark_running(v, accepted.queue_id, "run-queue-api-chat", Utc::now()).unwrap();
        let asked = RefCell::new(Vec::new());
        let stops = |key: &str| {
            asked.borrow_mut().push(key.to_string());
            true
        };
        let (outcome, _item, events) =
            cancel_row(v, accepted.queue_id, Utc::now(), &no_workflow, &stops).unwrap();
        assert!(
            matches!(
                outcome,
                CancelOutcome::Running {
                    source: RunSource::Chat,
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(*asked.borrow(), vec![accepted.session_key.clone()]);
        assert!(events.is_empty(), "the running turn ends itself");

        let finishing = |_: &str| false;
        let refused =
            cancel_row(v, accepted.queue_id, Utc::now(), &no_workflow, &finishing).unwrap_err();
        assert!(refused.contains("finishing"), "{refused}");
    }

    /// Overnight 11-13: Stop is confirmed after the row's turn ended and the
    /// same conversation's next turn took its stop slot. The production stop
    /// is by row, so it refuses and leaves the next turn running; a stop by
    /// key would have ended it.
    #[test]
    fn a_stale_running_row_never_stops_the_next_turn_stop_by_row() {
        let _turns = crate::chat::TURN_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = chat_vault();
        let v = dir.path();
        let accepted = queue_chat(v);
        run_queue::claim(v, accepted.queue_id, Utc::now())
            .unwrap()
            .unwrap();
        run_queue::mark_running(v, accepted.queue_id, "run-queue-api-stale", Utc::now()).unwrap();
        // The row still reads running, but another turn holds the slot.
        let next = crate::chat::register_stop(&accepted.session_key);
        let refused = cancel_row(
            v,
            accepted.queue_id,
            Utc::now(),
            &no_workflow,
            &stop_chat_row(accepted.queue_id),
        )
        .unwrap_err();
        assert!(refused.contains("finishing"), "{refused}");
        assert!(
            !crate::chat::stop_requested_for(&accepted.session_key),
            "the next turn keeps running"
        );
        drop(next);
    }
}
