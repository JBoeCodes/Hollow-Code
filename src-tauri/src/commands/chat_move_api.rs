//! M5 PR2 (2026-11-15): the person's own chat moves, user-only Tauri
//! commands (never app actions, agent tools, phone or peer routes, or
//! `hollow serve`): Move to runner, Keep it here, Release here, and Forget
//! this Mac, which also releases the chats moved to that Mac. Each answers
//! Hollow's main window only. The controls are the UI PR's (the M5 plan,
//! PR2's UI follow-up); `docs/reference/peers.md` lists the commands.
//!
//! **Releasing** ([`chat_move::force_release`]) is the one step that can
//! leave a chat on two Macs, so it takes [`UserConfirmed`], made here only:
//! in [`chat_move_release_here`] (a chat stuck `leaving` after the other
//! Mac stopped answering) and in [`peer_forget`] (every move to the Mac
//! being forgotten). Both run behind a confirm dialog in the window; a test
//! (`chat_move_trigger`'s) fails if any other module names the token.

use crate::chat_move::{self, UserConfirmed};
use crate::chat_move_send::{Driving, MoveLink};
use crate::chat_move_trigger::{self as trigger, AppTurns, ChatMoveResult, Me, MoveOutcome};
use crate::commands::peer_api::{self, PeerFailure};
use chrono::Utc;
use std::path::{Path, PathBuf};

async fn me() -> Result<Me, String> {
    tokio::task::spawn_blocking(Me::read)
        .await
        .map_err(|e| e.to_string())?
}

/// The chat's open move here: `(move_id, peer)` while it is `leaving`.
fn leaving(vault: &Path, key: &str) -> Result<(String, String), String> {
    match chat_move::home_of(vault, key)? {
        Some(home) if home.state == chat_move::LEAVING => Ok((home.move_id, home.peer)),
        Some(home) if home.state == chat_move::AWAY => {
            Err("This chat moved to another Mac and keeps going there.".into())
        }
        _ => Err("This chat isn't moving; it is this Mac's.".into()),
    }
}

/// The chats moving or moved between this Mac and another (`chat_homes`),
/// for the chat's notice and the lists' moved badge. A local read, window
/// only: never an action.
#[tauri::command]
pub async fn chat_move_homes(webview: tauri::Webview) -> Result<Vec<chat_move::HomeRow>, String> {
    peer_api::main_window(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    tokio::task::spawn_blocking(move || chat_move::home_rows(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// Move to runner: move chat `key` to the paired runner (the Mac the runner
/// alias reaches, else the only paired Mac that takes moves). A running
/// turn is asked to stop at its next safe point first, and stopped outright
/// after [`trigger::MANUAL_SAFE_WAIT`]; it then gets one continuation there.
/// `expect` is the machine id the window's confirm named (`moveTo`): when
/// the pick changed since (a Mac paired, forgotten, or the runner alias
/// edited), nothing moves and the person is asked to try again.
#[tauri::command]
pub async fn chat_move_to_runner(
    webview: tauri::Webview,
    key: String,
    expect: Option<String>,
) -> Result<ChatMoveResult, String> {
    peer_api::hub(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    let (link, to) = trigger::runner_link(trigger::ANSWER_WITHIN, false).await?;
    if let Some(refusal) = changed_target(expect.as_deref(), link.client().machine_id(), &to) {
        return Ok(ChatMoveResult::new(
            &key,
            MoveOutcome::Refused,
            Some(&to),
            refusal,
        ));
    }
    let me = me().await?;
    let now = tokio::time::Instant::now();
    let plan = trigger::StopPlan {
        safe_by: now + trigger::MANUAL_SAFE_WAIT,
        stop_by: now + trigger::MANUAL_SAFE_WAIT + trigger::STOP_SETTLE,
    };
    Ok(trigger::move_chat(&vault, &AppTurns, &link, &me, &to, &key, plan).await)
}

/// The refusal when Move to runner's pick isn't the Mac the person was
/// asked about; `None` when it is, or the window named none.
fn changed_target(expect: Option<&str>, picked: Option<&str>, name: &str) -> Option<String> {
    let expect = expect?;
    (picked != Some(expect)).then(|| {
        format!("The runner changed to {name} since you chose Move to runner. Nothing moved; choose it again.")
    })
}

/// Keep it here: call off chat `key`'s move. With the other Mac answering
/// it is settled at once (the chat runs here again, or, if it already ran
/// there, stays there); without, the chat waits for that Mac's answer.
#[tauri::command]
pub async fn chat_move_keep_here(
    webview: tauri::Webview,
    key: String,
) -> Result<ChatMoveResult, String> {
    let hub = peer_api::hub(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    let (_, peer) = leaving(&vault, &key)?;
    let link = trigger::link_for(&hub, &peer, None).await.ok();
    let me = me().await.ok();
    trigger::keep_chat_here(
        &vault,
        link.as_ref().map(|l| l as &dyn MoveLink),
        me.as_ref(),
        &key,
    )
    .await
}

/// M5 PR3: Continue here offline on chat `key`, which moved to another Mac
/// that can't be reached now: a new chat of this Mac's with the turns this
/// Mac has of it, synced there on the next contact (`chat_offline`). The
/// new chat's key (the same one while it hasn't synced).
#[tauri::command]
pub async fn chat_continue_offline(webview: tauri::Webview, key: String) -> Result<String, String> {
    peer_api::hub(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    tokio::task::spawn_blocking(move || {
        crate::chat_offline::continue_offline(&vault, &key, Utc::now()).map_err(|e| e.message)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// M5 offline UI: this Mac's offline continuations (`chat_offline`), for
/// the notices on a moved chat and on its offline copy (open, synced,
/// forked, or kept here with the reason) and the offline badge. A local
/// read, window only: never an action.
#[tauri::command]
pub async fn chat_offline_rows(
    webview: tauri::Webview,
) -> Result<Vec<crate::chat_offline::OfflineRow>, String> {
    peer_api::main_window(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    tokio::task::spawn_blocking(move || crate::chat_offline::offline_rows(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// Release here (after the window's confirm): give chat `key`, stuck
/// moving to a Mac that no longer answers, back to this Mac. If that Mac
/// took it after all, it may keep a copy there.
#[tauri::command]
pub async fn chat_move_release_here(
    webview: tauri::Webview,
    key: String,
) -> Result<ChatMoveResult, String> {
    peer_api::hub(&webview).map_err(|e| e.message)?;
    let vault = trigger::vault_now().await?;
    let confirmed = UserConfirmed::by_the_person();
    let k = key.clone();
    tokio::task::spawn_blocking(move || release_here(&vault, &k, confirmed))
        .await
        .map_err(|e| e.to_string())?
}

/// [`chat_move_release_here`]'s work: refused while this process drives the
/// move (that call settles it).
fn release_here(
    vault: &Path,
    key: &str,
    confirmed: UserConfirmed,
) -> Result<ChatMoveResult, String> {
    let (move_id, _) = leaving(vault, key)?;
    let Some(_driving) = Driving::claim(&move_id) else {
        return Ok(ChatMoveResult::new(
            key,
            MoveOutcome::Busy,
            None,
            "This chat's move is being sent right now. Try Release here again in a moment.".into(),
        ));
    };
    chat_move::force_release(vault, &move_id, confirmed, Utc::now()).map_err(|e| e.message)?;
    trigger::drop_owed(&move_id);
    Ok(ChatMoveResult::new(
        key,
        MoveOutcome::Kept,
        None,
        "Released: the chat runs on this Mac again.".into(),
    ))
}

/// Forget a paired Mac (Settings' Forget this Mac, after the window's
/// confirm): unpaired there first when it answers, its record, token,
/// streams and tunnel dropped here, then every chat still moving to it
/// (`leaving` or `aborting`) released to this Mac, even when the unpairing
/// failed. A chat that moved there (`away`) stays that Mac's: it runs there,
/// and releasing it would give it two owners ("moved to a Mac you forgot",
/// shown by the UI PR). Was `commands::peer_api`'s (M2).
#[tauri::command]
pub async fn peer_forget(webview: tauri::Webview, machine_id: String) -> Result<bool, PeerFailure> {
    let forgot = peer_api::hub(&webview)?.forget(&machine_id).await;
    let vault = trigger::vault_now().await.ok();
    forget_then_release(forgot, vault, machine_id, UserConfirmed::by_the_person()).await
}

/// After Forget's unpairing, whatever it said (a Mac already gone is
/// exactly the one whose moves never settle): release the moves to it still
/// open, and tell the person about any skipped because this process was
/// driving them. Answers what the unpairing said.
async fn forget_then_release(
    forgot: Result<bool, PeerFailure>,
    vault: Option<PathBuf>,
    machine_id: String,
    confirmed: UserConfirmed,
) -> Result<bool, PeerFailure> {
    let Some(vault) = vault else {
        return forgot;
    };
    let (released, skipped) = tokio::task::spawn_blocking(move || {
        let out = release_moves_to(&vault, &machine_id, confirmed);
        if !out.1.is_empty() {
            trigger::notify(
                &vault,
                "Some chats weren't released",
                format!(
                    "{} chat(s) moving to the Mac you forgot were being sent at that moment. Choose Release here on each.",
                    out.1.len()
                ),
            );
        }
        out
    })
    .await
    .unwrap_or_default();
    if released > 0 || !skipped.is_empty() {
        crate::launch::elog(&format!(
            "[chat move] forget: {released} chat(s) released, skipped (being sent): {skipped:?}"
        ));
    }
    forgot
}

/// Release every chat still moving to `machine_id`
/// ([`chat_move::open_moves_to`]): how many were released, and the keys
/// skipped because this process was driving their move.
fn release_moves_to(
    vault: &Path,
    machine_id: &str,
    confirmed: UserConfirmed,
) -> (usize, Vec<String>) {
    let moves = match chat_move::open_moves_to(vault, machine_id) {
        Ok(moves) => moves,
        Err(error) => {
            crate::launch::elog(&format!("[chat move] forget: {error}"));
            return (0, Vec::new());
        }
    };
    let mut released = 0;
    let mut skipped = Vec::new();
    for (move_id, key) in moves {
        let Some(_driving) = Driving::claim(&move_id) else {
            skipped.push(key);
            continue;
        };
        match chat_move::force_release(vault, &move_id, confirmed, Utc::now()) {
            Ok(()) => {
                released += 1;
                trigger::drop_owed(&move_id);
            }
            Err(error) => crate::launch::elog(&format!("[chat move] {key}: {}", error.message)),
        }
    }
    (released, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forget this Mac releases every chat still moving to that Mac and no
    /// other's: a `leaving` and an `aborting` move come home; a `done` one
    /// stays `away` (it runs there: never two owners).
    #[tokio::test]
    async fn forgetting_a_mac_releases_its_moves_only() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().canonicalize().unwrap();
        let quick = crate::workspaces::ensure_quick_chats(&vault).unwrap();
        let cwd = quick.to_string_lossy().into_owned();
        let gone = "m-8d21c4be90f17a3e";
        let other = "m-1111222233334444";
        let me = "m-3f9c1a07e2b45d18";
        let keys: Vec<String> = (1..=4).map(|n| format!("s-20261115-fg{n:06}")).collect();
        for key in &keys {
            let ord =
                crate::session_store::start_turn(&vault, key, "claude", &cwd, "run", "Hi").unwrap();
            crate::session_store::finish_turn(&vault, key, ord, "success").unwrap();
        }
        let ids: Vec<String> = (0..4).map(|_| chat_move::new_move_id()).collect();
        for (i, key) in keys.iter().enumerate() {
            let to = if i == 3 { other } else { gone };
            chat_move::begin_move_out(&vault, key, me, to, &ids[i], Utc::now()).unwrap();
        }
        chat_move::begin_abort(&vault, &ids[1], Utc::now()).unwrap();
        chat_move::finish_move_out(&vault, &ids[2], Utc::now()).unwrap();
        // Forget's unpairing failed (the Mac is gone): released anyway.
        let failed: Result<bool, PeerFailure> =
            Err(crate::peer_client::PeerError::Offline("Can't reach it.".into()).into());
        let answered = forget_then_release(
            failed,
            Some(vault.clone()),
            gone.to_string(),
            UserConfirmed::by_the_person(),
        )
        .await;
        assert!(answered.is_err(), "the unpairing's answer is passed on");
        let away = chat_move::home_of(&vault, &keys[2]).unwrap().unwrap();
        assert_eq!(away.state, chat_move::AWAY, "a done move stays there");
        assert!(chat_move::admission_refusal(&vault, &keys[2]).is_some());
        for key in &keys[..2] {
            assert!(chat_move::home_of(&vault, key).unwrap().is_none(), "{key}");
            assert!(chat_move::admission_refusal(&vault, key).is_none(), "{key}");
        }
        let still = chat_move::home_of(&vault, &keys[3]).unwrap().unwrap();
        assert_eq!(still.state, chat_move::LEAVING, "another Mac's move stays");
        assert_eq!(
            release_moves_to(&vault, gone, UserConfirmed::by_the_person()),
            (0, Vec::new())
        );
    }

    /// M5 UI: the window's rows say which chats are moving, waiting after
    /// Keep it here, or moved, and to which Mac.
    #[test]
    fn the_window_reads_each_chats_home_and_whether_it_waits() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().canonicalize().unwrap();
        let quick = crate::workspaces::ensure_quick_chats(&vault).unwrap();
        let cwd = quick.to_string_lossy().into_owned();
        let mini = "m-8d21c4be90f17a3e";
        let keys: Vec<String> = (1..=3).map(|n| format!("s-20261115-hr{n:06}")).collect();
        let mut ids = Vec::new();
        for key in &keys {
            let ord =
                crate::session_store::start_turn(&vault, key, "claude", &cwd, "run", "Hi").unwrap();
            crate::session_store::finish_turn(&vault, key, ord, "success").unwrap();
            let id = chat_move::new_move_id();
            chat_move::begin_move_out(&vault, key, "m-3f9c1a07e2b45d18", mini, &id, Utc::now())
                .unwrap();
            ids.push(id);
        }
        chat_move::begin_abort(&vault, &ids[1], Utc::now()).unwrap();
        chat_move::finish_move_out(&vault, &ids[2], Utc::now()).unwrap();
        let rows = chat_move::home_rows(&vault).unwrap();
        let of = |key: &str| rows.iter().find(|r| r.key == key).unwrap().clone();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.peer == mini));
        assert_eq!(
            (of(&keys[0]).state.as_str(), of(&keys[0]).waiting),
            ("leaving", false)
        );
        assert_eq!(
            (of(&keys[1]).state.as_str(), of(&keys[1]).waiting),
            ("leaving", true)
        );
        assert_eq!(
            (of(&keys[2]).state.as_str(), of(&keys[2]).waiting),
            ("away", false)
        );
    }

    /// Move to runner never moves a chat to a Mac the person wasn't asked
    /// about.
    #[test]
    fn move_to_runner_refuses_a_changed_target() {
        let mini = "m-8d21c4be90f17a3e";
        assert_eq!(changed_target(None, Some(mini), "Mac mini"), None);
        assert_eq!(changed_target(Some(mini), Some(mini), "Mac mini"), None);
        let changed = changed_target(Some(mini), Some("m-1111222233334444"), "Studio").unwrap();
        assert!(changed.contains("Studio") && changed.contains("Nothing moved"));
        assert!(changed_target(Some(mini), None, "Mac mini").is_some());
    }

    /// A move this process is driving is never released under it: Release
    /// here says busy, Forget skips it and says which.
    #[test]
    fn a_move_being_driven_is_not_released() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().canonicalize().unwrap();
        let quick = crate::workspaces::ensure_quick_chats(&vault).unwrap();
        let cwd = quick.to_string_lossy().into_owned();
        let key = "s-20261115-fg000009";
        let ord =
            crate::session_store::start_turn(&vault, key, "claude", &cwd, "run", "Hi").unwrap();
        crate::session_store::finish_turn(&vault, key, ord, "success").unwrap();
        let gone = "m-8d21c4be90f17a3e";
        let id = chat_move::new_move_id();
        chat_move::begin_move_out(&vault, key, "m-3f9c1a07e2b45d18", gone, &id, Utc::now())
            .unwrap();
        let held = Driving::claim(&id).unwrap();
        let busy = release_here(&vault, key, UserConfirmed::by_the_person()).unwrap();
        assert_eq!(busy.outcome, MoveOutcome::Busy);
        assert_eq!(
            release_moves_to(&vault, gone, UserConfirmed::by_the_person()),
            (0, vec![key.to_string()])
        );
        assert_eq!(
            chat_move::home_of(&vault, key).unwrap().unwrap().state,
            chat_move::LEAVING
        );
        drop(held);
        let done = release_here(&vault, key, UserConfirmed::by_the_person()).unwrap();
        assert_eq!(done.outcome, MoveOutcome::Kept);
        assert!(chat_move::home_of(&vault, key).unwrap().is_none());
    }
}
