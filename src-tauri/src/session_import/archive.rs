//! Safe reading of an untrusted export (H5b): a zip file or an already
//! extracted folder that the person picked. The ChatGPT reader uses it now;
//! the claude.ai and Gemini readers reuse it.
//!
//! **Nothing is extracted by this file.** [`Export::open`] reads the zip's
//! central directory and judges every entry before one byte is inflated;
//! [`Export::read`] then hands the caller a streaming reader with hard caps,
//! so no entry is ever held whole or written to disk by the archive code.
//! Nothing in an archive is executed, and no entry name is ever used as a
//! path on disk (a folder input is read through paths it listed itself,
//! links skipped).
//!
//! **What refuses the whole archive** ([`ArchiveError`]): a name that is
//! absolute, has a `..` or `.` part, an empty part, a backslash, a drive
//! letter or a control character; a symbolic link or other special entry
//! (device, pipe, socket); a duplicate name (compared case-insensitively,
//! since macOS folds case); an encrypted entry; a nested archive (unless the
//! reader opts to ignore them, see [`Nested`]); more than
//! [`Limits::max_entries`] entries; an entry or the total claiming more than
//! [`Limits::max_entry_bytes`] / [`Limits::max_total_bytes`] expanded; an
//! entry claiming a compression ratio over [`Limits::max_ratio`].
//!
//! **What stops a read** (a zip header can lie about sizes): bytes actually
//! inflated past the entry's claimed size or the per-entry cap, past the
//! total budget across reads, or past the ratio against the entry's
//! compressed size. The ratio is only judged once an entry has produced
//! [`Limits::ratio_floor`] bytes, so tiny files never trip it.

use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// What to do with an entry whose name is an archive (`.zip`, `.tar`,
/// `.gz`, `.7z`...). It is never opened either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nested {
    /// Refuse the whole archive.
    Refuse,
    /// List it as skipped and carry on (an export's attachment that happens
    /// to be a zip the person once uploaded).
    Ignore,
}

/// The caps on one export.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub max_entries: usize,
    /// One entry, expanded.
    pub max_entry_bytes: u64,
    /// All entries, expanded (the design's 4 GB).
    pub max_total_bytes: u64,
    /// Expanded over compressed.
    pub max_ratio: u64,
    /// The ratio is judged only past this many expanded bytes.
    pub ratio_floor: u64,
    /// Folder input: levels below the chosen folder.
    pub max_depth: usize,
    pub nested: Nested,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_entries: 50_000,
            max_entry_bytes: 2 * 1024 * 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
            max_ratio: 200,
            ratio_floor: 1024 * 1024,
            max_depth: 4,
            nested: Nested::Refuse,
        }
    }
}

/// Why an export was refused or a read stopped. `Display` is a sentence for
/// the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArchiveError {
    /// Not a zip or a folder, or a zip that can't be read.
    Unreadable(String),
    TooManyEntries(usize),
    /// A name that could escape the folder it would be extracted to.
    UnsafePath(String),
    Symlink(String),
    Special(String),
    Duplicate(String),
    Encrypted(String),
    Nested(String),
    TooLarge(String),
    /// Claimed or measured compression ratio over the limit.
    Ratio(String),
    /// No such entry in the archive, or one the reader may not open.
    Missing(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Unreadable(why) => write!(f, "This export can't be read: {why}."),
            ArchiveError::TooManyEntries(n) => {
                write!(f, "This export holds more than {n} files; Hollow won't open it.")
            }
            ArchiveError::UnsafePath(name) => write!(
                f,
                "This export has a file named {name:?} that could land outside its folder; Hollow won't open it."
            ),
            ArchiveError::Symlink(name) => write!(
                f,
                "This export holds a link ({name:?}); exports don't, so Hollow won't open it."
            ),
            ArchiveError::Special(name) => write!(
                f,
                "This export holds a special file ({name:?}); Hollow won't open it."
            ),
            ArchiveError::Duplicate(name) => write!(
                f,
                "This export lists {name:?} twice; Hollow won't open it."
            ),
            ArchiveError::Encrypted(name) => write!(
                f,
                "This export has a password-protected file ({name:?}); Hollow can't read it."
            ),
            ArchiveError::Nested(name) => write!(
                f,
                "This export holds another archive ({name:?}); Hollow won't open it."
            ),
            ArchiveError::TooLarge(what) => write!(f, "This export is too large: {what}."),
            ArchiveError::Ratio(name) => write!(
                f,
                "{name:?} expands far more than a real export does; Hollow stopped reading it."
            ),
            ArchiveError::Missing(name) => write!(f, "{name:?} isn't in this export."),
        }
    }
}

impl From<ArchiveError> for String {
    fn from(e: ArchiveError) -> String {
        e.to_string()
    }
}

/// One file in an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    /// The safe, `/`-separated name inside the export.
    pub name: String,
    /// Expanded size the archive claims (a folder file's real size).
    pub size: u64,
    compressed: u64,
    index: usize,
    /// A nested archive: listed, never opened.
    pub nested: bool,
}

enum Input {
    Zip(zip::ZipArchive<File>),
    Dir(PathBuf),
}

/// An opened export: every entry judged, none read yet.
pub(crate) struct Export {
    input: Input,
    items: Vec<Item>,
    limits: Limits,
    /// Expanded bytes handed out so far, against `max_total_bytes`.
    read_total: u64,
    /// Folder input: links skipped (never followed).
    pub links_skipped: usize,
}

const NESTED_EXTENSIONS: &[&str] = &[
    "zip", "jar", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst", "cab", "dmg", "iso",
];

const JUNK_PREFIXES: &[&str] = &["__MACOSX/", ".DS_Store"];

/// A name inside an archive as a safe `/`-separated relative name, with a
/// trailing `/` (a folder entry) removed.
pub(crate) fn safe_name(raw: &str) -> Result<String, ArchiveError> {
    let bad = || ArchiveError::UnsafePath(raw.chars().take(120).collect());
    if raw.is_empty() || raw.len() > 1024 {
        return Err(bad());
    }
    if raw.contains('\\') || raw.chars().any(|c| c.is_control()) || raw.starts_with('/') {
        return Err(bad());
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(bad());
    }
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed.is_empty() {
        return Err(bad());
    }
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(bad());
        }
    }
    Ok(trimmed.to_string())
}

fn is_nested_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| NESTED_EXTENSIONS.contains(&e.as_str()))
}

fn is_junk(name: &str) -> bool {
    JUNK_PREFIXES
        .iter()
        .any(|p| name.starts_with(p) || name.contains(&format!("/{p}")))
}

/// A short stand-in for the picked export, stable for the same file name,
/// size and modification time (the `imports.root` column; the export itself
/// is never hashed, it can be gigabytes).
pub(crate) fn fingerprint(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let meta = std::fs::metadata(path).ok();
    let mut h = Sha256::new();
    h.update(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_bytes(),
    );
    h.update(meta.as_ref().map_or(0, |m| m.len()).to_le_bytes());
    let secs = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    h.update(secs.to_le_bytes());
    format!("export:{}", &format!("{:x}", h.finalize())[..16])
}

/// The entry count the zip's end-of-central-directory record declares, or
/// `None` when it can't be found or is a zip64 archive (0xFFFF).
fn declared_entries(file: &mut File) -> Option<u64> {
    use std::io::{Seek, SeekFrom};
    let len = file.seek(SeekFrom::End(0)).ok()?;
    let take = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - take)).ok()?;
    let mut tail = vec![0u8; take as usize];
    file.read_exact(&mut tail).ok()?;
    let at = tail
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])?;
    let count = u16::from_le_bytes([*tail.get(at + 10)?, *tail.get(at + 11)?]);
    (count != 0xFFFF).then_some(u64::from(count))
}

impl Export {
    /// Open the zip or folder at `path` and judge every entry.
    pub fn open(path: &Path, limits: Limits) -> Result<Export, ArchiveError> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|_| ArchiveError::Unreadable("it isn't there".into()))?;
        if meta.file_type().is_symlink() {
            return Err(ArchiveError::Symlink(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ));
        }
        if meta.is_dir() {
            Export::open_dir(path, limits)
        } else if meta.is_file() {
            Export::open_zip(path, limits)
        } else {
            Err(ArchiveError::Unreadable("it isn't a file or folder".into()))
        }
    }

    fn open_zip(path: &Path, limits: Limits) -> Result<Export, ArchiveError> {
        let mut file = File::open(path).map_err(|e| ArchiveError::Unreadable(e.to_string()))?;
        let declared = declared_entries(&mut file);
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|_| ArchiveError::Unreadable("it isn't a zip file".into()))?;
        if zip.len() > limits.max_entries || declared.is_some_and(|n| n > limits.max_entries as u64)
        {
            return Err(ArchiveError::TooManyEntries(limits.max_entries));
        }
        // The zip crate keeps one entry per name and drops the repeats, so
        // fewer entries than the archive declares means a duplicate name.
        if declared.is_some_and(|n| n != zip.len() as u64) {
            return Err(ArchiveError::Duplicate("a repeated name".into()));
        }
        let mut items = Vec::with_capacity(zip.len());
        let mut seen: HashSet<String> = HashSet::with_capacity(zip.len());
        let mut total: u64 = 0;
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|_| ArchiveError::Unreadable("a file's header is damaged".into()))?;
            let name = safe_name(entry.name())?;
            if is_junk(&name) {
                continue;
            }
            if !seen.insert(name.to_lowercase()) {
                return Err(ArchiveError::Duplicate(name));
            }
            let mode_type = entry.unix_mode().map_or(0, |m| m & 0o170000);
            if entry.is_symlink() || mode_type == 0o120000 {
                return Err(ArchiveError::Symlink(name));
            }
            if !matches!(mode_type, 0 | 0o100000 | 0o040000) {
                return Err(ArchiveError::Special(name));
            }
            if entry.encrypted() {
                return Err(ArchiveError::Encrypted(name));
            }
            if entry.is_dir() {
                continue;
            }
            let nested = is_nested_name(&name);
            if nested && limits.nested == Nested::Refuse {
                return Err(ArchiveError::Nested(name));
            }
            let (size, compressed) = (entry.size(), entry.compressed_size());
            if size > limits.max_entry_bytes {
                return Err(ArchiveError::TooLarge(format!(
                    "{name:?} would expand past {} MB",
                    limits.max_entry_bytes / (1024 * 1024)
                )));
            }
            if size > limits.ratio_floor && size / compressed.max(1) > limits.max_ratio {
                return Err(ArchiveError::Ratio(name));
            }
            total = total.saturating_add(size);
            if total > limits.max_total_bytes {
                return Err(ArchiveError::TooLarge(format!(
                    "it would expand past {} MB",
                    limits.max_total_bytes / (1024 * 1024)
                )));
            }
            items.push(Item {
                name,
                size,
                compressed,
                index,
                nested,
            });
        }
        Ok(Export {
            input: Input::Zip(zip),
            items,
            limits,
            read_total: 0,
            links_skipped: 0,
        })
    }

    fn open_dir(root: &Path, limits: Limits) -> Result<Export, ArchiveError> {
        let mut items = Vec::new();
        let mut links = 0usize;
        let mut total: u64 = 0;
        let mut visited = 0usize;
        let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
        while let Some((dir, prefix, depth)) = stack.pop() {
            let read =
                std::fs::read_dir(&dir).map_err(|e| ArchiveError::Unreadable(e.to_string()))?;
            for entry in read.flatten() {
                visited += 1;
                if visited > limits.max_entries {
                    return Err(ArchiveError::TooManyEntries(limits.max_entries));
                }
                let Some(file) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let name = if prefix.is_empty() {
                    file
                } else {
                    format!("{prefix}/{file}")
                };
                if is_junk(&name) {
                    continue;
                }
                let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                    continue;
                };
                let kind = meta.file_type();
                if kind.is_symlink() {
                    links += 1;
                } else if kind.is_dir() {
                    if depth < limits.max_depth {
                        stack.push((entry.path(), name, depth + 1));
                    }
                } else if kind.is_file() {
                    let size = meta.len();
                    if size > limits.max_entry_bytes {
                        return Err(ArchiveError::TooLarge(format!(
                            "{name:?} is over {} MB",
                            limits.max_entry_bytes / (1024 * 1024)
                        )));
                    }
                    total = total.saturating_add(size);
                    if total > limits.max_total_bytes {
                        return Err(ArchiveError::TooLarge(format!(
                            "the folder is over {} MB",
                            limits.max_total_bytes / (1024 * 1024)
                        )));
                    }
                    let nested = is_nested_name(&name);
                    if nested && limits.nested == Nested::Refuse {
                        return Err(ArchiveError::Nested(name));
                    }
                    items.push(Item {
                        name,
                        size,
                        compressed: size,
                        index: 0,
                        nested,
                    });
                }
            }
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Export {
            input: Input::Dir(root.to_path_buf()),
            items,
            limits,
            read_total: 0,
            links_skipped: links,
        })
    }

    /// Every file, folders left out, in name order for a folder and archive
    /// order for a zip.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Stream file `name` into `f` through a capped reader. Refuses a name
    /// that isn't a listed file, and any nested archive.
    pub fn read<T>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut dyn Read) -> T,
    ) -> Result<T, ArchiveError> {
        let item = self
            .items
            .iter()
            .find(|i| i.name == name)
            .filter(|i| !i.nested)
            .cloned()
            .ok_or_else(|| ArchiveError::Missing(name.to_string()))?;
        let caps = Caps {
            name: item.name.clone(),
            claimed: item.size.min(self.limits.max_entry_bytes),
            compressed: item.compressed,
            ratio: self.limits.max_ratio,
            floor: self.limits.ratio_floor,
            budget: self.limits.max_total_bytes.saturating_sub(self.read_total),
        };
        let (out, read, failure) = match &mut self.input {
            Input::Zip(zip) => {
                let mut entry = zip
                    .by_index(item.index)
                    .map_err(|_| ArchiveError::Unreadable("a file can't be inflated".into()))?;
                run_capped(caps, &mut entry, f)
            }
            Input::Dir(root) => {
                let path = root.join(&item.name);
                let meta = std::fs::symlink_metadata(&path)
                    .map_err(|_| ArchiveError::Missing(name.to_string()))?;
                if !meta.is_file() {
                    return Err(ArchiveError::Symlink(item.name));
                }
                let mut file =
                    File::open(&path).map_err(|e| ArchiveError::Unreadable(e.to_string()))?;
                run_capped(caps, &mut file, f)
            }
        };
        self.read_total = self.read_total.saturating_add(read);
        match failure {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }
}

/// The caps one read runs under.
struct Caps {
    name: String,
    claimed: u64,
    compressed: u64,
    ratio: u64,
    floor: u64,
    budget: u64,
}

fn run_capped<T>(
    caps: Caps,
    inner: &mut dyn Read,
    f: impl FnOnce(&mut dyn Read) -> T,
) -> (T, u64, Option<ArchiveError>) {
    let mut capped = Capped {
        inner,
        caps,
        read: 0,
        failure: None,
    };
    let out = f(&mut capped);
    (out, capped.read, capped.failure)
}

/// A reader that stops with a named failure at the entry's caps.
struct Capped<'a> {
    inner: &'a mut dyn Read,
    caps: Caps,
    read: u64,
    failure: Option<ArchiveError>,
}

impl Read for Capped<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::Error::other("archive limit"));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        let c = &self.caps;
        let failure = if self.read > c.claimed {
            Some(ArchiveError::TooLarge(format!(
                "{:?} is larger than it says",
                c.name
            )))
        } else if self.read > c.budget {
            Some(ArchiveError::TooLarge(
                "it expands past the total limit".into(),
            ))
        } else if self.read > c.floor && self.read / c.compressed.max(1) > c.ratio {
            Some(ArchiveError::Ratio(c.name.clone()))
        } else {
            None
        };
        if let Some(e) = failure {
            self.failure = Some(e);
            return Err(io::Error::other("archive limit"));
        }
        Ok(n)
    }
}

// ── streaming a JSON array ────────────────────────────────────────────

/// How a streamed array ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Streamed {
    /// Every element was handed over.
    Done,
    /// The callback asked to stop.
    Stopped,
    /// The input isn't a JSON array, or broke part way; the elements before
    /// the break were handed over.
    Broken(String),
}

struct Each<'a, F>(&'a mut F);

impl<'de, F: FnMut(Value) -> bool> serde::de::Visitor<'de> for Each<'_, F> {
    type Value = bool;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON array")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<bool, A::Error> {
        while let Some(value) = seq.next_element::<Value>()? {
            if !(self.0)(value) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The most bytes one array element may take (a conversation).
pub(crate) const MAX_ELEMENT_BYTES: u64 = 256 * 1024 * 1024;

/// Counts the bytes read since the last element boundary and fails the read
/// past `max`, so one giant element is never built whole: the parser stops
/// after about `max` bytes instead of allocating it.
struct ElementReader<R> {
    inner: R,
    since: std::rc::Rc<std::cell::Cell<u64>>,
    max: u64,
}

impl<R: Read> Read for ElementReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.since.get() > self.max {
            return Err(io::Error::other("element over the size limit"));
        }
        let n = self.inner.read(buf)?;
        self.since.set(self.since.get() + n as u64);
        Ok(n)
    }
}

/// Parse a top-level JSON array from `reader`, handing each element to `f`
/// as it is read: one element in memory at a time, never the whole array.
/// `f` returns `false` to stop. An element over `max_element` bytes ends the
/// stream as [`Streamed::Broken`] (a parser can't resync inside one); the
/// elements before it were handed over.
pub(crate) fn for_each_element(
    reader: impl Read,
    max_element: u64,
    mut f: impl FnMut(Value) -> bool,
) -> Streamed {
    use serde::Deserializer;
    let since = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let counted = ElementReader {
        inner: reader,
        since: since.clone(),
        max: max_element,
    };
    let mut de =
        serde_json::Deserializer::from_reader(BufReader::with_capacity(64 * 1024, counted));
    let mut stopped = false;
    let mut each = |value: Value| {
        // Read-ahead of the next element already counted is at most one
        // buffer; the budget restarts here.
        since.set(0);
        let more = f(value);
        stopped = !more;
        more
    };
    let result = de.deserialize_seq(Each(&mut each));
    match result {
        Ok(true) => Streamed::Done,
        // Stopping early leaves the array's tail unread, which serde_json
        // reports as an error after the callback said stop.
        Ok(false) => Streamed::Stopped,
        Err(_) if stopped => Streamed::Stopped,
        Err(e) => Streamed::Broken(e.to_string()),
    }
}

// ── staging a conversation export ─────────────────────────────────────

/// Most conversations one export stages (bounds the staging folder).
pub(crate) const MAX_STAGED: usize = 100_000;
/// File names kept in [`Staged::other_files`].
const MAX_LISTED_FILES: usize = 500;

/// What [`stage_conversations`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Staged {
    /// Shard files read (`conversations.json`, `conversations-000.json`...).
    pub shards: usize,
    /// Conversations written, one JSON file each.
    pub conversations: usize,
    /// Array elements that weren't objects or had no usable id.
    pub malformed: usize,
    /// Conversations whose id was already staged from another shard.
    pub duplicates: usize,
    /// Shards that broke part way, or couldn't be read (named).
    pub damaged: Vec<String>,
    /// More than [`MAX_STAGED`]: the rest weren't staged.
    pub capped: bool,
    /// Every other file in the export, by name only (N28: attachments are
    /// never copied), at most 500 names.
    pub other_files: Vec<String>,
    pub other_file_count: usize,
    /// Nested archives, never opened.
    pub nested_skipped: usize,
}

/// A staged conversation's file name: the hash of its id, so the same id
/// always lands in the same file and no id is ever a path.
pub(crate) fn staged_file_name(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{}.json",
        &format!("{:x}", Sha256::digest(id.as_bytes()))[..24]
    )
}

/// Fill `out`'s nested-archive and other-file lists: every item that isn't
/// one of `shards` (names only, N28). Shared by [`stage_conversations`] and
/// the Gemini stager, which groups records itself.
pub(crate) fn tally_other_files(export: &Export, shards: &[String], out: &mut Staged) {
    for item in export.items() {
        if item.nested {
            out.nested_skipped += 1;
        } else if !shards.contains(&item.name) {
            out.other_file_count += 1;
            if out.other_files.len() < MAX_LISTED_FILES {
                out.other_files.push(item.name.clone());
            }
        }
    }
}

/// Split every shard of `export` (a file for which `is_shard(name)`, in
/// name order) into one JSON file per conversation in `dir`, streaming: a
/// shard of hundreds of MB is never held whole. `id_of` names a
/// conversation or returns `None` for an element that isn't one. Stops
/// between elements when `cancel` is set (the caller discards `dir`).
pub(crate) fn stage_conversations(
    export: &mut Export,
    dir: &Path,
    is_shard: impl Fn(&str) -> bool,
    id_of: impl Fn(&Value) -> Option<String>,
    cancel: &AtomicBool,
) -> Result<Staged, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("staging folder: {e}"))?;
    let mut shards: Vec<String> = export
        .items()
        .iter()
        .filter(|i| !i.nested && is_shard(&i.name))
        .map(|i| i.name.clone())
        .collect();
    shards.sort();
    let mut out = Staged::default();
    tally_other_files(export, &shards, &mut out);
    let mut seen: HashSet<String> = HashSet::new();
    let mut write_error: Option<String> = None;
    for shard in &shards {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        out.shards += 1;
        let read = export.read(shard, |reader| {
            for_each_element(reader, MAX_ELEMENT_BYTES, |value| {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                let Some(id) = id_of(&value) else {
                    out.malformed += 1;
                    return true;
                };
                let file = staged_file_name(&id);
                if seen.contains(&file) {
                    out.duplicates += 1;
                    return true;
                }
                if seen.len() >= MAX_STAGED {
                    out.capped = true;
                    return false;
                }
                let written = serde_json::to_vec(&value)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        File::create(dir.join(&file))
                            .and_then(|mut f| f.write_all(&bytes))
                            .map_err(|e| e.to_string())
                    });
                match written {
                    Ok(()) => {
                        seen.insert(file);
                        out.conversations += 1;
                        true
                    }
                    Err(e) => {
                        write_error = Some(e);
                        false
                    }
                }
            })
        });
        if let Some(e) = write_error.take() {
            return Err(format!("staging write: {e}"));
        }
        match read {
            // A bomb or an entry that lies about its size refuses the export.
            Err(e @ (ArchiveError::Ratio(_) | ArchiveError::TooLarge(_))) => {
                return Err(e.to_string())
            }
            Ok(Streamed::Broken(_)) | Err(_) => out.damaged.push(shard.clone()),
            Ok(_) => {}
        }
        if out.capped {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// A zip at `path` holding `entries`, built here (no real data).
    pub(crate) fn build_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zw = zip::ZipWriter::new(File::create(path).unwrap());
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(bytes).unwrap();
        }
        zw.finish().unwrap();
    }

    fn open(entries: &[(&str, &[u8])], limits: Limits) -> Result<Export, ArchiveError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.zip");
        build_zip(&path, entries);
        Export::open(&path, limits)
    }

    fn err(entries: &[(&str, &[u8])], limits: Limits) -> ArchiveError {
        match open(entries, limits) {
            Ok(_) => panic!("the archive should have been refused"),
            Err(e) => e,
        }
    }

    #[test]
    fn a_clean_zip_lists_its_files_and_streams_them() {
        let mut ex = open(
            &[("conversations.json", b"[1,2]"), ("dir/file-a.png", b"png")],
            Limits::default(),
        )
        .unwrap();
        let names: Vec<&str> = ex.items().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["conversations.json", "dir/file-a.png"]);
        let text = ex
            .read("conversations.json", |r| {
                let mut s = String::new();
                r.read_to_string(&mut s).unwrap();
                s
            })
            .unwrap();
        assert_eq!(text, "[1,2]");
        assert!(matches!(
            ex.read("nope.json", |_| ()),
            Err(ArchiveError::Missing(_))
        ));
    }

    #[test]
    fn unsafe_names_refuse_the_archive() {
        for name in [
            "../evil.json",
            "a/../../evil.json",
            "/abs/evil.json",
            "C:/evil.json",
            "c:evil.json",
            "a\\b.json",
            "./x.json",
            "a//b.json",
            "tab\tname.json",
        ] {
            assert!(
                matches!(
                    err(&[(name, b"x")], Limits::default()),
                    ArchiveError::UnsafePath(_)
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn a_symlink_entry_refuses_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.zip");
        let mut zw = zip::ZipWriter::new(File::create(&path).unwrap());
        zw.start_file("conversations.json", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"[]").unwrap();
        zw.add_symlink("evil", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        zw.finish().unwrap();
        assert!(matches!(
            Export::open(&path, Limits::default()),
            Err(ArchiveError::Symlink(n)) if n == "evil"
        ));
    }

    #[test]
    fn nested_archives_are_refused_or_listed_and_never_opened() {
        let entries: &[(&str, &[u8])] = &[("conversations.json", b"[]"), ("upload.zip", b"PK")];
        assert!(matches!(
            err(entries, Limits::default()),
            ArchiveError::Nested(_)
        ));
        let mut ex = open(
            entries,
            Limits {
                nested: Nested::Ignore,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(ex
            .items()
            .iter()
            .any(|i| i.name == "upload.zip" && i.nested));
        assert!(matches!(
            ex.read("upload.zip", |_| ()),
            Err(ArchiveError::Missing(_))
        ));
    }

    #[test]
    fn duplicate_names_refuse_the_archive() {
        // Case folds on macOS, so a case-only duplicate counts.
        assert!(matches!(
            err(&[("a.json", b"1"), ("A.json", b"2")], Limits::default()),
            ArchiveError::Duplicate(_)
        ));
        // An exact duplicate (the writer refuses one, so rename after).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.zip");
        build_zip(&path, &[("aa.json", b"1"), ("bb.json", b"2")]);
        let mut bytes = std::fs::read(&path).unwrap();
        let needle = b"bb.json";
        let mut i = 0;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                bytes[i..i + needle.len()].copy_from_slice(b"aa.json");
            }
            i += 1;
        }
        std::fs::write(&path, bytes).unwrap();
        let r = Export::open(&path, Limits::default());
        assert!(
            matches!(
                r,
                Err(ArchiveError::Duplicate(_) | ArchiveError::Unreadable(_))
            ),
            "{:?}",
            r.map(|e| e.items().to_vec())
        );
    }

    #[test]
    fn entry_count_and_size_limits_refuse_the_archive() {
        let small = Limits {
            max_entries: 2,
            ..Limits::default()
        };
        assert!(matches!(
            err(&[("a", b"1"), ("b", b"2"), ("c", b"3")], small),
            ArchiveError::TooManyEntries(2)
        ));
        let entry = Limits {
            max_entry_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(
            err(&[("a.json", &[b'x'; 100])], entry),
            ArchiveError::TooLarge(_)
        ));
        let total = Limits {
            max_total_bytes: 150,
            ..Limits::default()
        };
        assert!(matches!(
            err(&[("a", &[b'x'; 100]), ("b", &[b'y'; 100])], total),
            ArchiveError::TooLarge(_)
        ));
    }

    #[test]
    fn a_zip_bomb_is_refused_by_its_ratio() {
        // 20 MB of zeros deflates to about 20 KB: a ratio near 1000.
        let zeros = vec![0u8; 20 * 1024 * 1024];
        assert!(matches!(
            err(&[("conversations.json", &zeros)], Limits::default()),
            ArchiveError::Ratio(_)
        ));
    }

    #[test]
    fn a_huge_claimed_size_is_refused_before_anything_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claim.zip");
        build_zip(&path, &[("conversations.json", b"[]")]);
        let mut bytes = std::fs::read(&path).unwrap();
        // Central directory header: signature, then the uncompressed size
        // 24 bytes in.
        let at = bytes
            .windows(4)
            .position(|w| w == [0x50, 0x4b, 0x01, 0x02])
            .unwrap()
            + 24;
        bytes[at..at + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let e = Export::open(&path, Limits::default()).err().unwrap();
        assert!(
            matches!(e, ArchiveError::TooLarge(_) | ArchiveError::Ratio(_)),
            "{e:?}"
        );
    }

    #[test]
    fn a_read_stops_at_the_caps_even_when_the_header_is_honest() {
        // Ratio floor lowered so a small bomb trips the streaming check
        // (the open-time check is skipped by a floor above the claim).
        let zeros = vec![0u8; 4 * 1024 * 1024];
        let limits = Limits {
            ratio_floor: u64::MAX / 2,
            ..Limits::default()
        };
        let mut ex = open(&[("a.json", &zeros)], limits).unwrap();
        ex.limits.ratio_floor = 1024;
        let r = ex.read("a.json", |r| io::copy(r, &mut io::sink()).is_err());
        assert!(matches!(r, Err(ArchiveError::Ratio(_))), "{r:?}");
        // A second read past the total budget stops too.
        let mut ex = open(
            &[("a", &[b'x'; 100]), ("b", &[b'y'; 100])],
            Limits::default(),
        )
        .unwrap();
        ex.limits.max_total_bytes = 150;
        ex.read("a", |r| io::copy(r, &mut io::sink()).unwrap())
            .unwrap();
        let r = ex.read("b", |r| io::copy(r, &mut io::sink()).is_err());
        assert!(matches!(r, Err(ArchiveError::TooLarge(_))), "{r:?}");
    }

    #[test]
    fn corrupt_and_truncated_zips_are_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.zip");
        std::fs::write(&junk, b"this is not a zip file at all").unwrap();
        assert!(matches!(
            Export::open(&junk, Limits::default()),
            Err(ArchiveError::Unreadable(_))
        ));
        let good = dir.path().join("good.zip");
        build_zip(&good, &[("conversations.json", b"[1,2,3]")]);
        let bytes = std::fs::read(&good).unwrap();
        let cut = dir.path().join("cut.zip");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(
            Export::open(&cut, Limits::default()),
            Err(ArchiveError::Unreadable(_))
        ));
        assert!(matches!(
            Export::open(&dir.path().join("missing.zip"), Limits::default()),
            Err(ArchiveError::Unreadable(_))
        ));
    }

    #[test]
    fn a_folder_is_read_with_links_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("export");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("conversations.json"), b"[]").unwrap();
        std::fs::write(root.join("sub/pic.png"), b"png").unwrap();
        std::fs::write(root.join(".DS_Store"), b"junk").unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"outside").unwrap();
        std::os::unix::fs::symlink(&secret, root.join("link.json")).unwrap();
        let ex = Export::open(&root, Limits::default()).unwrap();
        let names: Vec<&str> = ex.items().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["conversations.json", "sub/pic.png"]);
        assert_eq!(ex.links_skipped, 1);
        // The chosen folder itself may not be a link.
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert!(matches!(
            Export::open(&alias, Limits::default()),
            Err(ArchiveError::Symlink(_))
        ));
    }

    #[test]
    fn an_array_streams_element_by_element_and_reports_a_break() {
        let mut seen = Vec::new();
        let r = for_each_element(&b"[1, {\"a\":2}, \"x\"]"[..], 1000, |v| {
            seen.push(v);
            true
        });
        assert_eq!(r, Streamed::Done);
        assert_eq!(seen.len(), 3);
        let mut first = 0;
        let r = for_each_element(&b"[1,2,3]"[..], 1000, |_| {
            first += 1;
            false
        });
        assert_eq!((r, first), (Streamed::Stopped, 1));
        let mut got = 0;
        let r = for_each_element(&b"[{\"a\":1},{\"b\":"[..], 1000, |_| {
            got += 1;
            true
        });
        assert!(matches!(r, Streamed::Broken(_)));
        assert_eq!(got, 1);
        assert!(matches!(
            for_each_element(&b"{\"not\":\"an array\"}"[..], 1000, |_| true),
            Streamed::Broken(_)
        ));
    }

    #[test]
    fn one_oversized_element_ends_the_stream_without_being_built() {
        // Built at run time: a small element, then one far over the cap.
        let mut json = String::from("[{\"id\":\"small\"},{\"id\":\"big\",\"pad\":\"");
        json.push_str(&"x".repeat(2_000_000));
        json.push_str("\"},{\"id\":\"after\"}]");
        let mut ids = Vec::new();
        let r = for_each_element(json.as_bytes(), 100_000, |v| {
            ids.push(v["id"].as_str().unwrap().to_string());
            true
        });
        assert!(matches!(r, Streamed::Broken(_)), "{r:?}");
        assert_eq!(ids, vec!["small".to_string()]);
        // Under the cap it all passes.
        let mut n = 0;
        let r = for_each_element(json.as_bytes(), 4_000_000, |_| {
            n += 1;
            true
        });
        assert_eq!((r, n), (Streamed::Done, 3));
    }
}
