//! "Open in" for the chat card's header (t3code parity 5.1, D1).
//!
//! A chat's folder opens in Finder or in a code editor installed on this Mac.
//! The list comes from a fixed table of well-known editors (checked by their
//! `.app` bundle in `/Applications` and `~/Applications`), never from free
//! text: the id the window sends picks a table row, and only that row's app
//! name reaches `open -a`. Provider-neutral, no telemetry, no shell: the path
//! and the app name are separate argv entries.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// One choice in the Open in menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenInTarget {
    pub id: &'static str,
    pub label: &'static str,
}

/// (id, menu label, app bundle name without `.app`). Order is the menu's.
const EDITORS: &[(&str, &str, &str)] = &[
    ("vscode", "Visual Studio Code", "Visual Studio Code"),
    ("cursor", "Cursor", "Cursor"),
    ("zed", "Zed", "Zed"),
    ("windsurf", "Windsurf", "Windsurf"),
    ("sublime", "Sublime Text", "Sublime Text"),
    ("xcode", "Xcode", "Xcode"),
];

const FINDER: OpenInTarget = OpenInTarget {
    id: "finder",
    label: "Finder",
};

/// Finder first, then each table editor `installed` says is on this Mac.
pub fn targets_with(installed: impl Fn(&str) -> bool) -> Vec<OpenInTarget> {
    let mut out = vec![FINDER];
    for (id, label, app) in EDITORS {
        if installed(app) {
            out.push(OpenInTarget { id, label });
        }
    }
    out
}

/// The app bundle name for a table editor id; `None` for Finder or anything else.
pub fn editor_app(id: &str) -> Option<&'static str> {
    EDITORS
        .iter()
        .find(|(editor, _, _)| *editor == id)
        .map(|(_, _, app)| *app)
}

/// The installed bundle of a table app: `/Applications` first, then `~/Applications`.
fn installed_bundle(app: &str) -> Option<PathBuf> {
    let name = format!("{app}.app");
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
        .into_iter()
        .map(|r| r.join(&name))
        .find(|p| p.is_dir())
}

/// The Open in menu's rows on this Mac (Finder first).
#[tauri::command]
pub fn open_in_targets() -> Vec<OpenInTarget> {
    #[cfg(target_os = "macos")]
    {
        targets_with(|app| installed_bundle(app).is_some())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = installed_bundle;
        vec![FINDER]
    }
}

/// Open `path` (an existing absolute path, normally the chat's folder) in the
/// table row `target` names.
#[tauri::command]
pub fn open_in_open(path: String, target: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.is_absolute() {
        return Err("path must be absolute".into());
    }
    if !p.exists() {
        return Err(format!("{path} doesn't exist"));
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        if target == "finder" {
            // A plain folder opens as a Finder window. A file, or a folder with an
            // extension (an app, a package), is only revealed: `open` would launch it.
            if p.is_dir() && p.extension().is_none() {
                cmd.arg(&path);
            } else {
                cmd.arg("-R").arg(&path);
            }
        } else {
            let app = editor_app(&target).ok_or_else(|| format!("unknown target {target}"))?;
            // The verified bundle's path, so Launch Services can't pick another app by name.
            let bundle = installed_bundle(app).ok_or_else(|| format!("{app} isn't installed"))?;
            cmd.arg("-a").arg(bundle).arg(&path);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("couldn't run open: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (p, target);
        Err("Open in is available on macOS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_is_always_first_and_only_installed_editors_follow() {
        let none = targets_with(|_| false);
        assert_eq!(none, vec![FINDER]);
        let some = targets_with(|app| app == "Zed" || app == "Cursor");
        let ids: Vec<_> = some.iter().map(|t| t.id).collect();
        assert_eq!(ids, ["finder", "cursor", "zed"]);
    }

    #[test]
    fn only_table_ids_name_an_app() {
        assert_eq!(editor_app("vscode"), Some("Visual Studio Code"));
        assert_eq!(editor_app("finder"), None);
        assert_eq!(editor_app("Terminal"), None);
        assert_eq!(editor_app("-a"), None);
    }

    #[test]
    fn relative_and_missing_paths_are_refused() {
        assert!(open_in_open("notes".into(), "finder".into()).is_err());
        assert!(open_in_open("/no/such/folder-5-1".into(), "finder".into()).is_err());
    }

    #[test]
    fn ids_are_unique_and_labels_have_no_dashes() {
        let mut ids: Vec<_> = EDITORS.iter().map(|e| e.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), EDITORS.len());
        for (_, label, _) in EDITORS {
            assert!(!label.contains('\u{2014}') && !label.contains('\u{2013}'));
        }
    }
}
