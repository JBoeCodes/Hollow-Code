//! Where each shared skill Hollow imported came from (repo ideas 2026-10-27
//! #2), kept in Hollow's app data (`skill-provenance.json`), never inside the
//! skill's folder.
//!
//! - **The record.** [`crate::skill_import`]'s commit writes one per installed
//!   name: what was picked (a path) or dropped (a file name), the origin when
//!   the importer gave one (a picked folder or file inside a git checkout
//!   whose `origin` remote is plain `https://`: its URL, HEAD commit and the
//!   folder's place in the repo, read from `.git` files; git never runs), the
//!   SHA-256 of every file as installed, and the time. Skills Hollow didn't
//!   import (or imported before 10-27) have none: the Skills page says "Not
//!   imported by Hollow", which is not a warning.
//! - **Changed outside Hollow.** [`view`] hashes the installed folder again
//!   (within the import caps) and lists what differs from the record.
//!   Hollow's own writes (Edit skill, the local note) refresh the record, so
//!   only other writers show.
//! - **Check for update…** [`stage_update`] stages a new copy through Import's
//!   own staging, from the recorded `https://` origin (a shallow `git clone`
//!   into staging, only on the person's click, never otherwise) or else the
//!   recorded path, and adds the file and script differences to Import's
//!   preview. Nothing replaces the installed copy until the person presses
//!   Install on that card; nothing updates by itself.
//! - **Local note.** [`set_note`] keeps a short note per shared skill here and
//!   writes it into the skill's SKILL.md as one marked block after the
//!   frontmatter, so every agent reads it; an import that replaces the skill
//!   writes it again ("Local setup on this Mac" without re-adding it by hand).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const FILE: &str = "skill-provenance.json";
/// The longest local note, in characters.
pub(crate) const MAX_NOTE_CHARS: usize = 4_000;
const NOTE_START: &str = "<!-- hollow:local-note -->";
const NOTE_END: &str = "<!-- /hollow:local-note -->";
const NOTE_HEADING: &str = "## Local note (kept by Hollow through updates)";
/// How long a Check for update clone may take.
const CLONE_TIMEOUT: Duration = Duration::from_secs(120);
/// Parent folders searched for a `.git` above a picked path.
const MAX_GIT_DEPTH: usize = 12;
/// The most a `.git` file Hollow reads may hold: `config`, `HEAD`, a loose
/// ref, and `packed-refs` (which can list many refs).
const GIT_FILE_MAX: u64 = 64 * 1024;
const PACKED_REFS_MAX: u64 = 4 * 1024 * 1024;
/// Check for update shows at most this much text of changed files, in all
/// and per file.
const DIFF_TOTAL_MAX: usize = 64 * 1024;
const DIFF_FILE_MAX: usize = 16 * 1024;
/// A changed file larger than this is listed, not diffed.
const DIFF_SOURCE_MAX: u64 = 256 * 1024;

static LOCK: Mutex<()> = Mutex::new(());

/// What was imported: a picked file or folder, or a dropped file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportedFrom {
    Path { path: String },
    Dropped { name: String },
}

/// The git checkout a picked path sat in: its `https://` origin, HEAD commit
/// (None when unreadable) and the path's place in the repo ("" at its root).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub subdir: String,
}

/// What a staged import came from (written beside the staged copy, read
/// back by commit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub from: ImportedFrom,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// One installed skill's record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub from: ImportedFrom,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// Path in the skill → SHA-256 (hex) as installed.
    pub files: BTreeMap<String, String>,
    pub imported_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default)]
    skills: BTreeMap<String, Record>,
    #[serde(default)]
    notes: BTreeMap<String, String>,
}

/// How the installed folder differs from what was recorded (or, for an
/// update, how the staged copy differs from the installed one).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
}

#[cfg(test)]
impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// The Skills page's Source block for one shared skill.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub name: String,
    /// The record, or None: "Not imported by Hollow".
    pub record: Option<Record>,
    /// Differences from the record (None without one, or when the folder
    /// couldn't be read: `error` says why).
    pub changed: Option<Changes>,
    pub error: Option<String>,
    /// Check for update has somewhere to look: an https origin or a path
    /// that still exists.
    pub can_check: bool,
    pub note: Option<String>,
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(FILE))
}

fn load(path: &Path) -> Store {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(path: &Path, store: &Store) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(path, &json).map_err(|e| format!("save skill records: {e}"))
}

/// The physical shared-skill folder `name`, under the same guard as Edit and
/// Delete skill (no linked `.agents`, `skills` or skill folder), so a record,
/// an update or a note can only ever touch a skill Hollow manages.
fn skill_dir(home: &Path, name: &str) -> Result<PathBuf, String> {
    if !crate::skills::valid_skill_name(name) {
        return Err(format!("Not a skill name: {name:?}"));
    }
    crate::skills::managed_skill_dir(home, name, false)
}

/// What Check for update found: the staged copy against the installed one,
/// and which of the added or changed files run as code.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDiff {
    pub changes: Changes,
    pub scripts_changed: Vec<String>,
    /// The text of each changed or added file, scripts first, within
    /// [`DIFF_TOTAL_MAX`] in all.
    pub diffs: Vec<FileDiff>,
}

/// Path → SHA-256 of every file in `dir`, within the import caps, with
/// SKILL.md hashed without Hollow's own note block or trailing line breaks:
/// writing, changing or removing a note is never a change, so the baseline
/// never has to move for it (review of #1050: it used to, which hid outside
/// changes).
fn hashes(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = crate::skill_import::file_hashes(dir)?;
    if out.contains_key("SKILL.md") {
        let text = read_skill_md(&dir.join("SKILL.md"))?;
        // Trailing line breaks don't count either: a note on a frontmatter
        // that ends the file adds one that stays (review of #1050).
        let bare = without_note(&text);
        let bare = bare.trim_end_matches(['\n', '\r']);
        out.insert(
            "SKILL.md".into(),
            crate::skill_import::sha256_hex(bare.as_bytes()),
        );
    }
    Ok(out)
}

/// A skill's SKILL.md: a regular file (never a link, FIFO or device) within
/// the import's total cap, opened without following a last-moment link.
fn read_skill_md(path: &Path) -> Result<String, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|e| format!("read SKILL.md: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("read SKILL.md: {e}"))?;
    if !meta.is_file() || meta.len() > crate::skill_import::MAX_TOTAL_BYTES {
        return Err("SKILL.md isn't a plain file Hollow can read.".into());
    }
    let mut text = String::new();
    file.take(crate::skill_import::MAX_TOTAL_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("read SKILL.md: {e}"))?;
    Ok(text)
}

/// What differs between `before` and `after` (both path → hash).
pub(crate) fn diff(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Changes {
    let mut out = Changes::default();
    for (path, hash) in after {
        match before.get(path) {
            None => out.added.push(path.clone()),
            Some(old) if old != hash => out.modified.push(path.clone()),
            _ => {}
        }
    }
    out.removed = before
        .keys()
        .filter(|p| !after.contains_key(*p))
        .cloned()
        .collect();
    out
}

// ── The origin, from `.git` files ───────────────────────────────────────────

/// The `https://` origin of the git checkout `path` sits in, read from its
/// `.git` folder's files (never by running git). None for no checkout, a
/// `.git` file (a worktree or submodule), another scheme, or a URL with a
/// user name or password in it (never recorded).
pub(crate) fn origin_of(path: &Path) -> Option<Origin> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    let mut dir = start.as_path();
    for _ in 0..MAX_GIT_DEPTH {
        let git = dir.join(".git");
        if let Ok(meta) = fs::symlink_metadata(&git) {
            if !meta.is_dir() {
                return None;
            }
            let config = read_git_file(&git, "config", GIT_FILE_MAX)?;
            let url = origin_url(&config)?;
            let subdir = path
                .strip_prefix(dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            return Some(Origin {
                url,
                commit: head_commit(&git),
                subdir,
            });
        }
        dir = dir.parent()?;
    }
    None
}

/// `[remote "origin"]`'s `url`, when it is a plain `https://` URL.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.replace(char::is_whitespace, "") == "[remote\"origin\"]";
            continue;
        }
        if !in_origin {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "url" {
            return safe_https(value.trim().trim_matches('"'));
        }
    }
    None
}

/// `url` when it is `https://host/…` with no user name, password, query or
/// fragment (a token can ride in either: never recorded, fetched or shown;
/// review of #1050), control characters or spaces; else None.
pub(crate) fn safe_https(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    if host.is_empty()
        || host.contains('@')
        || url.contains(['?', '#'])
        || url.len() > 2048
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    Some(url.to_string())
}

/// The commit HEAD names: a detached sha, or its branch's ref (loose or
/// packed). None when unreadable.
fn head_commit(git: &Path) -> Option<String> {
    let head = read_git_file(git, "HEAD", GIT_FILE_MAX)?;
    let head = head.trim();
    let sha = |s: &str| {
        (s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
    };
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        return sha(head);
    };
    if !reference.starts_with("refs/") {
        return None;
    }
    if let Some(loose) = read_git_file(git, reference, GIT_FILE_MAX) {
        return sha(loose.trim());
    }
    let packed = read_git_file(git, "packed-refs", PACKED_REFS_MAX)?;
    packed.lines().find_map(|line| {
        let (id, name) = line.split_once(' ')?;
        (name.trim() == reference).then(|| sha(id)).flatten()
    })
}

/// `rel` (slash-separated) inside the `.git` folder `git`, read only when
/// every component is a real folder and the last a regular file of at most
/// `max` bytes: never through a link (out of the checkout), a FIFO that
/// would block, or a device (review of #1050). Walked with `openat` from
/// `.git` itself, each step `O_NOFOLLOW` (folders `O_DIRECTORY`, the file
/// `O_NONBLOCK`), so a folder swapped for a link between steps can't
/// redirect the read, and checked again once open.
fn read_git_file(git: &Path, rel: &str, max: u64) -> Option<String> {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return None;
    }
    let open = |at: Option<&OwnedFd>, name: &CString, flags: libc::c_int| {
        let flags = flags | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: plain libc calls with a NUL-terminated name and a live
        // directory descriptor; a valid result is owned (closed on drop).
        let fd = unsafe {
            match at {
                Some(dir) => libc::openat(dir.as_raw_fd(), name.as_ptr(), flags),
                None => libc::open(name.as_ptr(), flags),
            }
        };
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let mut dir = open(
        None,
        &CString::new(git.as_os_str().as_bytes()).ok()?,
        libc::O_DIRECTORY,
    )?;
    for (i, part) in parts.iter().enumerate() {
        let name = CString::new(*part).ok()?;
        if i + 1 < parts.len() {
            dir = open(Some(&dir), &name, libc::O_DIRECTORY)?;
            continue;
        }
        let file = fs::File::from(open(Some(&dir), &name, libc::O_NONBLOCK)?);
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() > max {
            return None;
        }
        let mut text = String::new();
        file.take(max).read_to_string(&mut text).ok()?;
        return Some(text);
    }
    None
}

/// `rel` (slash-separated) under `root`, when no component is empty, `.`,
/// `..` or a link, every one but the last is a folder and the last is a
/// folder or a regular file; checked component by component, then by the
/// resolved paths. Update staging reads a clone through it, so an upstream
/// change that turns `skills/` into a link can't reach outside the clone
/// (which also checks links out as plain files). A known limit: the import
/// that follows reads by path, so a process running as the person that
/// swaps a folder in Hollow's private staging clone between this check and
/// that read isn't defended against; such a process could write the skill
/// folder itself.
fn contained(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let outside = || format!("{rel} isn't a plain path inside {}.", root.display());
    let parts: Vec<&str> = if rel.is_empty() {
        Vec::new()
    } else {
        rel.split('/').collect()
    };
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\'))
    {
        return Err(outside());
    }
    let mut path = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = fs::symlink_metadata(&path).map_err(|_| outside())?;
        let last = i + 1 == parts.len();
        if meta.file_type().is_symlink() || !(meta.is_dir() || (last && meta.is_file())) {
            return Err(outside());
        }
    }
    let (real_root, real) = (fs::canonicalize(root), fs::canonicalize(&path));
    match (real_root, real) {
        (Ok(r), Ok(p)) if p.starts_with(&r) => Ok(path),
        _ => Err(outside()),
    }
}

// ── Records ─────────────────────────────────────────────────────────────────

/// After an import installed `name` from `info` (`replaced`: over an
/// existing skill of that name): apply its local note, if any, then record
/// the installed files. A failure here never undoes the install.
pub(crate) fn record_install(home: &Path, name: &str, info: SourceInfo) -> Result<(), String> {
    record_install_at(&store_path()?, home, name, info)
}

pub(crate) fn record_install_at(
    store: &Path,
    home: &Path,
    name: &str,
    info: SourceInfo,
) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(store);
    let dir = skill_dir(home, name)?;
    if let Some(note) = data.notes.get(name) {
        write_note(&dir.join("SKILL.md"), Some(note))?;
    }
    let files = hashes(&dir)?;
    data.skills.insert(
        name.to_string(),
        Record {
            from: info.from,
            origin: info.origin,
            files,
            imported_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    save(store, &data)
}

/// Hollow itself changed the skill (Edit skill, which writes SKILL.md only):
/// take SKILL.md as it is now, so it doesn't read as changed outside Hollow.
/// Every other file keeps its recorded hash, so an outside change to a
/// script still shows (review of #1050). Nothing without a record.
pub(crate) fn refresh_after_edit(home: &Path, name: &str) -> Result<(), String> {
    refresh_at(&store_path()?, home, name)
}

fn refresh_at(store: &Path, home: &Path, name: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(store);
    let Some(record) = data.skills.get_mut(name) else {
        return Ok(());
    };
    let now = hashes(&skill_dir(home, name)?)?;
    match now.get("SKILL.md") {
        Some(hash) => record.files.insert("SKILL.md".into(), hash.clone()),
        None => record.files.remove("SKILL.md"),
    };
    save(store, &data)
}

/// The skill was deleted in Hollow: drop its record and note.
pub(crate) fn forget(name: &str) -> Result<(), String> {
    forget_at(&store_path()?, name)
}

fn forget_at(store: &Path, name: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(store);
    let had_record = data.skills.remove(name).is_some();
    let had_note = data.notes.remove(name).is_some();
    if !had_record && !had_note {
        return Ok(());
    }
    save(store, &data)
}

pub fn view(name: &str) -> Result<View, String> {
    view_at(&store_path()?, &home()?, name)
}

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "no home dir".to_string())
}

pub(crate) fn view_at(store: &Path, home: &Path, name: &str) -> Result<View, String> {
    let dir = skill_dir(home, name)?;
    let data = load(store);
    let record = data.skills.get(name).cloned();
    let (changed, error) = match &record {
        None => (None, None),
        Some(r) => match hashes(&dir) {
            Ok(now) => (Some(diff(&r.files, &now)), None),
            Err(e) => (None, Some(e)),
        },
    };
    let can_check = record.as_ref().is_some_and(|r| {
        r.origin.is_some()
            || matches!(&r.from, ImportedFrom::Path { path } if Path::new(path).exists())
    });
    Ok(View {
        name: name.to_string(),
        record,
        changed,
        error,
        can_check,
        note: data.notes.get(name).cloned(),
    })
}

// ── Check for update ────────────────────────────────────────────────────────

pub fn stage_update(name: &str) -> Result<crate::skill_import::ImportPreview, String> {
    stage_update_at(&store_path()?, &home()?, name, clone_https)
}

/// Stage `name`'s recorded source again: its origin (cloned by `clone`) or
/// its path. The preview carries the differences from the installed copy.
pub(crate) fn stage_update_at(
    store: &Path,
    home: &Path,
    name: &str,
    clone: impl Fn(&str, &Path) -> Result<(), String>,
) -> Result<crate::skill_import::ImportPreview, String> {
    let record = load(store).skills.get(name).cloned().ok_or_else(|| {
        format!("Hollow has no import record for {name}; import the new version with Import skill.")
    })?;
    let installed = hashes(&skill_dir(home, name)?)?;
    let mut preview = if let Some(origin) = &record.origin {
        let checkout = crate::skill_import::scratch_dir(home)?;
        let staged = (|| {
            clone(&origin.url, &checkout)?;
            // Component by component, then resolved: an upstream link on the
            // way (`skills -> /`) never reaches outside the clone.
            let source = contained(&checkout, &origin.subdir).map_err(|_| {
                "The recorded folder isn't a plain folder inside the repository any more."
                    .to_string()
            })?;
            let fresh = origin_of(&source).map(|o| Origin {
                url: origin.url.clone(),
                commit: o.commit,
                subdir: origin.subdir.clone(),
            });
            crate::skill_import::stage_with(
                home,
                &source,
                SourceInfo {
                    from: record.from.clone(),
                    origin: fresh.or_else(|| Some(origin.clone())),
                },
            )
        })();
        let _ = fs::remove_dir_all(&checkout);
        staged?
    } else {
        match &record.from {
            ImportedFrom::Path { path } if Path::new(path).exists() => crate::skill_import::stage_with(
                home,
                Path::new(path),
                SourceInfo { from: record.from.clone(), origin: None },
            )?,
            ImportedFrom::Path { path } => return Err(format!("{path} is gone; import the new version with Import skill.")),
            ImportedFrom::Dropped { name: file } => return Err(format!(
                "{name} came from a dropped file ({file}); drop the new version on this page to update it."
            )),
        }
    };
    if preview.name != name {
        let _ = crate::skill_import::discard_in(home, &preview.token);
        return Err(format!(
            "The source now holds a skill named {}, not {name}; import it with Import skill.",
            preview.name
        ));
    }
    // Both sides hashed alike: SKILL.md without Hollow's note, which is no change.
    let staged = hashes(&crate::skill_import::staged_skill_dir(
        home,
        &preview.token,
    )?)?;
    let changes = diff(&installed, &staged);
    let script = |path: &str| preview.files.iter().any(|f| f.script && f.path == path);
    let scripts = changes
        .added
        .iter()
        .chain(&changes.modified)
        .filter(|p| script(p))
        .cloned()
        .collect();
    // What each changed or added file now says (scripts first), so the
    // card shows the text, not only the names (review of #1050).
    let installed_dir = skill_dir(home, name)?;
    let staged_dir = crate::skill_import::staged_skill_dir(home, &preview.token)?;
    let mut order: Vec<&String> = changes.modified.iter().chain(&changes.added).collect();
    order.sort_by_key(|p| !script(p));
    let mut budget = DIFF_TOTAL_MAX;
    let mut diffs = Vec::new();
    for path in order {
        let old = changes
            .modified
            .contains(path)
            .then(|| diff_text(&installed_dir, path, path == "SKILL.md"));
        let new = diff_text(&staged_dir, path, false);
        let file = file_diff(path, script(path), old, new, budget.min(DIFF_FILE_MAX));
        budget = budget.saturating_sub(file.text.len());
        diffs.push(file);
    }
    preview.update = Some(UpdateDiff {
        changes,
        scripts_changed: scripts,
        diffs,
    });
    Ok(preview)
}

/// One changed or added file's text on the update card: a changed file's
/// lines from its first difference to its last (`-` before, `+` after, a
/// line of context each side), an added file's opening lines; cut at its
/// share of the budget. None for binary or oversized files (`listed only`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub script: bool,
    pub text: String,
    pub truncated: bool,
    /// Binary, unreadable or over 256 KB: listed, not shown.
    pub listed_only: bool,
}

/// `path` under `dir` as text (SKILL.md without Hollow's note when `bare`),
/// or None when it isn't readable text within [`DIFF_SOURCE_MAX`].
fn diff_text(dir: &Path, path: &str, bare: bool) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = contained(dir, path).ok()?;
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let handle = options.open(file).ok()?;
    let meta = handle.metadata().ok()?;
    if !meta.is_file() || meta.len() > DIFF_SOURCE_MAX {
        return None;
    }
    let mut bytes = Vec::new();
    handle.take(DIFF_SOURCE_MAX).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8(bytes)
        .ok()
        .filter(|t| !t.contains('\0'))?;
    Some(if bare { without_note(&text) } else { text })
}

/// The card text for one file: see [`FileDiff`]. `old` is None for an added
/// file and Some(None) for a changed file that isn't text.
pub(crate) fn file_diff(
    path: &str,
    script: bool,
    old: Option<Option<String>>,
    new: Option<String>,
    max: usize,
) -> FileDiff {
    let listed = |path: &str| FileDiff {
        path: path.to_string(),
        script,
        text: String::new(),
        truncated: false,
        listed_only: true,
    };
    let Some(new) = new else { return listed(path) };
    let lines: Vec<String> = match old {
        None => new.lines().map(|l| format!("+ {l}")).collect(),
        Some(None) => return listed(path),
        Some(Some(old)) => {
            let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
            let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
            let tail = a[head..]
                .iter()
                .rev()
                .zip(b[head..].iter().rev())
                .take_while(|(x, y)| x == y)
                .count();
            let from = head.saturating_sub(1);
            let mut out = Vec::new();
            if from > 0 {
                out.push(format!("… {from} unchanged lines"));
            }
            out.extend(a[from..head].iter().map(|l| format!("  {l}")));
            out.extend(a[head..a.len() - tail].iter().map(|l| format!("- {l}")));
            out.extend(b[head..b.len() - tail].iter().map(|l| format!("+ {l}")));
            let after = b.len() - tail;
            out.extend(
                b[after..(after + 1).min(b.len())]
                    .iter()
                    .map(|l| format!("  {l}")),
            );
            if b.len() > after + 1 {
                out.push(format!("… {} unchanged lines", b.len() - after - 1));
            }
            out
        }
    };
    let mut text = String::new();
    let mut truncated = false;
    for line in lines {
        if text.len() + line.len() + 1 > max {
            truncated = true;
            break;
        }
        text.push_str(&line);
        text.push('\n');
    }
    FileDiff {
        path: path.to_string(),
        script,
        text,
        truncated,
        listed_only: false,
    }
}

/// A shallow clone of `url` (https only, re-checked) into `into`. Isolated
/// from this Mac's git setup (review of #1050): no system or global config
/// (so no URL rewrites, protocol allowances or filter drivers), no `GIT_*`
/// environment, an empty HOME (no `~/.netrc` credentials), https as the only
/// transport (`GIT_ALLOW_PROTOCOL` overrides any config), no prompts,
/// credential helpers or hooks, and links checked out as plain files. Its
/// own process group, ended with everything it started at the time limit.
fn clone_https(url: &str, into: &Path) -> Result<(), String> {
    let url = safe_https(url).ok_or("Only an https:// origin is fetched.")?;
    let parent = into.parent().ok_or("No staging folder.")?;
    let home = parent.join(format!(
        "{}-home",
        into.file_name().and_then(|n| n.to_str()).unwrap_or("fetch")
    ));
    fs::create_dir_all(&home).map_err(|e| format!("mkdir: {e}"))?;
    let mut cmd = std::process::Command::new("git");
    cmd.env_clear()
        .envs(std::env::var_os("PATH").map(|p| ("PATH", p)))
        .envs(std::env::var_os("TMPDIR").map(|p| ("TMPDIR", p)))
        .env("HOME", &home)
        .env("LANG", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .env("SSH_ASKPASS", "/usr/bin/false")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "credential.helper=",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.symlinks=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args([
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
            "--",
            &url,
        ])
        .arg(into);
    let result = crate::process_capture::run_bounded_until(&mut cmd, CLONE_TIMEOUT, &|| false);
    let _ = fs::remove_dir_all(&home);
    match result {
        Ok((true, _, _)) => Ok(()),
        // Never git's own words: a redirect can name another URL, token
        // and all (review of #1050). One of a few fixed reasons instead.
        Ok((false, _, err)) => Err(format!("Couldn't fetch {url}: {}.", fetch_failure(&err))),
        Err(why) => Err(format!("Fetching {url} {why}; nothing changed.")),
    }
}

/// Why a clone failed, in fixed words picked from git's stderr, which is
/// never shown itself.
fn fetch_failure(stderr: &str) -> &'static str {
    let e = stderr.to_ascii_lowercase();
    if e.contains("not found") || e.contains(" 404") {
        "the repository wasn't found (it may be private or gone)"
    } else if e.contains("authentication")
        || e.contains("username")
        || e.contains(" 401")
        || e.contains(" 403")
    {
        "it asks for a sign-in, which Hollow never gives git"
    } else if e.contains("resolve host")
        || e.contains("connect")
        || e.contains("timed out")
        || e.contains("network")
    {
        "the network didn't answer"
    } else {
        "git couldn't fetch it"
    }
}

// ── The local note ──────────────────────────────────────────────────────────

/// Keep (or with None/blank, drop) `name`'s local note and write it into its
/// SKILL.md.
pub fn set_note(name: &str, note: Option<String>) -> Result<View, String> {
    let store = store_path()?;
    let home = home()?;
    set_note_at(&store, &home, name, note)?;
    view_at(&store, &home, name)
}

pub(crate) fn set_note_at(
    store: &Path,
    home: &Path,
    name: &str,
    note: Option<String>,
) -> Result<(), String> {
    let note = note
        .map(|n| n.trim().replace("\r\n", "\n"))
        .filter(|n| !n.is_empty());
    if let Some(n) = &note {
        if n.chars().count() > MAX_NOTE_CHARS {
            return Err(format!(
                "A local note holds at most {MAX_NOTE_CHARS} characters."
            ));
        }
        if n.contains("<!--") || n.contains("-->") || n.contains('\0') {
            return Err("A local note can't contain HTML comments.".into());
        }
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The same guard as Edit skill: a physical shared skill, no linked
    // folder on the way and no linked SKILL.md (review of #1050).
    let dir = skill_dir(home, name)?;
    write_note(&dir.join("SKILL.md"), note.as_deref())?;
    let mut data = load(store);
    match note {
        Some(n) => data.notes.insert(name.to_string(), n),
        None => data.notes.remove(name),
    };
    // The record hashes SKILL.md without the note, so it stays as it was.
    save(store, &data)
}

/// SKILL.md's text without Hollow's note block: exactly the text
/// [`with_note`] was given.
pub(crate) fn without_note(text: &str) -> String {
    match (text.find(NOTE_START), text.find(NOTE_END)) {
        (Some(a), Some(b)) if b > a => {
            let end = b + NOTE_END.len();
            let end = end
                + text[end..]
                    .chars()
                    .take(2)
                    .take_while(|c| *c == '\n')
                    .count();
            format!("{}{}", &text[..a], &text[end..])
        }
        _ => text.to_string(),
    }
}

/// `text` with `note` as its one block right after the frontmatter (or at
/// the top without one); None removes the block.
pub(crate) fn with_note(text: &str, note: Option<&str>) -> String {
    let bare = without_note(text);
    let Some(note) = note else { return bare };
    let after = frontmatter_end(&bare).unwrap_or(0);
    let (head, body) = bare.split_at(after);
    // A frontmatter that ends the file without a line break gets one, so the
    // block never runs into its closing `---` (review of #1050). That line
    // break stays when the note goes: the only byte removal doesn't restore.
    let gap = if head.is_empty() || head.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{head}{gap}{NOTE_START}\n{NOTE_HEADING}\n\n{note}\n{NOTE_END}\n\n{body}")
}

/// The byte just after the closing `---` line of a leading frontmatter.
fn frontmatter_end(text: &str) -> Option<usize> {
    let first = text.find('\n')?;
    if text[..first].trim_end() != "---" {
        return None;
    }
    let close = first + text[first..].find("\n---")? + 4;
    Some(
        text[close..]
            .find('\n')
            .map_or(text.len(), |n| close + n + 1),
    )
}

fn write_note(skill_md: &Path, note: Option<&str>) -> Result<(), String> {
    crate::skills::reject_symlink(skill_md)?;
    let text = read_skill_md(skill_md)?;
    let next = with_note(&text, note);
    if next != text {
        crate::vault::atomic_write(skill_md, next.as_bytes())
            .map_err(|e| format!("write SKILL.md: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str =
        "---\nname: pdf-tools\ndescription: Read PDFs.\n---\n\nUse scripts/fill.py.\n";

    fn install(home: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = home.join(".agents/skills").join(name);
        for (path, text) in files {
            let p = dir.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        dir
    }

    fn path_info(path: &str) -> SourceInfo {
        SourceInfo {
            from: ImportedFrom::Path { path: path.into() },
            origin: None,
        }
    }

    #[test]
    fn a_record_lists_installed_hashes_and_later_changes_read_as_changed_outside_hollow() {
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        let dir = install(
            home,
            "pdf-tools",
            &[("SKILL.md", SKILL), ("scripts/fill.py", "print(1)\n")],
        );
        record_install_at(&store, home, "pdf-tools", path_info("/tmp/pdf-tools.skill")).unwrap();
        let view = view_at(&store, home, "pdf-tools").unwrap();
        let record = view.record.clone().unwrap();
        assert_eq!(
            record.files.keys().collect::<Vec<_>>(),
            ["SKILL.md", "scripts/fill.py"]
        );
        assert_eq!(view.changed, Some(Changes::default()));
        fs::write(dir.join("scripts/fill.py"), "print(2)\n").unwrap();
        fs::write(dir.join("extra.md"), "x").unwrap();
        fs::remove_file(dir.join("SKILL.md")).unwrap();
        fs::write(dir.join("SKILL.md"), SKILL).unwrap();
        let changed = view_at(&store, home, "pdf-tools").unwrap().changed.unwrap();
        assert_eq!(
            changed,
            Changes {
                added: vec!["extra.md".into()],
                removed: vec![],
                modified: vec!["scripts/fill.py".into()]
            }
        );
        // Hollow's own edit (Edit skill writes SKILL.md) is taken as the new
        // baseline for SKILL.md only: the outside changes still show.
        fs::write(dir.join("SKILL.md"), SKILL.replace("Use", "Then use")).unwrap();
        refresh_at(&store, home, "pdf-tools").unwrap();
        assert_eq!(
            view_at(&store, home, "pdf-tools").unwrap().changed.unwrap(),
            Changes {
                added: vec!["extra.md".into()],
                removed: vec![],
                modified: vec!["scripts/fill.py".into()]
            }
        );
        fs::remove_file(dir.join("extra.md")).unwrap();
        fs::write(dir.join("scripts/fill.py"), "print(1)\n").unwrap();
        assert!(view_at(&store, home, "pdf-tools")
            .unwrap()
            .changed
            .unwrap()
            .is_empty());
        // The record lives in the store, never in the skill's folder.
        assert!(!fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("provenance")));
    }

    #[test]
    fn a_skill_without_a_record_is_not_imported_and_cant_be_checked() {
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        install(home, "made-here", &[("SKILL.md", SKILL)]);
        let view = view_at(&store, home, "made-here").unwrap();
        assert!(view.record.is_none() && view.changed.is_none() && !view.can_check);
        assert!(
            stage_update_at(&store, home, "made-here", |_, _| unreachable!())
                .unwrap_err()
                .contains("no import record")
        );
        assert!(view_at(&store, home, "../etc").is_err());
    }

    #[test]
    fn the_origin_comes_from_git_files_only_https_and_never_with_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let skill = repo.join("skills/hallmark");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(repo.join(".git/refs/heads")).unwrap();
        let sha = "6033bab2a805aaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(sha.len(), 40);
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(repo.join(".git/refs/heads/main"), format!("{sha}\n")).unwrap();
        fs::write(repo.join(".git/config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://github.com/Nutlope/hallmark.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n").unwrap();
        assert_eq!(
            origin_of(&skill),
            Some(Origin {
                url: "https://github.com/Nutlope/hallmark.git".into(),
                commit: Some(sha.into()),
                subdir: "skills/hallmark".into()
            })
        );
        // Packed refs, and a file inside the checkout.
        fs::remove_file(repo.join(".git/refs/heads/main")).unwrap();
        fs::write(
            repo.join(".git/packed-refs"),
            format!("# pack-refs\n{sha} refs/heads/main\n"),
        )
        .unwrap();
        fs::write(skill.join("SKILL.md"), SKILL).unwrap();
        assert_eq!(
            origin_of(&skill.join("SKILL.md")).unwrap().subdir,
            "skills/hallmark/SKILL.md"
        );
        for url in [
            "git@github.com:a/b.git",
            "ssh://github.com/a/b",
            "https://user:token@github.com/a/b",
            "file:///tmp/x",
            "ext::sh -c x",
            "https://a b",
        ] {
            fs::write(
                repo.join(".git/config"),
                format!("[remote \"origin\"]\n\turl = {url}\n"),
            )
            .unwrap();
            assert_eq!(origin_of(&skill), None, "{url}");
        }
        assert_eq!(origin_of(temp.path()), None);
    }

    #[test]
    fn the_local_note_is_one_block_after_the_frontmatter_and_survives_a_replacing_import() {
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        let dir = install(home, "pdf-tools", &[("SKILL.md", SKILL)]);
        set_note_at(
            &store,
            home,
            "pdf-tools",
            Some("Pinned to 0.8.169; never run `upgrade`.".into()),
        )
        .unwrap();
        let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(text.starts_with("---\nname: pdf-tools\ndescription: Read PDFs.\n---\n<!-- hollow:local-note -->\n## Local note"));
        assert!(text.contains("Pinned to 0.8.169") && text.ends_with("Use scripts/fill.py.\n"));
        assert_eq!(without_note(&text), SKILL);
        // Saving it again doesn't stack a second block.
        set_note_at(&store, home, "pdf-tools", Some("Second note.".into())).unwrap();
        let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert_eq!(text.matches(NOTE_START).count(), 1);
        assert!(!text.contains("Pinned"));
        // An import that replaces the folder writes it again, and the record
        // counts the note as Hollow's own.
        fs::write(
            dir.join("SKILL.md"),
            SKILL.replace("Read PDFs.", "Read PDFs, v2."),
        )
        .unwrap();
        record_install_at(&store, home, "pdf-tools", path_info("/tmp/new.skill")).unwrap();
        let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(text.contains("Second note.") && text.contains("v2"));
        assert!(view_at(&store, home, "pdf-tools")
            .unwrap()
            .changed
            .unwrap()
            .is_empty());
        // Removing it leaves the original text.
        set_note_at(&store, home, "pdf-tools", None).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            SKILL.replace("Read PDFs.", "Read PDFs, v2.")
        );
        for bad in ["<!-- x -->", &"n".repeat(MAX_NOTE_CHARS + 1)] {
            assert!(set_note_at(&store, home, "pdf-tools", Some(bad.to_string())).is_err());
        }
        forget_at(&store, "pdf-tools").unwrap();
        assert!(view_at(&store, home, "pdf-tools").unwrap().record.is_none());
    }

    #[test]
    fn check_for_update_stages_the_recorded_path_or_origin_with_the_differences() {
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        let source = temp.path().join("src/pdf-tools");
        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::write(source.join("SKILL.md"), SKILL).unwrap();
        fs::write(source.join("scripts/fill.py"), "print(1)\n").unwrap();
        install(
            home,
            "pdf-tools",
            &[
                ("SKILL.md", SKILL),
                ("scripts/fill.py", "print(1)\n"),
                ("old.md", "x"),
            ],
        );
        record_install_at(
            &store,
            home,
            "pdf-tools",
            path_info(&source.to_string_lossy()),
        )
        .unwrap();
        fs::write(source.join("scripts/fill.py"), "print(2)\n").unwrap();
        fs::write(source.join("new.md"), "y").unwrap();
        let preview = stage_update_at(&store, home, "pdf-tools", |_, _| unreachable!()).unwrap();
        let update = preview.update.clone().unwrap();
        assert_eq!(
            update.changes,
            Changes {
                added: vec!["new.md".into()],
                removed: vec!["old.md".into()],
                modified: vec!["scripts/fill.py".into()]
            }
        );
        assert_eq!(update.scripts_changed, vec!["scripts/fill.py".to_string()]);
        assert!(preview.existing.is_some());
        // Nothing replaced the installed copy.
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/pdf-tools/scripts/fill.py")).unwrap(),
            "print(1)\n"
        );
        crate::skill_import::discard_in(home, &preview.token).unwrap();
        // An origin is cloned (here by a stand-in) and staged from its subfolder.
        let mut data = load(&store);
        data.skills.get_mut("pdf-tools").unwrap().origin = Some(Origin {
            url: "https://example.com/r.git".into(),
            commit: None,
            subdir: "skills/pdf-tools".into(),
        });
        save(&store, &data).unwrap();
        let preview = stage_update_at(&store, home, "pdf-tools", |url, into| {
            assert_eq!(url, "https://example.com/r.git");
            let dir = into.join("skills/pdf-tools");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("SKILL.md"), SKILL).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(
            preview.update.unwrap().changes.removed,
            vec!["old.md".to_string(), "scripts/fill.py".to_string()]
        );
        // The clone is gone once staged.
        let scratch: Vec<_> = fs::read_dir(home.join(".agents/.skill-imports"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("fetch-"))
            .collect();
        assert!(scratch.is_empty());
        // A source that now holds another skill is refused.
        fs::write(source.join("SKILL.md"), SKILL.replace("pdf-tools", "other")).unwrap();
        let mut data = load(&store);
        data.skills.get_mut("pdf-tools").unwrap().origin = None;
        save(&store, &data).unwrap();
        assert!(
            stage_update_at(&store, home, "pdf-tools", |_, _| unreachable!())
                .unwrap_err()
                .contains("named other")
        );
    }

    #[test]
    fn only_a_plain_https_url_is_ever_cloned() {
        for bad in [
            "http://a/b",
            "file:///x",
            "https://u:p@h/x",
            "https://",
            "ext::x",
            "https://a\nb",
            "https://h/r.git?access_token=secret",
            "https://h/r.git#token",
        ] {
            assert!(safe_https(bad).is_none(), "{bad}");
        }
        assert!(clone_https("file:///tmp/x", Path::new("/tmp/never")).is_err());
        // git's own words (a redirect can carry a token) are never shown.
        let leaky = "fatal: unable to access 'https://h/r.git?token=SECRET': The requested URL returned error: 404";
        assert!(!fetch_failure(leaky).contains("SECRET"));
        assert_eq!(
            fetch_failure(leaky),
            "the repository wasn't found (it may be private or gone)"
        );
        assert_eq!(
            fetch_failure(
                "fatal: could not read Username for 'https://h': terminal prompts disabled"
            ),
            "it asks for a sign-in, which Hollow never gives git"
        );
        assert_eq!(
            fetch_failure("fatal: unable to access: Could not resolve host: h"),
            "the network didn't answer"
        );
        assert_eq!(fetch_failure("something else"), "git couldn't fetch it");
    }

    /// Review of #1050: `.git` files are read only as plain files inside the
    /// checkout, never through a link, from a FIFO or past their cap.
    #[test]
    fn git_files_are_read_only_as_plain_files_inside_the_checkout() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = https://example.com/r.git\n",
        )
        .unwrap();
        let sha = "a".repeat(40);
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), format!("{sha}\n")).unwrap();
        assert_eq!(
            origin_of(&repo).unwrap().commit.as_deref(),
            Some(sha.as_str())
        );
        // A linked ref (out of the checkout) is never followed.
        let outside = temp.path().join("outside");
        fs::write(&outside, format!("{}\n", "b".repeat(40))).unwrap();
        fs::remove_file(git.join("refs/heads/main")).unwrap();
        symlink(&outside, git.join("refs/heads/main")).unwrap();
        assert_eq!(origin_of(&repo).unwrap().commit, None);
        // Nor a linked folder on the way.
        fs::remove_file(git.join("refs/heads/main")).unwrap();
        fs::remove_dir(git.join("refs/heads")).unwrap();
        fs::create_dir_all(temp.path().join("elsewhere")).unwrap();
        fs::write(temp.path().join("elsewhere/main"), format!("{sha}\n")).unwrap();
        symlink(temp.path().join("elsewhere"), git.join("refs/heads")).unwrap();
        assert_eq!(origin_of(&repo).unwrap().commit, None);
        // A linked config: no origin at all.
        fs::rename(git.join("config"), temp.path().join("config")).unwrap();
        symlink(temp.path().join("config"), git.join("config")).unwrap();
        assert!(origin_of(&repo).is_none());
        // A FIFO answers at once (never blocks) and reads as nothing.
        fs::remove_file(git.join("config")).unwrap();
        let fifo = std::ffi::CString::new(git.join("config").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(origin_of(&repo).is_none());
        // An oversized file is refused.
        fs::remove_file(git.join("config")).unwrap();
        let big = format!(
            "[remote \"origin\"]\n\turl = https://example.com/r.git\n{}",
            "#".repeat(GIT_FILE_MAX as usize)
        );
        fs::write(git.join("config"), big).unwrap();
        assert!(origin_of(&repo).is_none());
    }

    /// Review of #1050: an upstream change that turns a folder on the
    /// recorded path into a link can't reach outside the clone.
    #[test]
    fn update_staging_never_follows_a_link_out_of_the_clone() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        install(home, "pdf-tools", &[("SKILL.md", SKILL)]);
        record_install_at(&store, home, "pdf-tools", path_info("/tmp/x")).unwrap();
        let mut data = load(&store);
        data.skills.get_mut("pdf-tools").unwrap().origin = Some(Origin {
            url: "https://example.com/r.git".into(),
            commit: None,
            subdir: "skills/pdf-tools".into(),
        });
        save(&store, &data).unwrap();
        let secret = temp.path().join("secret/pdf-tools");
        fs::create_dir_all(&secret).unwrap();
        fs::write(secret.join("SKILL.md"), SKILL).unwrap();
        fs::write(secret.join("id_rsa"), "PRIVATE").unwrap();
        let error = stage_update_at(&store, home, "pdf-tools", |_, into| {
            fs::create_dir_all(into).unwrap();
            symlink(temp.path().join("secret"), into.join("skills")).unwrap();
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.contains("isn't a plain folder inside the repository"),
            "{error}"
        );
        assert!(contained(temp.path(), "a/../b").is_err());
        assert!(contained(temp.path(), "secret//pdf-tools").is_err());
        assert_eq!(
            contained(temp.path(), "secret/pdf-tools").unwrap(),
            temp.path().join("secret/pdf-tools")
        );
    }

    /// Review of #1050: a note is never a change, so saving one keeps an
    /// outside change to a script visible; and a note only ever goes into a
    /// physical shared skill's own SKILL.md.
    #[test]
    fn a_note_keeps_outside_changes_visible_and_writes_only_a_managed_skill_md() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        let dir = install(
            home,
            "pdf-tools",
            &[("SKILL.md", SKILL), ("scripts/fill.py", "print(1)\n")],
        );
        record_install_at(&store, home, "pdf-tools", path_info("/tmp/x")).unwrap();
        fs::write(dir.join("scripts/fill.py"), "print('changed')\n").unwrap();
        set_note_at(&store, home, "pdf-tools", Some("Local setup".into())).unwrap();
        assert_eq!(
            view_at(&store, home, "pdf-tools")
                .unwrap()
                .changed
                .unwrap()
                .modified,
            vec!["scripts/fill.py".to_string()],
            "the note didn't hide the outside change, and isn't one itself"
        );
        // A linked SKILL.md is refused, and so is a linked skill folder.
        let target = temp.path().join("elsewhere.md");
        fs::write(&target, SKILL).unwrap();
        fs::remove_file(dir.join("SKILL.md")).unwrap();
        symlink(&target, dir.join("SKILL.md")).unwrap();
        assert!(set_note_at(&store, home, "pdf-tools", Some("x".into())).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), SKILL);
        let provider = temp.path().join("provider/skill");
        fs::create_dir_all(&provider).unwrap();
        fs::write(provider.join("SKILL.md"), SKILL).unwrap();
        symlink(&provider, home.join(".agents/skills/linked")).unwrap();
        assert!(set_note_at(&store, home, "linked", Some("x".into())).is_err());
        assert_eq!(
            fs::read_to_string(provider.join("SKILL.md")).unwrap(),
            SKILL
        );
    }

    #[test]
    fn a_frontmatter_that_ends_the_file_keeps_its_closing_line() {
        let text = "---\nname: x\n---";
        let with = with_note(text, Some("Note"));
        assert!(
            with.starts_with("---\nname: x\n---\n<!-- hollow:local-note -->"),
            "{with}"
        );
        assert_eq!(without_note(&with), format!("{text}\n"));
        // The line break that stays is no change outside Hollow.
        let temp = tempfile::tempdir().unwrap();
        let (home, store) = (temp.path(), temp.path().join("records.json"));
        install(home, "x", &[("SKILL.md", text)]);
        record_install_at(&store, home, "x", path_info("/tmp/x")).unwrap();
        set_note_at(&store, home, "x", Some("Note".into())).unwrap();
        set_note_at(&store, home, "x", None).unwrap();
        assert!(view_at(&store, home, "x")
            .unwrap()
            .changed
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_update_card_shows_changed_lines_with_context_and_added_files() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nb\nC\nd\ne\nf\n";
        let d = file_diff(
            "run.sh",
            true,
            Some(Some(old.into())),
            Some(new.into()),
            4_000,
        );
        assert_eq!(
            d.text,
            "… 1 unchanged lines\n  b\n- c\n- d\n- e\n+ C\n+ d\n+ e\n+ f\n"
        );
        assert!(!d.truncated && !d.listed_only && d.script);
        let d = file_diff(
            "one.md",
            false,
            Some(Some("x\ny\n".into())),
            Some("x\nz\n".into()),
            4_000,
        );
        assert_eq!(d.text, "  x\n- y\n+ z\n");
        let added = file_diff("new.md", false, None, Some("hello\nworld\n".into()), 4_000);
        assert_eq!(added.text, "+ hello\n+ world\n");
        let cut = file_diff("big.md", false, None, Some("line\n".repeat(100)), 30);
        assert!(cut.truncated && cut.text.len() <= 30);
        assert!(file_diff("bin", false, Some(None), Some("x".into()), 100).listed_only);
        assert!(file_diff("bin", false, None, None, 100).listed_only);
    }
}
