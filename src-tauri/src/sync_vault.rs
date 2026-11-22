//! The vault root of the context sync engine (CX6b): the whole vault, text
//! only, committed locally.
//!
//! **What it tracks.** Every file in the vault that passes [`path_allowed`]
//! and is text (a new file's first 8000 bytes are sniffed for a NUL byte
//! before it is staged; every staged blob is then checked whole) of at most
//! [`MAX_BYTES`]. Out: the personal root's folders (`me/`, `memory/`,
//! `learnings/`, `home/`: the personal repo owns them, so nothing is tracked
//! twice), `tasks/` (the Linear mirror each Mac pulls itself), every dot
//! file and folder at the vault's top (`.hollow-history/`, `.trash/`,
//! `.archive/`, the notification and health logs, the review queue, the
//! tracker ledger: history and per-Mac state), dependency and build folders,
//! write-once media folders (V4 copies those), `*.jsonl` logs, binary
//! extensions, and the CX1 rules' data file types (`file.data_type`) and
//! secret file names (`secret.key_file`). A file over the ceiling or with a
//! NUL byte stays out quietly; one already tracked that grows past it or
//! turns binary keeps its last version and is named.
//!
//! **The manifest.** CX0's manifest (`dry-run-manifest.json`, the
//! 2026-11-13 dry run) is a one-off migration report in a workspace's
//! outputs, not something a running Hollow can read; what it decided for the
//! vault (the personal-layer folders move to the personal repo, `library/`
//! and the workspaces stay in the vault) and the two-Macs V0 ignore list are
//! built in here as [`MANIFEST`], versioned. The generated ignore is written
//! to the root's own `info/exclude` in app data, never into the vault: git
//! honours it when listing new files, and [`path_allowed`] (the same
//! manifest) is the check that decides. A changed manifest rewrites
//! `info/exclude` at the next cycle; no vault file is touched. A file the
//! new manifest leaves out that was already committed stays in the commits
//! at its last version (untracking it would delete the other Mac's copy once
//! V6 adds a remote).
//!
//! **Local only.** The vault root has no remote until V6: a config naming
//! one is refused, so a cycle never fetches, merges or pushes; it commits on
//! this Mac and its state reads "local only".

use super::{git_ok, git_run, Root, ADD_TIMEOUT, PERSONAL_ALLOW};
use std::fs;
use std::io::Read;

/// The vault root's id.
pub(crate) const VAULT: &str = "vault";
/// A tracked file's ceiling in bytes (1 MB).
pub(crate) const MAX_BYTES: u64 = 1024 * 1024;
/// Bytes read to tell text from binary.
const SNIFF: usize = 8000;

/// What the vault root leaves out, as built in (see the module notes).
pub(crate) struct Manifest {
    /// Bumped whenever a list below changes; named in `info/exclude`.
    pub version: u32,
    /// Top-level folders owned by another root or never synced.
    pub top_dirs: &'static [&'static str],
    /// Folder names left out at any depth.
    pub any_dirs: &'static [&'static str],
    /// File extensions (lowercase, no dot) that are never text.
    pub binary_exts: &'static [&'static str],
    /// Secret file names as ignore globs (the Rust check is CX1's
    /// `secret.key_file` rule itself).
    pub key_file_globs: &'static [&'static str],
}

pub(crate) const MANIFEST: Manifest = Manifest {
    version: 1,
    top_dirs: &["tasks"],
    any_dirs: &[
        ".git",
        "node_modules",
        ".hollow-attachments",
        "screenshots",
        "captures",
        "recordings",
        "video",
        "test-results",
        "playwright-report",
        "__pycache__",
        ".venv",
        "venv",
        ".next",
        ".turbo",
        ".cache",
        "target",
    ],
    binary_exts: &[
        "png",
        "jpg",
        "jpeg",
        "gif",
        "webp",
        "heic",
        "heif",
        "tif",
        "tiff",
        "bmp",
        "ico",
        "icns",
        "psd",
        "mov",
        "mp4",
        "m4v",
        "webm",
        "mkv",
        "avi",
        "mp3",
        "m4a",
        "wav",
        "aif",
        "aiff",
        "caf",
        "flac",
        "ogg",
        "opus",
        "woff",
        "woff2",
        "ttf",
        "otf",
        "dmg",
        "pkg",
        "exe",
        "dll",
        "dylib",
        "so",
        "o",
        "a",
        "class",
        "jar",
        "wasm",
        "bin",
        "docx",
        "pptx",
        "key",
        "numbers",
        "pages",
        "sqlite-wal",
        "sqlite-shm",
        "db-wal",
        "db-shm",
        "lock",
        "tmp",
        "swp",
    ],
    key_file_globs: &[
        ".env",
        ".env.*",
        ".dev.vars",
        ".pypirc",
        ".netrc",
        ".git-credentials",
        "credentials.json",
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "id_rsa*",
        "id_dsa*",
        "id_ecdsa*",
        "id_ed25519*",
        "*.pem",
        "*.key",
        "*.p12",
        "*.pfx",
        "*.jks",
        "*.keystore",
    ],
};

/// Whether `rel` (vault-relative, `/` separators) may be tracked by the vault
/// root under `m`, by its path alone. Size and content are checked apart.
pub(crate) fn path_allowed_by(m: &Manifest, rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.eq_ignore_ascii_case(".git"))
    {
        return false;
    }
    let top = parts[0];
    if top.starts_with('.')
        || PERSONAL_ALLOW.iter().any(|p| p.eq_ignore_ascii_case(top))
        || m.top_dirs.iter().any(|d| d.eq_ignore_ascii_case(top))
    {
        return false;
    }
    let (dirs, name) = parts.split_at(parts.len() - 1);
    let name = name[0];
    if dirs
        .iter()
        .any(|d| m.any_dirs.iter().any(|a| a.eq_ignore_ascii_case(d)))
    {
        return false;
    }
    if name == ".DS_Store" || name.starts_with("..") || name.ends_with('~') {
        return false;
    }
    if let Some((_, ext)) = name.rsplit_once('.') {
        let ext = ext.to_ascii_lowercase();
        if ext == "jsonl" || m.binary_exts.contains(&ext.as_str()) {
            return false;
        }
    }
    !scan_rules_out(rel)
}

/// CX1's path rules the vault root keeps out: data file types and secret
/// file names.
fn scan_rules_out(rel: &str) -> bool {
    crate::scan_rules::path_rule_matches("file.data_type", rel)
        || crate::scan_rules::path_rule_matches("secret.key_file", rel)
}

/// Whether top-level folder `name` may hold tracked files under the
/// built-in [`MANIFEST`].
fn folder_allowed(name: &str) -> bool {
    !(name.starts_with('.')
        || PERSONAL_ALLOW.iter().any(|p| p.eq_ignore_ascii_case(name))
        || MANIFEST
            .top_dirs
            .iter()
            .any(|d| d.eq_ignore_ascii_case(name))
        || MANIFEST
            .any_dirs
            .iter()
            .any(|d| d.eq_ignore_ascii_case(name)))
}

/// [`path_allowed_by`] the built-in [`MANIFEST`].
pub(crate) fn path_allowed(rel: &str) -> bool {
    path_allowed_by(&MANIFEST, rel)
}

/// A NUL byte anywhere in `bytes`: not text. New files pass only their
/// first [`SNIFF`] bytes; a staged blob (at most [`MAX_BYTES`]) passes whole.
pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

/// The generated `info/exclude` for `m`: a header naming the manifest
/// version, then one line per rule. Deterministic, so a cycle rewrites the
/// file only when the manifest (or the CX1 rules) changed.
pub(crate) fn exclude_text(m: &Manifest) -> String {
    let mut out = format!(
        "# Generated by Hollow (context sync, vault root, manifest v{}).\n\
         # Rewritten when the manifest changes; edits here are replaced.\n\
         # Hollow also checks size (at most {} bytes) and text-only itself.\n",
        m.version, MAX_BYTES
    );
    out.push_str("/.*\n");
    for d in PERSONAL_ALLOW.iter().chain(m.top_dirs.iter()) {
        out.push_str(&format!("/{d}/\n"));
    }
    for d in m.any_dirs {
        out.push_str(&format!("{d}/\n"));
    }
    out.push_str(".DS_Store\n*~\n..*\n*.jsonl\n");
    for e in m.binary_exts {
        out.push_str(&format!("*.{e}\n"));
    }
    let data = crate::scan_rules::data_type_extensions();
    if !data.is_empty() {
        out.push_str("# CX1 file.data_type\n");
        for e in data {
            out.push_str(&format!("*.{e}\n"));
        }
    }
    out.push_str("# CX1 secret.key_file\n");
    for g in m.key_file_globs {
        out.push_str(&format!("{g}\n"));
    }
    out
}

/// Write the generated ignore into the root's `info/exclude` when it differs
/// from what is there. Only the git directory in app data is written.
pub(super) fn ensure_exclude(root: &Root, m: &Manifest) -> Result<bool, String> {
    let want = exclude_text(m);
    let path = root.git_dir.join("info").join("exclude");
    if fs::read_to_string(&path).ok().as_deref() == Some(want.as_str()) {
        return Ok(false);
    }
    fs::create_dir_all(root.git_dir.join("info")).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, want.as_bytes())?;
    Ok(true)
}

/// NUL-separated paths `git ls-files` printed.
fn listed(root: &Root, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(git_ok(root, args, true)?
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// Whether the untracked file at `rel` may go in: a plain file within the
/// ceiling and text, or a symlink (staged so the refusal names it).
fn untracked_ok(root: &Root, rel: &str) -> bool {
    let full = root.work_tree.join(rel);
    let Ok(meta) = fs::symlink_metadata(&full) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return false;
    }
    let mut head = Vec::with_capacity(SNIFF);
    match fs::File::open(&full).and_then(|f| f.take(SNIFF as u64).read_to_end(&mut head)) {
        Ok(_) => !looks_binary(&head),
        Err(_) => false,
    }
}

/// New files the vault root would add now: not ignored (the generated
/// ignore and the vault's own `.gitignore` files), allowed by path, text and
/// within the ceiling. A nested repository (listed as `dir/`) is left out.
pub(super) fn untracked(root: &Root) -> Result<Vec<String>, String> {
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    let specs = super::pathspecs(root);
    args.extend(specs.iter().map(String::as_str));
    Ok(listed(root, &args)?
        .into_iter()
        .filter(|p| !p.ends_with('/') && path_allowed(p) && untracked_ok(root, p))
        .collect())
}

/// Over the ceiling on disk now (`lstat`); `false` when it can't be read,
/// so a removed file still stages as a removal.
fn too_big_on_disk(root: &Root, rel: &str) -> bool {
    fs::symlink_metadata(root.work_tree.join(rel)).is_ok_and(|m| m.is_file() && m.len() > MAX_BYTES)
}

/// Stage the vault root: every change to a tracked file still allowed by
/// path and within the ceiling (a removal too), and every new file
/// [`untracked`] lets in, minus the engine's own exclusions (refused or
/// withheld paths). Nothing ignored, over the ceiling or binary is hashed.
/// Returns the tracked files left at their last version for their size
/// (`path (why)`), to be named.
pub(super) fn stage(root: &Root) -> Result<Vec<String>, String> {
    let mut args = vec!["ls-files", "-z", "--cached", "--"];
    let specs = super::pathspecs(root);
    args.extend(specs.iter().map(String::as_str));
    let mut skipped = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for p in listed(root, &args)? {
        if !path_allowed(&p) {
            continue;
        }
        if too_big_on_disk(root, &p) {
            skipped.push(format!("{p} (larger than {} MB)", MAX_BYTES >> 20));
        } else {
            paths.push(p);
        }
    }
    paths.extend(untracked(root)?);
    let skip: std::collections::BTreeSet<String> = super::excluded_paths(root);
    paths.retain(|p| !skip.contains(p));
    if paths.is_empty() {
        return Ok(skipped);
    }
    paths.sort();
    paths.dedup();
    let mut input = Vec::new();
    for p in &paths {
        input.extend_from_slice(b":(literal)");
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let out = git_run(
        root,
        &["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"],
        true,
        Some(input),
        &[],
        ADD_TIMEOUT,
    )?;
    if out.code != 0 {
        return Err(format!("git add failed: {}", out.stderr));
    }
    Ok(skipped)
}

/// The top-level folders `tracked` (vault-relative paths) has files in that
/// the manifest still allows: each must still be in the vault before a
/// cycle commits (one the manifest now leaves out is never staged, so its
/// absence removes nothing).
pub(super) fn tracked_folders(tracked: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tracked
        .iter()
        .filter_map(|p| p.split_once('/').map(|(top, _)| top.to_string()))
        .filter(|top| folder_allowed(top))
        .collect();
    out.sort();
    out.dedup();
    out
}
