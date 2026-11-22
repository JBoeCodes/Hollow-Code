//! In-app notification center, backed by `vault/.notifications.jsonl`.
//!
//! Storage is append-only JSON Lines — one object per line, newest at
//! the bottom. Reads are bounded (last `READ_CAP` entries) so growth is
//! self-limiting. Writes go through [`with_lock`] to serialize concurrent
//! creates from multiple workflows firing on the same tick.
//!
//! The vault lives this file under `.notifications.jsonl` (dotfile, so
//! Obsidian's default tree hides it). Putting it in the vault rather
//! than in the app data dir means Claude can read prior notifications
//! if a workflow wants to ("don't notify me again if I already saw
//! this today" is a workflow-prompt concern).
//!
//! Two delivery surfaces ride on each `create`:
//!   1. The persistent record in the JSONL — surfaced in the in-app
//!      notification center dropdown.
//!   2. An optional native macOS / Linux system notification, gated on
//!      the `notify_native` flag passed in by the caller.
//!
//! The in-app pop-up is the frontend's job — it lives by listening on
//! the `notification-event` tauri event we emit here.

use crate::core_host::Core;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;

pub const EVENT: &str = "notification-event";
const FILE_NAME: &str = ".notifications.jsonl";
/// Keep at most this many notifications in the file. Older entries
/// roll off on the next write that crosses the threshold.
const KEEP: usize = 500;
/// Cap on how many we return from `list`. The UI dropdown only ever
/// shows the most recent batch, so there's no reason to ship more
/// across the IPC bus.
const READ_CAP: usize = 200;
/// Unread failures are kept past [`KEEP`] (P0-08), up to this many entries
/// in all; past it the oldest fold into one "N older failures were
/// trimmed" failure with this source.
const HARD_CAP: usize = 1000;
const TRIMMED_SOURCE: &str = "notifications.trimmed";

/// Serializes read-modify-write on the JSONL file so two workflows
/// finishing on the same scheduler tick can't clobber each other's
/// notifications.
static FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Process-wide override for native OS notifications. The frontend
/// Settings toggle writes here via the `set_native_enabled` Tauri
/// command. When false, [`create`] suppresses the native pop even if
/// the caller passed `notify_native: true` — the in-app surface still
/// fires. Defaults to true so a fresh install gets the full experience.
static NATIVE_ENABLED: AtomicBool = AtomicBool::new(true);

/// The process's core host (the GUI's app handle), registered once at
/// setup, so AppHandle-free code (permission refusals, post-run flags) can
/// still notify.
static APP: OnceLock<Core> = OnceLock::new();

/// Register the process's core host (the GUI's app handle, or `hollow
/// serve`'s host), from `core_boot`. A second call is ignored.
pub(crate) fn register(core: Core) {
    let _ = APP.set(core);
}

/// The registered core host. None when none is registered.
pub(crate) fn registered() -> Option<&'static Core> {
    APP.get()
}

/// Whether this process registered a core host (the GUI's, today; false in
/// tests, the headless CLI and MCP subprocesses). A reclaim of another process's run
/// (Paperclip PR 18) happens only where its flags can be announced.
pub fn app_registered() -> bool {
    APP.get().is_some()
}

/// [`create`] through the registered app handle. None when no handle is
/// registered (tests, the headless CLI and MCP subprocesses).
pub fn create_registered(
    vault: &Path,
    input: NotificationInput,
) -> Option<Result<Notification, String>> {
    APP.get().map(|app| create(app, vault, input))
}

/// The registered GUI app handle, for a sync module that emits its own Tauri
/// event (PR 8's `budget-changed`). None when none is registered or the
/// registered host is not the GUI.
pub(crate) fn registered_app() -> Option<&'static AppHandle> {
    APP.get().and_then(Core::gui)
}

/// The notification scope the user is looking at right now (Paperclip PR
/// 22): `workflow:<slug>` while that workflow is open in Automations, else
/// None. Set by the `set_foreground_notification_scope` command.
static FOREGROUND_SCOPE: Mutex<Option<String>> = Mutex::new(None);

/// Set or clear the foreground scope.
pub fn set_foreground_scope(scope: Option<String>) {
    let scope = scope.filter(|s| !s.trim().is_empty());
    *FOREGROUND_SCOPE.lock().unwrap_or_else(|p| p.into_inner()) = scope;
}

/// Whether a notification's NATIVE banner is skipped because the user is
/// already watching what it reports: the scope is a `workflow:` scope, the
/// notification opens exactly that scope, it is not a safety notice
/// (`run-guard`, `budgets`), and the main window is focused and visible.
/// Every other scheme (`workflow-permissions:`, `workflow-failures:`,
/// `budget-pauses:`, `settings:`) never matches a `workflow:` scope. The
/// JSONL record and the in-app event are never affected.
pub(crate) fn foreground_suppressed_decision(
    scope: Option<&str>,
    action_url: Option<&str>,
    source: Option<&str>,
    focused: bool,
    visible: bool,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    scope.starts_with("workflow:")
        && action_url == Some(scope)
        && !matches!(source, Some("run-guard") | Some("budgets"))
        && focused
        && visible
}

/// [`foreground_suppressed_decision`] against the live scope and the main
/// window. An unreadable window state never suppresses.
fn foreground_suppressed(app: &impl crate::core_host::CoreHost, n: &Notification) -> bool {
    let scope = FOREGROUND_SCOPE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if scope.is_none() {
        return false;
    }
    // The host reads `is_focused()`/`is_visible()` of the main window, each
    // `.unwrap_or(false)`; no window reads as neither.
    let (focused, visible) = app.main_window().unwrap_or((false, false));
    foreground_suppressed_decision(
        scope.as_deref(),
        n.action_url.as_deref(),
        n.source.as_deref(),
        focused,
        visible,
    )
}

pub fn set_native_enabled(v: bool) {
    NATIVE_ENABLED.store(v, Ordering::Relaxed);
}

pub fn native_enabled() -> bool {
    NATIVE_ENABLED.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub body: String,
    /// `info` | `success` | `warning` | `error` | `workflow` — drives
    /// the color treatment in the UI.
    pub kind: String,
    /// Origin tag — for workflow runs this is the slug; for system
    /// messages it can be a free-form label. Used to dedupe and to
    /// drive the click-through.
    #[serde(default)]
    pub source: Option<String>,
    /// Label for the primary action button shown in the toast + center
    /// (e.g. "Open"). When `action_url` is set but no label, the UI
    /// falls back to "Open".
    #[serde(default)]
    pub action_label: Option<String>,
    /// Where clicking the notification jumps to. Supported schemes:
    ///   - `vault:<rel/path/to/note.md>` — open the vault note
    ///   - `workflow:<slug>` — open the workflow detail pane
    ///   - `workflow-permissions:<slug>` — open Automations → Permission
    ///     changes for that workflow's pending review
    ///   - `settings:<section>` — open a Settings section (e.g. `history`)
    ///   - `peer-chat:<machineId>/<key>`: open that chat on another Mac
    ///     (a routed turn ended there, `dispatcher_coo::ended_notice`)
    ///   - `https://...` / `http://...` — open in the user's browser
    #[serde(default)]
    pub action_url: Option<String>,
    pub created: String,
    pub read: bool,
}

/// Input shape from a Tauri command — id + created are server-assigned,
/// `read` always starts false.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationInput {
    pub title: String,
    pub body: String,
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub action_label: Option<String>,
    #[serde(default)]
    pub action_url: Option<String>,
    #[serde(default)]
    pub notify_native: bool,
}

fn file_path(vault: &Path) -> PathBuf {
    vault.join(FILE_NAME)
}

/// The newest notifications and how many the file holds (P0-08), so a
/// capped list can say "Newest 200 of N".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPage {
    pub items: Vec<Notification>,
    pub total: usize,
}

/// Read the full list (most recent up to `READ_CAP`, newest first), with
/// the file's total.
pub fn list(vault: &Path) -> NotificationPage {
    let all = read_all(vault);
    let total = all.len();
    NotificationPage {
        items: all.into_iter().rev().take(READ_CAP).collect(),
        total,
    }
}

/// Every unread `error` notification in the whole file, newest first: the
/// failures still waiting on the user ([`list`] keeps only the newest
/// `READ_CAP`). Marking one read is how it stops waiting.
///
/// Unlike [`read_all`] this fails closed: a missing file is an empty list,
/// but a file that exists and can't be read (permissions, or one byte that
/// isn't UTF-8) is an `Err`, so the "needs you" query names the source as
/// unavailable instead of silently showing no failures.
pub(crate) fn unread_errors(vault: &Path) -> Result<Vec<Notification>, String> {
    let path = file_path(vault);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    Ok(parse_lines(&content)
        .into_iter()
        .rev()
        .filter(|n| !n.read && n.kind == "error")
        .collect())
}

pub fn unread_count(vault: &Path) -> usize {
    read_all(vault).iter().filter(|n| !n.read).count()
}

fn read_all(vault: &Path) -> Vec<Notification> {
    let path = file_path(vault);
    if !path.exists() {
        return Vec::new();
    }
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_lines(&content)
}

fn parse_lines(content: &str) -> Vec<Notification> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Notification>(l).ok())
        .collect()
}

/// Append a new notification to the JSONL. Emits the `notification-event`
/// Tauri event so the frontend can pop a toast immediately. Optionally
/// fires a native system notification when `notify_native` is true.
pub fn create(
    app: &impl crate::core_host::CoreHost,
    vault: &Path,
    input: NotificationInput,
) -> Result<Notification, String> {
    let n = Notification {
        id: generate_id(),
        title: input.title,
        body: input.body,
        kind: input.kind,
        source: input.source,
        action_label: input.action_label,
        action_url: input.action_url,
        created: Utc::now().to_rfc3339(),
        read: false,
    };

    {
        let _g = FILE_LOCK
            .lock()
            .map_err(|_| "notifications lock poisoned".to_string())?;
        let path = file_path(vault);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir vault: {e}"))?;
        }

        // Read existing, append new, possibly trim oldest entries if
        // we've crossed the cap. Rewriting is cheap for the volume
        // we're dealing with (a few hundred entries max).
        let mut all = read_all(vault);
        all.push(n.clone());
        trim(&mut all);
        write_all(&path, &all)?;
    }

    if input.notify_native && native_enabled() && !foreground_suppressed(app, &n) {
        // Best-effort: a Tauri permission denial or platform-level failure
        // shouldn't break the in-app notification path. Log to stderr so
        // a misconfigured capability is visible to the dev console without
        // surfacing a noisy UI error.
        if let Err(e) = app.show_native(&n.title, &n.body) {
            eprintln!("[notifications] native notification failed: {e}");
        }
    }

    let _ = app.emit(EVENT, &n);
    Ok(n)
}

pub fn mark_read(vault: &Path, id: &str) -> Result<(), String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let mut all = read_all(vault);
    let mut touched = false;
    for n in all.iter_mut() {
        if n.id == id && !n.read {
            n.read = true;
            touched = true;
        }
    }
    if !touched {
        return Ok(());
    }
    write_all(&file_path(vault), &all)
}

/// A failure still waiting in Review → Failures ([`unread_errors`]).
fn waiting_failure(n: &Notification) -> bool {
    !n.read && n.kind == "error"
}

/// Bring the file back to [`KEEP`] entries (P0-08): the oldest read entries
/// go first, then the oldest unread ones that aren't failures. Unread
/// failures stay, so a busy day can't rotate one away unseen; past
/// [`HARD_CAP`] the oldest of them fold into one "N older failures were
/// trimmed" failure, which carries the earlier count forward.
fn trim(all: &mut Vec<Notification>) {
    let passes: [fn(&Notification) -> bool; 2] = [|n| n.read, |n| !waiting_failure(n)];
    for pass in passes {
        let mut excess = all.len().saturating_sub(KEEP);
        if excess == 0 {
            return;
        }
        all.retain(|n| {
            if excess > 0 && pass(n) {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }
    if all.len() <= HARD_CAP {
        return;
    }
    let mut trimmed = 0;
    // One summary, which keeps its id and date as its count grows, so a
    // Review row opened earlier still names it.
    let mut kept = None;
    if let Some(i) = all
        .iter()
        .position(|n| n.kind == "error" && n.source.as_deref() == Some(TRIMMED_SOURCE))
    {
        let earlier = all.remove(i);
        trimmed += earlier
            .title
            .split(' ')
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0);
        kept = Some((earlier.id, earlier.created));
    }
    let (id, created) = kept.unwrap_or_else(|| (generate_id(), Utc::now().to_rfc3339()));
    // Room for the summary line itself.
    let drop = (all.len() + 1).saturating_sub(HARD_CAP);
    all.drain(0..drop);
    trimmed += drop;
    all.insert(
        0,
        Notification {
            id,
            title: format!("{trimmed} older failures were trimmed"),
            body: format!("More than {HARD_CAP} notifications were waiting, so the oldest failures were removed unread. The newer ones are still listed."),
            kind: "error".into(),
            source: Some(TRIMMED_SOURCE.into()),
            action_label: None,
            action_url: None,
            created,
            read: false,
        },
    );
}

/// Mark the listed notifications read (Recent activity's Mark these read);
/// returns how many changed. A failure still waiting in Review is left
/// unread even when listed: it is marked read where it is shown.
pub fn mark_read_many(vault: &Path, ids: &[String]) -> Result<usize, String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let mut all = read_all(vault);
    let mut count = 0;
    for n in all.iter_mut() {
        if !n.read && !waiting_failure(n) && ids.contains(&n.id) {
            n.read = true;
            count += 1;
        }
    }
    if count > 0 {
        write_all(&file_path(vault), &all)?;
    }
    Ok(count)
}

/// Delete the listed notifications (Recent activity's Clear these); returns
/// how many went. A failure still waiting in Review stays.
pub fn delete_many(vault: &Path, ids: &[String]) -> Result<usize, String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let mut all = read_all(vault);
    let before = all.len();
    all.retain(|n| waiting_failure(n) || !ids.contains(&n.id));
    let count = before - all.len();
    if count > 0 {
        write_all(&file_path(vault), &all)?;
    }
    Ok(count)
}

pub fn mark_all_read(vault: &Path) -> Result<usize, String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let mut all = read_all(vault);
    let mut count = 0;
    for n in all.iter_mut() {
        if !n.read {
            n.read = true;
            count += 1;
        }
    }
    if count > 0 {
        write_all(&file_path(vault), &all)?;
    }
    Ok(count)
}

pub fn delete(vault: &Path, id: &str) -> Result<(), String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let mut all = read_all(vault);
    let before = all.len();
    all.retain(|n| n.id != id);
    if all.len() == before {
        return Ok(());
    }
    write_all(&file_path(vault), &all)
}

pub fn clear(vault: &Path) -> Result<usize, String> {
    let _g = FILE_LOCK
        .lock()
        .map_err(|_| "notifications lock poisoned".to_string())?;
    let all = read_all(vault);
    let count = all.len();
    let path = file_path(vault);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("remove: {e}"))?;
    }
    Ok(count)
}

fn write_all(path: &Path, all: &[Notification]) -> Result<(), String> {
    // Synced temp file, rename, directory sync: neither a kill nor a power
    // loss mid-write leaves a torn or empty inbox.
    let mut body = String::new();
    for n in all {
        let line = serde_json::to_string(n).map_err(|e| format!("serialize: {e}"))?;
        body.push_str(&line);
        body.push('\n');
    }
    crate::vault::atomic_write(path, body.as_bytes())
}

/// The text of a chat announcement banner (0.4): trimmed, control characters dropped and
/// clipped, so a long or odd chat title can't make an unreadable or oversized banner.
pub fn announce_text(title: &str, body: &str) -> (String, String) {
    // Control characters and invisible format characters (zero width, bidi overrides) never reach
    // a banner: a title can come from agent output.
    fn invisible(c: char) -> bool {
        c.is_control()
            || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    }
    fn clean(s: &str, max: usize) -> String {
        let t: String = s.chars().filter(|c| !invisible(*c)).collect();
        let t = t.trim();
        if t.chars().count() <= max {
            t.to_string()
        } else {
            let mut cut: String = t.chars().take(max.saturating_sub(1)).collect();
            cut.push('\u{2026}');
            cut
        }
    }
    let title = clean(title, 120);
    (
        if title.is_empty() {
            "Hollow".to_string()
        } else {
            title
        },
        clean(body, 240),
    )
}

/// The dock badge count for a chat count: 0 clears it, anything above 99 reads 99.
pub fn badge_value(count: u32) -> Option<i64> {
    if count == 0 {
        None
    } else {
        Some(i64::from(count.min(99)))
    }
}

fn generate_id() -> String {
    // 16-char base36-ish id — plenty of entropy for an in-vault store.
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
    (0..16)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn announce_text_clips_and_cleans() {
        let (t, b) = announce_text("  \u{7}Build\nnotes ", "Finished.");
        assert_eq!((t.as_str(), b.as_str()), ("Buildnotes", "Finished."));
        assert_eq!(announce_text("", "x").0, "Hollow");
        assert_eq!(announce_text("a\u{202E}b\u{200B}c", "x").0, "abc");
        let (long, _) = announce_text(&"a".repeat(500), &"b".repeat(500));
        assert_eq!(long.chars().count(), 120);
        assert!(long.ends_with('\u{2026}'));
        assert_eq!(announce_text("t", &"b".repeat(500)).1.chars().count(), 240);
    }

    #[test]
    fn badge_value_clears_at_zero_and_caps_at_99() {
        assert_eq!(badge_value(0), None);
        assert_eq!(badge_value(3), Some(3));
        assert_eq!(badge_value(400), Some(99));
    }

    use super::*;
    use tempfile::TempDir;

    fn temp_vault() -> TempDir {
        TempDir::new().unwrap()
    }

    fn mk(title: &str) -> Notification {
        Notification {
            id: generate_id(),
            title: title.into(),
            body: "body".into(),
            kind: "info".into(),
            source: None,
            action_label: None,
            action_url: None,
            created: Utc::now().to_rfc3339(),
            read: false,
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = temp_vault();
        let path = file_path(dir.path());
        write_all(&path, &[mk("alpha"), mk("beta")]).unwrap();
        let all = read_all(dir.path());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "alpha");
        assert_eq!(all[1].title, "beta");
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = temp_vault();
        let path = file_path(dir.path());
        write_all(&path, &[mk("oldest"), mk("middle"), mk("newest")]).unwrap();
        let listed = list(dir.path()).items;
        assert_eq!(listed[0].title, "newest");
        assert_eq!(listed[2].title, "oldest");
    }

    #[test]
    fn unread_count_skips_read_entries() {
        let dir = temp_vault();
        let mut a = mk("a");
        let mut b = mk("b");
        let c = mk("c");
        a.read = true;
        b.read = false;
        write_all(&file_path(dir.path()), &[a, b, c]).unwrap();
        assert_eq!(unread_count(dir.path()), 2);
    }

    #[test]
    fn mark_read_flips_one_entry() {
        let dir = temp_vault();
        let a = mk("a");
        let id = a.id.clone();
        write_all(&file_path(dir.path()), &[a, mk("b")]).unwrap();
        assert_eq!(unread_count(dir.path()), 2);
        mark_read(dir.path(), &id).unwrap();
        assert_eq!(unread_count(dir.path()), 1);
        // Calling again is a no-op.
        mark_read(dir.path(), &id).unwrap();
        assert_eq!(unread_count(dir.path()), 1);
    }

    #[test]
    fn mark_all_read_returns_count() {
        let dir = temp_vault();
        write_all(&file_path(dir.path()), &[mk("a"), mk("b"), mk("c")]).unwrap();
        let touched = mark_all_read(dir.path()).unwrap();
        assert_eq!(touched, 3);
        assert_eq!(unread_count(dir.path()), 0);
        // Second call has nothing to do.
        let again = mark_all_read(dir.path()).unwrap();
        assert_eq!(again, 0);
    }

    #[test]
    fn delete_removes_one_entry() {
        let dir = temp_vault();
        let a = mk("a");
        let id = a.id.clone();
        write_all(&file_path(dir.path()), &[a, mk("b")]).unwrap();
        delete(dir.path(), &id).unwrap();
        let all = read_all(dir.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "b");
    }

    #[test]
    fn clear_removes_the_file() {
        let dir = temp_vault();
        write_all(&file_path(dir.path()), &[mk("a"), mk("b")]).unwrap();
        let n = clear(dir.path()).unwrap();
        assert_eq!(n, 2);
        assert!(!file_path(dir.path()).exists());
        // Idempotent on missing file.
        assert_eq!(clear(dir.path()).unwrap(), 0);
    }

    fn failure(title: &str, read: bool) -> Notification {
        Notification {
            kind: "error".into(),
            read,
            ..mk(title)
        }
    }

    /// P0-08: Recent activity's Mark these read and Clear these act on the
    /// ids it lists, and never take a failure still waiting in Review with
    /// them, even when its id is passed.
    #[test]
    fn scoped_mark_read_and_delete_leave_unread_failures() {
        let dir = temp_vault();
        let mut all: Vec<Notification> = (0..3)
            .map(|i| failure(&format!("fail {i}"), false))
            .collect();
        // Five others: three unread infos and two failures already read.
        all.extend((0..5).map(|i| {
            if i % 2 == 1 {
                failure(&format!("seen {i}"), true)
            } else {
                mk(&format!("info {i}"))
            }
        }));
        write_all(&file_path(dir.path()), &all).unwrap();
        let ids: Vec<String> = all.iter().map(|n| n.id.clone()).collect();
        assert_eq!(mark_read_many(dir.path(), &ids).unwrap(), 3);
        assert_eq!(unread_errors(dir.path()).unwrap().len(), 3);
        assert_eq!(delete_many(dir.path(), &ids).unwrap(), 5);
        assert_eq!(unread_errors(dir.path()).unwrap().len(), 3);
        assert_eq!(read_all(dir.path()).len(), 3);
        // An id it doesn't list is left alone.
        let other = mk("other");
        let mut kept = read_all(dir.path());
        kept.push(other.clone());
        write_all(&file_path(dir.path()), &kept).unwrap();
        assert_eq!(delete_many(dir.path(), &ids).unwrap(), 0);
        assert!(read_all(dir.path()).iter().any(|n| n.id == other.id));
    }

    /// P0-08: trimming drops read and non-failure entries first, so one
    /// unread failure outlives 600 newer notifications.
    #[test]
    fn an_unread_failure_outlives_600_newer_notifications() {
        let first = failure("old failure", false);
        let id = first.id.clone();
        let mut all = vec![first];
        for i in 0..600 {
            all.push(mk(&format!("n{i}")));
            trim(&mut all);
        }
        assert_eq!(all.len(), KEEP);
        assert!(all.iter().any(|n| n.id == id && !n.read));
    }

    /// P0-08: past the hard cap, the oldest unread failures fold into one
    /// "N older failures were trimmed" failure, which keeps its count.
    #[test]
    fn past_the_hard_cap_old_failures_fold_into_one_line() {
        let mut all = Vec::new();
        let mut newest = String::new();
        for i in 0..(HARD_CAP + 10) {
            let f = failure(&format!("f{i}"), false);
            newest = f.id.clone();
            all.push(f);
            trim(&mut all);
        }
        assert_eq!(all.len(), HARD_CAP);
        assert!(all.iter().any(|n| n.id == newest));
        let summaries: Vec<&Notification> = all
            .iter()
            .filter(|n| n.source.as_deref() == Some(TRIMMED_SOURCE))
            .collect();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "11 older failures were trimmed");
        assert_eq!(summaries[0].kind, "error");
        assert!(!summaries[0].read);
        // The summary keeps its id and date as later writes fold more in
        // (review finding: a Review row opened earlier must still name it).
        let (id, created) = (summaries[0].id.clone(), summaries[0].created.clone());
        for i in 0..3 {
            all.push(failure(&format!("g{i}"), false));
            trim(&mut all);
        }
        let summary = all
            .iter()
            .find(|n| n.source.as_deref() == Some(TRIMMED_SOURCE))
            .unwrap();
        assert_eq!(summary.title, "14 older failures were trimmed");
        assert_eq!((&summary.id, &summary.created), (&id, &created));
    }

    #[test]
    fn list_counts_the_whole_file() {
        let dir = temp_vault();
        let big: Vec<Notification> = (0..(READ_CAP + 50)).map(|i| mk(&format!("n{i}"))).collect();
        write_all(&file_path(dir.path()), &big).unwrap();
        let page = list(dir.path());
        assert_eq!(page.items.len(), READ_CAP);
        assert_eq!(page.total, READ_CAP + 50);
    }

    #[test]
    fn list_caps_at_read_cap() {
        let dir = temp_vault();
        let big: Vec<Notification> = (0..(READ_CAP + 50)).map(|i| mk(&format!("n{i}"))).collect();
        write_all(&file_path(dir.path()), &big).unwrap();
        assert_eq!(list(dir.path()).items.len(), READ_CAP);
    }

    /// Paperclip PR 22: the native banner is skipped only for the workflow
    /// open in Automations while the window is focused and visible; alerts
    /// that need attention always show.
    #[test]
    fn native_banner_is_suppressed_only_for_the_focused_foreground_workflow() {
        let scope = Some("workflow:morning-brief");
        let url = Some("workflow:morning-brief");
        assert!(foreground_suppressed_decision(
            scope,
            url,
            Some("morning-brief"),
            true,
            true
        ));
        // Not watching: unfocused, hidden, no scope, another workflow.
        assert!(!foreground_suppressed_decision(
            scope,
            url,
            Some("morning-brief"),
            false,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope,
            url,
            Some("morning-brief"),
            true,
            false
        ));
        assert!(!foreground_suppressed_decision(
            None,
            url,
            Some("morning-brief"),
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope,
            Some("workflow:other"),
            Some("other"),
            true,
            true
        ));
        // Never: permission reviews, failure streaks, budgets, run-guard.
        assert!(!foreground_suppressed_decision(
            scope,
            Some("workflow-permissions:morning-brief"),
            Some("morning-brief"),
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope,
            Some("workflow-failures:morning-brief"),
            Some("morning-brief"),
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope,
            url,
            Some("run-guard"),
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope,
            url,
            Some("budgets"),
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            Some("settings:usage"),
            Some("settings:usage"),
            None,
            true,
            true
        ));
        assert!(!foreground_suppressed_decision(
            scope, None, None, true, true
        ));
    }

    /// Paperclip PR 22: the decision above is actually consulted. This pins
    /// the wiring in the source: the native banner is gated on
    /// `foreground_suppressed`, and that helper hands the notification's own
    /// action URL and source to the decision, with the live scope and the
    /// main window's focus and visibility (read by the core host since M1
    /// PR 1; `create_through_a_host_*` below drives it for real).
    #[test]
    fn create_gates_the_native_banner_on_the_foreground_decision() {
        let whole = include_str!("notifications.rs");
        let source = &whole[..whole
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("test module")];
        let body = |needle: &str| -> &str {
            let start = source.find(needle).unwrap_or_else(|| panic!("{needle}"));
            let end = start + source[start..].find("\n}\n").expect("end of fn");
            &source[start..end]
        };
        let create = body(concat!("pub fn ", "create(\n"));
        let gate = create
            .find(concat!(
                "if input.notify_native && native_enabled() && ",
                "!foreground_suppressed(app, &n)"
            ))
            .expect("native banner gated");
        let shown = create
            .find(concat!("app.show_native(", "&n.title, &n.body)"))
            .expect("native banner");
        assert!(gate < shown, "{create}");
        // The in-app event is never gated.
        assert!(create[shown..].contains(concat!("app.emit(", "EVENT, &n)")));

        let helper = body(concat!(
            "fn ",
            "foreground_suppressed(app: &impl crate::core_host::CoreHost"
        ));
        assert!(helper.contains(concat!("FOREGROUND_", "SCOPE")));
        assert!(helper.contains(concat!("app.main_window()", ".unwrap_or((false, false))")));
        // The GUI host reads the main window as before.
        let host = include_str!("core_host.rs");
        let window = crate::source_pins::find(host, concat!("fn ", "main_window<R: Runtime>"))
            .map(|at| &host[at..])
            .expect("tauri host main_window");
        let window = &window[..window.find("\n    }\n").expect("end of fn")];
        assert!(window.contains(concat!("get_window(", "\"main\")")));
        assert!(window.contains(concat!("is_focused()", ".unwrap_or(false)")));
        assert!(window.contains(concat!("is_visible()", ".unwrap_or(false)")));
        let call = helper
            .find(concat!("foreground_suppressed_", "decision("))
            .expect("consults the decision");
        let args = &helper[call..];
        assert!(
            args.contains(concat!("n.action_url", ".as_deref()")),
            "{args}"
        );
        assert!(args.contains(concat!("n.source", ".as_deref()")), "{args}");
    }

    /// M1 PR 1: `create` through a core host with no Tauri app writes the
    /// record, emits one `notification-event` with the record's bytes, and
    /// shows a banner only when asked and not suppressed. The only test that
    /// sets the foreground scope; it puts it back.
    #[test]
    fn create_through_a_host_records_emits_and_gates_the_banner() {
        use crate::core_host::{Core, RecordingHost};
        let dir = tempfile::tempdir().unwrap();
        let input = |native: bool, url: Option<&str>| NotificationInput {
            title: "T".into(),
            body: "B".into(),
            kind: "info".into(),
            source: None,
            action_label: None,
            action_url: url.map(str::to_string),
            notify_native: native,
        };
        let watching = RecordingHost::with(None, Some((true, true)));
        let core = Core::new(watching.clone());

        let n = create(&core, dir.path(), input(true, None)).unwrap();
        assert_eq!(read_all(dir.path()).len(), 1);
        let events = watching.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, EVENT);
        assert_eq!(events[0].2, serde_json::to_string(&n).unwrap());
        assert_eq!(watching.banners(), vec![("T".to_string(), "B".to_string())]);

        create(&core, dir.path(), input(false, None)).unwrap();
        assert_eq!(watching.banners().len(), 1, "not asked for");

        set_foreground_scope(Some("workflow:w".into()));
        create(&core, dir.path(), input(true, Some("workflow:w"))).unwrap();
        assert_eq!(watching.banners().len(), 1, "watching that workflow");
        // No main window never suppresses (the GUI's `None => (false, false)`).
        let windowless = RecordingHost::default();
        create(&windowless, dir.path(), input(true, Some("workflow:w"))).unwrap();
        set_foreground_scope(None);
        assert_eq!(windowless.banners().len(), 1);
        assert_eq!(watching.of(EVENT).len(), 3);
        assert_eq!(read_all(dir.path()).len(), 4);
    }

    #[test]
    fn generate_id_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(generate_id()));
        }
    }
}
