//! Importing an existing skill into the shared library (`~/.agents/skills`).
//!
//! The Skills page's Import accepts Claude's `.skill` export (a zip holding a
//! folder with `SKILL.md`, `scripts/`, `references/`, `assets/`), any `.zip`,
//! a skill folder, or a bare `SKILL.md`. A file dropped on the page arrives as
//! bytes (the webview's drop gives no path). A dropped folder is refused in the
//! page: WebKit reads through links there, so a folder goes through the picker.
//!
//! Two steps, so the person sees exactly what will be installed:
//!
//! 1. [`stage_renamed`] reads the source, checks every rule below and writes the
//!    result to `~/.agents/.skill-imports/<token>/skill/`, outside the
//!    skills folder, so nothing lists it yet. The [`ImportPreview`] it
//!    returns is built from those staged files.
//! 2. [`commit`] first claims the staged import (renames `<token>` to
//!    `<token>.claim`, so a Cancel or discard racing it finds nothing), then
//!    moves it into `~/.agents/skills/<name>` with one rename (staging sits
//!    beside `skills/`, on the same disk), handling a name clash as the person
//!    chose; [`discard`] drops an unclaimed one.
//!
//! Agents' `skills.import_skill` stages on its confirmation card (which lists
//! the files and scripts) and commits that same token once allowed.
//!
//! Rules (each has a regression test below):
//! - no absolute paths, `..` or `.` parts, backslashes or NUL in a path;
//! - no symbolic links (zip entries or folder entries), no special files;
//! - at most [`MAX_FILES`] files, [`MAX_TOTAL_BYTES`] in all,
//!   [`MAX_FILE_BYTES`] per file and [`MAX_SKILL_MD_BYTES`] for SKILL.md,
//!   counted on the bytes actually read, never on a zip header's claim;
//! - two paths that differ only by case (one file on macOS) are refused;
//! - `SKILL.md` sits at the top or inside one top-level folder, with a
//!   frontmatter `name` (validated like [`crate::skills::write_user_skill`])
//!   and a non-empty `description`;
//! - Finder's `__MACOSX/` and `.DS_Store`, and a folder's `.git/`, are
//!   skipped and listed as skipped.

use crate::skills::{self, Skill};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Files in one skill.
pub(crate) const MAX_FILES: usize = 1_000;
/// Bytes in one skill, uncompressed, and in the file or archive read.
pub(crate) const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
/// Bytes in one file of a skill.
pub(crate) const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// Bytes in SKILL.md itself.
pub(crate) const MAX_SKILL_MD_BYTES: u64 = 1024 * 1024;
/// Zip entries (files and folders) looked at before giving up.
const MAX_ZIP_ENTRIES: usize = 5_000;
/// Folder depth a skill folder may have.
const MAX_DEPTH: usize = 32;
/// The staging folder, under `~/.agents`.
const STAGING: &str = ".skill-imports";
/// A staged import nobody committed or discarded is removed after this.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// File extensions that run as code when an agent follows the skill.
const SCRIPT_EXTENSIONS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "csh",
    "ksh",
    "command",
    "py",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "tsx",
    "rb",
    "pl",
    "php",
    "r",
    "tcl",
    "awk",
    "ps1",
    "psm1",
    "bat",
    "cmd",
    "vbs",
    "wsf",
    "applescript",
    "scpt",
    "swift",
    "go",
    "lua",
    "groovy",
    "kts",
    "jar",
    "exe",
    "bin",
    "dylib",
    "so",
    "app",
];

/// Where an import comes from.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportSource {
    /// A file (`.skill`, `.zip`, `.md`) or a folder on disk, as a picker
    /// returns it.
    Path { path: String },
    /// One dropped file: its name and its bytes in base64.
    Bytes { name: String, data: String },
}

/// What [`commit`] does when `~/.agents/skills/<name>` already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnClash {
    /// Refuse.
    Cancel,
    /// Swap the existing folder out for the import.
    Replace,
    /// Install under the first free `<name>-2`, `<name>-3`, … and rewrite
    /// the frontmatter `name` to match.
    KeepBoth,
}

impl OnClash {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("cancel") {
            "cancel" => Ok(OnClash::Cancel),
            "replace" => Ok(OnClash::Replace),
            "keep-both" => Ok(OnClash::KeepBoth),
            other => Err(format!(
                "onClash must be cancel, replace or keep-both, not {other}"
            )),
        }
    }
}

/// One file of a staged import.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFile {
    pub path: String,
    pub size: u64,
    /// Runs as code: under `scripts/`, a script extension, or executable.
    pub script: bool,
}

/// What [`stage_renamed`] staged, for the person to check before [`commit`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub token: String,
    pub name: String,
    pub description: String,
    pub files: Vec<ImportFile>,
    pub total_bytes: u64,
    pub has_scripts: bool,
    /// Entries left out (`__MACOSX/`, `.DS_Store`, `.git/`).
    pub skipped: Vec<String>,
    /// The folder the import was found in, when it was one level down.
    pub folder: Option<String>,
    /// `~/.agents/skills/<name>/SKILL.md` when that name is taken.
    pub existing: Option<String>,
    /// Whether Replace can swap the existing folder (not a link).
    pub can_replace: bool,
    /// The name Keep both would use.
    pub keep_both_name: String,
    /// Check for update (repo ideas 2026-10-27 #2): how this staged copy
    /// differs from the installed one. Absent for an ordinary import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<crate::skill_provenance::UpdateDiff>,
    /// SD2: the name SKILL.md came with, when the caller asked for a rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
    /// SD2: the files whose references to the old name were rewritten.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rewritten: Vec<RewrittenFile>,
}

/// SD2: one file of a renamed import whose references to the old name were
/// rewritten, and how many.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RewrittenFile {
    pub path: String,
    pub count: usize,
}

/// What [`commit`] installed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub skill: Skill,
    pub files: usize,
    pub has_scripts: bool,
    /// An existing skill of that name was replaced.
    pub replaced: bool,
    /// Installed under a new name (Keep both): the name it came with.
    pub renamed_from: Option<String>,
}

/// Step 1 for the Skills page, with SD2's optional `rename_to`.
pub fn stage_renamed(
    source: ImportSource,
    rename_to: Option<&str>,
) -> Result<ImportPreview, String> {
    stage_in_as(&home()?, source, rename_to)
}

pub fn commit(token: &str, on_clash: OnClash) -> Result<ImportOutcome, String> {
    let _held = skills::SKILL_WRITE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    commit_in(&home()?, token, on_clash)
}

pub fn discard(token: &str) -> Result<(), String> {
    discard_in(&home()?, token)
}

/// Agents' `skills.import_skill` card: stage `path` and refuse up front what
/// the chosen `on_clash` can't install, so the card lists the staged files.
pub fn stage_for_agent(
    path: &str,
    on_clash: OnClash,
    rename_to: Option<&str>,
) -> Result<ImportPreview, String> {
    stage_for_agent_in(&home()?, path, on_clash, rename_to)
}

pub(crate) fn stage_for_agent_in(
    home: &Path,
    path: &str,
    on_clash: OnClash,
    rename_to: Option<&str>,
) -> Result<ImportPreview, String> {
    let preview = stage_in_as(
        home,
        ImportSource::Path {
            path: path.to_string(),
        },
        rename_to,
    )?;
    let refused = match (&preview.existing, on_clash) {
        (Some(_), OnClash::Cancel) => Some(format!(
            "A shared skill named {} already exists; nothing was installed. Pass onClash replace or keep-both.",
            preview.name
        )),
        (Some(_), OnClash::Replace) if !preview.can_replace => Some(format!(
            "~/.agents/skills/{} is a link or a file, not a skill folder; Hollow won't replace it.",
            preview.name
        )),
        _ => None,
    };
    if let Some(refused) = refused {
        let _ = discard_in(home, &preview.token);
        return Err(refused);
    }
    Ok(preview)
}

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "no home dir".to_string())
}

// ── Collecting ──────────────────────────────────────────────────────────────

struct Entry {
    parts: Vec<String>,
    data: Vec<u8>,
    executable: bool,
}

#[derive(Default)]
struct Collected {
    entries: Vec<Entry>,
    skipped: Vec<String>,
    total: u64,
}

impl Collected {
    fn add(&mut self, parts: Vec<String>, data: Vec<u8>, executable: bool) -> Result<(), String> {
        let shown = parts.join("/");
        if self.entries.len() >= MAX_FILES {
            return Err(format!(
                "This skill has more than {MAX_FILES} files; Hollow imports at most {MAX_FILES}."
            ));
        }
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(too_big_file(&shown));
        }
        self.total += data.len() as u64;
        if self.total > MAX_TOTAL_BYTES {
            return Err(too_big_total());
        }
        self.entries.push(Entry {
            parts,
            data,
            executable,
        });
        Ok(())
    }
}

fn too_big_file(path: &str) -> String {
    format!(
        "{path} is larger than {} MB, the most Hollow imports for one file.",
        MAX_FILE_BYTES / (1024 * 1024)
    )
}

fn too_big_total() -> String {
    format!(
        "This skill is larger than {} MB, the most Hollow imports.",
        MAX_TOTAL_BYTES / (1024 * 1024)
    )
}

/// A relative path's parts, or why it is refused.
fn safe_parts(raw: &str) -> Result<Vec<String>, String> {
    let refuse = |why: &str| Err(format!("Refused {raw:?}: {why}."));
    if raw.contains('\0') {
        return refuse("the path contains a NUL byte");
    }
    if raw.contains('\\') {
        return refuse("the path uses backslashes");
    }
    if raw.starts_with('/') {
        return refuse("the path is absolute");
    }
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed.is_empty() {
        return refuse("the path is empty");
    }
    let mut parts = Vec::new();
    for (i, part) in trimmed.split('/').enumerate() {
        match part {
            "" => return refuse("the path has an empty part"),
            "." | ".." => return refuse("the path leaves its folder"),
            _ => {}
        }
        if i == 0 && part.len() == 2 && part.ends_with(':') {
            return refuse("the path is absolute");
        }
        parts.push(part.to_string());
    }
    Ok(parts)
}

/// Finder and git leftovers that never belong in a skill.
fn is_junk(parts: &[String]) -> bool {
    parts.iter().any(|p| p == "__MACOSX" || p == ".git")
        || parts.last().is_some_and(|p| p == ".DS_Store")
}

fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06")
}

fn is_markdown_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".md")
}

/// A single file: a zip (by its bytes, whatever the extension) or Markdown.
fn collect_file(name: &str, data: Vec<u8>) -> Result<Collected, String> {
    if data.len() as u64 > MAX_TOTAL_BYTES {
        return Err(too_big_total());
    }
    if is_zip(&data) {
        return collect_zip(&data);
    }
    if is_markdown_name(name) {
        if data.len() as u64 > MAX_SKILL_MD_BYTES {
            return Err(too_big_skill_md());
        }
        let mut out = Collected::default();
        out.add(vec!["SKILL.md".into()], data, false)?;
        return Ok(out);
    }
    Err(format!(
        "{name} isn't a skill: Hollow imports a .skill or .zip file, a skill folder, or a SKILL.md."
    ))
}

fn too_big_skill_md() -> String {
    format!(
        "SKILL.md is larger than {} MB.",
        MAX_SKILL_MD_BYTES / (1024 * 1024)
    )
}

fn collect_zip(data: &[u8]) -> Result<Collected, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| format!("Couldn't read the archive: {e}"))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "This archive has {} entries; Hollow imports at most {MAX_FILES} files.",
            zip.len()
        ));
    }
    let mut out = Collected::default();
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("Couldn't read entry {} of the archive: {e}", i + 1))?;
        let name = file.name().to_string();
        let parts = safe_parts(&name)?;
        if is_junk(&parts) {
            out.skipped.push(name);
            continue;
        }
        if file.is_symlink() {
            return Err(format!(
                "Refused {name:?}: it is a symbolic link, and a skill can't contain links."
            ));
        }
        if file.is_dir() {
            continue;
        }
        if let Some(mode) = file.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o100000 {
                return Err(format!("Refused {name:?}: it isn't a regular file."));
            }
        }
        if file.size() > MAX_FILE_BYTES {
            return Err(too_big_file(&name));
        }
        // The header's size is a claim; count what decompresses.
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Couldn't unpack {name:?}: {e}"))?;
        let executable = file.unix_mode().is_some_and(|m| m & 0o111 != 0);
        out.add(parts, bytes, executable)?;
    }
    Ok(out)
}

fn collect_dir(root: &Path) -> Result<Collected, String> {
    let mut out = Collected::default();
    walk(root, &mut Vec::new(), &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, prefix: &mut Vec<String>, out: &mut Collected) -> Result<(), String> {
    if prefix.len() > MAX_DEPTH {
        return Err(format!(
            "{} is nested more than {MAX_DEPTH} folders deep.",
            prefix.join("/")
        ));
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("Couldn't read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(format!(
                "Refused {}: its name isn't valid text.",
                entry.path().display()
            ));
        };
        prefix.push(name);
        let shown = prefix.join("/");
        let meta = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("Couldn't read {shown}: {e}"))?;
        if is_junk(prefix) {
            out.skipped.push(if meta.is_dir() {
                format!("{shown}/")
            } else {
                shown
            });
        } else if meta.file_type().is_symlink() {
            return Err(format!(
                "Refused {shown:?}: it is a symbolic link, and a skill can't contain links."
            ));
        } else if meta.is_dir() {
            walk(&entry.path(), prefix, out)?;
        } else if meta.is_file() {
            if meta.len() > MAX_FILE_BYTES {
                return Err(too_big_file(&shown));
            }
            if out.total + meta.len() > MAX_TOTAL_BYTES {
                return Err(too_big_total());
            }
            let mut bytes = Vec::new();
            open_no_follow(&entry.path())
                .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
                .map_err(|e| format!("Couldn't read {shown}: {e}"))?;
            out.add(prefix.clone(), bytes, executable(&meta))?;
        } else {
            return Err(format!("Refused {shown:?}: it isn't a regular file."));
        }
        prefix.pop();
    }
    Ok(())
}

/// Open a file, failing if it was swapped for a link after it was checked.
fn open_no_follow(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(unix)]
fn executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_meta: &fs::Metadata) -> bool {
    false
}

fn decode(label: &str, data: &str) -> Result<Vec<u8>, String> {
    // base64 is 4 characters per 3 bytes: refuse before decoding.
    if data.len() as u64 > MAX_TOTAL_BYTES / 3 * 4 + 8 {
        return Err(too_big_total());
    }
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| format!("Couldn't read {label}: {e}"))
}

fn collect(source: ImportSource) -> Result<Collected, String> {
    match source {
        ImportSource::Path { path } => {
            let path = PathBuf::from(path);
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "{label} is a symbolic link; choose the skill itself."
                ));
            }
            if meta.is_dir() {
                return collect_dir(&path);
            }
            if !meta.is_file() {
                return Err(format!("{label} isn't a file or a folder."));
            }
            if meta.len() > MAX_TOTAL_BYTES {
                return Err(too_big_total());
            }
            let data = fs::read(&path).map_err(|e| format!("Couldn't read {label}: {e}"))?;
            collect_file(&label, data)
        }
        ImportSource::Bytes { name, data } => {
            let data = decode(&name, &data)?;
            collect_file(&name, data)
        }
    }
}

// ── Checking ────────────────────────────────────────────────────────────────

/// Put `SKILL.md` at the root: as is, or by dropping the one top-level
/// folder every file sits in. Returns that folder's name when dropped.
fn root_layout(entries: &mut [Entry]) -> Result<Option<String>, String> {
    if entries.iter().any(|e| e.parts == ["SKILL.md"]) {
        return Ok(None);
    }
    let top = entries.first().map(|e| e.parts[0].clone());
    let single = top.as_ref().is_some_and(|top| {
        entries
            .iter()
            .all(|e| e.parts.len() >= 2 && &e.parts[0] == top)
    }) && entries
        .iter()
        .any(|e| e.parts.len() == 2 && e.parts[1] == "SKILL.md");
    if !single {
        return Err(
            "No SKILL.md found: a skill needs SKILL.md at its top level or inside one top-level folder."
                .into(),
        );
    }
    for entry in entries.iter_mut() {
        entry.parts.remove(0);
    }
    Ok(top)
}

/// Refuse two files that would land on one path (macOS folds case), and a
/// file where another needs a folder.
fn check_collisions(entries: &[Entry]) -> Result<(), String> {
    let mut files = HashSet::new();
    let mut folders = HashSet::new();
    for entry in entries {
        let lower: Vec<String> = entry.parts.iter().map(|p| p.to_lowercase()).collect();
        for depth in 1..lower.len() {
            folders.insert(lower[..depth].join("/"));
        }
        if !files.insert(lower.join("/")) {
            return Err(format!(
                "Refused {:?}: another file has the same name (names differing only by case are one file on this Mac).",
                entry.parts.join("/")
            ));
        }
    }
    if let Some(clash) = files.intersection(&folders).next() {
        return Err(format!(
            "Refused {clash:?}: it is both a file and a folder."
        ));
    }
    Ok(())
}

/// SKILL.md's (name, description), or why it can't be imported.
fn check_skill_md(data: &[u8]) -> Result<(String, String), String> {
    if data.len() as u64 > MAX_SKILL_MD_BYTES {
        return Err(too_big_skill_md());
    }
    let text = std::str::from_utf8(data).map_err(|_| "SKILL.md isn't UTF-8 text.".to_string())?;
    let (description, _, name) = skills::parse_skill_meta(text);
    let name = name.unwrap_or_default();
    if name.trim().is_empty() {
        return Err(
            "SKILL.md needs frontmatter with a `name` (between --- lines at the top).".into(),
        );
    }
    if !skills::valid_skill_name(name.trim()) {
        return Err(format!(
            "SKILL.md's name {name:?} can only use letters, numbers, dashes and underscores."
        ));
    }
    if description.trim().is_empty() {
        return Err("SKILL.md needs a `description` in its frontmatter: it tells agents when to use the skill.".into());
    }
    Ok((name.trim().to_string(), description.trim().to_string()))
}

/// Refuse a text file that hides characters an agent would read but a person
/// would not: zero-width, bidi-control and Unicode tag characters. Binary
/// files (not UTF-8, or holding a NUL) are skipped. The message names the
/// class, the file and the line and column, never the text itself. The scan
/// is `memory_guard`'s, so the two stay one definition.
fn check_hidden_text(entries: &[Entry]) -> Result<(), String> {
    for entry in entries {
        // A file with a NUL is binary. Anything else is read as text with bad
        // bytes replaced, so one stray byte can't shield a payload.
        if entry.data.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&entry.data);
        let text = text.as_ref();
        // A byte-order mark at the very start is an encoding marker, not
        // hidden text.
        let (text, offset) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => (rest, 1),
            None => (text, 0),
        };
        let found = crate::memory_guard::invisible(text)
            .into_iter()
            .filter(|f| {
                matches!(
                    f.rule.as_str(),
                    "zero_width" | "bidi_control" | "tag_character"
                )
            })
            .min_by_key(|f| f.at);
        if let Some(f) = found {
            let at = f.at;
            let before: Vec<char> = text.chars().take(at).collect();
            let line = before.iter().filter(|&&c| c == '\n').count() + 1;
            let col = before.iter().rev().take_while(|&&c| c != '\n').count() + 1;
            let class = match f.rule.as_str() {
                "zero_width" => "a zero-width character",
                "bidi_control" => "a bidi-control character",
                _ => "a Unicode tag character",
            };
            // The BOM sits on line 1 only, so it shifts only that line's column.
            let col = if line == 1 { col + offset } else { col };
            // A path can hide characters too: show them as `?`.
            let path: String = entry
                .parts
                .join("/")
                .chars()
                .map(|c| {
                    if crate::memory_guard::invisible(&c.to_string()).is_empty() {
                        c
                    } else {
                        '?'
                    }
                })
                .collect();
            return Err(format!(
                "Refused: {path} holds {class} at line {line}, column {col} (character {}). Hidden text can carry instructions an agent follows but a person cannot see; remove it and import again.",
                at + offset + 1,
            ));
        }
    }
    Ok(())
}

fn is_script(parts: &[String], executable: bool) -> bool {
    executable
        || parts[..parts.len().saturating_sub(1)]
            .iter()
            .any(|p| p == "scripts" || p == "bin")
        || parts
            .last()
            .and_then(|p| p.rsplit_once('.'))
            .is_some_and(|(_, ext)| SCRIPT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

// ── Staging and installing ──────────────────────────────────────────────────

/// `~/.agents/.skill-imports`, a real folder (created).
fn staging_root(home: &Path) -> Result<PathBuf, String> {
    let agents = home.join(".agents");
    let root = agents.join(STAGING);
    for path in [&agents, &root] {
        skills::reject_symlink(path)?;
        fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
    }
    Ok(root)
}

/// `~/.agents/skills`, a real folder (created).
fn skills_root(home: &Path) -> Result<PathBuf, String> {
    let agents = home.join(".agents");
    let root = agents.join("skills");
    for path in [&agents, &root] {
        skills::reject_symlink(path)?;
        fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
    }
    Ok(root)
}

fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn new_token() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Remove staged imports older than [`STALE_AFTER`].
fn sweep_stale(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        // An import, its claim, and a Check for update clone with its empty
        // HOME (`fetch-<token>`, `fetch-<token>-home`) left by an interrupted run.
        let fetch = name
            .strip_prefix("fetch-")
            .map(|rest| rest.strip_suffix("-home").unwrap_or(rest))
            .is_some_and(valid_token);
        let ours =
            valid_token(&name) || name.strip_suffix(".claim").is_some_and(valid_token) || fetch;
        if ours && old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn write_staged(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    fs::create_dir(dir).map_err(|e| format!("mkdir: {e}"))?;
    for entry in entries {
        let mut path = dir.to_path_buf();
        path.extend(&entry.parts);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if entry.executable { 0o755 } else { 0o644 });
        }
        use std::io::Write;
        options
            .open(&path)
            .and_then(|mut f| f.write_all(&entry.data))
            .map_err(|e| format!("Couldn't stage {}: {e}", entry.parts.join("/")))?;
    }
    Ok(())
}

/// The first `<name>-N` (N from 2) free in `skills`.
fn keep_both_name(skills: &Path, name: &str) -> Result<String, String> {
    (2..1000)
        .map(|n| format!("{name}-{n}"))
        .find(|candidate| fs::symlink_metadata(skills.join(candidate)).is_err())
        .ok_or_else(|| format!("No free name left for another copy of {name}."))
}

/// What an import came from, for its provenance record: the picked path and
/// the git origin around it, or a dropped file's name.
fn source_info(source: &ImportSource) -> crate::skill_provenance::SourceInfo {
    use crate::skill_provenance::{origin_of, ImportedFrom, SourceInfo};
    match source {
        ImportSource::Path { path } => SourceInfo {
            from: ImportedFrom::Path { path: path.clone() },
            origin: origin_of(Path::new(path)),
        },
        ImportSource::Bytes { name, .. } => SourceInfo {
            from: ImportedFrom::Dropped {
                name: name.chars().take(255).collect(),
            },
            origin: None,
        },
    }
}

/// The record written beside a staged copy, read back by commit.
const SOURCE_FILE: &str = "source.json";

#[cfg(test)]
pub(crate) fn stage_in(home: &Path, source: ImportSource) -> Result<ImportPreview, String> {
    stage_in_as(home, source, None)
}

/// [`stage_in`] with SD2's optional `rename_to`: the legal name to install a
/// skill under when its SKILL.md names it something [`skills::valid_skill_name`]
/// refuses. Never applied unless passed.
pub(crate) fn stage_in_as(
    home: &Path,
    source: ImportSource,
    rename_to: Option<&str>,
) -> Result<ImportPreview, String> {
    let info = source_info(&source);
    stage_inner(home, source, info, rename_to)
}

/// Check for update: stage `path` again, recording `info` (the skill's
/// original source, with a fresh origin commit) rather than `path`, which may
/// be a clone that is about to go.
pub(crate) fn stage_with(
    home: &Path,
    path: &Path,
    info: crate::skill_provenance::SourceInfo,
) -> Result<ImportPreview, String> {
    stage_inner(
        home,
        ImportSource::Path {
            path: path.to_string_lossy().to_string(),
        },
        info,
        None,
    )
}

/// Path in the skill → SHA-256 of every file under `dir`, read with the
/// import's own rules and caps (junk such as `.git/` and `.DS_Store` left out).
pub(crate) fn file_hashes(
    dir: &Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    Ok(collect_dir(dir)?
        .entries
        .iter()
        .map(|e| (e.parts.join("/"), sha256_hex(&e.data)))
        .collect())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A staged (unclaimed) import's copy of the skill, for Check for update's
/// file text.
pub(crate) fn staged_skill_dir(home: &Path, token: &str) -> Result<PathBuf, String> {
    if !valid_token(token) {
        return Err("Unknown import.".into());
    }
    Ok(staging_root(home)?.join(token).join("skill"))
}

/// A fresh, not yet created folder in staging for a Check for update clone
/// (`fetch-<token>`; an old one is swept with stale imports).
pub(crate) fn scratch_dir(home: &Path) -> Result<PathBuf, String> {
    Ok(staging_root(home)?.join(format!("fetch-{}", new_token())))
}

fn stage_inner(
    home: &Path,
    source: ImportSource,
    info: crate::skill_provenance::SourceInfo,
    rename_to: Option<&str>,
) -> Result<ImportPreview, String> {
    let mut collected = collect(source)?;
    let folder = root_layout(&mut collected.entries)?;
    check_collisions(&collected.entries)?;
    // SD2: an illegal name is refused with a suggestion, or renamed (and the
    // skill's own references rewritten) when the caller passed `rename_to`.
    // Everything below runs on the renamed result.
    let renamed = apply_rename(&mut collected, &home.join(".agents/skills"), rename_to)?;
    let skill_md = collected
        .entries
        .iter()
        .find(|e| e.parts == ["SKILL.md"])
        .ok_or("No SKILL.md found.")?;
    let (name, description) = check_skill_md(&skill_md.data)?;
    check_hidden_text(&collected.entries)?;

    let root = staging_root(home)?;
    sweep_stale(&root);
    let token = new_token();
    let token_dir = root.join(&token);
    fs::create_dir(&token_dir).map_err(|e| format!("mkdir: {e}"))?;
    let written = write_staged(&token_dir.join("skill"), &collected.entries).and_then(|()| {
        let json = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
        fs::write(token_dir.join(SOURCE_FILE), json)
            .map_err(|e| format!("Couldn't stage the source record: {e}"))
    });
    if let Err(error) = written {
        let _ = fs::remove_dir_all(&token_dir);
        return Err(error);
    }

    let mut files: Vec<ImportFile> = collected
        .entries
        .iter()
        .map(|e| ImportFile {
            path: e.parts.join("/"),
            size: e.data.len() as u64,
            script: is_script(&e.parts, e.executable),
        })
        .collect();
    files.sort_by(|a, b| {
        (a.path != "SKILL.md")
            .cmp(&(b.path != "SKILL.md"))
            .then(a.path.cmp(&b.path))
    });
    let skills_dir = home.join(".agents/skills");
    let target = skills_dir.join(&name);
    let existing_meta = fs::symlink_metadata(&target).ok();
    Ok(ImportPreview {
        token,
        description,
        has_scripts: files.iter().any(|f| f.script),
        total_bytes: collected.total,
        files,
        skipped: collected.skipped,
        folder,
        existing: existing_meta
            .as_ref()
            .map(|_| target.join("SKILL.md").to_string_lossy().to_string()),
        can_replace: existing_meta
            .as_ref()
            .is_some_and(|m| m.is_dir() && !m.file_type().is_symlink()),
        keep_both_name: keep_both_name(&skills_dir, &name)?,
        name,
        update: None,
        renamed_from: renamed.as_ref().map(|r| r.from.clone()),
        rewritten: renamed.map(|r| r.rewritten).unwrap_or_default(),
    })
}

// ── SD2: renaming a skill with an illegal name ──────────────────────────────

/// What a rename changed: the old name and the files whose references to it
/// were rewritten.
struct Renamed {
    from: String,
    rewritten: Vec<RewrittenFile>,
}

/// A character a skill name is made of, for telling a whole-name reference
/// from a longer name that merely contains it.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// `text` with every whole-name occurrence of `old` replaced by `new`, and
/// how many there were. An occurrence is whole when no name character touches
/// it (`.` and `:` count as touching only when a name character follows them,
/// so `old.md` and `old:x` are other things but `old.` and `old:` end a
/// sentence). `/` is a boundary, so a path through the skill's folder
/// (`skills/old/references/x.md`) is rewritten.
fn replace_whole_name(text: &str, old: &str, new: &str) -> (String, usize) {
    let joins = |c: char| c == '.' || c == ':';
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut count = 0;
    for (at, _) in text.match_indices(old) {
        if at < last {
            continue;
        }
        let end = at + old.len();
        let mut before = text[..at].chars().rev();
        let touches_before = match before.next() {
            Some(c) if is_name_char(c) => true,
            Some(c) if joins(c) => before.next().is_some_and(is_name_char),
            _ => false,
        };
        let mut after = text[end..].chars();
        let touches_after = match after.next() {
            Some(c) if is_name_char(c) => true,
            Some(c) if joins(c) => after.next().is_some_and(is_name_char),
            _ => false,
        };
        if touches_before || touches_after {
            continue;
        }
        out.push_str(&text[last..at]);
        out.push_str(new);
        last = end;
        count += 1;
    }
    out.push_str(&text[last..]);
    (out, count)
}

/// SD2: SKILL.md's illegal name, handled. Returns `None` when the name is
/// fine (and refuses a `rename_to` then: a rename is for an illegal name
/// only). Without `rename_to` it refuses and names the suggested legal name
/// (never one that is installed). With it, checks the new name, then rewrites
/// whole-name references to the old one in the skill's own Markdown files and
/// SKILL.md's frontmatter `name`; nothing outside the skill is read or written.
/// A name this can't judge (no frontmatter name, not UTF-8, too big) is left
/// to [`check_skill_md`], which refuses it as before.
fn apply_rename(
    collected: &mut Collected,
    skills_dir: &Path,
    rename_to: Option<&str>,
) -> Result<Option<Renamed>, String> {
    let Some(skill_md) = collected.entries.iter().find(|e| e.parts == ["SKILL.md"]) else {
        return Ok(None);
    };
    if skill_md.data.len() as u64 > MAX_SKILL_MD_BYTES {
        return Ok(None);
    }
    let Ok(text) = std::str::from_utf8(&skill_md.data) else {
        return Ok(None);
    };
    let from = skills::parse_skill_meta(text)
        .2
        .unwrap_or_default()
        .trim()
        .to_string();
    if from.is_empty() {
        return Ok(None);
    }
    if skills::valid_skill_name(&from) {
        return match rename_to {
            None => Ok(None),
            Some(_) => Err(format!(
                "SKILL.md's name {from:?} is already a legal name, so there is nothing to rename; import it as it is."
            )),
        };
    }
    // NV2 on the files as they came, before the old name is echoed or
    // replaced: a rename must not wash hidden text out of the name line.
    check_hidden_text(&collected.entries)?;
    let taken = |name: &str| fs::symlink_metadata(skills_dir.join(name)).is_ok();
    let Some(new) = rename_to else {
        let Some(suggested) = skills::suggest_skill_name(&from) else {
            return Err(format!(
                "SKILL.md's name {from:?} can only use letters, numbers, dashes and underscores, and nothing legal remains of it to suggest; give the skill a new name in its SKILL.md."
            ));
        };
        let free = (1..1000)
            .map(|n| {
                if n == 1 {
                    suggested.clone()
                } else {
                    format!("{suggested}-{n}")
                }
            })
            .find(|candidate| !taken(candidate))
            .ok_or_else(|| format!("No free name left for {suggested}."))?;
        return Err(format!(
            "SKILL.md's name {from:?} can only use letters, numbers, dashes and underscores. Import it as \"{free}\" instead (pass rename_to \"{free}\"); nothing was renamed or installed."
        ));
    };
    let new = new.trim();
    if !skills::valid_skill_name(new) || new.len() > skills::MAX_SUGGESTED_NAME {
        return Err(format!(
            "Can't rename to {new:?}: a skill name uses only letters, numbers, dashes and underscores, at most {} of them.",
            skills::MAX_SUGGESTED_NAME
        ));
    }
    if taken(new) {
        return Err(format!(
            "Can't rename to {new:?}: a shared skill with that name already exists. Choose another name."
        ));
    }

    let mut rewritten = Vec::new();
    let mut added: i64 = 0;
    for entry in &mut collected.entries {
        let is_skill_md = entry.parts == ["SKILL.md"];
        if !entry.parts.last().is_some_and(|p| is_markdown_name(p)) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&entry.data) else {
            continue;
        };
        let mut count = 0;
        let mut text = text.to_string();
        if is_skill_md {
            text = rewrite_name(&text, new).ok_or("Couldn't rename the skill in its SKILL.md.")?;
            count += 1;
        }
        let (replaced, n) = replace_whole_name(&text, &from, new);
        count += n;
        if count == 0 {
            continue;
        }
        let grown = replaced.len() as i64 - entry.data.len() as i64;
        let limit = if is_skill_md {
            MAX_SKILL_MD_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if replaced.len() as u64 > limit {
            return Err(too_big_file(&entry.parts.join("/")));
        }
        added += grown;
        entry.data = replaced.into_bytes();
        rewritten.push(RewrittenFile {
            path: entry.parts.join("/"),
            count,
        });
    }
    collected.total = collected.total.saturating_add_signed(added);
    if collected.total > MAX_TOTAL_BYTES {
        return Err(too_big_total());
    }
    rewritten.sort_by(|a, b| {
        (a.path != "SKILL.md")
            .cmp(&(b.path != "SKILL.md"))
            .then(a.path.cmp(&b.path))
    });
    Ok(Some(Renamed { from, rewritten }))
}

/// SKILL.md with its frontmatter `name` set to `name`.
fn rewrite_name(content: &str, name: &str) -> Option<String> {
    let first = content.find('\n')?;
    if !content[..first].trim_end().eq("---") {
        return None;
    }
    let body_start = first + 1;
    let end = body_start + content[body_start..].find("\n---")?;
    let mut out = String::with_capacity(content.len() + name.len());
    out.push_str(&content[..body_start]);
    // Every top-level `name:` (the parser reads the last one).
    let mut replaced = false;
    for line in content[body_start..end].split_inclusive('\n') {
        let key = line.split(':').next().unwrap_or("");
        let top_level = !line.starts_with([' ', '\t']);
        if top_level && key.trim() == "name" && line.contains(':') {
            let ending = &line[line.trim_end_matches(['\r', '\n']).len()..];
            out.push_str(&format!("name: {name}{ending}"));
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    out.push_str(&content[end..]);
    replaced.then_some(out)
}

pub(crate) fn commit_in(
    home: &Path,
    token: &str,
    on_clash: OnClash,
) -> Result<ImportOutcome, String> {
    if !valid_token(token) {
        return Err("Unknown import.".into());
    }
    let root = staging_root(home)?;
    // Claim it: a discard racing this commit then finds nothing to delete.
    let token_dir = root.join(format!("{token}.claim"));
    fs::rename(root.join(token), &token_dir)
        .map_err(|_| "This import is no longer staged; import it again.".to_string())?;
    let result = commit_claimed(home, &root, token, &token_dir, on_clash);
    if result.is_ok() {
        let _ = fs::remove_dir_all(&token_dir);
    } else {
        // Hand it back, so the person can choose again or cancel.
        let _ = fs::rename(&token_dir, root.join(token));
    }
    result
}

fn commit_claimed(
    home: &Path,
    root: &Path,
    token: &str,
    token_dir: &Path,
    on_clash: OnClash,
) -> Result<ImportOutcome, String> {
    let staged = token_dir.join("skill");
    let meta = fs::symlink_metadata(&staged)
        .map_err(|_| "This import is no longer staged; import it again.".to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("This import is no longer staged; import it again.".into());
    }
    let skill_md = staged.join("SKILL.md");
    let content = fs::read(&skill_md).map_err(|e| format!("read SKILL.md: {e}"))?;
    let (name, description) = check_skill_md(&content)?;
    let (files, has_scripts) = staged_summary(&staged)?;

    let skills_dir = skills_root(home)?;
    let target = skills_dir.join(&name);
    let mut replaced = false;
    let mut renamed_from = None;
    let installed_name = match fs::symlink_metadata(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::rename(&staged, &target).map_err(|e| format!("install: {e}"))?;
            name.clone()
        }
        Err(e) => return Err(format!("skill path: {e}")),
        Ok(existing) => match on_clash {
            OnClash::Cancel => {
                return Err(format!(
                    "A shared skill named {name} already exists. Choose Replace or Keep both."
                ))
            }
            OnClash::Replace => {
                if existing.file_type().is_symlink() || !existing.is_dir() {
                    return Err(format!(
                        "~/.agents/skills/{name} is a link or a file, not a skill folder; Hollow won't replace it."
                    ));
                }
                // Outside the claimed folder, which a failure hands back to
                // staging (and a discard would delete).
                let previous = root.join(format!("{token}.previous"));
                fs::rename(&target, &previous).map_err(|e| format!("replace: {e}"))?;
                if let Err(error) = fs::rename(&staged, &target) {
                    return Err(match fs::rename(&previous, &target) {
                        Ok(()) => format!("replace: {error}; the old skill is back in place."),
                        Err(_) => format!(
                            "replace: {error}. The old skill is kept at {}; move it back to {} by hand.",
                            previous.display(),
                            target.display()
                        ),
                    });
                }
                let _ = fs::remove_dir_all(&previous);
                replaced = true;
                name.clone()
            }
            OnClash::KeepBoth => {
                let new_name = keep_both_name(&skills_dir, &name)?;
                let text = String::from_utf8_lossy(&content);
                let rewritten = rewrite_name(&text, &new_name)
                    .ok_or("Couldn't rename the skill in its SKILL.md.")?;
                crate::vault::atomic_write(&skill_md, rewritten.as_bytes())
                    .map_err(|e| format!("write SKILL.md: {e}"))?;
                fs::rename(&staged, skills_dir.join(&new_name))
                    .map_err(|e| format!("install: {e}"))?;
                renamed_from = Some(name.clone());
                new_name
            }
        },
    };
    // Repo ideas 2026-10-27 #2: where it came from, in Hollow's app data (its
    // local note written again first). The install stands if this fails.
    let source = fs::read(token_dir.join(SOURCE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::skill_provenance::SourceInfo>(&b).ok());
    if let Some(info) = source {
        if let Err(e) = crate::skill_provenance::record_install(home, &installed_name, info) {
            eprintln!(
                "[skills] {installed_name} installed, but its import record wasn't kept: {e}"
            );
        }
    }
    Ok(ImportOutcome {
        skill: Skill {
            name: installed_name.clone(),
            source: "shared".into(),
            source_kind: "user".into(),
            plugin: None,
            path: skills_dir
                .join(&installed_name)
                .join("SKILL.md")
                .to_string_lossy()
                .to_string(),
            description,
            version: None,
            writable: true,
            lint: None,
        },
        files,
        has_scripts,
        replaced,
        renamed_from,
    })
}

/// File count and whether any runs as code, read back from the staged copy.
fn staged_summary(dir: &Path) -> Result<(usize, bool), String> {
    let collected = collect_dir(dir)?;
    let scripts = collected
        .entries
        .iter()
        .any(|e| is_script(&e.parts, e.executable));
    Ok((collected.entries.len(), scripts))
}

pub(crate) fn discard_in(home: &Path, token: &str) -> Result<(), String> {
    if !valid_token(token) {
        return Err("Unknown import.".into());
    }
    let dir = staging_root(home)?.join(token);
    match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("discard: {e}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const SKILL: &str = "---\nname: pdf-tools\ndescription: Read and fill PDF forms.\n---\n\nUse scripts/fill.py.\n";

    fn b64(data: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    /// A zip of `(path, bytes, unix mode)` entries; mode `None` uses the
    /// writer's default.
    fn zip_of(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, data, mode) in entries {
            let mut options = SimpleFileOptions::default();
            if let Some(mode) = mode.filter(|m| *m != 0o120777) {
                options = options.unix_permissions(mode);
            }
            if *mode == Some(0o120777) {
                out.add_symlink(*path, std::str::from_utf8(data).unwrap(), options)
                    .unwrap();
            } else if path.ends_with('/') {
                out.add_directory(*path, options).unwrap();
            } else {
                out.start_file(*path, options).unwrap();
                out.write_all(data).unwrap();
            }
        }
        out.finish().unwrap().into_inner()
    }

    fn bytes(name: &str, data: &[u8]) -> ImportSource {
        ImportSource::Bytes {
            name: name.into(),
            data: b64(data),
        }
    }

    fn skills_in(home: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(home.join(".agents/skills"))
            .map(|d| {
                d.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn staged_tokens(home: &Path) -> usize {
        fs::read_dir(home.join(".agents").join(STAGING))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    #[test]
    fn a_skill_export_with_one_top_level_folder_installs_whole() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let archive = zip_of(&[
            ("pdf-tools/", b"", None),
            ("pdf-tools/SKILL.md", SKILL.as_bytes(), None),
            ("pdf-tools/scripts/fill.py", b"print('hi')\n", Some(0o755)),
            ("pdf-tools/references/forms.md", b"# Forms\n", None),
            ("pdf-tools/assets/logo.png", &[0x89, b'P', b'N', b'G'], None),
            ("__MACOSX/pdf-tools/._SKILL.md", b"junk", None),
            ("pdf-tools/.DS_Store", b"junk", None),
        ]);
        let preview = stage_in(home, bytes("pdf-tools.skill", &archive)).unwrap();
        assert_eq!(preview.name, "pdf-tools");
        assert_eq!(preview.description, "Read and fill PDF forms.");
        assert_eq!(preview.folder.as_deref(), Some("pdf-tools"));
        assert_eq!(
            preview
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            [
                "SKILL.md",
                "assets/logo.png",
                "references/forms.md",
                "scripts/fill.py"
            ]
        );
        assert!(preview.has_scripts);
        assert!(
            preview
                .files
                .iter()
                .find(|f| f.path == "scripts/fill.py")
                .unwrap()
                .script
        );
        assert!(
            !preview
                .files
                .iter()
                .find(|f| f.path == "references/forms.md")
                .unwrap()
                .script
        );
        assert_eq!(preview.skipped.len(), 2);
        assert!(preview.existing.is_none());
        // Nothing is listed while it is only staged.
        assert!(skills_in(home).is_empty());
        assert!(skills::list_all_in(home).is_empty());

        let outcome = commit_in(home, &preview.token, OnClash::Cancel).unwrap();
        assert_eq!(outcome.skill.name, "pdf-tools");
        assert_eq!(outcome.files, 4);
        assert!(outcome.has_scripts);
        let dir = home.join(".agents/skills/pdf-tools");
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), SKILL);
        assert_eq!(
            fs::read(dir.join("assets/logo.png")).unwrap(),
            [0x89, b'P', b'N', b'G']
        );
        assert!(!dir.join(".DS_Store").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("scripts/fill.py"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        let listed = skills::list_all_in(home);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].writable);
        assert_eq!(staged_tokens(home), 0, "the staging folder is cleaned up");
    }

    #[test]
    fn a_zip_with_skill_md_at_its_root_installs() {
        let temp = tempfile::tempdir().unwrap();
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("references/notes.md", b"notes", None),
        ]);
        let preview = stage_in(temp.path(), bytes("anything.zip", &archive)).unwrap();
        assert_eq!(preview.folder, None);
        assert!(!preview.has_scripts);
        commit_in(temp.path(), &preview.token, OnClash::Cancel).unwrap();
        assert!(temp
            .path()
            .join(".agents/skills/pdf-tools/references/notes.md")
            .is_file());
    }

    #[test]
    fn skill_md_must_be_at_the_top_or_in_one_top_level_folder() {
        let temp = tempfile::tempdir().unwrap();
        for archive in [
            zip_of(&[("a/b/SKILL.md", SKILL.as_bytes(), None)]),
            zip_of(&[
                ("one/SKILL.md", SKILL.as_bytes(), None),
                ("two/x.md", b"x", None),
            ]),
            zip_of(&[("README.md", b"no skill here", None)]),
        ] {
            let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
            assert!(error.contains("No SKILL.md"), "{error}");
        }
        assert_eq!(staged_tokens(temp.path()), 0);
    }

    #[test]
    fn traversal_and_absolute_paths_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        for bad in [
            "../evil.sh",
            "pdf-tools/../../evil",
            "/etc/evil",
            "a/./b",
            "C:/evil",
            "a\\..\\evil",
        ] {
            let archive = zip_of(&[("SKILL.md", SKILL.as_bytes(), None), (bad, b"x", None)]);
            let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
            assert!(error.starts_with("Refused"), "{bad}: {error}");
        }
        assert!(!temp.path().join("evil").exists());
        assert_eq!(staged_tokens(temp.path()), 0);
    }

    #[test]
    fn symlinks_are_refused_in_archives_and_folders() {
        let temp = tempfile::tempdir().unwrap();
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("secrets", b"/Users/someone/.ssh/id_ed25519", Some(0o120777)),
        ]);
        let error = stage_in(temp.path(), bytes("x.skill", &archive)).unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let folder = temp.path().join("src/pdf-tools");
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("SKILL.md"), SKILL).unwrap();
            symlink("/etc/hosts", folder.join("hosts")).unwrap();
            let error = stage_in(
                temp.path(),
                ImportSource::Path {
                    path: folder.to_string_lossy().into(),
                },
            )
            .unwrap_err();
            assert!(error.contains("symbolic link"), "{error}");
            // The folder itself as a link is refused too.
            fs::remove_file(folder.join("hosts")).unwrap();
            let link = temp.path().join("link");
            symlink(&folder, &link).unwrap();
            let error = stage_in(
                temp.path(),
                ImportSource::Path {
                    path: link.to_string_lossy().into(),
                },
            )
            .unwrap_err();
            assert!(error.contains("symbolic link"), "{error}");
        }
        assert!(skills_in(temp.path()).is_empty());
    }

    #[test]
    fn caps_on_files_and_bytes_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        // Too many files.
        let names: Vec<String> = (0..=MAX_FILES)
            .map(|i| format!("references/{i}.md"))
            .collect();
        let mut entries: Vec<(&str, &[u8], Option<u32>)> =
            vec![("SKILL.md", SKILL.as_bytes(), None)];
        entries.extend(names.iter().map(|n| (n.as_str(), &b"x"[..], None)));
        let error = stage_in(temp.path(), bytes("x.zip", &zip_of(&entries))).unwrap_err();
        assert!(error.contains("more than 1000 files"), "{error}");

        // One file over the per-file cap: a zip bomb's few KB decompress past it.
        let big = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("assets/big.bin", &big, None),
        ]);
        assert!(archive.len() < 1024 * 1024, "the archive itself is small");
        let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
        assert!(error.contains("larger than 25 MB"), "{error}");

        // Under the per-file cap, but over the total.
        let part = vec![0u8; (MAX_FILE_BYTES - 1) as usize];
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("a.bin", &part, None),
            ("b.bin", &part, None),
            ("c.bin", &part, None),
        ]);
        let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
        assert!(error.contains("larger than 50 MB"), "{error}");

        // A huge SKILL.md.
        let mut md = SKILL.as_bytes().to_vec();
        md.extend(vec![b'x'; MAX_SKILL_MD_BYTES as usize]);
        let error = stage_in(temp.path(), bytes("SKILL.md", &md)).unwrap_err();
        assert!(error.contains("SKILL.md is larger"), "{error}");
        assert_eq!(staged_tokens(temp.path()), 0);
    }

    #[test]
    fn a_lying_zip_header_does_not_get_past_the_cap() {
        // Patch the archive's recorded uncompressed size down to 1 byte: the
        // cap counts what actually decompresses.
        let big = vec![0u8; (MAX_FILE_BYTES + 10) as usize];
        let mut archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("big.bin", &big, None),
        ]);
        let declared = (big.len() as u32).to_le_bytes();
        let mut patched = 0;
        let mut i = 0;
        while i + 4 <= archive.len() {
            if archive[i..i + 4] == declared {
                archive[i..i + 4].copy_from_slice(&1u32.to_le_bytes());
                patched += 1;
            }
            i += 1;
        }
        assert!(patched >= 1);
        let temp = tempfile::tempdir().unwrap();
        // Refused, or (if the reader stops at the declared size) staged no
        // larger than the cap: never more.
        match stage_in(temp.path(), bytes("x.zip", &archive)) {
            Err(error) => assert!(
                error.contains("larger than") || error.contains("Couldn't"),
                "{error}"
            ),
            Ok(preview) => {
                let big = preview.files.iter().find(|f| f.path == "big.bin").unwrap();
                assert!(big.size <= MAX_FILE_BYTES, "{}", big.size);
            }
        }
        assert!(skills_in(temp.path()).is_empty());
    }

    #[test]
    fn frontmatter_needs_a_valid_name_and_a_description() {
        let temp = tempfile::tempdir().unwrap();
        for (md, expect) in [
            ("# Just a heading\n", "needs frontmatter with a `name`"),
            (
                "---\ndescription: d\n---\nbody\n",
                "needs frontmatter with a `name`",
            ),
            ("---\nname: ok-name\n---\nbody\n", "needs a `description`"),
            (
                "---\nname: ok-name\ndescription: \"\"\n---\n",
                "needs a `description`",
            ),
            (
                "---\nname: ../escape\ndescription: d\n---\n",
                "letters, numbers",
            ),
            (
                "---\nname: has space\ndescription: d\n---\n",
                "letters, numbers",
            ),
        ] {
            let error = stage_in(temp.path(), bytes("SKILL.md", md.as_bytes())).unwrap_err();
            assert!(error.contains(expect), "{md:?}: {error}");
        }
        let error = stage_in(temp.path(), bytes("SKILL.md", &[0xff, 0xfe, 0x00])).unwrap_err();
        assert!(error.contains("UTF-8"), "{error}");
        assert_eq!(staged_tokens(temp.path()), 0);
    }

    #[test]
    fn hidden_characters_in_any_text_file_are_refused_without_echoing_them() {
        let temp = tempfile::tempdir().unwrap();
        let skill = "---\nname: ok-name\ndescription: d\n---\nbody\n";
        // Tag characters spelling "hi" after a clean line.
        let tags = "---\nname: ok-name\ndescription: d\n---\nbody\u{E0068}\u{E0069}\n";
        let error = stage_in(temp.path(), bytes("SKILL.md", tags.as_bytes())).unwrap_err();
        assert!(error.contains("tag character"), "{error}");
        assert!(error.contains("line 5, column 5"), "{error}");
        assert!(!error.contains('\u{E0068}'), "{error}");
        // A planted file beside a clean SKILL.md, in a folder import.
        for (file, class) in [
            ("a\u{200B}b", "zero-width"),
            ("x \u{202E}evil", "bidi-control"),
            ("\u{E0041}", "tag character"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("SKILL.md"), skill).unwrap();
            fs::create_dir(dir.path().join("references")).unwrap();
            fs::write(dir.path().join("references/notes.md"), file).unwrap();
            let source = ImportSource::Path {
                path: dir.path().to_string_lossy().to_string(),
            };
            let error = stage_in(temp.path(), source).unwrap_err();
            assert!(error.contains(class), "{error}");
            assert!(error.contains("references/notes.md"), "{error}");
            assert!(!error.contains(file), "{error}");
        }
        assert_eq!(staged_tokens(temp.path()), 0);
        // A leading byte-order mark and an emoji joiner are not hidden text;
        // binary files are skipped.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("SKILL.md"), skill).unwrap();
        fs::write(
            dir.path().join("a.md"),
            "\u{FEFF}plain \u{1F468}\u{200D}\u{1F469}",
        )
        .unwrap();
        fs::write(dir.path().join("blob.bin"), [0u8, 0xE2, 0x80, 0x8B]).unwrap();
        let source = ImportSource::Path {
            path: dir.path().to_string_lossy().to_string(),
        };
        stage_in(temp.path(), source).unwrap();
    }

    #[test]
    fn a_bare_skill_md_and_a_folder_both_import() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert_eq!(preview.files.len(), 1);
        commit_in(home, &preview.token, OnClash::Cancel).unwrap();

        let folder = home.join("Downloads/report-writer");
        fs::create_dir_all(folder.join("scripts")).unwrap();
        fs::create_dir_all(folder.join(".git/objects")).unwrap();
        fs::write(folder.join(".git/HEAD"), "ref").unwrap();
        fs::write(
            folder.join("SKILL.md"),
            "---\nname: report-writer\ndescription: Write reports.\n---\n",
        )
        .unwrap();
        fs::write(folder.join("scripts/run.sh"), "echo hi").unwrap();
        let preview = stage_in(
            home,
            ImportSource::Path {
                path: folder.to_string_lossy().into(),
            },
        )
        .unwrap();
        assert_eq!(preview.name, "report-writer");
        assert!(preview.has_scripts);
        assert_eq!(preview.skipped, [".git/"]);
        commit_in(home, &preview.token, OnClash::Cancel).unwrap();
        assert_eq!(skills_in(home), ["pdf-tools", "report-writer"]);
        assert!(!home.join(".agents/skills/report-writer/.git").exists());
        assert!(
            folder.join("SKILL.md").is_file(),
            "the source is left alone"
        );
    }

    #[test]
    fn other_files_are_not_skills() {
        let temp = tempfile::tempdir().unwrap();
        let error =
            stage_in(temp.path(), bytes("photo.png", &[0x89, b'P', b'N', b'G'])).unwrap_err();
        assert!(error.contains("isn't a skill"), "{error}");
        let error = stage_in(temp.path(), bytes("broken.zip", b"PK\x03\x04garbage")).unwrap_err();
        assert!(error.contains("Couldn't read"), "{error}");
    }

    #[test]
    fn case_only_duplicates_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("references/Notes.md", b"a", None),
            ("references/notes.md", b"b", None),
        ]);
        let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
        assert!(error.contains("same name"), "{error}");
    }

    #[test]
    fn a_clash_cancels_replaces_or_keeps_both() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let existing = home.join(".agents/skills/pdf-tools");
        fs::create_dir_all(existing.join("old")).unwrap();
        fs::write(
            existing.join("SKILL.md"),
            "---\nname: pdf-tools\ndescription: old\n---\n",
        )
        .unwrap();
        fs::write(existing.join("old/keep.md"), "old").unwrap();

        // Cancel (the default) refuses and leaves both copies alone.
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert!(preview.existing.is_some());
        assert!(preview.can_replace);
        assert_eq!(preview.keep_both_name, "pdf-tools-2");
        let error = commit_in(home, &preview.token, OnClash::Cancel).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert!(existing.join("old/keep.md").is_file());

        // Keep both: the same staged import installs as pdf-tools-2.
        let kept = commit_in(home, &preview.token, OnClash::KeepBoth).unwrap();
        assert_eq!(kept.skill.name, "pdf-tools-2");
        assert_eq!(kept.renamed_from.as_deref(), Some("pdf-tools"));
        let copy = fs::read_to_string(home.join(".agents/skills/pdf-tools-2/SKILL.md")).unwrap();
        assert!(
            copy.starts_with(
                "---\nname: pdf-tools-2\ndescription: Read and fill PDF forms.\n---\n"
            ),
            "{copy}"
        );
        let listed = skills::list_all_in(home);
        assert!(listed.iter().any(|s| s.name == "pdf-tools-2" && s.writable));

        // Replace swaps the whole folder: the old files go.
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert_eq!(preview.keep_both_name, "pdf-tools-3");
        let replaced = commit_in(home, &preview.token, OnClash::Replace).unwrap();
        assert!(replaced.replaced);
        assert_eq!(
            fs::read_to_string(existing.join("SKILL.md")).unwrap(),
            SKILL
        );
        assert!(!existing.join("old").exists());
        assert_eq!(skills_in(home), ["pdf-tools", "pdf-tools-2"]);
        assert_eq!(staged_tokens(home), 0);
    }

    #[cfg(unix)]
    #[test]
    fn replace_never_follows_a_linked_skill() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let provider = home.join(".claude/skills/pdf-tools");
        fs::create_dir_all(&provider).unwrap();
        fs::write(provider.join("SKILL.md"), "provider copy").unwrap();
        fs::create_dir_all(home.join(".agents/skills")).unwrap();
        symlink(&provider, home.join(".agents/skills/pdf-tools")).unwrap();
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert!(!preview.can_replace);
        let error = commit_in(home, &preview.token, OnClash::Replace).unwrap_err();
        assert!(error.contains("won't replace"), "{error}");
        assert_eq!(
            fs::read_to_string(provider.join("SKILL.md")).unwrap(),
            "provider copy"
        );

        // A linked shared folder is refused for staging and installing alike.
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir_all(home.join("elsewhere")).unwrap();
        symlink(home.join("elsewhere"), home.join(".agents")).unwrap();
        let error = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap_err();
        assert!(error.contains("symlink"), "{error}");
    }

    #[test]
    fn discard_and_bad_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert_eq!(staged_tokens(home), 1);
        discard_in(home, &preview.token).unwrap();
        assert_eq!(staged_tokens(home), 0);
        let error = commit_in(home, &preview.token, OnClash::Cancel).unwrap_err();
        assert!(error.contains("no longer staged"), "{error}");
        for bad in ["../../skills", "", "ABCDEF", &"g".repeat(32)] {
            assert!(commit_in(home, bad, OnClash::Cancel).is_err());
            assert!(discard_in(home, bad).is_err());
        }
        assert!(skills_in(home).is_empty());
    }

    #[test]
    fn stale_staging_is_swept() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let root = staging_root(home).unwrap();
        let stale = root.join("0".repeat(32));
        fs::create_dir(&stale).unwrap();
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        fs::File::open(&stale).unwrap().set_modified(old).unwrap();
        let claim = root.join(format!("{}.claim", "1".repeat(32)));
        fs::create_dir(&claim).unwrap();
        fs::File::open(&claim).unwrap().set_modified(old).unwrap();
        let previous = root.join(format!("{}.previous", "2".repeat(32)));
        fs::create_dir(&previous).unwrap();
        fs::File::open(&previous)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let unrelated = root.join("keep-me");
        fs::create_dir(&unrelated).unwrap();
        // An interrupted Check for update's clone and its empty HOME.
        let fetch = root.join(format!("fetch-{}", "3".repeat(32)));
        let fetch_home = root.join(format!("fetch-{}-home", "3".repeat(32)));
        for dir in [&fetch, &fetch_home] {
            fs::create_dir(dir).unwrap();
            fs::File::open(dir).unwrap().set_modified(old).unwrap();
        }
        let not_ours = root.join("fetch-x-home");
        fs::create_dir(&not_ours).unwrap();
        fs::File::open(&not_ours)
            .unwrap()
            .set_modified(old)
            .unwrap();
        stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        assert!(!fetch.exists() && !fetch_home.exists());
        assert!(not_ours.exists(), "only token names are swept");
        assert!(!stale.exists());
        assert!(unrelated.exists(), "only token folders are swept");
        assert!(!claim.exists());
        assert!(
            previous.exists(),
            "a replaced skill kept after a failed rollback is never swept"
        );
    }

    #[test]
    fn a_commit_claims_its_import_before_a_discard_can_delete_it() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let preview = stage_in(home, bytes("SKILL.md", SKILL.as_bytes())).unwrap();
        let root = home.join(".agents").join(STAGING);
        // A commit in progress holds `<token>.claim`.
        let claimed = root.join(format!("{}.claim", preview.token));
        fs::rename(root.join(&preview.token), &claimed).unwrap();
        discard_in(home, &preview.token).unwrap();
        assert!(
            claimed.join("skill/SKILL.md").is_file(),
            "discard leaves a claimed import alone"
        );
        // A failed commit hands the import back for another choice.
        fs::rename(&claimed, root.join(&preview.token)).unwrap();
        fs::create_dir_all(home.join(".agents/skills/pdf-tools")).unwrap();
        assert!(commit_in(home, &preview.token, OnClash::Cancel).is_err());
        assert!(root.join(&preview.token).join("skill/SKILL.md").is_file());
        assert!(!claimed.exists());
        commit_in(home, &preview.token, OnClash::Replace).unwrap();
        assert_eq!(
            staged_tokens(home),
            0,
            "nothing left: no claim, no previous copy"
        );
    }

    #[test]
    fn agents_stage_on_the_card_and_unusable_clash_choices_are_refused_there() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let file = home.join("SKILL.md");
        fs::write(&file, SKILL).unwrap();
        let path = file.to_string_lossy().to_string();
        let preview = stage_for_agent_in(home, &path, OnClash::Cancel, None).unwrap();
        assert_eq!(preview.name, "pdf-tools");
        commit_in(home, &preview.token, OnClash::Cancel).unwrap();
        // Taken now: cancel is refused on the card and nothing stays staged.
        let error = stage_for_agent_in(home, &path, OnClash::Cancel, None).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(staged_tokens(home), 0);
        let kept = stage_for_agent_in(home, &path, OnClash::KeepBoth, None).unwrap();
        assert_eq!(kept.keep_both_name, "pdf-tools-2");
        discard_in(home, &kept.token).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = home.join(".agents/skills/pdf-tools");
            fs::remove_dir_all(&target).unwrap();
            symlink(home.join("elsewhere"), &target).unwrap();
            let error = stage_for_agent_in(home, &path, OnClash::Replace, None).unwrap_err();
            assert!(error.contains("won't replace"), "{error}");
            assert_eq!(staged_tokens(home), 0);
        }
    }

    #[test]
    fn a_path_that_is_both_a_file_and_a_folder_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let archive = zip_of(&[
            ("SKILL.md", SKILL.as_bytes(), None),
            ("assets", b"a file", None),
            ("assets/logo.png", b"png", None),
        ]);
        let error = stage_in(temp.path(), bytes("x.zip", &archive)).unwrap_err();
        assert!(error.contains("both a file and a folder"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn special_files_in_a_folder_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("skill");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("SKILL.md"), SKILL).unwrap();
        let fifo =
            std::ffi::CString::new(folder.join("pipe").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
        let error = stage_in(
            temp.path(),
            ImportSource::Path {
                path: folder.to_string_lossy().into(),
            },
        )
        .unwrap_err();
        assert!(error.contains("isn't a regular file"), "{error}");
    }

    #[test]
    fn scripts_are_flagged_by_folder_extension_or_mode() {
        let parts = |p: &str| p.split('/').map(String::from).collect::<Vec<_>>();
        assert!(is_script(&parts("scripts/fill.txt"), false));
        assert!(is_script(&parts("tools/scripts/run"), false));
        assert!(is_script(&parts("bin/tool"), false));
        assert!(is_script(&parts("setup.BAT"), false));
        assert!(is_script(&parts("references/x.md"), true));
        assert!(!is_script(&parts("references/x.md"), false));
        assert!(
            !is_script(&parts("scripts"), false),
            "a file named scripts is not a folder"
        );
    }

    #[test]
    fn rewrite_name_changes_only_the_frontmatter_name() {
        let md =
            "---\r\ndescription: uses name: inside\r\nname: \"old\"\r\n---\r\nname: body stays\r\n";
        let out = rewrite_name(md, "new-2").unwrap();
        assert_eq!(
            out,
            "---\r\ndescription: uses name: inside\r\nname: new-2\r\n---\r\nname: body stays\r\n"
        );
        // Duplicate keys: every top-level name changes, so the parser (which
        // reads the last) agrees with the folder.
        let twice = rewrite_name("---\nname: a\n  name: nested\nname: b\n---\n", "b-2").unwrap();
        assert_eq!(twice, "---\nname: b-2\n  name: nested\nname: b-2\n---\n");
        assert_eq!(skills::parse_skill_meta(&twice).2.as_deref(), Some("b-2"));
        assert_eq!(rewrite_name("no frontmatter", "x"), None);
        assert_eq!(rewrite_name("---\ndescription: d\n---\n", "x"), None);
    }

    #[test]
    fn on_clash_parses_the_wire_names() {
        assert_eq!(OnClash::parse(None).unwrap(), OnClash::Cancel);
        assert_eq!(OnClash::parse(Some("replace")).unwrap(), OnClash::Replace);
        assert_eq!(
            OnClash::parse(Some("keep-both")).unwrap(),
            OnClash::KeepBoth
        );
        assert!(OnClash::parse(Some("overwrite")).is_err());
        let source: ImportSource = serde_json::from_value(
            serde_json::json!({ "kind": "bytes", "name": "a.md", "data": "" }),
        )
        .unwrap();
        assert!(matches!(source, ImportSource::Bytes { .. }));
        let clash: OnClash = serde_json::from_value(serde_json::json!("keep-both")).unwrap();
        assert_eq!(clash, OnClash::KeepBoth);
    }

    // ── SD2: renaming a skill whose name is illegal ─────────────────────────

    const STITCH: &str = "stitch::generate-design";

    /// A folder skill named `stitch::generate-design` whose files point at
    /// it by name, plus lookalikes that must be left alone.
    fn stitch_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("references")).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: stitch::generate-design\ndescription: Generate a design with stitch::generate-design.\n---\n\nRun stitch::generate-design, then read\n`~/.agents/skills/stitch::generate-design/references/guide.md`.\nNot stitch::generate-design-2, not xstitch::generate-design, not stitch::generate-design.md.\n",
        )
        .unwrap();
        fs::write(
            root.join("references/guide.md"),
            "# Guide\n\nBack to stitch::generate-design (stitch::generate-design).\n",
        )
        .unwrap();
        fs::write(
            root.join("scripts/run.sh"),
            "#!/bin/sh\necho stitch::generate-design\n",
        )
        .unwrap();
        dir
    }

    fn folder_source(dir: &tempfile::TempDir) -> ImportSource {
        ImportSource::Path {
            path: dir.path().to_string_lossy().to_string(),
        }
    }

    #[test]
    fn an_illegal_name_is_refused_with_a_suggestion_and_installs_only_when_renamed() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let dir = stitch_folder();

        // No rename asked for: refused, the message names the suggestion,
        // nothing is staged or installed.
        let error = stage_in(home, folder_source(&dir)).unwrap_err();
        assert!(error.contains("letters, numbers"), "{error}");
        assert!(error.contains("\"stitch-generate-design\""), "{error}");
        assert_eq!(staged_tokens(home), 0);
        assert!(skills_in(home).is_empty());

        // The rename is applied only when the caller passes it.
        let preview =
            stage_in_as(home, folder_source(&dir), Some("stitch-generate-design")).unwrap();
        assert_eq!(preview.name, "stitch-generate-design");
        assert_eq!(preview.renamed_from.as_deref(), Some(STITCH));
        let counts: Vec<(&str, usize)> = preview
            .rewritten
            .iter()
            .map(|r| (r.path.as_str(), r.count))
            .collect();
        // SKILL.md: the name line, the description, the body twice (the
        // command and the skills path). guide.md: twice. The script is not
        // markdown and keeps its text.
        assert_eq!(counts, [("SKILL.md", 4), ("references/guide.md", 2)]);

        let outcome = commit_in(home, &preview.token, OnClash::Cancel).unwrap();
        assert_eq!(outcome.skill.name, "stitch-generate-design");
        let installed = home.join(".agents/skills/stitch-generate-design");
        let skill_md = fs::read_to_string(installed.join("SKILL.md")).unwrap();
        let (description, _, name) = skills::parse_skill_meta(&skill_md);
        assert_eq!(name.as_deref(), Some("stitch-generate-design"));
        assert_eq!(
            description,
            "Generate a design with stitch-generate-design."
        );
        // The references resolve: the path in the body names the installed
        // folder, and the sibling file it names exists there.
        assert!(skill_md.contains("`~/.agents/skills/stitch-generate-design/references/guide.md`"));
        assert!(installed.join("references/guide.md").is_file());
        assert_eq!(
            fs::read_to_string(installed.join("references/guide.md")).unwrap(),
            "# Guide\n\nBack to stitch-generate-design (stitch-generate-design).\n"
        );
        // Only whole names: lookalikes and non-markdown files are untouched.
        assert!(skill_md.contains(
            "Not stitch::generate-design-2, not xstitch::generate-design, not stitch::generate-design.md."
        ));
        assert_eq!(
            fs::read_to_string(installed.join("scripts/run.sh")).unwrap(),
            "#!/bin/sh\necho stitch::generate-design\n"
        );
        assert_eq!(skills_in(home), ["stitch-generate-design"]);
        assert_eq!(staged_tokens(home), 0);
    }

    #[test]
    fn a_legal_name_is_untouched_and_refuses_a_rename() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let preview = stage_in_as(home, bytes("SKILL.md", SKILL.as_bytes()), None).unwrap();
        assert_eq!(preview.name, "pdf-tools");
        assert_eq!(preview.renamed_from, None);
        assert!(preview.rewritten.is_empty());
        let staged = staged_skill_dir(home, &preview.token).unwrap();
        assert_eq!(fs::read_to_string(staged.join("SKILL.md")).unwrap(), SKILL);
        discard_in(home, &preview.token).unwrap();

        // A rename is for an illegal name only, never a silent second rename.
        let error =
            stage_in_as(home, bytes("SKILL.md", SKILL.as_bytes()), Some("other")).unwrap_err();
        assert!(error.contains("already"), "{error}");
        assert_eq!(staged_tokens(home), 0);
    }

    #[test]
    fn a_rename_never_collides_with_an_installed_skill() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir_all(home.join(".agents/skills/stitch-generate-design")).unwrap();
        fs::create_dir_all(home.join(".agents/skills/stitch-generate-design-2")).unwrap();
        let dir = stitch_folder();

        // The suggestion skips both taken names.
        let error = stage_in(home, folder_source(&dir)).unwrap_err();
        assert!(error.contains("\"stitch-generate-design-3\""), "{error}");

        // An explicit rename onto an installed skill is refused.
        let error =
            stage_in_as(home, folder_source(&dir), Some("stitch-generate-design")).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(staged_tokens(home), 0);
        assert_eq!(
            skills_in(home),
            ["stitch-generate-design", "stitch-generate-design-2"]
        );
        // The suggested free name works.
        let preview =
            stage_in_as(home, folder_source(&dir), Some("stitch-generate-design-3")).unwrap();
        assert_eq!(preview.name, "stitch-generate-design-3");
    }

    #[test]
    fn a_rename_must_itself_be_a_legal_name_and_nothing_legal_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let dir = stitch_folder();
        for bad in [
            "../escape",
            "a/b",
            "with space",
            "",
            "stitch::x",
            "caf\u{e9}",
        ] {
            let error = stage_in_as(home, folder_source(&dir), Some(bad)).unwrap_err();
            assert!(error.contains("rename"), "{bad:?}: {error}");
        }
        assert_eq!(staged_tokens(home), 0);
        // A name with nothing to build a legal one from says so.
        let error = stage_in(
            home,
            bytes("SKILL.md", b"---\nname: :::\ndescription: d\n---\nbody\n"),
        )
        .unwrap_err();
        assert!(error.contains("nothing legal"), "{error}");
        assert_eq!(staged_tokens(home), 0);
    }

    #[test]
    fn hidden_text_is_still_refused_after_a_rename() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        // In a sibling file, then in SKILL.md itself.
        for (file, text) in [
            (
                "references/guide.md",
                "back to stitch::generate-design\u{200B}\n",
            ),
            ("scripts/run.sh", "echo \u{E0068}\n"),
            (
                "SKILL.md",
                "---\nname: stitch::generate-design\ndescription: d\n---\nbody \u{202E}\n",
            ),
        ] {
            let dir = stitch_folder();
            fs::write(dir.path().join(file), text).unwrap();
            let error =
                stage_in_as(home, folder_source(&dir), Some("stitch-generate-design")).unwrap_err();
            assert!(error.contains("Hidden text"), "{file}: {error}");
            assert!(error.contains(file), "{file}: {error}");
            assert_eq!(staged_tokens(home), 0);
        }
        assert!(skills_in(home).is_empty());

        // Hidden text in the old name itself: the rename would replace that
        // line, so it is refused as it came, with or without a rename.
        let dir = stitch_folder();
        let hidden = "---\nname: stitch\u{200B}::generate-design\ndescription: d\n---\nbody\n";
        fs::write(dir.path().join("SKILL.md"), hidden).unwrap();
        for rename in [None, Some("stitch-generate-design")] {
            let error = stage_in_as(home, folder_source(&dir), rename).unwrap_err();
            assert!(error.contains("zero-width"), "{rename:?}: {error}");
            assert!(!error.contains('\u{200B}'), "{rename:?}: {error}");
        }
        assert_eq!(staged_tokens(home), 0);
    }

    #[test]
    fn the_agent_action_stages_a_rename_and_still_refuses_without_one() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let dir = stitch_folder();
        let path = dir.path().to_string_lossy().to_string();
        let error = stage_for_agent_in(home, &path, OnClash::Cancel, None).unwrap_err();
        assert!(error.contains("\"stitch-generate-design\""), "{error}");
        let preview =
            stage_for_agent_in(home, &path, OnClash::Cancel, Some("stitch-generate-design"))
                .unwrap();
        assert_eq!(preview.name, "stitch-generate-design");
        assert_eq!(preview.renamed_from.as_deref(), Some(STITCH));
    }
}
