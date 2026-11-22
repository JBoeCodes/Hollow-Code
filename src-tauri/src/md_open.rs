//! Open external markdown files in a lightweight preview window, with a
//! "save a copy into the vault" action.
//!
//! Hollow registers itself as a handler for `.md`/`.markdown` files
//! (`bundle.fileAssociations` in `tauri.conf.json` → `CFBundleDocumentTypes`
//! on macOS). When the OS hands us a file to open, `lib.rs` forwards the
//! `RunEvent::Opened` urls here and we spawn one decorated preview window per
//! file. The window renders the file (sanitized markdown) and offers a "Save
//! to vault" button that copies the file into `vault/imported/`.
//!
//! The feature is additive and self-contained: a managed [`MdOpenState`]
//! (window-label → source path) plus three commands. It reads the vault root
//! from the existing `AppState` but never mutates it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::commands::AppState;

/// Display content is capped so a giant file can't lock up the webview. The
/// copy-to-vault path is unaffected — it always copies the whole file.
const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

/// Markdown extensions we preview. Mirrors `bundle.fileAssociations` in
/// `tauri.conf.json`.
const MD_EXTS: &[&str] = &["md", "markdown", "mdown", "mkd"];

/// Per-run state for the preview windows. `paths` maps a runtime window label
/// (`md-preview-{n}`) to the file that window is showing; `counter` mints the
/// labels; `opened_a_file` lets `setup` skip auto-showing the dashboard when a
/// cold launch was triggered by opening a document.
#[derive(Default)]
pub struct MdOpenState {
    paths: Mutex<HashMap<String, PathBuf>>,
    counter: AtomicUsize,
    pub opened_a_file: AtomicBool,
}

impl MdOpenState {
    /// Drop a closed preview window's file mapping.
    pub fn forget(&self, label: &str) {
        if let Ok(mut map) = self.paths.lock() {
            map.remove(label);
        }
    }

    fn path_for(&self, label: &str) -> Option<PathBuf> {
        self.paths.lock().ok().and_then(|m| m.get(label).cloned())
    }
}

/// True if `path` has a markdown extension (case-insensitive).
pub fn is_markdown(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => MD_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// Spawn a decorated preview window for `path`. Records the label→path mapping
/// so the window's `take_preview_file` call can find its file, and flips the
/// `opened_a_file` flag. Best-effort: a build failure is logged, not fatal.
///
/// Must run on the main thread (window creation). The `RunEvent::Opened` path
/// is already on the main loop; the `open_markdown_preview` command hops there
/// via `run_on_main_thread`.
pub fn open_preview_window(app: &AppHandle, path: &Path) {
    let state = app.state::<MdOpenState>();
    state.opened_a_file.store(true, Ordering::SeqCst);

    let n = state.counter.fetch_add(1, Ordering::SeqCst);
    let label = format!("md-preview-{n}");

    if let Ok(mut map) = state.paths.lock() {
        map.insert(label.clone(), path.to_path_buf());
    }

    let title = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Markdown")
        .to_string();

    if let Err(e) = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App("index.html?window=md-preview".into()),
    )
    .title(&title)
    .inner_size(820.0, 880.0)
    .min_inner_size(420.0, 320.0)
    .center()
    .resizable(true)
    .focused(true)
    .build()
    {
        eprintln!("md-preview: failed to open window for {path:?}: {e}");
        state.forget(&label);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFile {
    /// File name (e.g. `README.md`).
    name: String,
    /// Absolute source path (display only).
    path: String,
    /// Markdown body (possibly truncated for display).
    content: String,
    /// True if the source already lives under the vault root.
    in_vault: bool,
    /// Vault-relative path when `in_vault` is true.
    vault_rel_path: Option<String>,
    /// True if `content` was truncated to the display cap.
    truncated: bool,
}

/// Read the file a preview window was opened with. Looked up by the window's
/// own label (the frontend passes `getCurrentWindow().label`).
#[tauri::command]
pub fn take_preview_file(
    label: String,
    state: State<'_, MdOpenState>,
    app_state: State<'_, AppState>,
) -> Result<PreviewFile, String> {
    let path = state
        .path_for(&label)
        .ok_or_else(|| format!("no file is associated with window {label}"))?;

    let (content, truncated) = read_capped(&path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled.md")
        .to_string();
    let (in_vault, vault_rel_path) = vault_location(&app_state.config.vault_path, &path);

    Ok(PreviewFile {
        name,
        path: path.to_string_lossy().to_string(),
        content,
        in_vault,
        vault_rel_path,
        truncated,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFile {
    /// Vault-relative path of the saved copy (e.g. `imported/notes.md`).
    vault_rel_path: String,
}

/// Copy the previewed file into `vault/imported/`, de-duplicating the name.
#[tauri::command]
pub fn save_preview_to_vault(
    label: String,
    state: State<'_, MdOpenState>,
    app_state: State<'_, AppState>,
) -> Result<SavedFile, String> {
    let src = state
        .path_for(&label)
        .ok_or_else(|| format!("no file is associated with window {label}"))?;
    if !src.is_file() {
        return Err(format!("source file no longer exists: {}", src.display()));
    }

    let vault = &app_state.config.vault_path;
    let imported = vault.join("imported");
    std::fs::create_dir_all(&imported).map_err(|e| format!("create imported/: {e}"))?;

    let base = src
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported.md")
        .to_string();
    let final_name = unique_dest_name(&imported, &base);

    // Defense in depth: keep the destination provably inside the vault.
    let rel = format!("imported/{final_name}");
    let dest = crate::vault::safe_join(vault, &rel)?;
    std::fs::copy(&src, &dest).map_err(|e| format!("copy into vault: {e}"))?;

    Ok(SavedFile {
        vault_rel_path: rel,
    })
}

/// Open an arbitrary markdown path in a preview window. Callable from the
/// renderer (a dev trigger today; the hook for a future in-app "open external
/// file"). The OS file-open path goes through `open_preview_window` directly.
#[tauri::command]
pub fn open_markdown_preview(path: String, app: AppHandle) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err(format!("not a file: {path}"));
    }
    let handle = app.clone();
    app.run_on_main_thread(move || open_preview_window(&handle, &p))
        .map_err(|e| format!("schedule preview window: {e}"))
}

// ---- helpers (unit-tested) ----------------------------------------------

/// Read up to the display cap. Returns `(content, truncated)`.
fn read_capped(path: &Path) -> Result<(String, bool), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut buf = Vec::new();
    // take(cap+1) so we can distinguish "exactly cap" from "more than cap".
    file.take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let truncated = buf.len() as u64 > MAX_PREVIEW_BYTES;
    if truncated {
        buf.truncate(MAX_PREVIEW_BYTES as usize);
    }
    let mut content = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        content.push_str("\n\n*…(truncated for preview — the full file is copied on save)*\n");
    }
    Ok((content, truncated))
}

/// `(in_vault, vault_rel_path)` for `src` relative to `vault`. Canonicalizes
/// both so symlinks / `..` can't dodge the containment check.
fn vault_location(vault: &Path, src: &Path) -> (bool, Option<String>) {
    let (cv, cs) = match (vault.canonicalize(), src.canonicalize()) {
        (Ok(cv), Ok(cs)) => (cv, cs),
        _ => return (false, None),
    };
    match cs.strip_prefix(&cv) {
        Ok(rel) => (true, Some(rel.to_string_lossy().replace('\\', "/"))),
        Err(_) => (false, None),
    }
}

/// A destination file name inside `dir` that doesn't collide. `notes.md` →
/// `notes.md` if free, else `notes (2).md`, `notes (3).md`, …
fn unique_dest_name(dir: &Path, file_name: &str) -> String {
    if !dir.join(file_name).exists() {
        return file_name.to_string();
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = path.extension().and_then(|s| s.to_str());
    for i in 2..10_000 {
        let candidate = match ext {
            Some(ext) => format!("{stem} ({i}).{ext}"),
            None => format!("{stem} ({i})"),
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    file_name.to_string() // absurd fallback; effectively unreachable
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn is_markdown_matches_known_exts() {
        for ok in ["a.md", "A.MD", "notes.markdown", "x.mdown", "y.mkd"] {
            assert!(is_markdown(Path::new(ok)), "{ok} should be markdown");
        }
        for no in ["a.txt", "b.pdf", "c", "d.mdx", "e.html"] {
            assert!(!is_markdown(Path::new(no)), "{no} should not be markdown");
        }
    }

    #[test]
    fn unique_dest_name_dedupes() {
        let dir = TempDir::new().unwrap();
        assert_eq!(unique_dest_name(dir.path(), "notes.md"), "notes.md");
        fs::write(dir.path().join("notes.md"), "x").unwrap();
        assert_eq!(unique_dest_name(dir.path(), "notes.md"), "notes (2).md");
        fs::write(dir.path().join("notes (2).md"), "x").unwrap();
        assert_eq!(unique_dest_name(dir.path(), "notes.md"), "notes (3).md");
        fs::write(dir.path().join("LICENSE"), "x").unwrap();
        assert_eq!(unique_dest_name(dir.path(), "LICENSE"), "LICENSE (2)");
    }

    #[test]
    fn vault_location_detects_containment() {
        let vault = TempDir::new().unwrap();
        fs::create_dir_all(vault.path().join("notes")).unwrap();
        let inside = vault.path().join("notes/in.md");
        fs::write(&inside, "x").unwrap();
        let (in_vault, rel) = vault_location(vault.path(), &inside);
        assert!(in_vault);
        assert_eq!(rel.as_deref(), Some("notes/in.md"));

        let outside = TempDir::new().unwrap();
        let out = outside.path().join("out.md");
        fs::write(&out, "x").unwrap();
        let (in_vault2, rel2) = vault_location(vault.path(), &out);
        assert!(!in_vault2);
        assert!(rel2.is_none());
    }

    #[test]
    fn read_capped_truncates_large_files() {
        let dir = TempDir::new().unwrap();
        let big = dir.path().join("big.md");
        fs::write(&big, "a".repeat((MAX_PREVIEW_BYTES as usize) + 1000)).unwrap();
        let (out, truncated) = read_capped(&big).unwrap();
        assert!(truncated);
        assert!(out.contains("truncated"));

        let small = dir.path().join("small.md");
        fs::write(&small, "# hi").unwrap();
        let (out2, truncated2) = read_capped(&small).unwrap();
        assert_eq!(out2, "# hi");
        assert!(!truncated2);
    }
}
