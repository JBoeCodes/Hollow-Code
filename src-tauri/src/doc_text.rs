//! The text inside PDF and Office files (repo features R14a), read by the
//! host so every agent gets the same evidence: Claude, Codex and Grok alike.
//!
//! - **Word** (`.docx`): one paragraph per line (the `w:t` runs in order).
//! - **PowerPoint** (`.pptx`): `## Slide N` before each slide's paragraphs,
//!   slides in number order (slide10 after slide9).
//! - **Excel** (`.xlsx`): `xlsx_cells::render`, sheets then rows then typed
//!   cells (never the shared-string pool).
//! - **PDF**: `## Page N` before each page's text layer, through PDFKit
//!   (macOS only; an error elsewhere). A PDF with no text on any page is
//!   `image-only PDF: no text layer (needs OCR)`; a locked one is
//!   `password-protected PDF`. There is no OCR.
//!
//! Caps ([`LIMITS`]): a file of at most 64 MB, at most 500 pages or slides,
//! at most 4 MB of text and 50 MB of uncompressed Office XML (a zip bomb
//! inflates no further). Anything a cap cut is stated on the first line,
//! `[truncated: …]`, as `xlsx_cells::render` does, so a clip can't hide it.
//!
//! Extraction is blocking work (PDFKit is synchronous, a large deck takes a
//! moment): call it from a blocking thread, never on an async worker. A PDF
//! document lives and dies on the calling thread inside an autorelease pool.
//!
//! [`cached`] keeps extracted text in memory, keyed by `(path, size, mtime)`,
//! at most 64 MB and 512 files in all, oldest first out, so retrieval reading the same deck
//! on every turn parses it once. A failed read is cached too (it is short),
//! so an image-only PDF isn't re-parsed each turn; a changed file re-reads.
//! Its parses run behind a locked two-strike crash marker in the app cache
//! ([`guarded`], overnight 9 A0b): a file that crashed the reader twice is
//! refused until it changes, so one bad PDF in `sources/` can't take Hollow
//! down on every turn. The content index parses behind the same marker
//! ([`extract_guarded`]), so the two strikes are the file's, not each
//! reader's. A reader that finds the file's parse still running after 5 s
//! gets [`BEING_READ`], not a second parse, and it isn't cached.
//!
//! The Office reader moved here from `m365_mcp` (where it read OneDrive
//! files), which now calls [`office_text`].
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What one read may take.
pub(crate) struct Limits {
    /// File size on disk.
    pub file_bytes: u64,
    /// PDF pages or PowerPoint slides.
    pub units: usize,
    /// Extracted text kept.
    pub text_bytes: usize,
    /// Uncompressed Office XML read from the archive.
    pub xml_bytes: u64,
}

pub(crate) const LIMITS: Limits = Limits {
    file_bytes: 64 * 1024 * 1024,
    units: 500,
    text_bytes: 4 * 1024 * 1024,
    xml_bytes: 50 * 1024 * 1024,
};

/// Extracted text the in-memory cache holds, across files.
const CACHE_BYTES: usize = 64 * 1024 * 1024;
/// Entries it holds (a lookup is a linear scan; most are small).
const CACHE_ENTRIES: usize = 512;

/// A document format Hollow reads itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocKind {
    Pdf,
    Docx,
    Pptx,
    Xlsx,
}

/// The document format `path`'s extension names, if Hollow reads it.
pub(crate) fn kind(path: &Path) -> Option<DocKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some(DocKind::Pdf),
        "docx" => Some(DocKind::Docx),
        "pptx" => Some(DocKind::Pptx),
        "xlsx" => Some(DocKind::Xlsx),
        _ => None,
    }
}

/// Whether `head` (a file's first bytes) is the format `path`'s extension
/// names: `%PDF-` for a PDF, a zip's `PK\x03\x04` for Office.
pub(crate) fn looks_like(path: &Path, head: &[u8]) -> bool {
    match kind(path) {
        Some(DocKind::Pdf) => head.starts_with(b"%PDF-"),
        Some(_) => head.starts_with(b"PK\x03\x04"),
        None => false,
    }
}

/// One file's text. `units` fills the content index's `files.units`
/// (`project_text_index`, R14b); `kind` is read by tests only.
pub(crate) struct Extracted {
    pub text: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub kind: DocKind,
    /// Pages (PDF), slides (PowerPoint), sheets (Excel) or paragraphs
    /// (Word) read.
    pub units: usize,
}

/// The text of the PDF or Office file at `path` (see the module doc).
pub(crate) fn extract(path: &Path) -> Result<Extracted, String> {
    #[cfg(test)]
    seam::entered(path);
    extract_with(path, &LIMITS)
}

/// Test seam: how often [`extract`] ran for a path, and a hook that runs in
/// it first (on whatever thread parses), per path so parallel tests don't meet.
#[cfg(test)]
pub(crate) mod seam {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    type Hook = Box<dyn FnOnce() + Send>;
    static PARSES: Mutex<Vec<(PathBuf, u32)>> = Mutex::new(Vec::new());
    static HOOKS: Mutex<Vec<(PathBuf, Hook)>> = Mutex::new(Vec::new());

    /// Run `hook` at the start of the next [`super::extract`] of `path`.
    pub(crate) fn on_extract(path: &Path, hook: impl FnOnce() + Send + 'static) {
        let mut hooks = HOOKS.lock().unwrap_or_else(|e| e.into_inner());
        hooks.push((path.to_path_buf(), Box::new(hook)));
    }

    /// [`super::extract`] calls for `path` so far.
    pub(crate) fn extracts(path: &Path) -> u32 {
        let parses = PARSES.lock().unwrap_or_else(|e| e.into_inner());
        parses
            .iter()
            .find(|(p, _)| p == path)
            .map_or(0, |(_, n)| *n)
    }

    pub(super) fn entered(path: &Path) {
        {
            let mut parses = PARSES.lock().unwrap_or_else(|e| e.into_inner());
            match parses.iter_mut().find(|(p, _)| p == path) {
                Some((_, n)) => *n += 1,
                None => parses.push((path.to_path_buf(), 1)),
            }
        }
        let hook = {
            let mut hooks = HOOKS.lock().unwrap_or_else(|e| e.into_inner());
            let at = hooks.iter().position(|(p, _)| p == path);
            at.map(|i| hooks.remove(i).1)
        };
        if let Some(hook) = hook {
            hook();
        }
    }
}

fn extract_with(path: &Path, lim: &Limits) -> Result<Extracted, String> {
    let kind = kind(path).ok_or("not a PDF or Office file")?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let too_large = || {
        format!(
            "Not read: the file is larger than the {} MB document reader limit. Open the original, or export a smaller file.",
            lim.file_bytes / (1024 * 1024)
        )
    };
    if len > lim.file_bytes {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(lim.file_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > lim.file_bytes {
        return Err(too_large());
    }
    if kind == DocKind::Pdf {
        // Handed to NSData without a copy.
        let (body, units, cut) = pdf_pages(bytes, lim)?;
        return Ok(finish_extracted(body, units, cut, kind, lim));
    }
    extract_bytes(&bytes, kind, lim)
}

/// [`extract`] on bytes already read.
pub(crate) fn extract_bytes(
    bytes: &[u8],
    kind: DocKind,
    lim: &Limits,
) -> Result<Extracted, String> {
    let (body, units, cut) = match kind {
        DocKind::Pdf => pdf_pages(bytes.to_vec(), lim)?,
        DocKind::Xlsx => {
            // The workbook states its own cuts on its first line.
            let wb = crate::xlsx_cells::read(bytes, &crate::xlsx_cells::LIMITS)?;
            (crate::xlsx_cells::render(&wb), wb.sheets.len(), Vec::new())
        }
        DocKind::Docx | DocKind::Pptx => office_parts(bytes, kind, lim)?,
    };
    Ok(finish_extracted(body, units, cut, kind, lim))
}

fn finish_extracted(
    body: String,
    units: usize,
    cut: Vec<String>,
    kind: DocKind,
    lim: &Limits,
) -> Extracted {
    // A NUL would make the readers call the text binary.
    let body = if body.contains('\0') {
        body.replace('\0', " ")
    } else {
        body
    };
    Extracted {
        text: finish(body, cut, lim),
        kind,
        units,
    }
}

/// `body` cut to the text cap at a line end, with every cut stated first.
fn finish(mut body: String, mut cut: Vec<String>, lim: &Limits) -> String {
    if body.len() > lim.text_bytes {
        let total = body.len();
        let mut at = lim.text_bytes;
        while !body.is_char_boundary(at) {
            at -= 1;
        }
        if let Some(nl) = body[..at].rfind('\n') {
            at = nl + 1;
        }
        body.truncate(at);
        cut.push(format!("first {at} of {total} bytes of text"));
    }
    if cut.is_empty() {
        body
    } else {
        format!("[truncated: {}]\n\n{body}", cut.join("; "))
    }
}

/// The text in a Word or PowerPoint file (the XML text runs, in order), or an
/// Excel file's cells by sheet and row, typed and capped (`xlsx_cells`).
/// `kind` is the extension: `docx`, `pptx` or `xlsx`. `lim` caps the read
/// (OneDrive keeps far less text than [`LIMITS`] allows).
pub(crate) fn office_text(bytes: &[u8], kind: &str, lim: &Limits) -> Result<String, String> {
    let kind = match kind {
        "xlsx" => DocKind::Xlsx,
        "docx" => DocKind::Docx,
        _ => DocKind::Pptx,
    };
    extract_bytes(bytes, kind, lim).map(|e| e.text)
}

/// A Word or PowerPoint file's paragraphs, the units read and the cuts.
fn office_parts(
    bytes: &[u8],
    kind: DocKind,
    lim: &Limits,
) -> Result<(String, usize, Vec<String>), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "not a readable Office file".to_string())?;
    let mut names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let docx = kind == DocKind::Docx;
    let (wanted, run): (Box<dyn Fn(&str) -> bool>, &str) = if docx {
        (Box::new(|n: &str| n == "word/document.xml"), "w:t")
    } else {
        (
            Box::new(|n: &str| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")),
            "a:t",
        )
    };
    // Slides in number order (slide10 after slide9).
    names.sort_by_key(|n| (n.len(), n.clone()));
    let parts: Vec<String> = names.into_iter().filter(|n| wanted(n)).collect();
    let run_re =
        regex::Regex::new(&format!(r"(?s)<{run}(?:\s[^>]*)?>(.*?)</{run}>")).expect("run pattern");
    let mut out = String::new();
    let mut cut = Vec::new();
    let mut read = 0u64;
    let mut units = 0usize;
    for (i, name) in parts.iter().enumerate() {
        if !docx && i >= lim.units {
            cut.push(format!("first {} of {} slides", lim.units, parts.len()));
            break;
        }
        // The cap was reached exactly by the parts before: this one is left out.
        if read >= lim.xml_bytes {
            cut.push(xml_cap_note(lim.xml_bytes));
            break;
        }
        let mut entry = zip.by_name(name).map_err(|e| e.to_string())?;
        // Bounded: a hostile archive can't inflate past the XML cap. Raw
        // bytes, one past the budget, so a part exactly the budget long
        // isn't read as cut and a cut can land inside a character.
        let budget = lim.xml_bytes - read;
        let mut raw = Vec::new();
        (&mut entry)
            .take(budget + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "unreadable Office XML".to_string())?;
        let over = raw.len() as u64 > budget;
        raw.truncate(budget as usize);
        read += raw.len() as u64;
        let xml = match String::from_utf8(raw) {
            Ok(xml) => xml,
            // Only a character cut at the very end is the cap's doing (RF-794).
            Err(e) if over && e.utf8_error().error_len().is_none() => {
                let valid = e.utf8_error().valid_up_to();
                let mut raw = e.into_bytes();
                raw.truncate(valid);
                String::from_utf8(raw).expect("valid up to the cut")
            }
            Err(_) => return Err("unreadable Office XML".to_string()),
        };
        if !docx {
            if !out.is_empty() {
                out.push('\n');
            }
            let number = name
                .trim_start_matches("ppt/slides/slide")
                .trim_end_matches(".xml");
            out.push_str(&format!("## Slide {number}\n"));
            units += 1;
        }
        // Paragraph ends become line breaks.
        let para_end = if docx { "</w:p>" } else { "</a:p>" };
        for para in xml.split(para_end) {
            let line: String = run_re
                .captures_iter(para)
                .map(|c| c[1].to_string())
                .collect();
            if !line.trim().is_empty() {
                out.push_str(&unescape(&line));
                out.push('\n');
                if docx {
                    units += 1;
                }
            }
        }
        if over {
            cut.push(xml_cap_note(lim.xml_bytes));
            break;
        }
        if out.len() > lim.text_bytes {
            break; // `finish` cuts it and says so
        }
    }
    Ok((out, units, cut))
}

/// The cut note for XML past the cap.
fn xml_cap_note(cap: u64) -> String {
    if cap.is_multiple_of(1024 * 1024) {
        format!("first {} MB of uncompressed XML", cap / (1024 * 1024))
    } else {
        format!("first {cap} bytes of uncompressed XML")
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A PDF's text layer, page by page, through PDFKit.
#[cfg(target_os = "macos")]
fn pdf_pages(bytes: Vec<u8>, lim: &Limits) -> Result<(String, usize, Vec<String>), String> {
    use objc2::AllocAnyThread;
    use objc2_foundation::NSData;
    use objc2_pdf_kit::PDFDocument;
    objc2::rc::autoreleasepool(|_| {
        // The Vec's buffer becomes the NSData's, without a copy.
        let data = NSData::from_vec(bytes);
        // SAFETY: a freshly allocated PDFDocument initialised from data it
        // retains; it stays on this thread and is released in this pool.
        let doc = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
            .ok_or("not a readable PDF")?;
        // SAFETY: plain property reads on a live document.
        if unsafe { doc.isLocked() } {
            return Err("password-protected PDF".to_string());
        }
        let pages = unsafe { doc.pageCount() };
        let mut out = String::new();
        let mut cut = Vec::new();
        let mut any_text = false;
        let mut read = 0usize;
        for i in 0..pages {
            if read >= lim.units {
                cut.push(format!("first {} of {pages} pages", lim.units));
                break;
            }
            if out.len() > lim.text_bytes {
                break; // `finish` cuts it and says so
            }
            read += 1;
            let text = objc2::rc::autoreleasepool(|_| {
                // SAFETY: `i` is below the page count; the page and its
                // string are retained, then released with this pool.
                let page = unsafe { doc.pageAtIndex(i) }?;
                unsafe { page.string() }.map(|s| s.to_string())
            })
            .unwrap_or_default();
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("## Page {}\n", i + 1));
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            let text = text.trim_end();
            if !text.trim().is_empty() {
                any_text = true;
                out.push_str(text);
                out.push('\n');
            }
        }
        if pages == 0 {
            return Err("not a readable PDF: it has no pages".to_string());
        }
        if !any_text {
            return Err("image-only PDF: no text layer (needs OCR)".to_string());
        }
        Ok((out, read, cut))
    })
}

#[cfg(not(target_os = "macos"))]
fn pdf_pages(_bytes: Vec<u8>, _lim: &Limits) -> Result<(String, usize, Vec<String>), String> {
    Err("PDF text is read on macOS only".into())
}

type Key = (PathBuf, u64, Option<std::time::SystemTime>);
struct Cache {
    /// Oldest first.
    entries: VecDeque<(Key, Result<Arc<str>, String>)>,
    bytes: usize,
}
static CACHE: Mutex<Cache> = Mutex::new(Cache {
    entries: VecDeque::new(),
    bytes: 0,
});

fn weight(r: &Result<Arc<str>, String>) -> usize {
    match r {
        Ok(text) => text.len(),
        Err(e) => e.len(),
    }
}

/// What a file that crashed the reader twice reads as ([`guarded`]).
pub(crate) const CRASHED_READER: &str =
    "This file crashed Hollow's document reader earlier, so it isn't read again until it changes. Open it outside Hollow.";

/// What a file reads as while another parse of it is still running after
/// [`HELD_WAIT`] ([`guarded`]). Never cached: the next read tries again.
pub(crate) const BEING_READ: &str =
    "Not read yet: Hollow is still reading this file for another request. Try again in a moment.";

/// How long [`guarded`] waits for a parse of the same file in flight
/// elsewhere before it answers [`BEING_READ`] (RF-798-F1).
const HELD_WAIT: Duration = Duration::from_secs(5);

/// Where [`guarded`] keeps its parse markers: the app cache, outside the vault.
fn markers_dir() -> PathBuf {
    #[cfg(test)]
    return crate::test_temp::process_dir("hollow-doc-markers-");
    #[cfg(not(test))]
    crate::instance::cache_dir().join("doc-text-parsing")
}

/// The marker file of `path` as `meta` describes it: a changed file is a new
/// file, with no strikes.
fn marker_name(path: &Path, meta: &std::fs::Metadata) -> String {
    use sha2::{Digest, Sha256};
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let mut hash = Sha256::new();
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update(format!("\0{}\0{mtime}", meta.len()).as_bytes());
    format!("{:x}", hash.finalize())
}

/// The strikes a marker's text records: its first word, or one for text that
/// isn't a number (a torn write still counts the crash it marked).
fn strikes_in(text: &str) -> u32 {
    match text.split_whitespace().next() {
        None => 0,
        Some(n) => n.parse().unwrap_or(1),
    }
}

/// Run `parse` for `path` behind a two-strike marker (overnight 9 A0b).
/// PDFKit runs in this process, and a file that crashes it (an Objective-C
/// exception or a fault can't be caught, and release builds abort on panic)
/// would crash Hollow again on every turn that attaches it.
///
/// The marker holds the strike count and is held under an exclusive lock
/// for the parse ([`std::fs::File::try_lock`]); after the parse it is
/// emptied and removed, whatever the parse returned. A lock dies with its
/// process, so a marker nobody holds that still counts a strike was left by
/// a parse that never returned: a crash (or Hollow quit mid-parse). After
/// one the file is parsed once more; after two it is refused with
/// [`CRASHED_READER`] until it changes. A marker someone holds is a parse in
/// flight in this or another process (the app, the content index's pass and
/// a `hollow vault-mcp` subprocess read the same sources), never a strike:
/// this reader waits up to [`HELD_WAIT`] for its lock, polling, and then reads
/// the marker as it is left (gone: the parse finished, so a fresh marker and
/// a parse; still counting: it crashed, so its strikes count). A holder still
/// running then (a parse that hangs, or a slow one) gets [`BEING_READ`], an
/// error [`cached`] doesn't keep: one hung parse never starts another beside
/// it or holds its readers for good (RF-798-F1; until then a held marker
/// under two strikes was parsed unguarded, and A3's two-strike wait had no
/// deadline). A marker replaced between the open and the lock is opened
/// again, until the same deadline ([`BEING_READ`] after it), never parsed
/// unguarded. A marker that can't be made or locked leaves the parse
/// unguarded, as before.
fn guarded<T>(
    path: &Path,
    meta: &std::fs::Metadata,
    markers: &Path,
    parse: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;
    let marker = markers.join(marker_name(path, meta));
    let _ = std::fs::create_dir_all(markers);
    let deadline = Instant::now() + held_wait();
    let mut file = loop {
        let opened = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&marker);
        let Ok(file) = opened else {
            return parse();
        };
        match file.try_lock() {
            Ok(()) => {}
            // In flight elsewhere: wait for it rather than parse beside it.
            // If it crashes, its lock dies with it and the marker still
            // counts; if it finishes, the marker is gone (reopened below).
            Err(std::fs::TryLockError::WouldBlock) => {
                if !lock_by(&file, deadline) {
                    return Err(BEING_READ.to_string());
                }
            }
            Err(std::fs::TryLockError::Error(_)) => return parse(),
        }
        #[cfg(test)]
        AFTER_LOCK.with(|hook| {
            if let Some(f) = hook.take() {
                f(&marker)
            }
        });
        // A marker removed or replaced between the open and the lock isn't
        // the path's any more, and strikes written to it would be lost
        // (RF-794-F2): open the path again, as often as the wait allows
        // (each reader before this one replaces it once; RF-R1 review F2).
        let Some(ours) = file.metadata().ok().map(|m| m.ino()) else {
            return parse();
        };
        if std::fs::metadata(&marker).ok().map(|m| m.ino()) == Some(ours) {
            break file;
        }
        if Instant::now() >= deadline {
            return Err(BEING_READ.to_string());
        }
    };
    let mut left = String::new();
    let _ = file.rewind();
    let _ = file.read_to_string(&mut left);
    let strikes = strikes_in(&left);
    if strikes >= 2 {
        return Err(CRASHED_READER.to_string());
    }
    let _ = file.set_len(0);
    let _ = file.rewind();
    let _ = write!(file, "{}", strikes + 1);
    let result = parse();
    // Emptied first, so a reader that opened it before the removal counts
    // nothing; removed only while the path is still this marker.
    let _ = file.set_len(0);
    let ours = file.metadata().ok().map(|m| m.ino());
    if std::fs::metadata(&marker).ok().map(|m| m.ino()) == ours {
        let _ = std::fs::remove_file(&marker);
    }
    result
}

/// Take `file`'s exclusive lock by `deadline`, polling (std has no timed
/// lock); whether it was taken.
fn lock_by(file: &std::fs::File, deadline: Instant) -> bool {
    loop {
        match file.try_lock() {
            Ok(()) => return true,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(_)) => return false,
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(20)));
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam: runs once in [`guarded`] right after it takes the lock.
    static AFTER_LOCK: std::cell::Cell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::Cell::new(None) };
    /// Test seam: [`HELD_WAIT`] on this thread.
    static HELD_WAIT_FOR_TEST: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// [`HELD_WAIT`], or a test's own on its thread.
fn held_wait() -> Duration {
    #[cfg(test)]
    if let Some(wait) = HELD_WAIT_FOR_TEST.with(|w| w.get()) {
        return wait;
    }
    HELD_WAIT
}

/// [`extract`] behind [`guarded`]'s crash marker, without the memory cache:
/// what the content index (`project_text_index`) parses with, so retrieval
/// and the index count one document's strikes together (RF-795-F1).
/// Blocking.
pub(crate) fn extract_guarded(path: &Path) -> Result<Extracted, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    guarded(path, &meta, &markers_dir(), || extract(path))
}

/// [`extract`]'s text for `path`, from memory while the file's size and
/// modification time are unchanged (see the module doc). Blocking. A parse
/// runs behind [`guarded`]'s crash marker.
pub(crate) fn cached(path: &Path) -> Result<Arc<str>, String> {
    cached_in(path, &markers_dir())
}

fn cached_in(path: &Path, markers: &Path) -> Result<Arc<str>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let key: Key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    let hit = || {
        let cache = CACHE.lock().ok()?;
        let found = cache.entries.iter().find(|(k, _)| *k == key);
        found.map(|(_, r)| r.clone())
    };
    if let Some(hit) = hit() {
        return hit;
    }
    let result: Result<Arc<str>, String> = guarded(path, &meta, markers, || {
        // A reader this one waited for may have just read it.
        if let Some(hit) = hit() {
            return hit;
        }
        extract(path).map(|e| Arc::from(e.text))
    });
    // Another parse still running: the next read tries again.
    if result.as_ref().is_err_and(|e| e == BEING_READ) {
        return result;
    }
    if let Ok(mut cache) = CACHE.lock() {
        let cache = &mut *cache;
        // A changed file replaces its older text.
        let mut kept = 0usize;
        cache.entries.retain(|(k, r)| {
            let keep = k.0 != key.0;
            if keep {
                kept += weight(r);
            }
            keep
        });
        cache.bytes = kept + weight(&result);
        cache.entries.push_back((key, result.clone()));
        evict(cache, CACHE_BYTES, CACHE_ENTRIES);
    }
    result
}

/// Drop the oldest entries until `cache` is within both caps (the newest
/// entry always stays).
fn evict(cache: &mut Cache, max_bytes: usize, max_entries: usize) {
    while (cache.bytes > max_bytes || cache.entries.len() > max_entries) && cache.entries.len() > 1
    {
        if let Some((_, old)) = cache.entries.pop_front() {
            cache.bytes -= weight(&old);
        }
    }
}

/// [`cached`] from any thread: on a multi-thread async runtime's worker it
/// tells the runtime this thread blocks (`block_in_place`), so a PDF parse
/// never stalls other tasks; elsewhere (a blocking thread, a current-thread
/// runtime, no runtime) it just runs.
pub(crate) fn cached_blocking(path: &Path) -> Result<Arc<str>, String> {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => {
            tokio::task::block_in_place(|| cached(path))
        }
        _ => cached(path),
    }
}

/// What [`cached`] holds for `path` as it is now, without reading it:
/// `None` when it hasn't been read (or changed since).
pub(crate) fn peek(path: &Path) -> Option<Result<Arc<str>, String>> {
    let meta = std::fs::metadata(path).ok()?;
    let key: Key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    let cache = CACHE.lock().ok()?;
    cache
        .entries
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, r)| r.clone())
}

/// Cached entries and bytes, for tests.
#[cfg(test)]
fn cache_stats() -> (usize, usize) {
    let cache = CACHE.lock().unwrap();
    (cache.entries.len(), cache.bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A file and an empty marker folder for [`guarded`] (overnight 9 A0b).
    fn guard_case(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(name);
        std::fs::write(&file, b"%PDF-1.7 not really").unwrap();
        let markers = dir.path().join("markers");
        (dir, file, markers)
    }

    /// What a crash leaves: the marker another process wrote.
    fn crashed(file: &Path, markers: &Path, strike: u32) -> PathBuf {
        let meta = std::fs::metadata(file).unwrap();
        std::fs::create_dir_all(markers).unwrap();
        let marker = markers.join(marker_name(file, &meta));
        std::fs::write(&marker, format!("{strike}")).unwrap();
        marker
    }

    fn parse_counted(
        file: &Path,
        markers: &Path,
        parses: &std::cell::Cell<u32>,
    ) -> Result<Arc<str>, String> {
        let meta = std::fs::metadata(file).unwrap();
        guarded(file, &meta, markers, || {
            parses.set(parses.get() + 1);
            Ok(Arc::from("text"))
        })
    }

    #[test]
    fn a_clean_parse_removes_the_marker() {
        let (_dir, file, markers) = guard_case("clean.pdf");
        let meta = std::fs::metadata(&file).unwrap();
        let marker = markers.join(marker_name(&file, &meta));
        let seen: Result<Arc<str>, String> = guarded(&file, &meta, &markers, || {
            // Written before the parse, so a crash here leaves it.
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
            // Held for the parse: another reader sees a parse in flight.
            let other = std::fs::File::open(&marker).unwrap();
            assert!(other.try_lock().is_err());
            Err("no text layer".into())
        });
        assert_eq!(seen.unwrap_err(), "no text layer", "the parse's own answer");
        assert!(!marker.exists(), "removed whatever the parse returned");
    }

    #[test]
    fn a_parsing_marker_left_by_a_crash_counts_one_strike() {
        let (_dir, file, markers) = guard_case("once.pdf");
        let marker = crashed(&file, &markers, 1);
        let parses = std::cell::Cell::new(0);
        assert_eq!(&*parse_counted(&file, &markers, &parses).unwrap(), "text");
        assert_eq!(parses.get(), 1, "one crash: parsed once more");
        assert!(!marker.exists());
    }

    #[test]
    fn a_second_strike_refuses_without_parsing() {
        let (_dir, file, markers) = guard_case("twice.pdf");
        let marker = crashed(&file, &markers, 2);
        let parses = std::cell::Cell::new(0);
        assert_eq!(
            parse_counted(&file, &markers, &parses).unwrap_err(),
            CRASHED_READER
        );
        assert_eq!(parses.get(), 0, "never parsed again");
        assert!(marker.exists(), "still refused next time");
        assert!(parse_counted(&file, &markers, &parses).is_err());
        assert_eq!(parses.get(), 0);
    }

    #[test]
    fn a_changed_file_clears_its_strikes() {
        let (_dir, file, markers) = guard_case("changed.pdf");
        crashed(&file, &markers, 2);
        std::fs::write(&file, b"%PDF-1.7 a fixed copy, longer").unwrap();
        let parses = std::cell::Cell::new(0);
        assert!(parse_counted(&file, &markers, &parses).is_ok());
        assert_eq!(parses.get(), 1);
    }

    /// A held marker is a parse in flight (this process or the vault-mcp
    /// subprocess), never a crash (A0b review): a reader waits for it, then
    /// parses if it finished or counts its strike if it crashed (overnight 10
    /// A3), and answers "still being read" if it outlasts the wait, parsing
    /// nothing and leaving the marker alone (RF-798-F1).
    #[test]
    fn a_parse_in_flight_elsewhere_is_no_strike() {
        let (_dir, file, markers) = guard_case("busy.pdf");
        let marker = crashed(&file, &markers, 1);
        let holder = std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .unwrap();
        holder.lock().unwrap();
        let parses = std::cell::Cell::new(0);
        HELD_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(50))));
        assert_eq!(
            parse_counted(&file, &markers, &parses).unwrap_err(),
            BEING_READ
        );
        assert_eq!(parses.get(), 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1", "untouched");
        drop(holder);
        // Waits long enough for the holders below, which let go at 150 ms.
        HELD_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_secs(60))));
        let holder = std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .unwrap();
        holder.lock().unwrap();
        let crashes = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(holder);
        });
        // It crashed at one strike (its lock died): parsed once more.
        assert!(parse_counted(&file, &markers, &parses).is_ok());
        assert_eq!(parses.get(), 1);
        crashes.join().unwrap();
        assert!(!marker.exists());
        // Held at two strikes (a last retry): wait for it. It finishes and
        // removes its marker, so this parse runs on a fresh one.
        let retry = |crashes: bool| {
            std::fs::write(&marker, "2").unwrap();
            let holder = std::fs::OpenOptions::new()
                .write(true)
                .open(&marker)
                .unwrap();
            holder.lock().unwrap();
            let path = marker.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if !crashes {
                    std::fs::remove_file(&path).unwrap();
                }
                drop(holder);
            })
        };
        let done = retry(false);
        assert_eq!(&*parse_counted(&file, &markers, &parses).unwrap(), "text");
        assert_eq!(parses.get(), 2, "parsed after the retry finished");
        done.join().unwrap();
        assert!(!marker.exists());
        // The retry crashes (its lock dies, the marker still counts two).
        let done = retry(true);
        assert_eq!(
            parse_counted(&file, &markers, &parses).unwrap_err(),
            CRASHED_READER,
            "waited, then refused: never parsed beside it"
        );
        assert_eq!(parses.get(), 2);
        done.join().unwrap();
        std::fs::write(&marker, "2").unwrap();
        assert_eq!(
            parse_counted(&file, &markers, &parses).unwrap_err(),
            CRASHED_READER,
            "once nobody holds it, its strikes count"
        );
        assert_eq!(strikes_in(""), 0);
        assert_eq!(strikes_in("x"), 1, "a torn marker still counts");
    }

    /// A marker removed between the open and the lock is opened again, so
    /// the strike lands on the path a crash would leave (RF-794-F2).
    #[test]
    fn a_marker_replaced_before_the_lock_is_opened_again() {
        let (_dir, file, markers) = guard_case("swapped.pdf");
        let meta = std::fs::metadata(&file).unwrap();
        let marker = markers.join(marker_name(&file, &meta));
        let hooked = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = hooked.clone();
        AFTER_LOCK.with(|hook| {
            hook.set(Some(Box::new(move |path: &Path| {
                count.set(count.get() + 1);
                std::fs::remove_file(path).unwrap();
            })))
        });
        let seen: Result<Arc<str>, String> = guarded(&file, &meta, &markers, || {
            let on_path = std::fs::read_to_string(&marker);
            assert_eq!(
                on_path.ok().as_deref(),
                Some("1"),
                "the strike is on the path"
            );
            Ok(Arc::from("text"))
        });
        assert_eq!(&*seen.unwrap(), "text");
        assert_eq!(hooked.get(), 1, "the hook runs once");
        assert!(!marker.exists(), "removed after the parse");
    }

    /// Remove the marker right after each of the next `times` locks.
    fn remove_after_lock(times: u32, count: std::rc::Rc<std::cell::Cell<u32>>) {
        AFTER_LOCK.with(|hook| {
            hook.set(Some(Box::new(move |path: &Path| {
                count.set(count.get() + 1);
                std::fs::remove_file(path).unwrap();
                if times > 1 {
                    remove_after_lock(times - 1, count);
                }
            })))
        });
    }

    /// A marker replaced twice (three readers in a row) is still opened
    /// again, never parsed unguarded; one replaced under every lock until
    /// the wait ends reads as "still being read" (RF-R1 review F2).
    #[test]
    fn a_marker_replaced_again_and_again_is_never_parsed_unguarded() {
        let (_dir, file, markers) = guard_case("churn.pdf");
        let meta = std::fs::metadata(&file).unwrap();
        let marker = markers.join(marker_name(&file, &meta));
        let hooked = std::rc::Rc::new(std::cell::Cell::new(0));
        remove_after_lock(2, hooked.clone());
        let seen: Result<Arc<str>, String> = guarded(&file, &meta, &markers, || {
            let on_path = std::fs::read_to_string(&marker);
            assert_eq!(on_path.ok().as_deref(), Some("1"), "guarded");
            Ok(Arc::from("text"))
        });
        assert_eq!(&*seen.unwrap(), "text");
        assert_eq!(hooked.get(), 2);
        HELD_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(200))));
        remove_after_lock(u32::MAX, hooked.clone());
        let parses = std::cell::Cell::new(0);
        assert_eq!(
            parse_counted(&file, &markers, &parses).unwrap_err(),
            BEING_READ
        );
        assert_eq!(parses.get(), 0);
        AFTER_LOCK.with(|hook| hook.set(None));
    }

    /// What a crash of the app's reader leaves for `file`, in the markers
    /// folder [`cached`] and [`extract_guarded`] use.
    pub(crate) fn crashed_in_app_cache(file: &Path, strike: u32) -> PathBuf {
        crashed(file, &markers_dir(), strike)
    }

    /// A parse that never returns holds its readers no longer than the wait:
    /// a second reader of the file, while the first parse holds its marker at
    /// one strike or at two (A3's last-retry wait), answers "still being
    /// read" without parsing, and that answer isn't cached (RF-798-F1).
    #[test]
    fn a_second_reader_of_a_file_whose_parse_hangs_returns_at_once() {
        use std::sync::mpsc;
        let long = Duration::from_secs(60);
        let (_dir, file, markers) = guard_case("hangs.pdf");
        // Before the hung parse: no strike (it writes one), then one (it
        // writes two: a last retry).
        for before in [0, 1] {
            if before > 0 {
                crashed(&file, &markers, before);
            }
            let (started_tx, started) = mpsc::channel();
            let (release, released) = mpsc::channel::<()>();
            let hung = {
                let (file, markers) = (file.clone(), markers.clone());
                std::thread::spawn(move || {
                    let meta = std::fs::metadata(&file).unwrap();
                    guarded(&file, &meta, &markers, || {
                        started_tx.send(()).unwrap();
                        let _ = released.recv();
                        Ok(Arc::<str>::from("text"))
                    })
                })
            };
            started.recv_timeout(long).expect("the first parse began");
            let (answer_tx, answer) = mpsc::channel();
            {
                let (file, markers) = (file.clone(), markers.clone());
                std::thread::spawn(move || {
                    HELD_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(100))));
                    let _ = answer_tx.send(cached_in(&file, &markers));
                });
            }
            let second = answer
                .recv_timeout(long)
                .expect("the second reader returned");
            assert_eq!(second.unwrap_err(), BEING_READ, "strikes before: {before}");
            assert_eq!(seam::extracts(&file), 0, "never parsed beside it");
            assert!(peek(&file).is_none(), "not cached");
            release.send(()).unwrap();
            assert_eq!(&*hung.join().unwrap().unwrap(), "text");
        }
        // Once nobody holds it, it is read: the answer above wasn't kept.
        let read = cached_in(&file, &markers);
        assert_ne!(read, Err(BEING_READ.to_string()));
        assert_eq!(seam::extracts(&file), 1);
    }

    /// A zip holding `parts` (name, contents).
    pub(crate) fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, xml) in parts {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(xml.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    /// A one-sheet workbook: `A1` holds `text`.
    pub(crate) fn xlsx_with(sheet: &str, text: &str) -> Vec<u8> {
        zip_of(&[
            (
                "xl/workbook.xml",
                &format!(
                    r#"<workbook xmlns:r="urn:r"><sheets><sheet name="{sheet}" sheetId="1" r:id="rId1"/></sheets></workbook>"#
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="urn:worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                &format!(
                    r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{text}</t></is></c></row></sheetData></worksheet>"#
                ),
            ),
        ])
    }

    /// A Word file with one paragraph per line of `paras`.
    pub(crate) fn docx_with(paras: &[&str]) -> Vec<u8> {
        let body: String = paras
            .iter()
            .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
            .collect();
        zip_of(&[("word/document.xml", &format!("<w:body>{body}</w:body>"))])
    }

    /// A deck with one slide per entry of `slides`.
    pub(crate) fn pptx_with(slides: &[&str]) -> Vec<u8> {
        let xml: Vec<(String, String)> = slides
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    format!("ppt/slides/slide{}.xml", i + 1),
                    format!("<p:sld><a:p><a:r><a:t>{s}</a:t></a:r></a:p></p:sld>"),
                )
            })
            .collect();
        let parts: Vec<(&str, &str)> = xml.iter().map(|(n, x)| (n.as_str(), x.as_str())).collect();
        zip_of(&parts)
    }

    #[test]
    fn office_text_reads_word_runs_by_paragraph() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(br#"<w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world &amp; co</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body>"#).unwrap();
            z.finish().unwrap();
        }
        assert_eq!(
            office_text(buf.get_ref(), "docx", &LIMITS).unwrap(),
            "Hello world & co\nSecond\n"
        );
        assert!(office_text(b"not a zip", "docx", &LIMITS).is_err());
    }

    #[test]
    fn office_text_reads_excel_cells_not_the_string_pool() {
        // Review B: the pool holds a string no cell uses, and the sheet holds
        // a number, a boolean and a formula the pool never sees.
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, xml) in [
                (
                    "xl/workbook.xml",
                    r#"<workbook xmlns:r="urn:r"><sheets><sheet name="Q3" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<Relationships><Relationship Id="rId1" Type="urn:worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/sharedStrings.xml",
                    "<sst><si><t>Region</t></si><si><t>Draft only</t></si></sst>",
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>42.5</v></c><c r="C1" t="b"><v>0</v></c><c r="D1"><f>B1*2</f><v>85</v></c></row></sheetData></worksheet>"#,
                ),
            ] {
                z.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(xml.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let text = office_text(buf.get_ref(), "xlsx", &LIMITS).unwrap();
        assert!(
            !text.contains("Draft only"),
            "an unused pooled string is not sheet content:\n{text}"
        );
        assert!(
            text.contains(
                "## Sheet 1: Q3\nA1 \"Region\" | B1 42.5 | C1 FALSE | D1 85 (formula =B1*2)\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn pptx_slides_are_headed_and_slide10_follows_slide9() {
        let titles: Vec<String> = (1..=10).map(|i| format!("Title {i}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let text = office_text(&pptx_with(&refs), "pptx", &LIMITS).unwrap();
        assert!(
            text.starts_with("## Slide 1\nTitle 1\n\n## Slide 2\n"),
            "{text}"
        );
        let nine = text.find("## Slide 9\nTitle 9\n").expect("slide 9");
        let ten = text.find("## Slide 10\nTitle 10\n").expect("slide 10");
        assert!(nine < ten, "slide10 after slide9:\n{text}");
        assert!(text.find("## Slide 2\n").unwrap() < nine);
    }

    #[test]
    fn caps_cut_slides_text_and_file_size_and_say_so_first() {
        let small = Limits {
            file_bytes: 64 * 1024 * 1024,
            units: 3,
            text_bytes: 40,
            xml_bytes: 50 * 1024 * 1024,
        };
        // Slides past the cap are left out, and the first line says so.
        let deck = pptx_with(&["a", "b", "c", "d", "e"]);
        let e = extract_bytes(
            &deck,
            DocKind::Pptx,
            &Limits {
                text_bytes: 4096,
                ..small
            },
        )
        .unwrap();
        assert!(
            e.text
                .starts_with("[truncated: first 3 of 5 slides]\n\n## Slide 1\n"),
            "{}",
            e.text
        );
        assert!(!e.text.contains("## Slide 4"), "{}", e.text);
        assert_eq!(e.units, 3);
        // Text past the cap is cut at a line end, the cut stated first.
        let long = docx_with(&[
            "one line of words",
            "two line of words",
            "three line of words",
        ]);
        let e = extract_bytes(&long, DocKind::Docx, &small).unwrap();
        assert_eq!(
            e.text,
            "[truncated: first 36 of 56 bytes of text]\n\none line of words\ntwo line of words\n"
        );
        // A file over the size cap is refused before it is read.
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.docx");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();
        let err = extract(&big).err().unwrap();
        assert!(err.contains("64 MB document reader limit"), "{err}");
        // The XML cap is the zip-bomb bound: the inflated text stops there.
        let bomb = docx_with(&[&"x".repeat(5000)]);
        let e = extract_bytes(
            &bomb,
            DocKind::Docx,
            &Limits {
                xml_bytes: 1000,
                text_bytes: 1 << 20,
                ..small
            },
        )
        .unwrap();
        assert!(
            e.text
                .starts_with("[truncated: first 1000 bytes of uncompressed XML]"),
            "{}",
            &e.text[..80.min(e.text.len())]
        );
        assert!(e.text.len() < 1100);
        // Production caps as specified.
        assert_eq!(
            (
                LIMITS.file_bytes,
                LIMITS.units,
                LIMITS.text_bytes,
                LIMITS.xml_bytes
            ),
            (64 << 20, 500, 4 << 20, 50 << 20)
        );
    }

    /// The XML cap can land inside a character: what came before it is read
    /// (RF-794); any other bad byte is still unreadable.
    #[test]
    fn the_xml_cap_cuts_on_a_whole_character() {
        let lim = |xml_bytes| Limits {
            file_bytes: 64 * 1024 * 1024,
            units: 500,
            text_bytes: 1 << 20,
            xml_bytes,
        };
        let doc = docx_with(&["first paragraph", "café au lait"]);
        let xml = "<w:body><w:p><w:r><w:t>first paragraph</w:t></w:r></w:p><w:p><w:r><w:t>caf";
        let cap = xml.len() as u64 + 1; // one byte into the two-byte é
        let e = extract_bytes(&doc, DocKind::Docx, &lim(cap)).unwrap();
        assert!(
            e.text.starts_with(&format!(
                "[truncated: first {cap} bytes of uncompressed XML]"
            )),
            "{}",
            e.text
        );
        assert!(e.text.contains("first paragraph\n"), "{}", e.text);
        // A part exactly the budget long is whole, not cut.
        let whole = "<w:body><w:p><w:r><w:t>first paragraph</w:t></w:r></w:p><w:p><w:r><w:t>café au lait</w:t></w:r></w:p></w:body>";
        let e = extract_bytes(&doc, DocKind::Docx, &lim(whole.len() as u64)).unwrap();
        assert!(!e.text.contains("truncated"), "{}", e.text);
        assert!(e.text.contains("café au lait"), "{}", e.text);
        // A deck: the cap inside slide 2's é keeps slide 1.
        let deck = pptx_with(&["one", "deux é"]);
        let slide1 = "<p:sld><a:p><a:r><a:t>one</a:t></a:r></a:p></p:sld>";
        let into2 = "<p:sld><a:p><a:r><a:t>deux ";
        let cap = (slide1.len() + into2.len() + 1) as u64;
        let e = extract_bytes(&deck, DocKind::Pptx, &lim(cap)).unwrap();
        assert!(e.text.contains("## Slide 1\none\n"), "{}", e.text);
        assert!(e.text.starts_with("[truncated: first "), "{}", e.text);
        // Slide 1 exactly at the cap: slide 2 is left out, with no empty heading.
        let e = extract_bytes(&deck, DocKind::Pptx, &lim(slide1.len() as u64)).unwrap();
        assert!(e.text.starts_with("[truncated: first "), "{}", e.text);
        assert!(!e.text.contains("## Slide 2"), "{}", e.text);
        assert_eq!(e.units, 1);
        // A bad byte that isn't the cap's doing is still an error.
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(b"<w:body><w:p><w:r><w:t>a\xFFb</w:t></w:r></w:p></w:body>")
                .unwrap();
            z.finish().unwrap();
        }
        let bad = buf.into_inner();
        let err = extract_bytes(&bad, DocKind::Docx, &lim(50 << 20))
            .err()
            .unwrap();
        assert_eq!(err, "unreadable Office XML");
        // A cap right after the bad byte (its last kept byte) is no excuse.
        let err = extract_bytes(&bad, DocKind::Docx, &lim(25)).err().unwrap();
        assert_eq!(err, "unreadable Office XML");
    }

    #[test]
    fn kinds_are_by_extension_and_images_are_not_read() {
        assert_eq!(kind(Path::new("/p/Deck.PPTX")), Some(DocKind::Pptx));
        assert_eq!(kind(Path::new("/p/a.pdf")), Some(DocKind::Pdf));
        assert_eq!(kind(Path::new("/p/a.docx")), Some(DocKind::Docx));
        assert_eq!(kind(Path::new("/p/a.xlsx")), Some(DocKind::Xlsx));
        assert_eq!(kind(Path::new("/p/a.png")), None);
        assert_eq!(kind(Path::new("/p/a.doc")), None);
        assert!(looks_like(Path::new("a.pdf"), b"%PDF-1.7"));
        assert!(looks_like(Path::new("a.docx"), b"PK\x03\x04"));
        assert!(!looks_like(Path::new("a.pdf"), b"plain words"));
        assert!(!looks_like(Path::new("a.png"), b"%PDF-1.7"));
    }

    #[test]
    fn the_cache_reads_once_and_rereads_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.docx");
        std::fs::write(&path, docx_with(&["first version"])).unwrap();
        let a = cached(&path).unwrap();
        let b = cached(&path).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "the second read is served from memory");
        // Another size (and so another key): read again.
        std::fs::write(&path, docx_with(&["the second, longer version"])).unwrap();
        let c = cached(&path).unwrap();
        assert_eq!(&*c, "the second, longer version\n");
        let (entries, bytes) = cache_stats();
        assert!(entries >= 1 && bytes <= CACHE_BYTES);
        // A failed read is an error, cached like text.
        let bad = dir.path().join("bad.pptx");
        std::fs::write(&bad, b"not a zip").unwrap();
        assert_eq!(cached(&bad).err().unwrap(), "not a readable Office file");
    }

    /// `cached_blocking` from a multi-thread runtime's worker: it reads
    /// (through `block_in_place`) instead of panicking or stalling, and a
    /// current-thread runtime just runs it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cached_blocking_reads_from_an_async_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("from-a-worker.docx");
        std::fs::write(&path, docx_with(&["read on a worker"])).unwrap();
        let text = tokio::spawn(async move { cached_blocking(&path).map(|t| t.to_string()) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, "read on a worker\n");
        let other = dir.path().join("current-thread.docx");
        std::fs::write(&other, docx_with(&["read on one thread"])).unwrap();
        let text = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async move { cached_blocking(&other).map(|t| t.to_string()) })
        })
        .join()
        .unwrap()
        .unwrap();
        assert_eq!(text, "read on one thread\n");
    }

    #[test]
    fn the_cache_holds_at_most_its_entry_and_byte_caps_oldest_out_first() {
        // A local cache: the shared one serves other tests in parallel.
        let mut cache = Cache {
            entries: VecDeque::new(),
            bytes: 0,
        };
        for i in 0..10u64 {
            let r: Result<Arc<str>, String> = Ok(Arc::from("0123456789"));
            cache.bytes += weight(&r);
            cache
                .entries
                .push_back(((PathBuf::from(format!("{i}")), i, None), r));
            evict(&mut cache, 1_000, 4);
        }
        let kept: Vec<u64> = cache.entries.iter().map(|(k, _)| k.1).collect();
        assert_eq!(kept, [6, 7, 8, 9], "the entry cap, oldest out first");
        assert_eq!(cache.bytes, 40);
        evict(&mut cache, 25, 4);
        assert_eq!(cache.entries.len(), 2, "the byte cap");
        evict(&mut cache, 0, 0);
        assert_eq!(cache.entries.len(), 1, "the newest always stays");
    }

    /// The text PDF fixture: one page, "Hollow PDF fixture" and a second line.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_one_page_text_pdf_is_read_by_page() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_text_one_page.pdf");
        assert!(std::fs::metadata(&path).unwrap().len() <= 2048);
        let e = extract(&path).unwrap();
        assert_eq!(e.kind, DocKind::Pdf);
        assert_eq!(e.units, 1);
        assert!(e.text.starts_with("## Page 1\n"), "{}", e.text);
        assert!(e.text.contains("Garage 14 escalator"), "{}", e.text);
        assert!(!e.text.contains("## Page 2"), "{}", e.text);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_image_only_pdf_says_why_it_has_no_text() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_text_image_only.pdf");
        assert!(std::fs::metadata(&path).unwrap().len() <= 2048);
        assert_eq!(
            extract(&path).err().as_deref(),
            Some("image-only PDF: no text layer (needs OCR)")
        );
        assert_eq!(
            extract_bytes(b"%PDF-1.4 nothing else", DocKind::Pdf, &LIMITS)
                .err()
                .as_deref(),
            Some("not a readable PDF")
        );
    }
}
