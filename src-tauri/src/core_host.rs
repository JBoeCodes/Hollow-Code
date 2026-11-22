//! The core host seam (M1 PR 1). What a core service (the run queue pump,
//! chat turns, workflows, the heartbeat, the COO timer, notifications, the
//! phone API) needs from the process it runs in: the GUI app
//! (`tauri::AppHandle`) or, from M1 PR 2, `hollow serve`, which runs the core
//! with no window server (M0 found Tauri's `.setup` never runs without one).
//!
//! Across the daemon path the handle is used for five things: emit to every
//! listener, emit to the main window, read the managed `AppState`, look at
//! the main window, show a native banner. [`CoreHost`] is exactly that, plus
//! [`CoreHost::gui`], the escape hatch for a callee not migrated yet (sites
//! carry a `M1 PR2: migrate` comment).
//!
//! Rules that keep call sites untouched:
//! - Long-lived services hold [`Core`] (`Arc<dyn CoreHost>`); small helpers
//!   take `&impl crate::core_host::CoreHost`; owned entry points take
//!   `impl Into<Core>`, so a call passing an `AppHandle` compiles as before.
//! - Never `use crate::core_host::CoreHost;` at module level: a module that
//!   also calls `tauri::Emitter::emit` on an `AppHandle` would see two
//!   `emit`s (E0034). Bounds and [`Core`]'s inherent methods need no import.
//!
//! Emits are byte-identical to Tauri's own: `Emitter::emit` serializes with
//! `serde_json::to_string` (`EmitArgs::new`) and `emit_str` hands a string
//! already serialized the same way to the same `manager().emit`.

use crate::commands::AppState;
use serde::Serialize;
use std::sync::Arc;

/// What a core service needs from the process it runs in. Every required
/// method is sync and object safe; the generic `emit` is `where Self:
/// Sized`, so `dyn CoreHost` stays a trait object. A helper taking `&impl
/// CoreHost` reads state through `core_state()`; [`Core`] also has
/// `emit_to`, `try_state` and `state` under Tauri's names.
pub(crate) trait CoreHost: Send + Sync + 'static {
    /// One event to every listener. `json` is the payload already serialized
    /// with `serde_json::to_string`.
    fn emit_json(&self, event: &str, json: String) -> Result<(), String>;
    /// One event to one window label (the phone API's `emit_to("main", ..)`).
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String>;
    /// The managed app state; None before `.manage` and in a bare test host.
    fn core_state(&self) -> Option<&AppState>;
    /// (focused, visible) of the main window; None when this host has none.
    fn main_window(&self) -> Option<(bool, bool)>;
    /// A native banner.
    fn show_native(&self, title: &str, body: &str) -> Result<(), String>;
    /// The GUI's handle, for a callee not migrated yet. None in serve and tests.
    fn gui(&self) -> Option<&tauri::AppHandle>;
    /// This host as an owned [`Core`] (each impl is a cheap clone handle).
    fn core(&self) -> Core;
    /// What Hollow app tool handlers run on when this host has no GUI
    /// handle (`hollow serve`, M1 PR 5). None: the broker refuses them.
    fn action_host(&self) -> Option<Box<dyn crate::app_actions::Host>> {
        None
    }

    /// Same name and payload rule as `tauri::Emitter::emit`.
    fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String>
    where
        Self: Sized,
    {
        self.emit_json(event, to_json(&payload)?)
    }
}

fn to_json<S: Serialize>(payload: &S) -> Result<String, String> {
    serde_json::to_string(payload).map_err(|e| e.to_string())
}

/// The managed types a core service reads through the host (only `AppState`).
pub(crate) trait Managed: Send + Sync + 'static {
    fn from_host<H: CoreHost + ?Sized>(host: &H) -> Option<&Self>;
}

impl Managed for AppState {
    fn from_host<H: CoreHost + ?Sized>(host: &H) -> Option<&Self> {
        host.core_state()
    }
}

/// The owned handle long-lived services hold: one allocation, cloned into
/// tasks. Its inherent methods mirror the trait's, so a body written against
/// an `AppHandle` (`app.emit(..)`, `app.try_state::<AppState>()`) compiles
/// against a `Core` with no trait import.
#[derive(Clone)]
pub(crate) struct Core(Arc<dyn CoreHost>);

impl Core {
    /// A host that is not the GUI's `AppHandle` (`hollow serve`, tests).
    pub(crate) fn new(host: impl CoreHost) -> Self {
        Self(Arc::new(host))
    }

    pub(crate) fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
        self.0.emit_json(event, to_json(&payload)?)
    }

    pub(crate) fn emit_to<S: Serialize>(
        &self,
        target: &str,
        event: &str,
        payload: S,
    ) -> Result<(), String> {
        self.0.emit_json_to(target, event, to_json(&payload)?)
    }

    pub(crate) fn try_state<T: Managed>(&self) -> Option<&T> {
        T::from_host(&*self.0)
    }

    pub(crate) fn state<T: Managed>(&self) -> &T {
        self.try_state::<T>()
            .expect("state() called before manage() for AppState")
    }

    pub(crate) fn gui(&self) -> Option<&tauri::AppHandle> {
        self.0.gui()
    }

    pub(crate) fn action_host(&self) -> Option<Box<dyn crate::app_actions::Host>> {
        self.0.action_host()
    }
}

impl CoreHost for Core {
    fn emit_json(&self, event: &str, json: String) -> Result<(), String> {
        self.0.emit_json(event, json)
    }
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String> {
        self.0.emit_json_to(target, event, json)
    }
    fn core_state(&self) -> Option<&AppState> {
        self.0.core_state()
    }
    fn main_window(&self) -> Option<(bool, bool)> {
        self.0.main_window()
    }
    fn show_native(&self, title: &str, body: &str) -> Result<(), String> {
        self.0.show_native(title, body)
    }
    fn gui(&self) -> Option<&tauri::AppHandle> {
        self.0.gui()
    }
    fn core(&self) -> Core {
        self.clone()
    }
    fn action_host(&self) -> Option<Box<dyn crate::app_actions::Host>> {
        self.0.action_host()
    }
}

impl From<tauri::AppHandle> for Core {
    fn from(app: tauri::AppHandle) -> Self {
        Self(Arc::new(app))
    }
}

impl From<&tauri::AppHandle> for Core {
    fn from(app: &tauri::AppHandle) -> Self {
        Self(Arc::new(app.clone()))
    }
}

impl From<&Core> for Core {
    fn from(core: &Core) -> Self {
        core.clone()
    }
}

/// The emits, state and window reads both Tauri runtimes share (the GUI's
/// Wry and, in tests, the mock runtime).
mod tauri_host {
    use crate::commands::AppState;
    use tauri::{Emitter, Manager, Runtime};

    pub(super) fn emit_json<R: Runtime>(
        app: &tauri::AppHandle<R>,
        event: &str,
        json: String,
    ) -> Result<(), String> {
        app.emit_str(event, json).map_err(|e| e.to_string())
    }

    pub(super) fn emit_json_to<R: Runtime>(
        app: &tauri::AppHandle<R>,
        target: &str,
        event: &str,
        json: String,
    ) -> Result<(), String> {
        app.emit_str_to(target, event, json)
            .map_err(|e| e.to_string())
    }

    pub(super) fn core_state<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<&AppState> {
        app.try_state::<AppState>().map(|s| s.inner())
    }

    pub(super) fn main_window<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<(bool, bool)> {
        app.get_window("main").map(|w| {
            (
                w.is_focused().unwrap_or(false),
                w.is_visible().unwrap_or(false),
            )
        })
    }
}

impl CoreHost for tauri::AppHandle {
    fn emit_json(&self, event: &str, json: String) -> Result<(), String> {
        tauri_host::emit_json(self, event, json)
    }
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String> {
        tauri_host::emit_json_to(self, target, event, json)
    }
    fn core_state(&self) -> Option<&AppState> {
        tauri_host::core_state(self)
    }
    fn main_window(&self) -> Option<(bool, bool)> {
        tauri_host::main_window(self)
    }
    fn show_native(&self, title: &str, body: &str) -> Result<(), String> {
        use tauri_plugin_notification::NotificationExt;
        self.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| e.to_string())
    }
    fn gui(&self) -> Option<&tauri::AppHandle> {
        Some(self)
    }
    fn core(&self) -> Core {
        Core::from(self)
    }
}

/// A mock app: real Tauri emits and managed state, no window, no
/// notification plugin (a mock app never registers it), no GUI handle.
#[cfg(test)]
impl CoreHost for tauri::AppHandle<tauri::test::MockRuntime> {
    fn emit_json(&self, event: &str, json: String) -> Result<(), String> {
        tauri_host::emit_json(self, event, json)
    }
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String> {
        tauri_host::emit_json_to(self, target, event, json)
    }
    fn core_state(&self) -> Option<&AppState> {
        tauri_host::core_state(self)
    }
    fn main_window(&self) -> Option<(bool, bool)> {
        tauri_host::main_window(self)
    }
    fn show_native(&self, _title: &str, _body: &str) -> Result<(), String> {
        Ok(())
    }
    fn gui(&self) -> Option<&tauri::AppHandle> {
        None
    }
    fn core(&self) -> Core {
        Core::new(self.clone())
    }
}

/// A host that records what it was asked to do, for tests of services run
/// through the seam with no Tauri app at all.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct RecordingHost(Arc<RecInner>);

#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecInner {
    /// (target, event, json); target None for an emit to everyone.
    events: std::sync::Mutex<Vec<(Option<String>, String, String)>>,
    banners: std::sync::Mutex<Vec<(String, String)>>,
    state: Option<AppState>,
    window: Option<(bool, bool)>,
}

#[cfg(test)]
impl RecordingHost {
    /// A recorder with a managed state and a main window (focused, visible).
    pub(crate) fn with(state: Option<AppState>, window: Option<(bool, bool)>) -> Self {
        Self(Arc::new(RecInner {
            state,
            window,
            ..RecInner::default()
        }))
    }

    /// Every emit, in order: (target, event, json).
    pub(crate) fn events(&self) -> Vec<(Option<String>, String, String)> {
        self.0.events.lock().unwrap().clone()
    }

    /// The parsed payloads of one event, in order.
    pub(crate) fn of(&self, event: &str) -> Vec<serde_json::Value> {
        self.events()
            .into_iter()
            .filter(|(_, e, _)| e == event)
            .map(|(_, _, json)| serde_json::from_str(&json).unwrap())
            .collect()
    }

    /// Every native banner shown: (title, body).
    pub(crate) fn banners(&self) -> Vec<(String, String)> {
        self.0.banners.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl CoreHost for RecordingHost {
    fn emit_json(&self, event: &str, json: String) -> Result<(), String> {
        self.0
            .events
            .lock()
            .unwrap()
            .push((None, event.to_string(), json));
        Ok(())
    }
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String> {
        self.0
            .events
            .lock()
            .unwrap()
            .push((Some(target.to_string()), event.to_string(), json));
        Ok(())
    }
    fn core_state(&self) -> Option<&AppState> {
        self.0.state.as_ref()
    }
    fn main_window(&self) -> Option<(bool, bool)> {
        self.0.window
    }
    fn show_native(&self, title: &str, body: &str) -> Result<(), String> {
        self.0
            .banners
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
    fn gui(&self) -> Option<&tauri::AppHandle> {
        None
    }
    fn core(&self) -> Core {
        Core::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn mock_app(v: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
        let mut config = crate::config::Config::placeholder();
        config.vault_path = v.to_path_buf();
        config.claude_projects_path = v.join("no-claude-projects");
        tauri::test::mock_builder()
            .manage(AppState::new(config, v.join("config.json")))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    /// Every payload string one event delivers to an app listener.
    fn heard<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: &str) -> Arc<Mutex<Vec<String>>> {
        use tauri::Listener;
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        app.listen_any(event, move |e| {
            sink.lock().unwrap().push(e.payload().to_string())
        });
        got
    }

    /// A payload with the shapes chat events carry: nesting, unicode,
    /// escapes, floats, nulls and key order.
    fn payload() -> serde_json::Value {
        json!({
            "kind": "text", "chat_id": "c-1", "z": 1, "a": [1.5, null, true],
            "text": "quote \" newline \n tab \t snowman \u{2603} nbsp \u{a0}",
            "nested": {"b": {"c": "d"}, "empty": {}}
        })
    }

    #[test]
    fn a_core_over_the_app_emits_the_same_bytes_as_tauri() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app(dir.path());
        let handle = app.handle().clone();
        for event in ["chat-event", "run-queue-event", "notification-event"] {
            let got = heard(&handle, event);
            tauri::Emitter::emit(&handle, event, payload()).unwrap();
            Core::new(handle.clone()).emit(event, payload()).unwrap();
            CoreHost::emit(&handle, event, payload()).unwrap();
            let got = got.lock().unwrap().clone();
            assert_eq!(got.len(), 3, "{event}: {got:?}");
            assert_eq!(got[0], got[1], "{event}");
            assert_eq!(got[0], got[2], "{event}");
            assert_eq!(got[0], serde_json::to_string(&payload()).unwrap());
        }
    }

    /// A targeted emit (the phone API's `emit_to("main", ..)`) through a
    /// `Core` reaches a listener with the same bytes as Tauri's own
    /// `emit_to` with that label.
    #[test]
    fn a_targeted_emit_sends_the_same_bytes_as_tauri() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app(dir.path());
        let handle = app.handle().clone();
        let got = heard(&handle, "phone-modes-wanted");
        tauri::Emitter::emit_to(&handle, "main", "phone-modes-wanted", payload()).unwrap();
        Core::new(handle.clone())
            .emit_to("main", "phone-modes-wanted", payload())
            .unwrap();
        CoreHost::emit_json_to(
            &handle,
            "main",
            "phone-modes-wanted",
            serde_json::to_string(&payload()).unwrap(),
        )
        .unwrap();
        let got = got.lock().unwrap().clone();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], got[1]);
        assert_eq!(got[0], got[2]);
        assert_eq!(got[0], serde_json::to_string(&payload()).unwrap());
    }

    /// The chat and notification helpers, now generic over the host, send
    /// the same bytes whether handed the app or a `Core` over it.
    #[test]
    fn migrated_helpers_send_the_same_bytes_either_way() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app(dir.path());
        let handle = app.handle().clone();
        let core = Core::new(handle.clone());
        // The event name as a literal: `phone_access_tests` allows the chat
        // module's constant in `chat.rs` only.
        let chat = heard(&handle, "chat-event");
        crate::chat::emit_event(&handle, payload());
        crate::chat::emit_event(&core, payload());
        let chat = chat.lock().unwrap().clone();
        assert_eq!(chat.len(), 2);
        assert_eq!(chat[0], chat[1]);

        let notes = heard(&handle, crate::notifications::EVENT);
        let input = || crate::notifications::NotificationInput {
            title: "T".into(),
            body: "B".into(),
            kind: "info".into(),
            source: None,
            action_label: None,
            action_url: None,
            notify_native: false,
        };
        let a = crate::notifications::create(&handle, dir.path(), input()).unwrap();
        let b = crate::notifications::create(&core, dir.path(), input()).unwrap();
        let notes = notes.lock().unwrap().clone();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0], serde_json::to_string(&a).unwrap());
        assert_eq!(notes[1], serde_json::to_string(&b).unwrap());
    }

    #[test]
    fn a_core_over_the_app_reads_the_managed_state() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app(dir.path());
        let core = Core::new(app.handle().clone());
        let state = core.try_state::<AppState>().expect("managed");
        assert_eq!(state.config.vault_path, dir.path());
        assert_eq!(core.state::<AppState>().config.vault_path, dir.path());
        // A mock app has no main window and no GUI handle.
        assert_eq!(CoreHost::main_window(&core), None);
        assert!(core.gui().is_none());
    }

    #[test]
    fn a_bare_recorder_has_no_state() {
        let core = Core::new(RecordingHost::default());
        assert!(core.try_state::<AppState>().is_none());
        assert!(core.gui().is_none());
    }

    #[test]
    #[should_panic(expected = "state() called before manage()")]
    fn state_on_a_bare_recorder_panics() {
        let core = Core::new(RecordingHost::default());
        let _ = core.state::<AppState>();
    }

    #[test]
    fn the_recorder_records_emits_in_order_with_their_target() {
        let rec = RecordingHost::default();
        let core = Core::new(rec.clone());
        core.emit("a", json!({"n": 1})).unwrap();
        core.emit_to("main", "b", ()).unwrap();
        CoreHost::show_native(&core, "T", "B").unwrap();
        assert_eq!(
            rec.events(),
            vec![
                (None, "a".to_string(), "{\"n\":1}".to_string()),
                (
                    Some("main".to_string()),
                    "b".to_string(),
                    "null".to_string()
                ),
            ]
        );
        assert_eq!(rec.of("a"), vec![json!({"n": 1})]);
        assert_eq!(rec.banners(), vec![("T".to_string(), "B".to_string())]);
        // core() hands back the same host.
        CoreHost::core(&core).emit("c", 1).unwrap();
        assert_eq!(rec.of("c"), vec![json!(1)]);
    }
}
