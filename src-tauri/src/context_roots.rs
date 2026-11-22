//! Context layers, part 1 (CX3): where the personal and organization
//! context clones live on this Mac, which machine this is, and the path
//! rules every later reader and writer of those clones goes through.
//!
//! Nothing reads or writes the roots yet. The home layer (CX4) and the org
//! layer (CX5) are the first callers; while no root is saved, Hollow behaves
//! exactly as before.
//!
//! **Settings.** `context-roots.json` in the app data folder:
//! `{"personal": "<absolute path>" | null, "org": "<absolute path>" | null}`.
//! Absent or null is unset. The default a Settings row offers is
//! `~/Hollow/Context/personal` and `~/Hollow/Context/org` ([`default_root`]);
//! it is only a suggestion and is never read as a saved root.
//!
//! **Machine.** `~/.config/hollow/machine.json` (`$XDG_CONFIG_HOME/hollow`
//! when set), local to this Mac and never synced:
//! `{"version": 1, "slug": "<slug>"}`. Unknown fields are kept on write. The
//! slug names this machine's home file (`home/<slug>.md` in the personal
//! clone). It is the person's choice, suggested from the Mac's name
//! ([`suggest_slug`]), so no machine name is ever a literal in code.
//!
//! **Path rules.** A root is an absolute folder path with no `.` or `..`
//! part, never `/`, the home folder or a folder above it. It is saved
//! resolved: the folders above it are canonicalized once, at save time, and
//! the root itself must not be a symlink, so the saved path holds no symlink
//! at all. At use time [`check_root`] refuses a saved root that gained one
//! (any part of it a symlink) or is no longer a folder. An entry inside a
//! root ([`resolve_in_root`]) is a relative path of plain names, none of them
//! a symlink, which keeps a symlink in a cloned repo (git checks them out)
//! from pointing a read at `~/.ssh`. [`read_in_root`] opens every folder from
//! `/` down with `O_NOFOLLOW`, so a swap between the check and the read can't
//! redirect it either.
//!
//! **Later entries.** Claude Code's auto memory can be pointed at a folder
//! (CX0b: the `autoMemoryDirectory` setting), and a later row points it at
//! `memory/<project>` in the personal clone. That path goes through
//! [`resolve_in_root`] the way `home/<slug>.md` does ([`home_file`]); the
//! CLI's project folder names start with `-` and keep their case, so that
//! entry gets its own name check then, not the machine slug's.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Longest machine slug, in bytes.
pub(crate) const SLUG_MAX: usize = 40;

/// The settings file in the app data folder.
const SETTINGS_FILE: &str = "context-roots.json";

/// One of the two context clones this module places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Layer {
    Personal,
    Org,
}

impl Layer {
    #[cfg_attr(not(test), allow(dead_code))] // CX17's Settings rows
    pub(crate) const ALL: [Layer; 2] = [Layer::Personal, Layer::Org];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Layer::Personal => "personal",
            Layer::Org => "org",
        }
    }
}

/// Why a root, an entry in one or a slug was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathError {
    Empty,
    NotAbsolute,
    /// A `.` or `..` part, an empty part or a NUL.
    BadPart,
    /// `/`, the home folder or a folder above it.
    TooBroad,
    Symlink(PathBuf),
    NotFolder(PathBuf),
    Missing(PathBuf),
    /// The personal and org roots are the same folder or one holds the other.
    Overlap,
    NotFile(PathBuf),
    TooLarge(u64),
    BadSlug(String),
    Io(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => f.write_str("No folder given."),
            PathError::NotAbsolute => {
                f.write_str("Give a full folder path (starting with / or ~/).")
            }
            PathError::BadPart => {
                f.write_str("A path can't have . or .. parts, empty parts or a NUL.")
            }
            PathError::TooBroad => {
                f.write_str("Choose a folder of its own, not /, your home folder or a folder above it.")
            }
            PathError::Symlink(p) => {
                write!(f, "{} is a symlink; context folders can't use symlinks.", p.display())
            }
            PathError::NotFolder(p) => write!(f, "{} is not a folder.", p.display()),
            PathError::Missing(p) => write!(f, "{} doesn't exist.", p.display()),
            PathError::Overlap => f.write_str(
                "The personal and organization context folders must be separate: neither may be or hold the other.",
            ),
            PathError::NotFile(p) => write!(f, "{} is not a plain file.", p.display()),
            PathError::TooLarge(max) => write!(f, "The file is over {max} bytes."),
            PathError::BadSlug(s) => write!(
                f,
                "{s:?} is not a machine name: use 1 to {SLUG_MAX} lowercase letters, digits and single dashes, starting and ending with a letter or digit."
            ),
            PathError::Io(e) => f.write_str(e),
        }
    }
}

impl From<PathError> for String {
    fn from(e: PathError) -> String {
        e.to_string()
    }
}

fn io(path: &Path, e: std::io::Error) -> PathError {
    PathError::Io(format!("{}: {e}", path.display()))
}

// ── Path rules ──────────────────────────────────────────────────────────

/// `text` as an absolute path with no `.`, `..` or empty interior part
/// (repeated and trailing slashes collapse), else why not.
fn lexical(text: &str) -> Result<PathBuf, PathError> {
    if text.is_empty() {
        return Err(PathError::Empty);
    }
    if text.contains('\0') {
        return Err(PathError::BadPart);
    }
    let Some(rest) = text.strip_prefix('/') else {
        return Err(PathError::NotAbsolute);
    };
    let mut out = PathBuf::from("/");
    for part in rest.split('/') {
        match part {
            "" => continue,
            "." | ".." => return Err(PathError::BadPart),
            name => out.push(name),
        }
    }
    Ok(out)
}

/// `~` and `~/…` against `home`; anything else unchanged (`~user` stays
/// relative, so it is refused as not absolute).
fn expand_home(text: &str, home: &Path) -> Result<String, PathError> {
    let joined = if text == "~" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else {
        return Ok(text.to_string());
    };
    joined
        .into_os_string()
        .into_string()
        .map_err(|_| PathError::BadPart)
}

/// `/`, `home` itself or a folder above it.
fn too_broad(path: &Path, home: &Path) -> bool {
    path == Path::new("/") || holds(path, home)
}

/// Each part of `path`, lowercased.
fn folded(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// `inner` is `outer` or below it, ignoring case: APFS is case-insensitive
/// by default and `canonicalize` keeps the case as typed, so `/users/ada`
/// is the home folder too (review 2026-11-13).
fn holds(outer: &Path, inner: &Path) -> bool {
    let (outer, inner) = (folded(outer), folded(inner));
    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

/// The two roots are the same folder or one holds the other.
pub(crate) fn overlap(a: &Path, b: &Path) -> bool {
    holds(a, b) || holds(b, a)
}

/// The path a root is saved as: `input` (`~/` allowed) checked by the path
/// rules, its existing folders above it canonicalized, any missing tail kept
/// as given. The root itself, when it exists, must be a real folder, not a
/// symlink.
pub(crate) fn normalize_root(input: &str, home: &Path) -> Result<PathBuf, PathError> {
    let path = lexical(&expand_home(input.trim(), home)?)?;
    let real_home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if too_broad(&path, home) || too_broad(&path, &real_home) {
        return Err(PathError::TooBroad);
    }
    let mut existing = path.clone();
    let mut tail = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(meta) => {
                if existing == path {
                    if meta.file_type().is_symlink() {
                        return Err(PathError::Symlink(path));
                    }
                    if !meta.is_dir() {
                        return Err(PathError::NotFolder(path));
                    }
                }
                break;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let Some(name) = existing.file_name() else {
                    return Err(io(&existing, e));
                };
                tail.push(name.to_os_string());
                existing.pop();
            }
            // A file where a folder above the root should be.
            Err(e) if e.kind() == ErrorKind::NotADirectory => {
                return Err(PathError::NotFolder(path));
            }
            Err(e) => return Err(io(&existing, e)),
        }
    }
    let mut resolved = fs::canonicalize(&existing).map_err(|e| io(&existing, e))?;
    if !resolved.is_dir() {
        return Err(PathError::NotFolder(existing));
    }
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    if too_broad(&resolved, &real_home) {
        return Err(PathError::TooBroad);
    }
    Ok(resolved)
}

/// A saved root at use time: still in its saved form (absolute, no `.` or
/// `..`), every part of it a real folder and none a symlink.
pub(crate) fn check_root(root: &Path) -> Result<(), PathError> {
    let text = root.to_str().ok_or(PathError::BadPart)?;
    if lexical(text)? != root || root == Path::new("/") {
        return Err(PathError::BadPart);
    }
    let mut at = PathBuf::from("/");
    for part in root.components().skip(1) {
        at.push(part);
        let meta = match fs::symlink_metadata(&at) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(PathError::Missing(at)),
            Err(e) => return Err(io(&at, e)),
        };
        if meta.file_type().is_symlink() {
            return Err(PathError::Symlink(at));
        }
        if !meta.is_dir() {
            return Err(PathError::NotFolder(at));
        }
    }
    Ok(())
}

/// `rel`'s parts: a relative path of plain names (no leading, trailing or
/// doubled slash, no `.`, `..` or NUL).
fn entry_parts(rel: &str) -> Result<Vec<&str>, PathError> {
    if rel.is_empty() {
        return Err(PathError::Empty);
    }
    if rel.starts_with('/') {
        return Err(PathError::NotAbsolute);
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\0'))
    {
        return Err(PathError::BadPart);
    }
    Ok(parts)
}

/// `root/rel` once [`check_root`] passes and no existing part of `rel` is a
/// symlink (a folder part must be a folder). A missing tail is allowed: the
/// caller decides what a missing entry means. The answer holds only when it
/// is given: read through [`read_in_root`], never a plain `fs::read` of the
/// path, and a writer needs the same `openat` walk.
pub(crate) fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, PathError> {
    check_root(root)?;
    let parts = entry_parts(rel)?;
    let last = parts.len() - 1;
    let mut at = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(PathError::Symlink(at)),
            Ok(meta) if i < last && !meta.is_dir() => return Err(PathError::NotFolder(at)),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {
                at.extend(&parts[i + 1..]);
                return Ok(at);
            }
            Err(e) => return Err(io(&at, e)),
        }
    }
    Ok(at)
}

/// Read `root/rel` as UTF-8 text of at most `max` bytes, `None` when it is
/// missing. Every folder from `/` down is opened with `O_NOFOLLOW` relative
/// to the one before, and the file too, so no symlink anywhere on the path
/// is followed even if one appears after [`resolve_in_root`] checked it.
#[cfg(unix)]
pub(crate) fn read_in_root(root: &Path, rel: &str, max: u64) -> Result<Option<String>, PathError> {
    match resolve_in_root(root, rel) {
        Err(PathError::Missing(_)) => return Ok(None),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let mut names: Vec<String> = root
        .components()
        .skip(1)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    names.extend(entry_parts(rel)?.iter().map(|s| s.to_string()));
    read_walk(&names, max)
}

/// [`read_in_root`]'s walk with no check before it: `names` are the parts
/// of the absolute path from `/`, each folder opened `O_NOFOLLOW` relative
/// to the one before; the file is `lstat`ed through its folder first and
/// opened only when it is a plain file (no device or FIFO is ever opened),
/// then checked to be that same file.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // stat field types differ across platforms
pub(crate) fn read_walk(names: &[String], max: u64) -> Result<Option<String>, PathError> {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::MetadataExt;

    let full: PathBuf = std::iter::once("/".to_string())
        .chain(names.iter().cloned())
        .collect();
    let open = |dir: Option<&OwnedFd>,
                name: &str,
                flags: libc::c_int|
     -> Result<OwnedFd, std::io::Error> {
        let c = CString::new(name).map_err(|_| std::io::Error::from(ErrorKind::InvalidInput))?;
        let base = dir.map_or(libc::AT_FDCWD, |d| d.as_raw_fd());
        // SAFETY: `c` is a valid NUL-terminated string and `base` is either
        // AT_FDCWD or an fd this function owns; the returned fd is checked.
        let fd = unsafe { libc::openat(base, c.as_ptr(), flags | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `fd` was just returned by openat and nothing else owns it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let refused = |e: std::io::Error, at: &Path| -> Result<Option<String>, PathError> {
        match e.raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            Some(libc::ELOOP) => Err(PathError::Symlink(at.to_path_buf())),
            // macOS answers ENOTDIR, not ELOOP, for O_DIRECTORY | O_NOFOLLOW
            // on a symlink: already refused, this only names it right.
            Some(libc::ENOTDIR)
                if fs::symlink_metadata(at).is_ok_and(|m| m.file_type().is_symlink()) =>
            {
                Err(PathError::Symlink(at.to_path_buf()))
            }
            Some(libc::ENOTDIR) => Err(PathError::NotFolder(at.to_path_buf())),
            _ => Err(io(at, e)),
        }
    };

    let dir_flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
    let mut dir = match open(None, "/", libc::O_RDONLY | libc::O_DIRECTORY) {
        Ok(fd) => fd,
        Err(e) => return refused(e, Path::new("/")),
    };
    let full = full.as_path();
    let (file_name, folders) = names.split_last().ok_or(PathError::Empty)?;
    let mut at = PathBuf::from("/");
    for name in folders {
        at.push(name);
        dir = match open(Some(&dir), name, dir_flags) {
            Ok(fd) => fd,
            Err(e) => return refused(e, &at),
        };
    }
    let c_name = CString::new(file_name.as_str()).map_err(|_| PathError::BadPart)?;
    // SAFETY: a zeroed `stat` is a valid out-parameter for fstatat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `dir` is an open folder fd, `c_name` NUL-terminated, `st` valid.
    if unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            c_name.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return refused(std::io::Error::last_os_error(), full);
    }
    match st.st_mode & libc::S_IFMT {
        libc::S_IFREG => {}
        libc::S_IFLNK => return Err(PathError::Symlink(full.to_path_buf())),
        _ => return Err(PathError::NotFile(full.to_path_buf())),
    }
    let fd = match open(
        Some(&dir),
        file_name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    ) {
        Ok(fd) => fd,
        Err(e) => return refused(e, full),
    };
    let file = fs::File::from(fd);
    let meta = file.metadata().map_err(|e| io(full, e))?;
    if !meta.is_file() || meta.ino() != st.st_ino as u64 || meta.dev() != st.st_dev as u64 {
        return Err(PathError::NotFile(full.to_path_buf()));
    }
    if meta.len() > max {
        return Err(PathError::TooLarge(max));
    }
    let mut text = String::new();
    file.take(max.saturating_add(1))
        .read_to_string(&mut text)
        .map_err(|e| io(full, e))?;
    if text.len() as u64 > max {
        return Err(PathError::TooLarge(max));
    }
    Ok(Some(text))
}

/// This machine's home file inside the personal clone, relative to its root.
pub(crate) fn home_file(slug: &str) -> Result<String, PathError> {
    valid_slug(slug)?;
    Ok(format!("home/{slug}.md"))
}

/// Longest project folder name under `memory/`: a file name's limit.
const MEMORY_NAME_MAX: usize = 255;

/// The personal clone's `memory/<project>` folder for Claude Code's auto
/// memory (CX4b), relative to the root. `project` is the CLI's own folder
/// name for the project ([`crate::run_manifest::claude_project_slug`]):
/// it starts with `-` and keeps its case, so it is not a machine slug. The
/// name must be plain ASCII letters, digits and `-`, so it can't climb out
/// of `memory/`.
pub(crate) fn memory_dir(project: &str) -> Result<String, PathError> {
    let ok = !project.is_empty()
        && project.len() <= MEMORY_NAME_MAX
        && project
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(format!("memory/{project}"))
    } else {
        Err(PathError::BadSlug(project.to_string()))
    }
}

// ── Saved roots ─────────────────────────────────────────────────────────

/// The saved roots; `None` is unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RootSettings {
    pub personal: Option<String>,
    pub org: Option<String>,
}

impl RootSettings {
    pub(crate) fn get(&self, layer: Layer) -> Option<&str> {
        match layer {
            Layer::Personal => self.personal.as_deref(),
            Layer::Org => self.org.as_deref(),
        }
    }

    fn slot(&mut self, layer: Layer) -> &mut Option<String> {
        match layer {
            Layer::Personal => &mut self.personal,
            Layer::Org => &mut self.org,
        }
    }
}

pub(crate) fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(SETTINGS_FILE))
}

/// The roots saved in `path`; a missing or unreadable file is unset.
pub(crate) fn settings_at(path: &Path) -> RootSettings {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!(
                "[context-roots] {}: {e}; treating both roots as unset",
                path.display()
            );
            RootSettings::default()
        }),
        Err(_) => RootSettings::default(),
    }
}

/// The roots saved in `path` for a write: a missing file is unset, one that
/// doesn't parse is refused so the other layer's root is never dropped.
fn load_for_write(path: &Path) -> Result<RootSettings, PathError> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            PathError::Io(format!(
                "{} isn't valid ({e}); fix or remove it first.",
                path.display()
            ))
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(RootSettings::default()),
        Err(e) => Err(io(path, e)),
    }
}

/// The saved roots.
#[allow(dead_code)] // CX17's Settings rows
pub(crate) fn settings() -> RootSettings {
    settings_path().map(|p| settings_at(&p)).unwrap_or_default()
}

/// Save `layer`'s root in `path` (`None` or blank unsets it), checked by the
/// path rules against `home` and kept apart from the other layer's root.
/// Returns the saved settings.
pub(crate) fn set_root_at(
    path: &Path,
    home: &Path,
    layer: Layer,
    value: Option<&str>,
) -> Result<RootSettings, PathError> {
    let mut saved = load_for_write(path)?;
    let root = match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => None,
        Some(v) => {
            let root = normalize_root(v, home)?;
            let other = match layer {
                Layer::Personal => Layer::Org,
                Layer::Org => Layer::Personal,
            };
            if let Some(other) = saved.get(other).map(Path::new) {
                if overlap(&root, other) {
                    return Err(PathError::Overlap);
                }
            }
            Some(
                root.into_os_string()
                    .into_string()
                    .map_err(|_| PathError::BadPart)?,
            )
        }
    };
    *saved.slot(layer) = root;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let text = serde_json::to_string_pretty(&saved).map_err(|e| PathError::Io(e.to_string()))?;
    crate::vault::atomic_write_private(path, text.as_bytes()).map_err(PathError::Io)?;
    Ok(saved)
}

/// Save `layer`'s root (`None` or blank unsets it); returns the saved
/// settings.
#[allow(dead_code)] // CX17's Settings rows
pub(crate) fn set_root(layer: Layer, value: Option<&str>) -> Result<RootSettings, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = dirs::home_dir().ok_or("Couldn't find the home folder.")?;
    Ok(set_root_at(&settings_path()?, &home, layer, value)?)
}

/// The folder a Settings row offers for `layer`: `~/Hollow/Context/<layer>`.
/// A suggestion only; nothing reads it as a root.
#[cfg_attr(not(test), allow(dead_code))] // CX17's Settings rows
pub(crate) fn default_root(layer: Layer) -> Option<PathBuf> {
    dirs::home_dir().map(|h| default_root_in(&h, layer))
}

fn default_root_in(home: &Path, layer: Layer) -> PathBuf {
    home.join("Hollow").join("Context").join(layer.as_str())
}

/// `layer`'s saved root when it passes [`check_root`] now, else `None`: the
/// one call CX4 and CX5 make before they read a clone. The save-time rules
/// are checked again, since the file can be edited by hand: a root that is
/// too broad or overlaps the other layer's is `None`.
#[cfg_attr(test, allow(dead_code))] // context_layers, outside tests
pub(crate) fn root(layer: Layer) -> Option<PathBuf> {
    root_in(&settings(), layer, &dirs::home_dir()?)
}

pub(crate) fn root_in(saved: &RootSettings, layer: Layer, home: &Path) -> Option<PathBuf> {
    let path = PathBuf::from(saved.get(layer)?);
    check_root(&path).ok()?;
    if too_broad(&path, home) {
        return None;
    }
    let other = match layer {
        Layer::Personal => Layer::Org,
        Layer::Org => Layer::Personal,
    };
    if saved
        .get(other)
        .is_some_and(|o| overlap(&path, Path::new(o)))
    {
        return None;
    }
    Some(path)
}

// ── Machine ─────────────────────────────────────────────────────────────

/// `machine.json`: which machine this is. Local, never synced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineFile {
    #[serde(default = "machine_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Fields a later Hollow adds (M2's machine record); kept on write.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn machine_version() -> u32 {
    1
}

/// `~/.config/hollow/machine.json` (`$XDG_CONFIG_HOME/hollow/machine.json`
/// when it is set to an absolute path, as the XDG spec requires).
pub(crate) fn machine_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|v| v.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .map(|base| base.join("hollow").join("machine.json"))
}

/// A machine slug: 1 to [`SLUG_MAX`] bytes of lowercase ASCII letters,
/// digits and single dashes, starting and ending with a letter or digit.
pub(crate) fn valid_slug(slug: &str) -> Result<(), PathError> {
    let bytes = slug.as_bytes();
    let ok = !bytes.is_empty()
        && bytes.len() <= SLUG_MAX
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
        && !slug.contains("--");
    if ok {
        Ok(())
    } else {
        Err(PathError::BadSlug(slug.to_string()))
    }
}

/// The slug saved in `path`, when the file reads and the slug is valid.
pub(crate) fn machine_slug_at(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let file: MachineFile = serde_json::from_str(&text)
        .map_err(|e| eprintln!("[context-roots] {}: {e}", path.display()))
        .ok()?;
    file.slug.filter(|s| valid_slug(s).is_ok())
}

/// This machine's slug, `None` until the person sets one.
#[cfg_attr(test, allow(dead_code))] // context_layers, outside tests
pub(crate) fn machine_slug() -> Option<String> {
    machine_slug_at(&machine_path()?)
}

/// Save `slug` in `path` (`None` removes it), keeping the file's other
/// fields. A file that isn't valid JSON is left alone and refused, so a hand
/// edit is never overwritten.
pub(crate) fn set_machine_slug_at(
    path: &Path,
    slug: Option<&str>,
) -> Result<Option<String>, PathError> {
    let slug = slug.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = slug {
        valid_slug(s)?;
    }
    let mut file = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<MachineFile>(&text).map_err(|e| {
            PathError::Io(format!(
                "{} isn't valid machine JSON ({e}); fix or remove it first.",
                path.display()
            ))
        })?,
        Err(e) if e.kind() == ErrorKind::NotFound => MachineFile {
            version: machine_version(),
            slug: None,
            extra: Map::new(),
        },
        Err(e) => return Err(io(path, e)),
    };
    file.slug = slug.map(str::to_string);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| PathError::Io(e.to_string()))?;
    crate::vault::atomic_write_private(path, format!("{text}\n").as_bytes())
        .map_err(PathError::Io)?;
    Ok(file.slug)
}

/// Held by this process's writers of `machine.json` (the slug, the id).
static MACHINE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Save this machine's slug (`None` removes it); returns the slug saved.
#[allow(dead_code)] // CX17
pub(crate) fn set_machine_slug(slug: Option<&str>) -> Result<Option<String>, String> {
    let _held = MACHINE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = machine_path().ok_or("Couldn't find the home folder.")?;
    Ok(set_machine_slug_at(&path, slug)?)
}

/// `machine.json`'s key for this Mac's id (M2).
const MACHINE_ID_KEY: &str = "machineId";

/// The `machineId` saved in `path`, when the file reads and the id is one
/// Hollow writes (`m-` and 16 lower-case hex digits).
#[cfg_attr(not(test), allow(dead_code))] // M2 PR2: the client reads its own
pub(crate) fn machine_id_at(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let file: MachineFile = serde_json::from_str(&text).ok()?;
    file.extra
        .get(MACHINE_ID_KEY)
        .and_then(Value::as_str)
        .filter(|id| crate::phone_access::valid_machine_id(id))
        .map(str::to_string)
}

/// This Mac's stable id for the peer protocol (M2), written into `path` on
/// first need and kept after: `m-` and 16 random hex digits. It stays when
/// the slug changes (the slug is the person's name for the machine; the id
/// is what another Mac's record of this one is keyed by), and the file's
/// other fields stay too. A file that isn't valid JSON is refused, never
/// overwritten (a hand edit), as [`set_machine_slug_at`] does. Held under
/// this process's `machine.json` lock, so two first readers write one id.
pub(crate) fn ensure_machine_id_at(path: &Path) -> Result<String, String> {
    let _held = MACHINE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<MachineFile>(&text).map_err(|e| {
            format!(
                "{} isn't valid machine JSON ({e}); fix or remove it first.",
                path.display()
            )
        })?,
        Err(e) if e.kind() == ErrorKind::NotFound => MachineFile {
            version: machine_version(),
            slug: None,
            extra: Map::new(),
        },
        Err(e) => return Err(io(path, e).to_string()),
    };
    if let Some(id) = file
        .extra
        .get(MACHINE_ID_KEY)
        .and_then(Value::as_str)
        .filter(|id| crate::phone_access::valid_machine_id(id))
    {
        return Ok(id.to_string());
    }
    let mut bytes = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let id = format!(
        "m-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    file.extra
        .insert(MACHINE_ID_KEY.to_string(), Value::String(id.clone()));
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io(dir, e).to_string())?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(path, format!("{text}\n").as_bytes())?;
    Ok(id)
}

/// This Mac's `machineId` ([`ensure_machine_id_at`] on `machine.json`).
pub(crate) fn ensure_machine_id() -> Result<String, String> {
    let path = machine_path().ok_or("Couldn't find the home folder.")?;
    ensure_machine_id_at(&path)
}

/// A slug suggested from a Mac's name: an owner's `Name's ` prefix dropped
/// ("Ada's MacBook Pro" is `macbook-pro`), the rest lowercased with runs of
/// anything else as one dash. `None` when nothing usable is left.
pub(crate) fn suggest_slug(name: &str) -> Option<String> {
    let name = ["'s ", "\u{2019}s "]
        .iter()
        .filter_map(|mark| name.find(mark).map(|at| (at, at + mark.len())))
        .min()
        .map_or(name, |(_, rest)| &name[rest..]);
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if out.len() >= SLUG_MAX {
                break;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    valid_slug(&out).ok().map(|_| out)
}

/// The slug a Settings row suggests for this Mac.
#[allow(dead_code)] // CX17
pub(crate) fn suggested_slug() -> Option<String> {
    suggest_slug(&crate::phone_access::mac_name())
}

// ── Status ──────────────────────────────────────────────────────────────

/// What Settings shows for one root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootStatus {
    pub layer: Layer,
    /// The saved root, else the default a Settings row offers.
    pub path: String,
    /// A root is saved (false: `path` is the default).
    pub configured: bool,
    pub exists: bool,
    /// `path` is the top of a git work tree.
    pub git_clone: bool,
    /// HEAD's committer date (ISO 8601), when it has a commit.
    pub last_commit_at: Option<String>,
    /// Why the folder can't be used as it is, when it can't.
    pub problem: Option<String>,
}

/// `layer`'s status: the saved root, else the default.
#[allow(dead_code)] // CX17's Settings rows
pub(crate) async fn status(layer: Layer) -> RootStatus {
    let saved = settings();
    match saved.get(layer) {
        Some(p) => status_of(layer, Path::new(p), true).await,
        None => {
            let path = default_root(layer).unwrap_or_default();
            status_of(layer, &path, false).await
        }
    }
}

/// The status of `path` as `layer`'s root. Reads only: two `git` reads
/// when a `.git` sits in the folder, nothing else runs or changes.
pub(crate) async fn status_of(layer: Layer, path: &Path, configured: bool) -> RootStatus {
    let mut status = RootStatus {
        layer,
        path: path.to_string_lossy().into_owned(),
        configured,
        exists: false,
        git_clone: false,
        last_commit_at: None,
        problem: None,
    };
    match check_root(path) {
        Ok(()) => status.exists = true,
        Err(PathError::Missing(_)) => return status,
        Err(e) => {
            status.exists = fs::symlink_metadata(path).is_ok();
            status.problem = Some(e.to_string());
            return status;
        }
    }
    match resolve_in_root(path, ".git") {
        Ok(dot_git) if dot_git.exists() => {}
        Ok(_) => return status,
        Err(e) => {
            status.problem = Some(e.to_string());
            return status;
        }
    }
    use crate::build_workspaces::{git, QUICK};
    match git(path, &["rev-parse", "--show-toplevel"], QUICK).await {
        Ok(top) if Path::new(&top) == path => status.git_clone = true,
        Ok(top) => {
            status.problem = Some(format!(
                "git says this folder belongs to the work tree at {top}."
            ));
            return status;
        }
        Err(e) => {
            status.problem = Some(format!("Not a usable git clone: {e}"));
            return status;
        }
    }
    status.last_commit_at = git(
        path,
        &["log", "-1", "--no-show-signature", "--format=%cI"],
        QUICK,
    )
    .await
    .ok()
    .filter(|t| !t.is_empty());
    status
}
