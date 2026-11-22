//! The webview bridge for Hollow app tools (PR A4): what only the webview
//! owns (UI settings in its stores and localStorage, and navigation) is
//! changed by the webview itself, through its own store setters.
//!
//! The broker calls [`request`]: it emits `app-ui-request {id, op, args}`,
//! and `src/appBridge.ts` applies it and answers with the `app_ui_respond`
//! command, which wakes the waiting request. [`TIMEOUT`] is 10 s. With
//! Hollow's window closed (the tray keeps the app running, the window is
//! hidden) nothing is asked: the user can't see the change, so a read is
//! refused with "open Hollow's window". A `set` is queued instead (see
//! [`queue_ui_setting_for`]): `pending-ui-settings.json` in the app-data
//! folder. Closing the window only hides it, so its page usually stays
//! loaded: the queue nudges it (`app-ui-pending`), and the webview drains
//! the queue then, at boot, and whenever the window is shown or focused
//! again, one at a time with live requests. Each entry carries a version;
//! the drain reads them (`take_pending_ui_settings`, which clears nothing)
//! and acknowledges what it settled (`ack_pending_ui_settings`), which
//! removes only those versions: an entry survives a reload mid-drain, and a
//! newer value queued meanwhile is kept for the next drain.

use serde_json::Map;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// The event the webview listens to.
pub(crate) const REQUEST_EVENT: &str = "app-ui-request";
/// How long a bridge request waits for the webview's answer.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);
/// The refusal while the window is closed.
pub(crate) const WINDOW_CLOSED: &str = "Hollow's window is closed, so this can't change what it shows. Open Hollow's window and try again; nothing changed.";

static SEQ: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Option<HashMap<String, tokio::sync::oneshot::Sender<Value>>>> =
    Mutex::new(None);

/// The pending answers by request id.
pub(crate) fn with_pending<R>(
    f: impl FnOnce(&mut HashMap<String, tokio::sync::oneshot::Sender<Value>>) -> R,
) -> R {
    let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The webview's answer to one `app-ui-request`: `{ok: true, value}` or
/// `{ok: false, error}`. Unknown or already-answered ids are an error.
#[tauri::command]
pub fn app_ui_respond(id: String, result: Value) -> Result<(), String> {
    let sender = with_pending(|pending| pending.remove(&id))
        .ok_or_else(|| format!("no pending Hollow app request {id}"))?;
    sender
        .send(result)
        .map_err(|_| "the request already gave up".to_string())
}

/// The queue file in the app-data folder: `{"settings": {key: value},
/// "versions": {key: n}, "next": n}` (a file from before versions reads as
/// version 0 for each key).
pub(crate) const PENDING_FILE: &str = "pending-ui-settings.json";
/// The nudge a queued change sends a loaded (hidden) webview to drain now.
pub(crate) const PENDING_EVENT: &str = "app-ui-pending";
/// What the COO tells the caller about a queued change.
pub(crate) const QUEUED_NOTE: &str = "Hollow's window is closed, so the COO queued this change. It applies at once if Hollow's page is still loaded behind the closed window, else when the window next opens.";

/// Serializes the queue's read-modify-write.
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

fn pending_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(PENDING_FILE))
}

/// The queue as stored: each setting, its version, the next version.
#[derive(Default)]
struct Queue {
    settings: Map<String, Value>,
    versions: Map<String, Value>,
    next: u64,
}

impl Queue {
    /// `key`'s version (0 for an entry from before versions).
    fn version(&self, key: &str) -> u64 {
        self.versions.get(key).and_then(Value::as_u64).unwrap_or(0)
    }
}

/// The queue in `path` (a missing or unreadable file is empty). Versions
/// never repeat: the counter outlives an emptied queue (the file keeps it),
/// and a missing or damaged file starts it from the clock in microseconds
/// (each queue is a file write, far slower than one a microsecond, and the
/// number stays below 2^53 for the webview), above any version handed out
/// before, so a stale ack never matches a newer entry (L13 delta review).
fn read_queue(path: &Path) -> Queue {
    let file = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    let object = |name: &str| {
        file.get(name)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    Queue {
        settings: object("settings"),
        versions: object("versions"),
        next: match file.get("next").and_then(Value::as_u64) {
            Some(next) => next,
            None if file.is_object() => 0,
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_micros() as u64),
        },
    }
}

/// Save `queue` to `path`. An emptied queue keeps the file with its
/// counter only, so versions never repeat ([`read_queue`]).
fn write_queue(path: &Path, queue: &Queue) -> Result<(), String> {
    let versions: Map<String, Value> = queue
        .settings
        .keys()
        .map(|key| (key.clone(), json!(queue.version(key))))
        .collect();
    let bytes = serde_json::to_vec_pretty(&json!({
        "settings": queue.settings,
        "versions": versions,
        "next": queue.next,
    }))
    .map_err(|e| format!("Encode pending settings: {e}"))?;
    crate::vault::atomic_write(path, &bytes).map_err(|e| format!("Save pending settings: {e}"))
}

/// Queue `key = value` in `path`, last write per key wins. Only a UI-owned,
/// non-secret key with a valid value is queued; nothing else is written.
pub(crate) fn queue_in(path: &Path, key: &str, value: Value) -> Result<(), String> {
    use crate::settings_keys::{self as keys, Owner, Type};
    let setting = keys::find(key).ok_or_else(|| format!("unknown setting {key}"))?;
    if !matches!(setting.owner, Owner::Ui) {
        return Err(format!("{key} is not a window setting; nothing queued."));
    }
    if matches!(setting.kind, Type::Secret) {
        return Err(format!(
            "{key} is a secret and is never queued; nothing changed."
        ));
    }
    keys::validate(setting, &value)?;
    let _guard = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut queue = read_queue(path);
    // Above every version handed out, an old file's 0s included.
    let next = queue.next.max(
        queue
            .settings
            .keys()
            .map(|k| queue.version(k))
            .max()
            .unwrap_or(0),
    ) + 1;
    queue.next = next;
    queue.settings.insert(key.to_string(), value);
    queue.versions.insert(key.to_string(), json!(next));
    write_queue(path, &queue)
}

/// The queued value for `key` in `path`, if any.
pub(crate) fn pending_in(path: &Path, key: &str) -> Option<Value> {
    let _guard = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_queue(path).settings.get(key).cloned()
}

/// Everything queued in `path`, with each entry's version:
/// `{"settings": {key: value}, "versions": {key: n}}`. Clears nothing: the
/// webview acknowledges what it settled with [`ack_in`].
pub(crate) fn take_in(path: &Path) -> Value {
    let _guard = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let queue = read_queue(path);
    let versions: Map<String, Value> = queue
        .settings
        .keys()
        .map(|key| (key.clone(), json!(queue.version(key))))
        .collect();
    json!({ "settings": queue.settings, "versions": versions })
}

/// Remove each entry of `path` whose version is the one `acked` names (the
/// webview applied it, or refused it for good). An entry queued again since
/// has a newer version and stays. Returns how many were removed.
pub(crate) fn ack_in(path: &Path, acked: &Map<String, Value>) -> Result<usize, String> {
    let _guard = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut queue = read_queue(path);
    let settled: Vec<String> = queue
        .settings
        .keys()
        .filter(|key| {
            acked
                .get(key.as_str())
                .and_then(Value::as_u64)
                .is_some_and(|version| version == queue.version(key))
        })
        .cloned()
        .collect();
    if settled.is_empty() {
        return Ok(0);
    }
    for key in &settled {
        queue.settings.remove(key);
        queue.versions.remove(key);
    }
    write_queue(path, &queue)?;
    Ok(settled.len())
}

/// Queue a UI setting change while the window is closed, then nudge the
/// webview: a hidden window's page is usually still loaded and applies it
/// at once ([`PENDING_EVENT`]); otherwise it is applied when the window
/// next opens.
pub(crate) fn queue_ui_setting_for(app: &AppHandle, key: &str, value: Value) -> Result<(), String> {
    queue_in(&pending_path()?, key, value)?;
    if tauri::Manager::get_window(app, "main").is_some() {
        let _ = app.emit(PENDING_EVENT, json!({ "key": key }));
    }
    Ok(())
}

/// The value the COO queued for `key`, if one is waiting.
pub(crate) fn pending_ui_setting(key: &str) -> Option<Value> {
    pending_in(&pending_path().ok()?, key)
}

/// The webview's drain (at boot, on a nudge, when the window is shown):
/// every queued UI setting with its version, `{settings, versions}`. Clears
/// nothing; see [`ack_pending_ui_settings`].
#[tauri::command]
pub fn take_pending_ui_settings() -> Result<Value, String> {
    Ok(take_in(&pending_path()?))
}

/// The webview settled these queued settings (`{key: version}`, from
/// [`take_pending_ui_settings`]): each is removed unless it was queued
/// again since. Returns how many were removed.
#[tauri::command]
pub fn ack_pending_ui_settings(versions: Map<String, Value>) -> Result<usize, String> {
    ack_in(&pending_path()?, &versions)
}

/// Whether Hollow's main window is open (visible or positively focused).
pub(crate) fn window_open(app: &AppHandle) -> bool {
    crate::lifecycle::main_window_state(app).is_some_and(|state| state.ui_available())
}

/// Ask the webview to do `op` with `args` and wait for its answer, on this
/// (non-runtime) thread. Refused while the window is closed.
pub(crate) fn request(app: &AppHandle, op: &str, args: Value) -> Result<Value, String> {
    if !window_open(app) {
        return Err(WINDOW_CLOSED.into());
    }
    let id = format!("ui-{}", SEQ.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = tokio::sync::oneshot::channel();
    with_pending(|pending| pending.insert(id.clone(), tx));
    if let Err(error) = app.emit(REQUEST_EVENT, json!({ "id": id, "op": op, "args": args })) {
        with_pending(|pending| pending.remove(&id));
        return Err(format!(
            "Could not reach Hollow's window ({error}); nothing changed."
        ));
    }
    wait(&id, rx, TIMEOUT)
}

/// Wait for `id`'s answer (split out for tests; the test bed driver waits
/// on its own ids with it).
pub(crate) fn wait(
    id: &str,
    mut rx: tokio::sync::oneshot::Receiver<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + timeout;
    let answer = loop {
        match rx.try_recv() {
            Ok(answer) => break answer,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                return Err("Hollow's window stopped answering; nothing changed.".into());
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        if std::time::Instant::now() >= deadline {
            with_pending(|pending| pending.remove(id));
            return Err(format!("Hollow's window did not answer within {} seconds. Open Hollow's window and try again.", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if answer.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(answer.get("value").cloned().unwrap_or(Value::Null))
    } else {
        Err(answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Hollow's window refused the change; nothing changed.")
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_wakes_its_request_once() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        with_pending(|p| p.insert("ui-test-1".into(), tx));
        app_ui_respond(
            "ui-test-1".into(),
            json!({ "ok": true, "value": { "old": "light", "new": "dark" } }),
        )
        .unwrap();
        assert_eq!(
            wait("ui-test-1", rx, Duration::from_secs(1)).unwrap(),
            json!({ "old": "light", "new": "dark" })
        );
        assert!(
            app_ui_respond("ui-test-1".into(), json!({ "ok": true })).is_err(),
            "answered once"
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        with_pending(|p| p.insert("ui-test-2".into(), tx));
        app_ui_respond(
            "ui-test-2".into(),
            json!({ "ok": false, "error": "unknown setting" }),
        )
        .unwrap();
        assert_eq!(
            wait("ui-test-2", rx, Duration::from_secs(1)).unwrap_err(),
            "unknown setting"
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        with_pending(|p| p.insert("ui-test-3".into(), tx));
        assert!(wait("ui-test-3", rx, Duration::from_millis(60))
            .unwrap_err()
            .contains("did not answer"));
        assert!(
            app_ui_respond("ui-test-3".into(), json!({ "ok": true })).is_err(),
            "a timed-out request is gone"
        );
    }
}
