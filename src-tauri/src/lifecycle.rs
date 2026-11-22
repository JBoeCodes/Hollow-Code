//! App-lifecycle toggles that the window-event / run-event handlers in
//! `lib.rs` read, but that the frontend owns. `AppState.config` is an
//! immutable `Arc<Config>`, so a *runtime* preference can't live there —
//! it's a process-wide `AtomicBool` synced from the Settings UI (the same
//! shape `nudges`/`team_mcp`/`computer_mcp` use for their toggles).

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

static EXPECTED_EXIT: AtomicBool = AtomicBool::new(false);
static CHECKING_QUIT: AtomicBool = AtomicBool::new(false);
static QUIT_EPOCH: AtomicU64 = AtomicU64::new(0);
static PENDING_QUIT: Mutex<Option<QuitRequest>> = Mutex::new(None);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuitRequest {
    pub id: String,
    pub names: Vec<String>,
    pub uncertain: bool,
    #[serde(skip)]
    keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunningWork {
    key: String,
    name: String,
}

pub(crate) fn verify_restart_keys(
    confirmed: &[String],
    current: &[(String, String)],
) -> Result<(), String> {
    if current
        .iter()
        .any(|(key, _)| key.starts_with("unverified:"))
    {
        return Err("Running work has no stable identity. Restart later after it finishes.".into());
    }
    if current.iter().any(|(key, _)| !confirmed.contains(key)) {
        return Err("RESTART_WORK_CHANGED: Running work changed; confirm again".into());
    }
    Ok(())
}

pub(crate) fn check_restart_work(
    vault: &std::path::Path,
    confirmed: &[String],
) -> Result<(), String> {
    verify_restart_keys(confirmed, &work_snapshot(vault)?)
}

/// What a quit would end, now, as `(key, name)` pairs: the list the restart
/// checks compare with (the updater's "Restart to update", `update_client`).
pub(crate) fn running_work_now(vault: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    work_snapshot(vault)
}

fn record_request(slot: &mut Option<QuitRequest>, work: Vec<(String, String)>) -> QuitRequest {
    let (keys, names): (Vec<_>, Vec<_>) = work.into_iter().unzip();
    if let Some(previous) = slot.as_ref().filter(|p| p.keys == keys && p.names == names) {
        return previous.clone();
    }
    let request = QuitRequest {
        id: format!("quit-{:016x}", rand::random::<u64>()),
        uncertain: keys.iter().any(|key| key == "unknown"),
        keys,
        names,
    };
    *slot = Some(request.clone());
    request
}

fn resolve_request(
    slot: &mut Option<QuitRequest>,
    id: &str,
    work: Option<Vec<(String, String)>>,
) -> Result<Option<QuitRequest>, String> {
    let previous = slot
        .as_ref()
        .filter(|p| p.id == id)
        .ok_or("That quit request is no longer waiting")?;
    if let Some(work) = work {
        if work.iter().any(|(key, _)| !previous.keys.contains(key)) {
            return Ok(Some(record_request(slot, work)));
        }
    }
    *slot = None;
    Ok(None)
}

/// Only an explicitly approved quit (the person's dialog, or a COO's
/// `system.quit_hollow` card) or the app's install/setup path bypasses the
/// ordinary exit guard.
pub(crate) fn expect_exit() {
    EXPECTED_EXIT.store(true, Ordering::SeqCst);
}

/// Include queued work (it also stops at quit), background lanes and any
/// live session not yet attached to a queue row. Read errors fail closed.
fn work_snapshot(vault: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    let rows = crate::run_queue::live_rows(vault)?;
    let mut covered: std::collections::HashSet<String> = rows
        .iter()
        .filter_map(|row| row.history_run_id.clone())
        .collect();
    let mut names: Vec<(String, String)> = rows
        .iter()
        .map(|r| (format!("queue:{}", r.id), r.label.clone()))
        .collect();
    for live in crate::agent_sessions::live_all() {
        covered.insert(live.run_id.clone());
        if !rows.iter().any(|r| {
            r.history_run_id.as_deref() == Some(&live.run_id)
                || r.source_id.as_deref() == Some(&live.key)
        }) {
            names.push((live.run_id, live.key));
        }
    }
    for id in crate::automated_history::scoped_run_ids(vault) {
        if covered.insert(id.clone()) {
            names.push((id, "A background run".into()));
        }
    }
    if let Some(holder) = crate::automated_history::lease_holder(vault) {
        let name = crate::agent_sessions::describe_holder(&holder);
        match holder.run_id() {
            Some(id) if covered.insert(id.to_owned()) => names.push((id.to_owned(), name)),
            None => names.push((format!("unverified:{name}"), name)),
            _ => {}
        }
    }
    Ok(names)
}

fn present(app: &tauri::AppHandle, request: QuitRequest) {
    bring_main_forward(app);
    if let Some(window) = app.get_window("main") {
        let _ = window.emit("quit-requested", request);
    }
}

fn unknown_work(_: impl std::fmt::Display) -> Vec<(String, String)> {
    vec![("unknown".into(), "Running work could not be checked".into())]
}

pub(crate) fn guard_exit(app: &tauri::AppHandle, api: &tauri::ExitRequestApi) {
    if EXPECTED_EXIT.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some(state) = app.try_state::<crate::commands::AppState>() else {
        return;
    };
    if !state.configured {
        return;
    }
    api.prevent_exit();
    if CHECKING_QUIT.swap(true, Ordering::SeqCst) {
        return;
    }
    let epoch = QUIT_EPOCH.load(Ordering::SeqCst);
    let vault = state.config.vault_path.clone();
    let app = app.clone();
    // No SQLite/file read or run-registry lock on the event-loop thread.
    tauri::async_runtime::spawn_blocking(move || {
        struct Checking;
        impl Drop for Checking {
            fn drop(&mut self) {
                CHECKING_QUIT.store(false, Ordering::SeqCst);
            }
        }
        let _checking = Checking;
        let work = work_snapshot(&vault).unwrap_or_else(unknown_work);
        let request = {
            let mut slot = PENDING_QUIT.lock().unwrap_or_else(|e| e.into_inner());
            // Check and record under the same lock used by answers: Cancel
            // cannot be followed by a stale read repopulating the request.
            if QUIT_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            if work.is_empty() {
                *slot = None;
                None
            } else {
                Some(record_request(&mut slot, work))
            }
        };
        if let Some(request) = request {
            // Re-emit on every request, even if the previous webview lost it.
            present(&app, request);
        } else {
            expect_exit();
            app.exit(0);
        }
    });
}

/// What a quit would end, for the quit dialog and for `system.exit_running_work`.
pub(crate) async fn running_work(vault: std::path::PathBuf) -> Result<Vec<RunningWork>, String> {
    crate::vault::spawn_blocking_logged(move || {
        work_snapshot(&vault).map(|work| {
            work.into_iter()
                .map(|(key, name)| RunningWork { key, name })
                .collect()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn exit_running_work(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Vec<RunningWork>, String> {
    running_work(state.config.vault_path.clone()).await
}

/// How long a COO-approved quit waits before it exits, so the action's own
/// result (and the card's approval) reach the chat first.
const QUIT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// `system.quit_hollow` (after its card was allowed): quit Hollow, or send it
/// to the menu bar. A quit ends every running turn, the calling one
/// included, so it happens a moment after this returns. It skips the quit
/// dialog (the card was the confirmation) and drops any request waiting on it.
pub(crate) async fn agent_resolve_quit(
    app: &tauri::AppHandle,
    choice: &str,
) -> Result<serde_json::Value, String> {
    match choice {
        "quit" => {
            {
                let mut slot = PENDING_QUIT.lock().unwrap_or_else(|e| e.into_inner());
                *slot = None;
                QUIT_EPOCH.fetch_add(1, Ordering::SeqCst);
            }
            let app = app.clone();
            drop(tauri::async_runtime::spawn(async move {
                tokio::time::sleep(QUIT_DELAY).await;
                expect_exit();
                app.exit(0);
            }));
            Ok(serde_json::json!({
                "quitting": true,
                "note": "Hollow quits in a moment. That ends every running turn, including this one.",
            }))
        }
        "tray" => {
            let window = app.get_window("main").ok_or(
                "Hollow's window isn't open, so there is nothing to send to the menu bar.",
            )?;
            window.hide().map_err(|e| e.to_string())?;
            set_in_tray(true);
            Ok(serde_json::json!({
                "quitting": false,
                "note": "Hollow's window is hidden in the menu bar. Running turns keep going.",
            }))
        }
        _ => Err("Choose quit or tray. Nothing changed.".into()),
    }
}

/// The names a quit card lists: what is running now, in words.
pub(crate) fn quit_card_lines(work: &[RunningWork]) -> Vec<String> {
    if work.is_empty() {
        return vec!["Nothing else is running.".into()];
    }
    let mut lines = vec![format!("Running now ({}), all of it ends:", work.len())];
    for item in work.iter().take(10) {
        lines.push(format!("- {}", item.name));
    }
    if work.len() > 10 {
        lines.push(format!("- and {} more", work.len() - 10));
    }
    lines
}

/// `system.quit_hollow`'s card: what a quit ends, calling chat included.
pub(crate) fn quit_card(
    cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    use crate::app_actions::Card;
    let tray = a.0.get("choice").and_then(|v| v.as_str()) == Some("tray");
    if tray {
        return Ok(Some(
            Card::new("Hollow app tools: send Hollow to the menu bar")
                .line("Hides Hollow's window in the menu bar. Running turns keep going."),
        ));
    }
    let work = tauri::async_runtime::block_on(running_work(cx.vault())).unwrap_or_else(|_| {
        vec![RunningWork {
            key: "unknown".into(),
            name: "Running work could not be checked".into(),
        }]
    });
    let mut card = Card::new("Hollow app tools: quit Hollow")
        .line("Quits Hollow in a moment. That ends every running turn, including this chat's.");
    for line in quit_card_lines(&work) {
        card = card.line(line);
    }
    Ok(Some(card))
}

#[tauri::command]
pub fn pending_quit_request() -> Option<QuitRequest> {
    PENDING_QUIT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[tauri::command]
pub async fn resolve_quit(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::commands::AppState>,
    id: String,
    choice: String,
) -> Result<Option<QuitRequest>, String> {
    if !matches!(choice.as_str(), "quit" | "tray" | "cancel") {
        return Err("Unknown quit choice".into());
    }
    let work = if choice == "quit" {
        let vault = state.config.vault_path.clone();
        Some(
            crate::vault::spawn_blocking_logged(move || work_snapshot(&vault))
                .await
                .map_err(|e| e.to_string())
                .and_then(|result| result)
                .unwrap_or_else(unknown_work),
        )
    } else {
        None
    };
    let changed = {
        let mut slot = PENDING_QUIT.lock().unwrap_or_else(|e| e.into_inner());
        let changed = resolve_request(&mut slot, &id, work)?;
        QUIT_EPOCH.fetch_add(1, Ordering::SeqCst);
        changed
    };
    if let Some(request) = changed {
        present(&app, request.clone());
        return Ok(Some(request));
    }
    match choice.as_str() {
        "quit" => {
            expect_exit();
            app.exit(0);
        }
        "tray" => {
            if let Some(window) = app.get_window("main") {
                window.hide().map_err(|e| e.to_string())?;
                set_in_tray(true);
            }
        }
        _ => {}
    }
    Ok(None)
}

/// Whether closing the `main` window minimizes Hollow to the menu-bar tray
/// (true) or quits the app (false). Defaults **true** — it preserves
/// today's close-to-tray behavior during the brief window between process
/// start and the frontend's boot-time sync, and a close won't realistically
/// fire that fast. Only `main` honors this.
static MINIMIZE_TO_TRAY: AtomicBool = AtomicBool::new(true);

pub fn set_minimize_to_tray(v: bool) {
    MINIMIZE_TO_TRAY.store(v, Ordering::Relaxed);
}

pub fn minimize_to_tray() -> bool {
    MINIMIZE_TO_TRAY.load(Ordering::Relaxed)
}

/// Whether Hollow itself put the `main` window away into the tray (closed
/// with minimize-to-tray on, toggled off from the tray or its shortcut, or
/// launched `--hidden` at login) and it hasn't been focused since (S15). A
/// window hidden with ⌘H, minimized or behind other apps never sets it, so
/// the app broker can tell "in the tray" from "one ⌘Tab away".
// Low bit is in-tray; upper bits version every hide/show/focus and repair.
// A background visibility read may repair only the version it observed.
static IN_TRAY: AtomicU64 = AtomicU64::new(0);

fn write_tray_state(state: &AtomicU64, value: bool) {
    let _ = state.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some((current.wrapping_add(2) & !1) | u64::from(value))
    });
}

fn repair_tray_state(state: &AtomicU64, observed: u64) {
    if observed & 1 != 0 {
        let _ = state.compare_exchange(
            observed,
            observed.wrapping_add(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

pub fn set_in_tray(v: bool) {
    write_tray_state(&IN_TRAY, v);
}

/// Native observations repair a missed focus/show event. Never infer a tray
/// close merely from losing focus or minimizing: only explicit hides set it.
pub(crate) fn main_window_state(
    app: &tauri::AppHandle,
) -> Option<crate::window_state::WindowState> {
    observe_main_window(app, &IN_TRAY)
}

fn observe_main_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    tray: &AtomicU64,
) -> Option<crate::window_state::WindowState> {
    let tray_version = tray.load(Ordering::Acquire);
    // Browser tabs and canvas export add child webviews. Tauri's
    // get_webview_window then returns None even while native main is open.
    let window = app.get_window("main")?;
    let mut state = crate::window_state::WindowState {
        visible: window.is_visible().ok(),
        minimized: window.is_minimized().ok(),
        focused: window.is_focused().ok(),
        in_tray: tray_version & 1 != 0,
    };
    if !state.reconciled_in_tray() {
        repair_tray_state(tray, tray_version);
    }
    state.in_tray = tray.load(Ordering::Acquire) & 1 != 0;
    Some(state)
}

/// Where ⌘⌥⇧T, ⌘⌥⇧I or their tray items brought the `main` window forward
/// from (S5 review): the quick windows they used to open hid themselves after
/// a save and macOS went back to the app in front, so the Add sheet that
/// replaced them steps back the same way (`after_summoned_capture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameFrom {
    /// Put away in the tray: hide the window again.
    Tray,
    /// Minimized: minimize it again.
    Minimized,
    /// Another app was in front (or Hollow was hidden with ⌘H): hide Hollow.
    OtherApp,
}

static CAME_FROM: Mutex<Option<CameFrom>> = Mutex::new(None);

/// What to step back to, from the window's state before it was brought
/// forward. Nothing when Hollow was already the window in front.
pub(crate) fn came_from(state: &crate::window_state::WindowState) -> Option<CameFrom> {
    if state.in_tray {
        Some(CameFrom::Tray)
    } else if state.minimized == Some(true) {
        Some(CameFrom::Minimized)
    } else if state.visible != Some(true) || state.focused != Some(true) {
        Some(CameFrom::OtherApp)
    } else {
        None
    }
}

/// Record where the next quick capture or import came from (`None`: from
/// Hollow itself). Each summon replaces the last.
pub(crate) fn note_came_from(from: Option<CameFrom>) {
    *CAME_FROM.lock().unwrap_or_else(|e| e.into_inner()) = from;
}

/// Bring the `main` window forward: unminimized, shown and focused (the
/// system-wide keys' and tray items' path, lib.rs `show_main_with`).
pub(crate) fn bring_main_forward<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    show_main_window(app, &IN_TRAY);
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>, tray: &AtomicU64) {
    if let Some(window) = app.get_window("main") {
        let _ = window.unminimize();
        if window.show().is_ok() {
            write_tray_state(tray, false);
        }
        let _ = window.set_focus();
    }
}

/// Hide Hollow so macOS gives the previous app focus (a hidden or
/// minimized window alone leaves Hollow the active app with no window).
fn hide_app(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.hide();
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// After an Add sheet a system-wide key or tray item opened is done (S5
/// review). `saved`: go back where the person was, and only once the save
/// (an import included) has succeeded. Not saved (an import failed after the
/// sheet closed): bring Hollow forward, so the failure and the restored draft
/// are seen. Async so the window and app calls it makes are dispatched to
/// the main thread rather than run on it.
#[tauri::command]
pub async fn after_summoned_capture(app: tauri::AppHandle, saved: bool) {
    let from = CAME_FROM.lock().unwrap_or_else(|e| e.into_inner()).take();
    if !saved {
        bring_main_forward(&app);
        return;
    }
    let Some(window) = app.get_window("main") else {
        return;
    };
    match from {
        Some(CameFrom::Tray) => {
            if window.hide().is_ok() {
                set_in_tray(true);
            }
            hide_app(&app);
        }
        Some(CameFrom::Minimized) => {
            let _ = window.minimize();
            hide_app(&app);
        }
        Some(CameFrom::OtherApp) => {
            #[cfg(target_os = "macos")]
            let _ = app.hide();
            #[cfg(not(target_os = "macos"))]
            let _ = window.minimize();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(visible: bool, minimized: bool, focused: bool, in_tray: bool) -> Option<CameFrom> {
        came_from(&crate::window_state::WindowState {
            visible: Some(visible),
            minimized: Some(minimized),
            focused: Some(focused),
            in_tray,
        })
    }

    #[test]
    fn a_capture_steps_back_to_where_it_was_summoned_from() {
        assert_eq!(seen(false, false, false, true), Some(CameFrom::Tray));
        assert_eq!(seen(true, true, false, false), Some(CameFrom::Minimized));
        assert_eq!(seen(true, false, false, false), Some(CameFrom::OtherApp));
        // Hidden with ⌘H, not put in the tray: hide the app, not the window.
        assert_eq!(seen(false, false, false, false), Some(CameFrom::OtherApp));
        assert_eq!(seen(true, false, true, false), None);
        // Each summon replaces the last.
        note_came_from(Some(CameFrom::Tray));
        note_came_from(None);
        let last = *CAME_FROM.lock().unwrap();
        assert_eq!(last, None);
    }

    fn work(id: &str) -> Vec<(String, String)> {
        vec![(id.into(), "A lane".into())]
    }

    #[test]
    fn restart_requires_current_run_identity_even_when_the_name_is_unchanged() {
        assert!(verify_restart_keys(&["run-1".into()], &work("run-2"))
            .unwrap_err()
            .starts_with("RESTART_WORK_CHANGED:"));
        assert!(verify_restart_keys(&[], &work("run-1")).is_err());
        assert!(verify_restart_keys(&["run-1".into()], &work("run-1")).is_ok());
    }

    #[test]
    fn restart_allows_finished_work_and_ignores_snapshot_order() {
        let keys = vec!["run-1".into(), "run-2".into()];
        assert!(verify_restart_keys(&keys, &[]).is_ok());
        let mut current = work("run-2");
        current.extend(work("run-1"));
        assert!(verify_restart_keys(&keys, &current).is_ok());
    }

    #[test]
    fn unidentifiable_work_cannot_inherit_an_earlier_restart_approval() {
        for key in ["unverified:background", "unverified:Busy"] {
            assert!(verify_restart_keys(&[key.into()], &work(key))
                .unwrap_err()
                .contains("no stable identity"));
        }
    }

    #[test]
    fn restart_snapshot_includes_concurrent_scoped_and_full_vault_run_ids() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("workspaces/test")).unwrap();
        std::fs::write(dir.path().join("workspaces/test/WORKSPACE.md"), "# Test").unwrap();
        let full = crate::automated_history::try_begin_owned(dir.path(), "workflow", Some("test"))
            .unwrap();
        let scoped = crate::automated_history::try_begin_scoped(
            dir.path(),
            "workspaces/test",
            "chat",
            Some("test"),
        )
        .unwrap();
        let current = work_snapshot(dir.path()).unwrap();
        assert!(
            current.iter().any(|(key, _)| key == full.id()),
            "full run missing: {current:?}"
        );
        assert!(
            current.iter().any(|(key, _)| key == scoped.id()),
            "scoped run missing: {current:?}"
        );
        let scoped_id = scoped.id().to_owned();
        scoped.finish("success").unwrap();
        let remaining = work_snapshot(dir.path()).unwrap();
        assert!(!remaining.iter().any(|(key, _)| key == &scoped_id));
        assert!(remaining.iter().any(|(key, _)| key == full.id()));
        full.finish("success").unwrap();
    }

    #[test]
    fn an_unanswered_quit_is_recoverable_and_repeated_requests_keep_its_id() {
        let mut slot = None;
        let first = record_request(&mut slot, work("run-1"));
        assert_eq!(slot.as_ref().unwrap().id, first.id);
        assert_eq!(record_request(&mut slot, work("run-1")).id, first.id);
        assert!(resolve_request(&mut slot, &first.id, None)
            .unwrap()
            .is_none());
        assert!(slot.is_none());
        assert!(resolve_request(&mut slot, &first.id, None).is_err());
    }

    #[test]
    fn a_new_run_even_with_the_same_title_requires_another_confirmation() {
        let mut slot = None;
        let first = record_request(&mut slot, work("run-1"));
        let next = resolve_request(&mut slot, &first.id, Some(work("run-2")))
            .unwrap()
            .unwrap();
        assert_ne!(next.id, first.id);
        assert!(resolve_request(&mut slot, &first.id, None).is_err());
        assert_eq!(slot.as_ref().unwrap().id, next.id);
        assert!(resolve_request(&mut slot, &next.id, Some(work("run-2")))
            .unwrap()
            .is_none());
    }

    #[test]
    fn finished_work_does_not_force_another_confirmation() {
        let mut slot = None;
        let first = record_request(&mut slot, work("run-1"));
        assert!(resolve_request(&mut slot, &first.id, Some(vec![]))
            .unwrap()
            .is_none());
    }

    #[test]
    fn unreadable_work_asks_explicitly_but_does_not_trap_confirmed_quit() {
        let mut slot = None;
        let first = record_request(&mut slot, work("run-1"));
        let unknown = resolve_request(
            &mut slot,
            &first.id,
            Some(unknown_work("database unavailable")),
        )
        .unwrap()
        .unwrap();
        assert!(unknown.uncertain);
        assert_ne!(unknown.id, first.id);
        assert!(resolve_request(
            &mut slot,
            &unknown.id,
            Some(unknown_work("still unavailable"))
        )
        .unwrap()
        .is_none());
        assert!(slot.is_none());
    }

    #[test]
    fn defaults_to_minimize() {
        // Fresh process default is the legacy behavior.
        assert!(minimize_to_tray());
    }

    #[test]
    fn round_trips() {
        set_minimize_to_tray(false);
        assert!(!minimize_to_tray());
        set_minimize_to_tray(true);
        assert!(minimize_to_tray());
    }

    #[test]
    fn in_tray_round_trips() {
        let tray = AtomicU64::new(0);
        write_tray_state(&tray, true);
        assert_eq!(tray.load(Ordering::Acquire) & 1, 1);
        write_tray_state(&tray, false);
        assert_eq!(tray.load(Ordering::Acquire) & 1, 0);
    }

    fn main_with_child_webview() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        assert!(app.get_webview_window("main").is_some());
        main.as_ref()
            .window()
            .add_child(
                tauri::webview::WebviewBuilder::new("browser-tab-regression", Default::default()),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(100.0, 100.0),
            )
            .unwrap();
        // This is Tauri's real manager lookup, even with its mock dispatcher.
        assert!(app.get_webview_window("main").is_none());
        assert!(app.get_window("main").is_some());
        app
    }

    #[test]
    fn child_webview_does_not_make_the_card_window_missing() {
        let app = main_with_child_webview();
        let tray = AtomicU64::new(1);
        let state = observe_main_window(app.handle(), &tray).expect("native main still exists");
        assert!(state.ui_available());
        assert!(state.card_available());
        assert!(!state.in_tray, "live visibility repairs stale tray state");
        assert_eq!(tray.load(Ordering::Acquire) & 1, 0);
    }

    #[test]
    fn reopen_with_child_webview_clears_tray_before_the_card_gate() {
        let app = main_with_child_webview();
        let window = app.get_window("main").unwrap();
        window.hide().unwrap();
        let tray = AtomicU64::new(1);
        // MockRuntime does not model native hide/show visibility. Assert the
        // real reopen lookup clears the flag before observation can repair it.
        show_main_window(app.handle(), &tray);
        assert_eq!(tray.load(Ordering::Acquire) & 1, 0);
        let state = observe_main_window(app.handle(), &tray).unwrap();
        assert!(state.card_available());
        assert!(state.ui_available());
    }

    #[test]
    fn tray_snapshot_cannot_overwrite_a_newer_close() {
        let state = AtomicU64::new(0);
        let snapshot = state.load(Ordering::Acquire);
        write_tray_state(&state, true);
        repair_tray_state(&state, snapshot);
        assert_eq!(state.load(Ordering::Acquire) & 1, 1);
    }

    #[test]
    fn tray_snapshot_repair_rejects_an_aba_hide_show_hide_race() {
        let state = AtomicU64::new(1);
        let snapshot = state.load(Ordering::Acquire);
        write_tray_state(&state, false);
        write_tray_state(&state, true);
        repair_tray_state(&state, snapshot);
        assert_eq!(state.load(Ordering::Acquire) & 1, 1);
        let fresh = state.load(Ordering::Acquire);
        repair_tray_state(&state, fresh);
        assert_eq!(state.load(Ordering::Acquire) & 1, 0);
    }

    #[test]
    fn a_coo_quit_card_names_what_it_ends_and_caps_the_list() {
        assert_eq!(quit_card_lines(&[]), vec!["Nothing else is running."]);
        let work: Vec<RunningWork> = (0..12)
            .map(|n| RunningWork {
                key: format!("queue:{n}"),
                name: format!("Chat {n}"),
            })
            .collect();
        let lines = quit_card_lines(&work);
        assert_eq!(lines[0], "Running now (12), all of it ends:");
        assert_eq!(lines.len(), 12, "a header, ten names, the rest counted");
        assert_eq!(lines[1], "- Chat 0");
        assert_eq!(lines[11], "- and 2 more");
    }
}
