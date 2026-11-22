//! The importer's vault side: a staged overlay of planned files, the
//! journaled apply, the byte-exact rollback, and the preflight snapshot.
//!
//! - [`Overlay`] reads the vault with planned changes on top, so a dry run
//!   can plan the rekey on top of the import without writing anything.
//! - [`apply`] writes each planned file whose bytes differ, after saving the
//!   original under `<backup-dir>/<stamp>/files/`, and records every path in
//!   `journal.jsonl` *before* touching it, then in `manifest.json` once done.
//! - [`rollback`] walks the manifest backwards: restores saved bytes, removes
//!   created files and folders, moves archived files back, and checks every
//!   SHA-256. A path already in its before state is left as is, so an
//!   interrupted apply rolls back too.

use super::{sha256, Report};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// A vault-relative path the importer may touch: no absolute paths, no
/// `..`, no empty parts, no backslashes.
pub(crate) fn check_rel(rel: &str) -> Result<(), String> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.contains('\0')
        || rel
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(format!("Unsafe vault path {rel:?}."));
    }
    Ok(())
}

/// Refuse a path that passes through a symbolic link.
fn no_symlink(root: &Path, rel: &str) -> Result<(), String> {
    check_rel(rel)?;
    let mut probe = root.to_path_buf();
    for part in rel.split('/') {
        probe.push(part);
        match fs::symlink_metadata(&probe) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("{rel} passes through a symbolic link; left alone."))
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

fn read_disk(root: &Path, rel: &str) -> Option<Vec<u8>> {
    no_symlink(root, rel).ok()?;
    let path = root.join(rel);
    path.is_file().then(|| fs::read(&path).ok()).flatten()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Move {
    pub from: String,
    pub to: String,
}

/// The vault with planned changes on top.
pub(crate) struct Overlay {
    root: PathBuf,
    changes: BTreeMap<String, Vec<u8>>,
    pub moves: Vec<Move>,
}

impl Overlay {
    pub(crate) fn new(root: &Path) -> Overlay {
        Overlay {
            root: root.to_path_buf(),
            changes: BTreeMap::new(),
            moves: Vec::new(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.changes
            .get(rel)
            .cloned()
            .or_else(|| read_disk(&self.root, rel))
    }

    pub(crate) fn text(&self, rel: &str) -> Option<String> {
        self.read(rel).and_then(|b| String::from_utf8(b).ok())
    }

    pub(crate) fn exists(&self, rel: &str) -> bool {
        self.changes.contains_key(rel) || read_disk(&self.root, rel).is_some()
    }

    /// Plan `bytes` at `rel`; planning the bytes already on disk is a no-op.
    pub(crate) fn set(&mut self, rel: &str, bytes: Vec<u8>) -> Result<(), String> {
        check_rel(rel)?;
        if read_disk(&self.root, rel).as_deref() == Some(bytes.as_slice()) {
            self.changes.remove(rel);
        } else {
            self.changes.insert(rel.to_string(), bytes);
        }
        Ok(())
    }

    /// File names directly in `dir` (vault-relative), disk and planned.
    pub(crate) fn files_in(&self, dir: &str) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = fs::read_dir(self.root.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let prefix = format!("{dir}/");
        for rel in self.changes.keys() {
            if let Some(name) = rel.strip_prefix(&prefix).filter(|n| !n.contains('/')) {
                names.insert(name.to_string());
            }
        }
        names.into_iter().collect()
    }

    /// Sub-folder names directly in `dir` on disk (not dot folders).
    pub(crate) fn dirs_in(&self, dir: &str) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(self.root.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect();
        out.sort();
        out
    }

    /// Every planned path and whether it exists on disk now (H4a's
    /// owner-switch dry run lists them).
    pub(crate) fn planned(&self) -> Vec<(String, bool)> {
        self.changes
            .keys()
            .map(|rel| (rel.clone(), read_disk(&self.root, rel).is_some()))
            .collect()
    }

    /// Count what the plan would create and change.
    fn count(&self, report: &mut Report) {
        report.files_created = 0;
        report.files_changed = 0;
        for rel in self.changes.keys() {
            if read_disk(&self.root, rel).is_some() {
                report.files_changed += 1;
            } else {
                report.files_created += 1;
            }
        }
    }

    /// The dry run: every planned file into `staging` at its vault path.
    pub(crate) fn stage(&self, staging: &Path, report: &mut Report) -> Result<(), String> {
        self.count(report);
        for (rel, bytes) in &self.changes {
            write_out(staging, rel, bytes)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MovedFile {
    /// Inside the moved path ("" when the move is one file).
    pub rel: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub(crate) enum Entry {
    #[serde(rename_all = "camelCase")]
    Created { path: String, after_sha256: String },
    #[serde(rename_all = "camelCase")]
    Changed {
        path: String,
        before_sha256: String,
        after_sha256: String,
        backup: String,
    },
    #[serde(rename_all = "camelCase")]
    Moved {
        from: String,
        path: String,
        files: Vec<MovedFile>,
    },
    /// A folder the run created (removed on rollback once empty).
    #[serde(rename_all = "camelCase")]
    Folder { path: String },
    /// The vault the run wrote to (the journal's first line).
    #[serde(rename_all = "camelCase")]
    Vault { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub version: u32,
    pub mode: String,
    pub created_at: String,
    pub vault: String,
    pub entries: Vec<Entry>,
}

pub(crate) struct Applied {
    pub manifest: Option<PathBuf>,
    pub moved: usize,
}

struct Journal {
    dir: PathBuf,
    file: fs::File,
    manifest: Manifest,
}

impl Journal {
    fn record(&mut self, entry: Entry) -> Result<(), String> {
        let line =
            serde_json::to_string(&entry).map_err(|_| "Journal entry couldn't be encoded.")?;
        writeln!(self.file, "{line}")
            .and_then(|_| self.file.sync_data())
            .map_err(|_| {
                "The journal couldn't be written; nothing more was changed.".to_string()
            })?;
        self.manifest.entries.push(entry);
        Ok(())
    }
}

fn stamp_dir(backup: &Path, mode: &str) -> Result<PathBuf, String> {
    let base = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    for n in 0..100 {
        let name = if n == 0 {
            format!("{base}-{mode}")
        } else {
            format!("{base}-{mode}-{n}")
        };
        let dir = backup.join(name);
        if fs::create_dir(&dir).is_ok() {
            return Ok(dir);
        }
    }
    Err("No free backup folder name.".into())
}

/// Create the missing parents of `rel`, journaling each one first.
fn make_parents(root: &Path, rel: &str, journal: &mut Journal) -> Result<(), String> {
    let parts: Vec<&str> = rel.split('/').collect();
    for depth in 1..parts.len() {
        let dir = parts[..depth].join("/");
        let path = root.join(&dir);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => continue,
            Ok(_) => return Err(format!("{dir} isn't a plain folder; left alone.")),
            Err(_) => {
                journal.record(Entry::Folder { path: dir.clone() })?;
                fs::create_dir(&path).map_err(|_| format!("Couldn't create {dir}."))?;
            }
        }
    }
    Ok(())
}

/// A file written outside the vault (staging, reports, notes) at `rel`
/// under `root`: no link on the way, missing folders created one by one,
/// and the file itself opened with `O_NOFOLLOW`, so a planted link is
/// refused rather than followed.
pub(crate) fn write_out(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    no_symlink(root, rel)?;
    let parts: Vec<&str> = rel.split('/').collect();
    let mut dir = root.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "Couldn't write {rel}: a part of its path isn't a folder."
                ))
            }
            Err(_) => fs::create_dir(&dir).map_err(|_| format!("Couldn't write {rel}."))?,
        }
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(rel))
        .map_err(|_| format!("Couldn't write {rel} (a link there is refused)."))?;
    file.write_all(bytes)
        .map_err(|_| format!("Couldn't write {rel}."))
}

/// A brand-new file (never an existing one or a link), flushed to disk.
fn create_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Write through a fresh, randomly named temp file created with
/// `create_new`, rename it over `path`, then flush the folder.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let failed = || format!("Couldn't write {}.", path.display());
    let parent = path.parent().ok_or_else(failed)?;
    for _ in 0..8 {
        let tmp = parent.join(format!(
            ".{name}.tracker-import-{:016x}.tmp",
            rand::random::<u64>()
        ));
        match create_synced(&tmp, bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                let _ = fs::remove_file(&tmp);
                return Err(failed());
            }
        }
        return fs::rename(&tmp, path)
            .and_then(|_| sync_dir(parent))
            .map_err(|_| {
                let _ = fs::remove_file(&tmp);
                failed()
            });
    }
    Err(failed())
}

/// Hash every file under `path` (or the file itself), relative to it.
fn hash_tree(path: &Path) -> Result<Vec<MovedFile>, String> {
    let mut out = Vec::new();
    let meta = fs::symlink_metadata(path).map_err(|_| "A moved path is missing.".to_string())?;
    if meta.is_file() {
        let bytes = fs::read(path).map_err(|_| "A moved file couldn't be read.".to_string())?;
        out.push(MovedFile {
            rel: String::new(),
            sha256: sha256(&bytes),
        });
        return Ok(out);
    }
    fn walk(base: &Path, dir: &Path, out: &mut Vec<MovedFile>) -> Result<(), String> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|_| "A moved folder couldn't be read.".to_string())?
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|_| "A moved entry couldn't be read.")?;
            if kind.is_dir() {
                walk(base, &path, out)?;
            } else if kind.is_file() {
                let bytes =
                    fs::read(&path).map_err(|_| "A moved file couldn't be read.".to_string())?;
                out.push(MovedFile {
                    rel: path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    sha256: sha256(&bytes),
                });
            }
        }
        Ok(())
    }
    walk(path, path, &mut out)?;
    Ok(out)
}

/// Write the overlay's changes and moves into the vault, journaled.
pub(crate) fn apply(
    vault: &Path,
    backup: &Path,
    mode: &str,
    overlay: &Overlay,
    report: &mut Report,
) -> Result<Applied, String> {
    overlay.count(report);
    let moves: Vec<&Move> = overlay
        .moves
        .iter()
        .filter(|m| fs::symlink_metadata(vault.join(&m.from)).is_ok())
        .collect();
    if overlay.changes.is_empty() && moves.is_empty() {
        return Ok(Applied {
            manifest: None,
            moved: 0,
        });
    }
    let dir = stamp_dir(backup, mode)?;
    let file = fs::OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(dir.join("journal.jsonl"))
        .map_err(|_| "The journal couldn't be created.".to_string())?;
    let canonical = vault
        .canonicalize()
        .map_err(|_| "The vault folder doesn't exist.".to_string())?
        .to_string_lossy()
        .to_string();
    let mut journal = Journal {
        dir: dir.clone(),
        file,
        manifest: Manifest {
            version: 1,
            mode: mode.into(),
            created_at: chrono::Local::now().to_rfc3339(),
            vault: canonical.clone(),
            entries: Vec::new(),
        },
    };
    journal.record(Entry::Vault { path: canonical })?;
    let result = write_all(vault, overlay, &moves, &mut journal);
    let manifest_path = journal.dir.join("manifest.json");
    let encoded = serde_json::to_vec_pretty(&journal.manifest)
        .map_err(|_| "The manifest couldn't be encoded.".to_string())?;
    fs::write(&manifest_path, encoded).map_err(|_| {
        "The manifest couldn't be written; journal.jsonl in the backup folder has every entry."
            .to_string()
    })?;
    result.map_err(|e| format!("{e} Roll back with --rollback {}.", manifest_path.display()))?;
    Ok(Applied {
        manifest: Some(manifest_path),
        moved: moves.len(),
    })
}

fn write_all(
    vault: &Path,
    overlay: &Overlay,
    moves: &[&Move],
    journal: &mut Journal,
) -> Result<(), String> {
    for (rel, bytes) in &overlay.changes {
        no_symlink(vault, rel)?;
        let path = vault.join(rel);
        let before = read_disk(vault, rel);
        if before.as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        if path.exists() && before.is_none() {
            return Err(format!("{rel} exists but isn't a plain file; left alone."));
        }
        make_parents(vault, rel, journal)?;
        match before {
            Some(old) => {
                let backup = format!("files/{rel}");
                let saved = journal.dir.join(&backup);
                // The backup and every folder above it, up to the backup
                // root, are on disk before the vault file changes.
                let backed_up = fs::create_dir_all(saved.parent().unwrap())
                    .and_then(|_| create_synced(&saved, &old))
                    .and_then(|_| {
                        let mut dir = saved.parent();
                        while let Some(d) = dir {
                            sync_dir(d)?;
                            if Some(d) == journal.dir.parent() {
                                break;
                            }
                            dir = d.parent();
                        }
                        Ok(())
                    });
                backed_up
                    .map_err(|_| format!("Couldn't back up {rel}; nothing more was changed."))?;
                journal.record(Entry::Changed {
                    path: rel.clone(),
                    before_sha256: sha256(&old),
                    after_sha256: sha256(bytes),
                    backup,
                })?;
            }
            None => journal.record(Entry::Created {
                path: rel.clone(),
                after_sha256: sha256(bytes),
            })?,
        }
        write_atomic(&path, bytes)?;
    }
    for m in moves {
        no_symlink(vault, &m.from)?;
        no_symlink(vault, &m.to)?;
        if fs::symlink_metadata(vault.join(&m.to)).is_ok() {
            return Err(format!(
                "{} already exists; {} was left in place.",
                m.to, m.from
            ));
        }
        let files = hash_tree(&vault.join(&m.from))?;
        make_parents(vault, &m.to, journal)?;
        journal.record(Entry::Moved {
            from: m.from.clone(),
            path: m.to.clone(),
            files,
        })?;
        fs::rename(vault.join(&m.from), vault.join(&m.to))
            .map_err(|_| format!("Couldn't move {} to {}.", m.from, m.to))?;
    }
    Ok(())
}

fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let raw = fs::read(path).map_err(|_| "The manifest couldn't be read.".to_string())?;
    if path.extension().is_some_and(|e| e == "jsonl") {
        let mut entries = Vec::new();
        for line in String::from_utf8_lossy(&raw)
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            entries.push(
                serde_json::from_str(line)
                    .map_err(|_| "The journal has a damaged line.".to_string())?,
            );
        }
        let vault = entries
            .iter()
            .find_map(|e| match e {
                Entry::Vault { path } => Some(path.clone()),
                _ => None,
            })
            .unwrap_or_default();
        return Ok(Manifest {
            version: 1,
            mode: "journal".into(),
            created_at: String::new(),
            vault,
            entries,
        });
    }
    serde_json::from_slice(&raw).map_err(|_| "The manifest is damaged.".to_string())
}

fn sha_of(vault: &Path, rel: &str) -> Option<String> {
    read_disk(vault, rel).map(|b| sha256(&b))
}

/// Every path a manifest names stays inside the vault (or, for a backup,
/// inside the run's `files/`), with no links on the way.
fn check_entry(vault: &Path, base: &Path, entry: &Entry) -> Result<(), String> {
    match entry {
        Entry::Created { path, .. } | Entry::Folder { path } => no_symlink(vault, path),
        Entry::Changed { path, backup, .. } => {
            no_symlink(vault, path)?;
            if backup != &format!("files/{path}") {
                return Err(format!("The backup of {path} isn't under files/; refused."));
            }
            no_symlink(base, backup)
        }
        Entry::Moved { from, path, files } => {
            no_symlink(vault, from)?;
            no_symlink(vault, path)?;
            files
                .iter()
                .filter(|f| !f.rel.is_empty())
                .try_for_each(|f| check_rel(&f.rel))
        }
        Entry::Vault { .. } => Ok(()),
    }
}

fn undo(vault: &Path, base: &Path, entry: &Entry) -> Result<(), String> {
    check_entry(vault, base, entry)?;
    match entry {
        Entry::Vault { .. } => Ok(()),
        Entry::Created { path, after_sha256 } => match sha_of(vault, path) {
            None => Ok(()),
            Some(sha) if &sha == after_sha256 => {
                fs::remove_file(vault.join(path)).map_err(|_| format!("Couldn't remove {path}."))
            }
            Some(_) => Err(format!("{path} changed after the run; left in place.")),
        },
        Entry::Changed {
            path,
            before_sha256,
            after_sha256,
            backup,
        } => {
            let current = sha_of(vault, path);
            if current.as_ref() == Some(before_sha256) {
                return Ok(());
            }
            if current.as_ref() != Some(after_sha256) {
                return Err(format!("{path} changed after the run; left in place."));
            }
            let saved = fs::read(base.join(backup))
                .map_err(|_| format!("The backup of {path} is missing."))?;
            if &sha256(&saved) != before_sha256 {
                return Err(format!(
                    "The backup of {path} doesn't match its hash; left in place."
                ));
            }
            write_atomic(&vault.join(path), &saved)?;
            (sha_of(vault, path).as_ref() == Some(before_sha256))
                .then_some(())
                .ok_or_else(|| format!("{path} didn't verify after restoring."))
        }
        Entry::Moved { from, path, files } => {
            let at_dest = fs::symlink_metadata(vault.join(path)).is_ok();
            let at_source = fs::symlink_metadata(vault.join(from)).is_ok();
            if !at_dest && at_source {
                return Ok(());
            }
            if !at_dest || at_source {
                return Err(format!(
                    "{path} can't be moved back to {from}; left in place."
                ));
            }
            if &hash_tree(&vault.join(path))? != files {
                return Err(format!("{path} changed after the run; left in place."));
            }
            fs::rename(vault.join(path), vault.join(from))
                .map_err(|_| format!("Couldn't move {path} back to {from}."))?;
            (&hash_tree(&vault.join(from))? == files)
                .then_some(())
                .ok_or_else(|| format!("{from} didn't verify after moving back."))
        }
        Entry::Folder { path } => {
            let dir = vault.join(path);
            if dir.is_dir()
                && fs::read_dir(&dir)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
            {
                fs::remove_dir(&dir).map_err(|_| format!("Couldn't remove the folder {path}."))?;
            }
            Ok(())
        }
    }
}

/// Undo one apply or rekey from its manifest (or its journal).
pub(crate) fn rollback(vault: &Path, manifest_path: &Path) -> Result<String, String> {
    let manifest = load_manifest(manifest_path)?;
    let base = manifest_path
        .parent()
        .ok_or("The manifest has no folder.")?
        .to_path_buf();
    let marker = base.join("rolled-back.json");
    if marker.exists() {
        return Err("This run was already rolled back.".into());
    }
    let here = vault
        .canonicalize()
        .map_err(|_| "The vault folder doesn't exist.".to_string())?;
    if manifest.vault.is_empty() || here.to_string_lossy() != manifest.vault {
        return Err("The manifest doesn't name this vault; nothing was changed.".into());
    }
    // Refuse the whole manifest before touching anything if any path is unsafe.
    for entry in &manifest.entries {
        check_entry(vault, &base, entry)?;
    }
    let mut errors = Vec::new();
    for entry in manifest.entries.iter().rev() {
        if let Err(e) = undo(vault, &base, entry) {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        // Only a clean rollback is final; a partial one can be run again.
        let note = serde_json::json!({ "at": chrono::Local::now().to_rfc3339() });
        fs::write(
            &marker,
            serde_json::to_vec_pretty(&note).unwrap_or_default(),
        )
        .map_err(|_| "The rollback marker couldn't be written.".to_string())?;
        Ok(format!(
            "rollback: {} entries restored and verified.",
            manifest.entries.len()
        ))
    } else {
        Err(format!(
            "rollback finished with {} problem(s): {}",
            errors.len(),
            errors.join(" ")
        ))
    }
}

/// The preflight snapshot: `tasks/`, every `workspaces/notion-*/` brief,
/// project and NOTION.md, and `.notion-sync.json`, copied (clone-on-write on
/// APFS, which `fs::copy` uses on macOS) into `dest`.
pub(crate) fn snapshot(vault: &Path, dest: &Path) -> Result<usize, String> {
    // A fresh folder only, so nothing inside it can be a planted link.
    fs::create_dir(dest)
        .map_err(|_| format!("{} already exists or can't be created.", dest.display()))?;
    let mut wanted: Vec<String> = Vec::new();
    for name in fs::read_dir(vault.join("tasks"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if name.file_type().is_ok_and(|t| t.is_file()) {
            wanted.push(format!("tasks/{}", name.file_name().to_string_lossy()));
        }
    }
    for dir in fs::read_dir(vault.join("workspaces"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let id = dir.file_name().to_string_lossy().to_string();
        if id.starts_with("notion-") && dir.file_type().is_ok_and(|t| t.is_dir()) {
            for file in ["WORKSPACE.md", "PROJECT.md", "NOTION.md"] {
                wanted.push(format!("workspaces/{id}/{file}"));
            }
        }
    }
    wanted.push(".notion-sync.json".into());
    let mut count = 0;
    for rel in wanted {
        if read_disk(vault, &rel).is_none() {
            continue;
        }
        no_symlink(dest, &rel)?;
        let to = dest.join(&rel);
        fs::create_dir_all(to.parent().unwrap())
            .and_then(|_| fs::copy(vault.join(&rel), &to))
            .map_err(|_| format!("Couldn't copy {rel}."))?;
        count += 1;
    }
    Ok(count)
}
