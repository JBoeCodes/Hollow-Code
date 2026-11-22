//! The macOS menu bar and copyable build identity (audit H14).
//!
//! Custom items other than Quit emit `app-menu` with their id; the webview runs the same
//! action its shortcut, rail button or palette row runs (`src/actionDefs.ts`,
//! dispatched by `src/actionRun.ts` through `src/appMenu.ts`),
//! including the rule that no global action fires behind a modal. The
//! standard Edit items stay: without them copy/paste stops working in the
//! WebView on macOS.
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, Runtime,
};

pub const MENU_EVENT: &str = "app-menu";

/// Custom items: (id, label, accelerator). Accelerators mirror the webview's
/// existing global shortcuts (App.tsx), so a menu key and a typed key do the
/// same thing.
pub const ITEMS: &[(&str, &str, Option<&str>)] = &[
    ("settings", "Settings…", Some("CmdOrCtrl+,")),
    ("new-conversation", "New Conversation", Some("CmdOrCtrl+N")),
    ("new-quick-chat", "New Chat", None),
    ("new-workspace", "New Project", None),
    ("new-note", "New Note", Some("CmdOrCtrl+Shift+N")),
    ("new-task", "New Task…", None),
    ("search", "Search…", Some("CmdOrCtrl+K")),
    ("toggle-sidebar", "Toggle Sidebar", Some("CmdOrCtrl+B")),
    (
        "toggle-side-pane",
        "Toggle Preview Pane",
        Some("CmdOrCtrl+\\"),
    ),
    ("main-drawer", "COO", Some("CmdOrCtrl+Shift+M")),
    ("stop-run", "Stop Running Turn", None),
    // t3code parity 4.6: no chord (every browser's Reopen closed tab is the
    // macOS-reserved-by-habit Cmd+Shift+T, which Hollow keeps off).
    ("reopen-closed-view", "Reopen Closed View", None),
    // Edit → Undo Chat Action (3.1): no accelerator, so ⌘Z stays the text
    // field's own; greyed out unless a Settle, Snooze, Pin or Wake can undo.
    ("undo-chat-action", "Undo Chat Action", None),
    // Push-to-talk (overnight 10-11 B7): not a macOS chord.
    ("dictate", "Dictate", Some("CmdOrCtrl+Shift+D")),
    // Work until L1 (11-12); the id stays for saved keys.
    ("go-work", "Projects", Some("CmdOrCtrl+1")),
    ("go-tasks", "Tasks", Some("CmdOrCtrl+2")),
    ("go-library", "Library", Some("CmdOrCtrl+3")),
    ("go-automations", "Automations", Some("CmdOrCtrl+4")),
    // Review (P1-14): the next free digit, so the older ones keep their pages.
    ("go-review", "Needs You", Some("CmdOrCtrl+5")),
    ("go-back", "Back", Some("CmdOrCtrl+[")),
    ("go-forward", "Forward", Some("CmdOrCtrl+]")),
    (
        "prev-conversation",
        "Previous Conversation",
        Some("CmdOrCtrl+Shift+["),
    ),
    (
        "next-conversation",
        "Next Conversation",
        Some("CmdOrCtrl+Shift+]"),
    ),
    ("shortcuts", "Keyboard Shortcuts", None),
    ("copy-build", "Copy Build Identity", None),
    ("diagnostics", "Diagnostics", None),
];

fn item<R: Runtime>(app: &AppHandle<R>, id: &str) -> tauri::Result<MenuItem<R>> {
    let (id, label, accel) = ITEMS
        .iter()
        .find(|(i, _, _)| *i == id)
        .copied()
        .unwrap_or((id, id, None));
    MenuItem::with_id(app, id, label, true, accel)
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let identity = build_identity();
    let about = AboutMetadata {
        name: Some("Hollow".into()),
        version: Some(identity.version.clone()),
        short_version: Some(identity.short_commit.clone()),
        comments: Some(identity.text.clone()),
        ..Default::default()
    };
    let sep = || PredefinedMenuItem::separator(app);
    let hollow = Submenu::with_items(
        app,
        "Hollow",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Hollow"), Some(about))?,
            &sep()?,
            &item(app, "settings")?,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            // Cocoa's predefined Quit bypasses ExitRequested. Keep the
            // standard Quit binding but route it through our run guard.
            &MenuItem::with_id(app, "app-quit", "Quit Hollow", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item(app, "new-conversation")?,
            &item(app, "new-quick-chat")?,
            &item(app, "new-workspace")?,
            &item(app, "new-note")?,
            &item(app, "new-task")?,
            &sep()?,
            &item(app, "search")?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &item(app, "undo-chat-action")?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
            &sep()?,
            &item(app, "dictate")?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &item(app, "toggle-sidebar")?,
            &item(app, "toggle-side-pane")?,
            &item(app, "reopen-closed-view")?,
            &item(app, "main-drawer")?,
            &sep()?,
            &item(app, "stop-run")?,
            &sep()?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let go = Submenu::with_items(
        app,
        "Go",
        true,
        &[
            &item(app, "go-work")?,
            &item(app, "go-review")?,
            &item(app, "go-tasks")?,
            &item(app, "go-library")?,
            &item(app, "go-automations")?,
            &sep()?,
            &item(app, "prev-conversation")?,
            &item(app, "next-conversation")?,
            &sep()?,
            &item(app, "go-back")?,
            &item(app, "go-forward")?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &item(app, "shortcuts")?,
            &item(app, "diagnostics")?,
            &sep()?,
            &item(app, "copy-build")?,
        ],
    )?;
    Menu::with_items(app, &[&hollow, &file, &edit, &view, &go, &window, &help])
}

/// Forward a custom item to the webview, bringing the main window forward
/// first (a menu can be used while the window is hidden to the tray).
pub fn handle<R: Runtime>(app: &AppHandle<R>, id: &str) {
    // Distinct from the tray's id: both menu callbacks see tray events.
    if id == "app-quit" {
        app.exit(0);
        return;
    }
    if !ITEMS.iter().any(|(i, _, _)| *i == id) {
        return;
    }
    crate::lifecycle::bring_main_forward(app);
    let _ = app.emit(MENU_EVENT, id.to_string());
}

/// Items that grey out when they can't act (P1-49): Stop Running Turn with
/// nothing running, Back and Forward at the ends of the history. The
/// webview tells the menu (`src/menuState.ts`).
pub const FOLLOWS_STATE: &[&str] = &["stop-run", "go-back", "go-forward", "undo-chat-action"];

/// Enable or grey out [`FOLLOWS_STATE`] items; any other id is refused.
/// Async: menu calls wait on the main thread.
#[tauri::command]
pub async fn set_menu_enabled(
    app: AppHandle,
    items: std::collections::BTreeMap<String, bool>,
) -> Result<(), String> {
    for (id, enabled) in &items {
        if !FOLLOWS_STATE.contains(&id.as_str()) {
            return Err(format!(
                "`{id}` doesn't follow state; only {}.",
                FOLLOWS_STATE.join(", ")
            ));
        }
        if let Some(item) = crate::shortcuts::menu_item(&app, id) {
            item.set_enabled(*enabled).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The build this binary was compiled from: version, commit, whether the
/// checkout had uncommitted changes, when and the source fingerprint the
/// installer verifies. Embedded at compile time, so it describes this binary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildIdentity {
    pub version: String,
    pub commit: String,
    pub short_commit: String,
    pub dirty: bool,
    pub generated_at: String,
    pub source_fingerprint: String,
    pub input_count: u64,
    /// One copyable line.
    pub text: String,
}

impl BuildIdentity {
    /// Native window/tray label; full copyable provenance remains `text`.
    pub fn short_text(&self) -> String {
        format!(
            "Hollow {} · {}{}",
            self.version,
            self.short_commit,
            if self.dirty {
                " (uncommitted changes)"
            } else {
                ""
            }
        )
    }
}

const EMBEDDED: &str = include_str!("../build-provenance.json");

pub fn parse_identity(json: &str, fallback_version: &str) -> BuildIdentity {
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase", default)]
    struct Raw {
        version: String,
        commit: String,
        dirty: bool,
        generated_at: String,
        source_fingerprint: String,
        input_count: u64,
    }
    let raw: Raw = serde_json::from_str(json).unwrap_or_default();
    let version = if raw.version.is_empty() {
        fallback_version.to_string()
    } else {
        raw.version
    };
    let short: String = raw.commit.chars().take(7).collect();
    let short_commit = if short.is_empty() {
        "unknown".to_string()
    } else {
        short
    };
    let fp: String = raw.source_fingerprint.chars().take(12).collect();
    let text = format!(
        "Hollow {version} · commit {short_commit}{} · built {} · source {}",
        if raw.dirty {
            " (uncommitted changes)"
        } else {
            ""
        },
        if raw.generated_at.is_empty() {
            "unknown"
        } else {
            &raw.generated_at
        },
        if fp.is_empty() {
            "unknown".to_string()
        } else {
            format!("{fp}…")
        },
    );
    BuildIdentity {
        version,
        commit: raw.commit,
        short_commit,
        dirty: raw.dirty,
        generated_at: raw.generated_at,
        source_fingerprint: raw.source_fingerprint,
        input_count: raw.input_count,
        text,
    }
}

pub fn build_identity() -> BuildIdentity {
    parse_identity(EMBEDDED, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_identity_has_one_native_title_and_a_distinct_tray_copy_route() {
        let source = include_str!("lib.rs");
        use crate::source_pins::{contains, count};
        assert!(contains(
            source,
            "let identity_title = identity.short_text()"
        ));
        assert!(contains(source, "window.set_title(&identity_title)"));
        assert!(contains(
            source,
            "MenuItem::with_id(app, \"tray-build-identity\", &identity_title, false, None::<&str>)"
        ));
        assert!(contains(
            source,
            "\"tray-copy-build\" => app_menu::handle(app, \"copy-build\")"
        ));
        assert_eq!(count(source, "app_menu::handle(app, \"copy-build\")"), 1);
        assert!(!ITEMS.iter().any(|(id, _, _)| *id == "tray-copy-build"));
    }

    #[test]
    fn quit_menu_uses_the_exit_requested_path_not_cocoa_terminate() {
        // A native wiring contract: Cocoa's predefined Quit bypasses Tauri's
        // ExitRequested callback. Browser mocks cannot exercise that path.
        let source = include_str!("app_menu.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!source.contains("PredefinedMenuItem::quit"));
        assert!(source.contains("Some(\"CmdOrCtrl+Q\")"));
        assert!(source.contains("if id == \"app-quit\""));
        assert!(source.contains("app.exit(0)"));
    }

    #[test]
    fn identity_reads_provenance_and_formats_one_copyable_line() {
        let id = parse_identity(
            r#"{"sourceFingerprint":"6975a31ae3360896feed","commit":"c0c9fc582fe368119d0d","generatedAt":"2026-09-27T22:00:00Z","inputCount":437,"dirty":false,"version":"0.5.3"}"#,
            "0.0.0",
        );
        assert_eq!(id.short_commit, "c0c9fc5");
        assert_eq!(id.short_text(), "Hollow 0.5.3 · c0c9fc5");
        assert_eq!(
            id.text,
            "Hollow 0.5.3 · commit c0c9fc5 · built 2026-09-27T22:00:00Z · source 6975a31ae336…"
        );
        let dirty = parse_identity(r#"{"commit":"abc","dirty":true}"#, "9.9.9");
        assert!(dirty
            .text
            .starts_with("Hollow 9.9.9 · commit abc (uncommitted changes)"));
        assert_eq!(
            dirty.short_text(),
            "Hollow 9.9.9 · abc (uncommitted changes)"
        );
        assert_eq!(parse_identity("not json", "1.0.0").short_commit, "unknown");
        assert_eq!(
            parse_identity("not json", "1.0.0").short_text(),
            "Hollow 1.0.0 · unknown"
        );
    }

    #[test]
    fn the_embedded_identity_parses() {
        assert!(!build_identity().version.is_empty());
    }

    #[test]
    fn every_item_has_a_frontend_action() {
        // Each item is a shared action's `menu` (src/actionDefs.ts), which
        // the frontend's one dispatcher runs (src/actionRun.ts).
        let defs = include_str!("../../src/actionDefs.ts");
        for (id, _, _) in ITEMS {
            assert!(
                defs.contains(&format!("menu: '{id}'")),
                "src/actionDefs.ts declares no action for menu item {id}"
            );
        }
    }
}
