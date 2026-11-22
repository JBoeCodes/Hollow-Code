//! What an Open may do with a path it was handed (overnight H4).
//!
//! `chat_preview_open` (Open original on a workspace source, the chat
//! preview's Open, Results' Open in its app) passes a path to Launch Services,
//! which launches an app, an installer or a script as readily as it opens a
//! document. The frontend already reveals by extension (`opensAsDocument` in
//! `src/sourceAuthority.ts`), but a name says nothing about what it points at:
//! a symlink called `report.pdf` can lead to `Calculator.app` or to an
//! executable script. [`open_or_reveal`] decides on the canonical target
//! instead, so anything that would run is shown in Finder, never opened.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Extensions Launch Services runs rather than shows: apps, installers,
/// scripts, web pages (they run their scripts in the browser) and web or file
/// locations. Mirrors `RUNNABLE` in `src/sourceAuthority.ts`; a test holds
/// the two together.
pub const RUNNABLE_EXTENSIONS: &[&str] = &[
    "html",
    "htm",
    "xhtml",
    "shtml",
    "app",
    "command",
    "sh",
    "zsh",
    "bash",
    "tool",
    "terminal",
    "pkg",
    "mpkg",
    "dmg",
    "workflow",
    "scpt",
    "scptd",
    "applescript",
    "jar",
    "exe",
    "py",
    "pl",
    "rb",
    "js",
    "webloc",
    "inetloc",
    "fileloc",
    "shortcut",
    "action",
    "prefpane",
    "kext",
    "plugin",
    "bundle",
    "osax",
    "definition",
    "url",
    "webarchive",
    "svg",
    "xht",
    "mht",
    "mhtml",
    "afploc",
    "ftploc",
    "mobileconfig",
];

/// Extensions an executable file may still be opened with: types macOS opens
/// in a viewer or editor and never runs. Launch Services types a file with
/// any execute bit and an extension it doesn't know as a Unix executable,
/// which Terminal runs, `#!` line or not (a header-less one goes to the
/// shell), so every other executable is revealed (R1). The list exists
/// because exFAT, SMB and some unzipped files report every bit set, and a
/// PDF with +x is still a PDF. Every entry must be a type macOS itself
/// declares, not one an installed app adds: on a Mac without that app the
/// extension is unknown again, and the file runs (review: `mkv` is declared
/// only by third-party players; `key`, `pages` and `numbers` only while iWork
/// is installed). Left off too, for their +x copies only (a 644 copy still
/// opens): XML (it may open in a browser, which runs an XHTML or XSLT page's
/// scripts) and `doc`, `xls`, `ppt` and the OpenDocument formats (macros).
/// Web pages and SVG are runnable ([`RUNNABLE_EXTENSIONS`]); a test keeps the
/// lists apart. On exFAT or SMB, where every file is +x, any type not listed
/// here is revealed rather than opened.
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "md", "markdown", "rtf", "csv", "tsv", "json", "yaml", "yml", "log", "docx",
    "xlsx", "pptx", "png", "jpg", "jpeg", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp",
    "ico", "mp3", "m4a", "wav", "aiff", "aif", "flac", "mp4", "m4v", "mov", "avi", "zip",
];

/// What to do with a path asked to open, and the canonical path to do it to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAction {
    /// A document: open it in its default app.
    Open(PathBuf),
    /// Not a regular file (a folder or bundle, a pipe, a device), a Finder
    /// alias, an executable that isn't a known document type, or a runnable
    /// extension: opening it would run something, so show it in Finder.
    Reveal(PathBuf),
}

/// Resolve `path` through every symlink and decide on what it really is.
///
/// The returned path is the canonical one, and the caller acts on it rather
/// than on `path`, so a link in the path given, repointed after this check,
/// can't change what is opened. `open` still resolves the canonical path
/// again by name, so whatever is put at that name (or behind a folder on
/// it) in the moment before is what opens: this is defence in depth, not a
/// sandbox. A path that doesn't resolve (missing, or a dangling link) is an
/// error. No file content is read.
///
/// `canonicalize` doesn't follow a Finder alias (a regular file with
/// bookmark data that `open` resolves and launches), so an alias is revealed
/// too (review of H4). An executable is revealed unless its extension is in
/// [`DOCUMENT_EXTENSIONS`] (any case); a file with no extension, `payload.`
/// or `notes.34` has none of those (R1).
pub fn open_or_reveal(path: &Path) -> io::Result<OpenAction> {
    let target = fs::canonicalize(path)?;
    let meta = fs::metadata(&target)?;
    let runnable_executable =
        is_executable(&meta) && !has_extension_in(&target, DOCUMENT_EXTENSIONS);
    if !meta.is_file()
        || runnable_executable
        || has_extension_in(&target, RUNNABLE_EXTENSIONS)
        || is_finder_alias(&target)
        || has_open_with_binding(&target)
    {
        Ok(OpenAction::Reveal(target))
    } else {
        Ok(OpenAction::Open(target))
    }
}

/// Any execute bit: Launch Services runs such a file in Terminal whatever
/// its name.
#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// A Finder alias: `com.apple.FinderInfo` with the `kIsAlias` flag (bit 15
/// of the big-endian Finder flags at bytes 8–9) or the old `alis` file type.
#[cfg(target_os = "macos")]
fn is_finder_alias(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (Ok(path), Ok(name)) = (
        CString::new(path.as_os_str().as_bytes()),
        CString::new("com.apple.FinderInfo"),
    ) else {
        return false;
    };
    let mut info = [0u8; 32];
    // SAFETY: both strings are NUL-terminated and `info` is a writable
    // buffer of the length passed; position 0 and options 0 read the whole
    // attribute, following links (the path is already canonical).
    let read = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            info.as_mut_ptr().cast(),
            info.len(),
            0,
            0,
        )
    };
    read >= 10 && (u16::from_be_bytes([info[8], info[9]]) & 0x8000 != 0 || &info[..4] == b"alis")
}

#[cfg(not(target_os = "macos"))]
fn is_finder_alias(_path: &Path) -> bool {
    false
}

/// A per-file "Open with" binding (Get Info → Open with, the
/// `com.apple.LaunchServices.OpenWith` attribute) or a resource fork (where
/// the old `usro` binding lives): `open` honours it over the extension, so
/// `report.pdf` bound to Terminal would run. Revealed whatever the name (R1
/// review). Only the attributes' presence is read, never their content.
#[cfg(target_os = "macos")]
fn has_open_with_binding(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    [
        "com.apple.LaunchServices.OpenWith",
        "com.apple.ResourceFork",
    ]
    .iter()
    .any(|name| {
        let Ok(name) = CString::new(*name) else {
            return false;
        };
        // SAFETY: NUL-terminated strings; a null buffer of length 0 asks
        // only for the attribute's size (-1 when it is absent).
        let size =
            unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
        size >= 0
    })
}

#[cfg(not(target_os = "macos"))]
fn has_open_with_binding(_path: &Path) -> bool {
    false
}

/// Whether the path's extension is one of `list`, in any case.
fn has_extension_in(path: &Path, list: &[&str]) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    list.iter().any(|known| known.eq_ignore_ascii_case(ext))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use tempfile::TempDir;

    /// A file at `dir/name` with `mode`, returned canonical (the temp dir
    /// may sit behind a link, as `/var` and `/tmp` do on macOS).
    fn file(dir: &TempDir, name: &str, mode: u32) -> PathBuf {
        file_with(dir, name, mode, b"%PDF-1.4\n")
    }

    /// As [`file`], holding `contents`.
    fn file_with(dir: &TempDir, name: &str, mode: u32, contents: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        fs::canonicalize(path).unwrap()
    }

    fn link(dir: &TempDir, name: &str, to: &Path) -> PathBuf {
        let path = dir.path().join(name);
        symlink(to, &path).unwrap();
        path
    }

    #[test]
    fn a_plain_document_opens() {
        let dir = TempDir::new().unwrap();
        let pdf = file(&dir, "plain.pdf", 0o644);
        assert_eq!(open_or_reveal(&pdf).unwrap(), OpenAction::Open(pdf));
    }

    #[test]
    fn a_link_to_a_document_opens_the_document_it_points_at() {
        let dir = TempDir::new().unwrap();
        let pdf = file(&dir, "plain.pdf", 0o644);
        let linked = link(&dir, "report.pdf", &pdf);
        assert_eq!(open_or_reveal(&linked).unwrap(), OpenAction::Open(pdf));
    }

    #[test]
    fn a_document_named_link_to_an_app_bundle_is_revealed() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("X.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let linked = link(&dir, "report.pdf", &dir.path().join("X.app/"));
        assert_eq!(
            open_or_reveal(&linked).unwrap(),
            OpenAction::Reveal(fs::canonicalize(app).unwrap())
        );
    }

    #[test]
    fn a_document_named_link_to_an_executable_is_revealed_by_its_mode() {
        let dir = TempDir::new().unwrap();
        // No extension, so only the execute bit can give it away.
        let script = file(&dir, "payload", 0o755);
        let linked = link(&dir, "notes.pdf", &script);
        assert_eq!(
            open_or_reveal(&linked).unwrap(),
            OpenAction::Reveal(script.clone())
        );
        // The same file without the bit is only a document.
        fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(open_or_reveal(&linked).unwrap(), OpenAction::Open(script));
    }

    #[test]
    fn a_trailing_dot_is_no_extension_for_the_execute_bit() {
        let dir = TempDir::new().unwrap();
        for name in ["payload.", "payload.\u{e9}"] {
            let script = file(&dir, name, 0o755);
            let linked = link(&dir, &format!("notes-{}.pdf", name.len()), &script);
            assert_eq!(
                open_or_reveal(&linked).unwrap(),
                OpenAction::Reveal(script),
                "{name}"
            );
        }
    }

    #[test]
    fn an_execute_bit_on_a_document_extension_still_opens() {
        let dir = TempDir::new().unwrap();
        // exFAT and SMB report every bit set; a PDF with +x is still a PDF,
        // in any case.
        for name in ["scan.pdf", "scan.PDF"] {
            let pdf = file(&dir, name, 0o777);
            assert_eq!(
                open_or_reveal(&pdf).unwrap(),
                OpenAction::Open(pdf),
                "{name}"
            );
        }
        let notes = file_with(&dir, "notes.txt", 0o755, b"hello\n");
        assert_eq!(open_or_reveal(&notes).unwrap(), OpenAction::Open(notes));
    }

    #[test]
    fn an_executable_with_an_unknown_extension_is_revealed_whatever_it_holds() {
        // Launch Services types it as a Unix executable and Terminal runs
        // it: a `#!` line, a Mach-O image, or plain shell with no header
        // (R1).
        let contents: [(&str, &[u8]); 3] = [
            ("payload.34", b"echo hi\n"),
            ("payload.34", b"#!/bin/sh\necho hi\n"),
            ("tool.dat", &[0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]),
        ];
        for (name, body) in contents {
            let dir = TempDir::new().unwrap();
            let target = file_with(&dir, name, 0o755, body);
            assert_eq!(
                open_or_reveal(&target).unwrap(),
                OpenAction::Reveal(target.clone()),
                "{name} {body:02x?}"
            );
            // The same file without the bit is only a document.
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(open_or_reveal(&target).unwrap(), OpenAction::Open(target));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_finder_alias_named_like_a_document_is_revealed() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = TempDir::new().unwrap();
        let alias = file(&dir, "report.pdf", 0o644);
        assert_eq!(
            open_or_reveal(&alias).unwrap(),
            OpenAction::Open(alias.clone())
        );
        let mut info = [0u8; 32];
        info[8] = 0x80; // kIsAlias
        let path = CString::new(alias.as_os_str().as_bytes()).unwrap();
        let name = CString::new("com.apple.FinderInfo").unwrap();
        // SAFETY: NUL-terminated strings and a buffer of the length passed.
        let set = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                info.as_ptr().cast(),
                info.len(),
                0,
                0,
            )
        };
        assert_eq!(set, 0, "set FinderInfo");
        assert_eq!(open_or_reveal(&alias).unwrap(), OpenAction::Reveal(alias));
    }

    /// R1 review: a per-file "Open with" binding beats the extension, so a
    /// bound file is revealed whatever its name or mode.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_bound_to_an_app_of_its_own_is_revealed() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = TempDir::new().unwrap();
        for attr in [
            "com.apple.LaunchServices.OpenWith",
            "com.apple.ResourceFork",
        ] {
            let bound = file(&dir, &format!("report-{}.pdf", attr.len()), 0o644);
            assert_eq!(
                open_or_reveal(&bound).unwrap(),
                OpenAction::Open(bound.clone())
            );
            let path = CString::new(bound.as_os_str().as_bytes()).unwrap();
            let name = CString::new(attr).unwrap();
            let value = b"x";
            // SAFETY: NUL-terminated strings and a buffer of the length passed.
            let set = unsafe {
                libc::setxattr(
                    path.as_ptr(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            };
            assert_eq!(set, 0, "set {attr}");
            assert_eq!(
                open_or_reveal(&bound).unwrap(),
                OpenAction::Reveal(bound),
                "{attr}"
            );
        }
    }

    #[test]
    fn a_runnable_extension_behind_a_link_is_revealed_in_any_case() {
        let dir = TempDir::new().unwrap();
        for name in ["Run.command", "Setup.PKG", "page.html"] {
            let target = file(&dir, name, 0o644);
            let linked = link(&dir, &format!("{name}.pdf"), &target);
            assert_eq!(
                open_or_reveal(&linked).unwrap(),
                OpenAction::Reveal(target),
                "{name}"
            );
        }
    }

    #[test]
    fn a_folder_is_revealed() {
        let dir = TempDir::new().unwrap();
        let folder = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            open_or_reveal(dir.path()).unwrap(),
            OpenAction::Reveal(folder)
        );
    }

    #[test]
    fn a_dangling_link_is_an_error() {
        let dir = TempDir::new().unwrap();
        let linked = link(&dir, "gone.pdf", &dir.path().join("missing.pdf"));
        assert!(open_or_reveal(&linked).is_err());
    }

    #[test]
    fn the_document_extensions_are_lowercase_and_never_runnable() {
        // The backend's runnable list equals the frontend's (the test
        // below), so this keeps the allowlist apart from both.
        let documents: BTreeSet<&str> = DOCUMENT_EXTENSIONS.iter().copied().collect();
        assert_eq!(documents.len(), DOCUMENT_EXTENSIONS.len(), "duplicates");
        for ext in &documents {
            assert_eq!(*ext, ext.to_ascii_lowercase(), "{ext}");
        }
        let runnable: BTreeSet<&str> = RUNNABLE_EXTENSIONS.iter().copied().collect();
        let overlap: Vec<&&str> = documents.intersection(&runnable).collect();
        assert!(overlap.is_empty(), "{overlap:?}");
    }

    #[test]
    fn the_runnable_extensions_match_the_frontend() {
        let ts = include_str!("../../src/sourceAuthority.ts");
        let list = ts
            .split("const RUNNABLE = new Set([")
            .nth(1)
            .and_then(|rest| rest.split("])").next())
            .expect("RUNNABLE in src/sourceAuthority.ts");
        let frontend: BTreeSet<&str> = list
            .split(',')
            .map(|ext| ext.trim().trim_matches('\''))
            .filter(|ext| !ext.is_empty())
            .collect();
        let backend: BTreeSet<&str> = RUNNABLE_EXTENSIONS.iter().copied().collect();
        assert_eq!(backend, frontend);
    }
}
