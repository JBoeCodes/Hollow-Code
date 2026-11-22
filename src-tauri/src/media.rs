//! Local video playback: the `hollow-media` scheme (inline video, 2026-10-17).
//!
//! The frontend asks [`admit`] (through the `media_grant` command) for one
//! file, gets an opaque token, and plays `hollow-media://localhost/<token>`
//! (`http://hollow-media.localhost/<token>` in the Windows form). Nothing
//! else is served: no path ever appears in a media URL.
//!
//! **Why a scheme of our own, not Tauri's asset protocol.** The asset protocol
//! reads a whole file for a GET without `Range`, runs on the main thread,
//! checks its scope and then opens the path again by name, and puts raw paths
//! in URLs. `data:`/`blob:` would copy the whole file over IPC.
//!
//! **Bounded responses.** wry copies a response body into one `NSData` and
//! hands it to WebKit in one call, so it can't stream. Every body is capped at
//! [`MAX_CHUNK`] (2 MiB) and answered `206`; WebKit's media loader asks again
//! for the rest. A GET without `Range` gets the first chunk only. The spike
//! (tauri 2.11.1 / wry 0.55.1, 2026-10-17) played and seeked mp4 (faststart
//! and moov-at-end) and mov this way, even with the cap forced to 32 KiB.
//! WebM is **not** served: WebKit's WebM player sends one GET without
//! `Range` and never asks for more after a short 206, so a webm over 2 MiB
//! stalled. It stays a plain file with Open and Reveal.
//!
//! **Main thread.** wry calls the handler on the main thread, so [`serve`]
//! returns at once and reads on the blocking pool, at most [`READS`] at a
//! time. Extra requests queue: WebKit treats any 5xx as a fatal network
//! error and ignores `Retry-After`, so this never answers 503. wry drops a
//! response to a request WebKit already cancelled (seeks cancel many).
//!
//! **Who may load.** Only the `main` webview: the side panel's browser
//! webviews, `md-preview-*` windows and overlays get 403 whatever the token.
//! The label is per webview, not per frame, so an artifact or canvas iframe
//! inside `main` passes it. Frames are held off by CSP (every artifact mode's
//! `media-src` is `data: blob:` at most `https:`, which matches neither
//! `hollow-media:` nor `http:`), by their opaque-origin sandbox (they can't
//! read the main DOM where tokens live), by `main_nav` refusing navigation to
//! this scheme, and by token secrecy: a token is never logged, put in an error,
//! sent to an agent or passed to a canvas bridge. Never enable
//! `use_https_scheme`: the Windows form would become `https://…`, which the
//! Https and Interactive artifact modes' `media-src https:` would match.
//!
//! **A grant pins the file.** It keeps the canonical path plus device, inode,
//! length and mtime; every request re-opens the canonical path and checks
//! them (410 for another file, 409 for a changed one: ffmpeg rewrites in
//! place). The store holds [`CAP`] grants, evicting the least recently used.

use once_cell::sync::Lazy;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::http::{Request, Response, StatusCode};

/// The extensions that play inline. `src/mediaKinds.ts` mirrors this list (a
/// vitest reads this file to check). `process_capture.rs` keeps its own list.
pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov"];

/// The largest response body (see the module docs).
pub const MAX_CHUNK: u64 = 2 * 1024 * 1024;
/// Grants kept at once; the least recently used goes first.
const CAP: usize = 32;
/// Bytes read to check the container.
const SNIFF_LEN: usize = 4096;
/// The longest path admitted, in bytes.
const MAX_PATH: usize = 4096;

/// Reads in flight app-wide (one per grant would not bound the app).
static READS: Lazy<tokio::sync::Semaphore> = Lazy::new(|| tokio::sync::Semaphore::new(4));

/// Why a file can't play. [`MediaRefusal::code`] is the stable string the
/// frontend matches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaRefusal {
    OutsideRoots,
    NotVideo,
    UnsupportedContainer,
    Missing,
    Hidden,
}

impl MediaRefusal {
    pub fn code(self) -> &'static str {
        match self {
            MediaRefusal::OutsideRoots => "OUTSIDE_ROOTS",
            MediaRefusal::NotVideo => "NOT_VIDEO",
            MediaRefusal::UnsupportedContainer => "UNSUPPORTED_CONTAINER",
            MediaRefusal::Missing => "MISSING",
            MediaRefusal::Hidden => "HIDDEN",
        }
    }
}

/// What `media_grant` returns: the token and what the player shows.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGrant {
    pub token: String,
    pub mime: &'static str,
    pub size: u64,
    pub modified_ms: Option<u64>,
}

/// The identity a grant pins and every request checks again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: Option<SystemTime>,
}

impl FileId {
    fn of(meta: &std::fs::Metadata) -> FileId {
        #[cfg(unix)]
        let (dev, ino) = {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino())
        };
        #[cfg(not(unix))]
        let (dev, ino) = (0, 0);
        FileId {
            dev,
            ino,
            len: meta.len(),
            mtime: meta.modified().ok(),
        }
    }

    fn modified_ms(&self) -> Option<u64> {
        self.mtime
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
    }
}

/// A file [`admit`] accepted.
#[derive(Clone, Debug)]
pub struct Admitted {
    canonical: PathBuf,
    id: FileId,
    mime: &'static str,
}

#[derive(Clone, Debug)]
struct Grant {
    file: Admitted,
    last_used: u64,
}

static STORE: Lazy<Mutex<Vec<(String, Grant)>>> = Lazy::new(|| Mutex::new(Vec::new()));
static CLOCK: AtomicU64 = AtomicU64::new(0);

fn tick() -> u64 {
    CLOCK.fetch_add(1, Ordering::Relaxed)
}

/// The container's MIME type from its first bytes and the extension, or a
/// refusal. ISO-BMFF (`ftyp`, or a bare top-level atom at offset 4) plays as
/// any of mp4/m4v/mov: AVFoundation plays all three, and `.mov` files with
/// an `isom` brand (and the reverse) are common. Image brands (HEIC, AVIF)
/// are refused. Anything else, EBML/WebM included, is refused.
pub fn sniff(head: &[u8], ext: &str) -> Result<&'static str, MediaRefusal> {
    const IMAGE_BRANDS: [&[u8; 4]; 6] = [b"heic", b"heix", b"mif1", b"msf1", b"avif", b"avis"];
    const BARE_ATOMS: [&[u8; 4]; 6] = [b"moov", b"mdat", b"wide", b"free", b"skip", b"pnot"];
    let atom = head.get(4..8);
    let iso = match atom {
        Some(b"ftyp") => match head.get(8..12) {
            Some(brand) => !IMAGE_BRANDS.iter().any(|b| &b[..] == brand),
            None => false,
        },
        Some(a) => BARE_ATOMS.iter().any(|b| &b[..] == a),
        None => false,
    };
    if !iso {
        return Err(MediaRefusal::UnsupportedContainer);
    }
    match ext {
        "mp4" => Ok("video/mp4"),
        "m4v" => Ok("video/x-m4v"),
        "mov" => Ok("video/quicktime"),
        _ => Err(MediaRefusal::UnsupportedContainer),
    }
}

/// Whether a lowercase extension plays inline.
pub fn is_video_ext(ext: &str) -> bool {
    VIDEO_EXTS.contains(&ext)
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Check one path against the rules (absolute, inside a root, nothing hidden
/// or secret-named on the way, a real video container) and pin its identity.
/// `roots` must be canonical ([`roots`] makes them).
pub fn admit(path: &str, roots: &[PathBuf]) -> Result<Admitted, MediaRefusal> {
    if path.is_empty()
        || path.len() > MAX_PATH
        || path.contains('\0')
        || !Path::new(path).is_absolute()
    {
        return Err(MediaRefusal::OutsideRoots);
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| MediaRefusal::Missing)?;
    let root = roots
        .iter()
        .filter(|r| canonical.starts_with(r))
        .max_by_key(|r| r.as_os_str().len())
        .ok_or(MediaRefusal::OutsideRoots)?;
    let rest: Vec<String> = canonical
        .strip_prefix(root)
        .map_err(|_| MediaRefusal::OutsideRoots)?
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let Some((leaf, dirs)) = rest.split_last() else {
        return Err(MediaRefusal::NotVideo);
    };
    // Folders: hidden or secret-named. The file itself: hidden only, since
    // `token-demo.mp4` is a fine name and the sniff below proves it's video.
    if dirs
        .iter()
        .any(|d| crate::source_evidence::forbidden_component(d))
        || leaf.starts_with('.')
    {
        return Err(MediaRefusal::Hidden);
    }
    let ext = lower_ext(&canonical);
    if !is_video_ext(&ext) {
        return Err(MediaRefusal::NotVideo);
    }
    // The type before any open: opening a FIFO named `x.mp4` would block.
    match std::fs::metadata(&canonical) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err(MediaRefusal::NotVideo),
        Err(_) => return Err(MediaRefusal::Missing),
    }
    let mut file = open_regular(&canonical).ok_or(MediaRefusal::Missing)?;
    let meta = file.metadata().map_err(|_| MediaRefusal::Missing)?;
    if !meta.is_file() {
        return Err(MediaRefusal::NotVideo);
    }
    // Still the file that was checked: a folder swapped for a link between
    // the canonicalize and the open lands somewhere else.
    if std::fs::canonicalize(path).ok().as_ref() != Some(&canonical) {
        return Err(MediaRefusal::Missing);
    }
    if meta.len() == 0 {
        return Err(MediaRefusal::UnsupportedContainer);
    }
    let mut head = Vec::with_capacity(SNIFF_LEN);
    (&mut file)
        .take(SNIFF_LEN as u64)
        .read_to_end(&mut head)
        .map_err(|_| MediaRefusal::Missing)?;
    let mime = sniff(&head, &ext)?;
    Ok(Admitted {
        canonical,
        id: FileId::of(&meta),
        mime,
    })
}

/// Open a canonical path without following a link at its end and without
/// blocking on a FIFO (`O_NOFOLLOW | O_NONBLOCK`; a regular file reads the
/// same). The caller checks the handle is a regular file.
pub(crate) fn open_regular(path: &Path) -> Option<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path).ok()
}

/// Store a grant for an admitted file and return what the player needs.
pub fn grant(file: Admitted) -> MediaGrant {
    let token = fresh_token();
    let out = MediaGrant {
        token: token.clone(),
        mime: file.mime,
        size: file.id.len,
        modified_ms: file.id.modified_ms(),
    };
    if let Ok(mut store) = STORE.lock() {
        store.push((
            token,
            Grant {
                file,
                last_used: tick(),
            },
        ));
        while store.len() > CAP {
            if let Some(oldest) = store
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, g))| g.last_used)
                .map(|(i, _)| i)
            {
                store.remove(oldest);
            }
        }
    }
    out
}

/// Forget a grant (the player unmounted or went off screen).
pub fn release(token: &str) {
    if let Ok(mut store) = STORE.lock() {
        store.retain(|(t, _)| t != token);
    }
}

/// The grant for a token, marked as just used.
fn touch(token: &str) -> Option<Admitted> {
    let mut store = STORE.lock().ok()?;
    let (_, grant) = store.iter_mut().find(|(t, _)| t == token)?;
    grant.last_used = tick();
    Some(grant.file.clone())
}

/// 128 bits from the OS CSPRNG as 32 lowercase hex characters.
fn fresh_token() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The folders a video may play from: the vault, each connected project,
/// each live build worktree and, for a conversation, its working folder and
/// the repositories its receipts name. Canonical; missing ones dropped.
pub fn roots(vault: &Path, session_key: Option<&str>) -> Vec<PathBuf> {
    let mut raw: Vec<PathBuf> = vec![vault.to_path_buf()];
    raw.extend(
        crate::directory_connections::projects(vault)
            .into_iter()
            .map(|p| PathBuf::from(p.path)),
    );
    // One load of the binding store, not one per ancestor.
    raw.extend(
        crate::build_workspaces::all_worktrees(vault)
            .into_iter()
            .filter(|(_, _, info)| info.exists)
            .map(|(_, _, info)| PathBuf::from(info.worktree)),
    );
    if let Some(key) = session_key {
        let mut chat: Vec<PathBuf> = Vec::new();
        if let Some(cwd) = crate::session_store::session_cwd(vault, key) {
            chat.push(PathBuf::from(cwd));
        }
        let _ = crate::evidence::with_receipts(vault, |receipts| {
            for r in receipts
                .iter()
                .filter(|r| r.session_key.as_deref() == Some(key))
            {
                chat.extend(r.repo_changes.iter().map(|c| PathBuf::from(&c.root)));
            }
        });
        let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
        raw.extend(
            chat.into_iter()
                .filter(|p| chat_root_ok(p, home.as_deref())),
        );
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for p in raw {
        if let Ok(c) = p.canonicalize() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// A conversation's folder counts as a root unless it is too broad (`/`, a
/// top-level folder such as `/Users` or `/Volumes`, the home folder or any
/// folder above it) or inside a hidden folder.
fn chat_root_ok(path: &Path, home: Option<&Path>) -> bool {
    let Ok(c) = path.canonicalize() else {
        return false;
    };
    let depth = c
        .components()
        .filter(|comp| matches!(comp, Component::Normal(_)))
        .count();
    if depth < 2 || home.is_some_and(|h| h.starts_with(&c)) {
        return false;
    }
    !c.components()
        .any(|comp| matches!(comp, Component::Normal(s) if s.to_string_lossy().starts_with('.')))
}

/// Handle one `hollow-media` request: return at once, read on the blocking
/// pool behind the global queue.
pub fn serve(label: &str, request: Request<Vec<u8>>, responder: tauri::UriSchemeResponder) {
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        responder.respond(queued(label, request).await);
    });
}

/// Wait for a read slot, then answer on the blocking pool.
async fn queued(label: String, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let _permit = READS.acquire().await;
    tauri::async_runtime::spawn_blocking(move || answer(&label, &request))
        .await
        .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR))
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    let mut r = Response::new(Vec::new());
    *r.status_mut() = status;
    r
}

/// The response for one request (blocking IO).
fn answer(label: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    if label != "main" {
        return empty(StatusCode::FORBIDDEN);
    }
    let token = request.uri().path().trim_start_matches('/');
    let Some(file) = touch(token) else {
        return empty(StatusCode::NOT_FOUND);
    };
    let Some(mut handle) = open_regular(&file.canonical) else {
        return empty(StatusCode::GONE);
    };
    let Ok(meta) = handle.metadata() else {
        return empty(StatusCode::GONE);
    };
    if !meta.is_file() {
        return empty(StatusCode::GONE);
    }
    let now = FileId::of(&meta);
    if (now.dev, now.ino) != (file.id.dev, file.id.ino) {
        return empty(StatusCode::GONE);
    }
    if now.len != file.id.len || now.mtime != file.id.mtime {
        return empty(StatusCode::CONFLICT);
    }
    let len = now.len;
    let headed = |status: StatusCode| {
        Response::builder()
            .status(status)
            .header("Content-Type", file.mime)
            .header("Accept-Ranges", "bytes")
            .header("X-Content-Type-Options", "nosniff")
            .header("Cache-Control", "no-store")
            .header("Referrer-Policy", "no-referrer")
    };
    if request.method() == tauri::http::Method::HEAD {
        return headed(StatusCode::OK)
            .header("Content-Length", len.to_string())
            .body(Vec::new())
            .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR));
    }
    let range = request.headers().get("range").and_then(|v| v.to_str().ok());
    let (start, end) = match parse_range(range, len) {
        Err(Unsatisfiable) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", format!("bytes */{len}"))
                .body(Vec::new())
                .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR));
        }
        Ok(None) => (0, MAX_CHUNK.min(len) - 1),
        Ok(Some((s, e))) => (s, e.min(s + MAX_CHUNK - 1)),
    };
    let want = end - start + 1;
    let mut body = Vec::with_capacity(want as usize);
    if handle.seek(SeekFrom::Start(start)).is_err()
        || (&mut handle).take(want).read_to_end(&mut body).is_err()
        || body.is_empty()
    {
        return empty(StatusCode::GONE);
    }
    let end = start + body.len() as u64 - 1;
    headed(StatusCode::PARTIAL_CONTENT)
        .header("Content-Range", format!("bytes {start}-{end}/{len}"))
        .header("Content-Length", body.len().to_string())
        .body(body)
        .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR))
}

/// A range that can't be served (416).
#[derive(Debug, PartialEq, Eq)]
struct Unsatisfiable;

/// One `bytes=a-b | a- | -n` range as inclusive `(start, end)` clamped to the
/// file, `None` for no or a malformed header (served as no range), or
/// [`Unsatisfiable`]. A multi-range uses its first range. `len` is > 0.
fn parse_range(header: Option<&str>, len: u64) -> Result<Option<(u64, u64)>, Unsatisfiable> {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return Ok(None);
    };
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((a, b)) = first.split_once('-') else {
        return Ok(None);
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let Ok(n) = b.parse::<u64>() else {
            return Ok(None);
        };
        if n == 0 {
            return Err(Unsatisfiable);
        }
        return Ok(Some((len - n.min(len), len - 1)));
    }
    let Ok(start) = a.parse::<u64>() else {
        return Ok(None);
    };
    let end = if b.is_empty() {
        len - 1
    } else {
        match b.parse::<u64>() {
            Ok(e) => e,
            Err(_) => return Ok(None),
        }
    };
    if start >= len || start > end {
        return Err(Unsatisfiable);
    }
    Ok(Some((start, end.min(len - 1))))
}

// ── Posters (inline video PR 3) ─────────────────────────────────────────

/// The largest poster kept or returned.
const POSTER_MAX: u64 = 150 * 1024;
/// Each poster step's time limit.
const POSTER_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);
/// Posters made at once app-wide (each spawns QuickLook or ffmpeg). Cache
/// hits never wait for it.
static POSTERS: Lazy<tokio::sync::Semaphore> = Lazy::new(|| tokio::sync::Semaphore::new(2));
/// Work folders a crash left behind are swept once per run.
static SWEPT: std::sync::Once = std::sync::Once::new();

/// A file's poster cache name: device, inode, length and mtime, so a
/// re-render (even in place) gets a new one. None without an mtime: such a
/// poster is made but not cached.
fn poster_key(id: &FileId) -> Option<String> {
    id.modified_ms()
        .map(|m| format!("{}-{}-{}-{}", id.dev, id.ino, id.len, m))
}

/// The cached poster, or the remembered failure, for this file; None when
/// neither is there yet.
fn cached_poster(file: &Admitted, cache: &Path) -> Option<Result<String, String>> {
    let key = poster_key(&file.id)?;
    let jpg = cache.join(format!("{key}.jpg"));
    if jpg.is_file() {
        return Some(read_poster(&jpg));
    }
    cache
        .join(format!("{key}.none"))
        .is_file()
        .then(|| Err("no poster for this file".into()))
}

/// A poster file as a `data:` URL: a JPEG of at most [`POSTER_MAX`]; a bad
/// one is deleted.
fn read_poster(path: &Path) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > POSTER_MAX || !bytes.starts_with(&[0xFF, 0xD8]) {
        let _ = std::fs::remove_file(path);
        return Err("the poster isn't a small JPEG".into());
    }
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// A poster frame for an admitted video as a `data:image/jpeg` URL, cached
/// in `cache` (the app's cache folder, never the vault) under
/// [`poster_key`]. QuickLook first (`qlmanage -t` writes a PNG; `sips` makes
/// it a JPEG), then ffmpeg when it is installed; every step is bounded and
/// runs in its own process group. A file the tools read to the end without
/// a frame is remembered (`<key>.none`), so it isn't tried again until it
/// changes; a timeout or a tool that didn't start leaves no mark. The file
/// must still be the one admitted when the poster is done, or nothing is
/// kept.
pub fn poster(file: &Admitted, cache: &Path) -> Result<String, String> {
    poster_with(file, cache, make_poster)
}

fn poster_with(
    file: &Admitted,
    cache: &Path,
    make: impl FnOnce(&Path, &Path) -> Result<PathBuf, Miss>,
) -> Result<String, String> {
    if let Some(hit) = cached_poster(file, cache) {
        return hit;
    }
    SWEPT.call_once(|| sweep_poster_work(cache));
    let work = cache.join("tmp").join(fresh_token());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let made = make(&file.canonical, &work);
    let same = open_regular(&file.canonical)
        .and_then(|f| f.metadata().ok())
        .map(|m| FileId::of(&m))
        == Some(file.id.clone());
    let key = poster_key(&file.id);
    let result = match (made, same, key) {
        (_, false, _) => Err("the video changed while its poster was made".to_string()),
        (Ok(jpg), true, Some(key)) => {
            let out = cache.join(format!("{key}.jpg"));
            let _ = std::fs::remove_file(cache.join(format!("{key}.none")));
            std::fs::rename(&jpg, &out)
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    prune_posters(cache, &file.id, &key);
                    read_poster(&out)
                })
        }
        (Ok(jpg), true, None) => read_poster(&jpg),
        (Err(Miss::NoFrame(e)), true, Some(key)) => {
            let _ = std::fs::write(cache.join(format!("{key}.none")), b"");
            prune_posters(cache, &file.id, &key);
            Err(e)
        }
        (Err(Miss::NoFrame(e) | Miss::Transient(e)), true, _) => Err(e),
    };
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Drop the posters (and failure marks) of this file's earlier versions.
fn prune_posters(cache: &Path, id: &FileId, keep: &str) {
    let prefix = format!("{}-{}-", id.dev, id.ino);
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".jpg").trim_end_matches(".none");
        if name.starts_with(&prefix) && stem != keep && stem.len() < name.len() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Work folders older than an hour (a crash mid-poster) go.
fn sweep_poster_work(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache.join("tmp")) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// [`poster`] for a path: admit it under `roots` and read the cache on the
/// blocking pool, and only queue for a poster slot when one must be made.
pub async fn poster_for(
    path: String,
    roots: Vec<PathBuf>,
    cache: PathBuf,
) -> Result<String, String> {
    let lookup = cache.clone();
    let (file, hit) = tauri::async_runtime::spawn_blocking(move || {
        let file = admit(&path, &roots).map_err(|r| r.code().to_string())?;
        let hit = cached_poster(&file, &lookup);
        Ok::<_, String>((file, hit))
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(hit) = hit {
        return hit;
    }
    let _permit = POSTERS.acquire().await.map_err(|e| e.to_string())?;
    // Another request for the same file may have made it meanwhile: `poster`
    // reads the cache first.
    tauri::async_runtime::spawn_blocking(move || poster(&file, &cache))
        .await
        .map_err(|e| e.to_string())?
}

/// Why no poster was made: the tools ran and found no frame (remembered),
/// or one timed out or didn't start (tried again next time).
#[derive(Debug)]
enum Miss {
    NoFrame(String),
    Transient(String),
}

/// A JPEG poster of `video` in `work`: QuickLook, else ffmpeg.
fn make_poster(video: &Path, work: &Path) -> Result<PathBuf, Miss> {
    quicklook_jpeg(video, work).or_else(|first| match crate::process_capture::find_ffmpeg() {
        Some(ffmpeg) => {
            // A partial file sips left must not pass for ffmpeg's.
            let _ = std::fs::remove_file(work.join("poster.jpg"));
            ffmpeg_jpeg(&ffmpeg, video, work)
        }
        None => Err(first),
    })
}

/// `qlmanage -t` (it writes `<name>.png` into `work`, whatever is asked),
/// then `sips` to a JPEG at quality 70.
fn quicklook_jpeg(video: &Path, work: &Path) -> Result<PathBuf, Miss> {
    use crate::process_capture::run_bounded_until;
    let out = work.join("poster.jpg");
    let (ok, _, _) = run_bounded_until(
        std::process::Command::new("/usr/bin/qlmanage")
            .args(["-t", "-s", "480", "-o"])
            .arg(work)
            .arg(video),
        POSTER_LIMIT,
        &|| false,
    )
    .map_err(Miss::Transient)?;
    let png = std::fs::read_dir(work)
        .map_err(|e| Miss::Transient(e.to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "png"))
        .filter(|_| ok)
        .ok_or_else(|| Miss::NoFrame("QuickLook made no thumbnail".into()))?;
    let (ok, _, _) = run_bounded_until(
        std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "jpeg", "-s", "formatOptions", "70"])
            .arg(&png)
            .arg("--out")
            .arg(&out),
        POSTER_LIMIT,
        &|| false,
    )
    .map_err(Miss::Transient)?;
    if ok && out.is_file() {
        Ok(out)
    } else {
        Err(Miss::NoFrame("sips made no JPEG".into()))
    }
}

/// ffmpeg's frame at 1 s, else its first (a clip under a second). The input
/// is pinned to the sniffed container (`-f mov`, which reads mp4/m4v/mov),
/// read only from a file (`-protocol_whitelist file`, a `file:` path, so it
/// is never an option or a protocol), with audio, subtitles and data off.
fn ffmpeg_jpeg(ffmpeg: &Path, video: &Path, work: &Path) -> Result<PathBuf, Miss> {
    use crate::process_capture::run_bounded_until;
    let out = work.join("poster.jpg");
    let input = format!("file:{}", video.to_string_lossy());
    let mut transient = None;
    for at in ["1", "0"] {
        let ran = run_bounded_until(
            std::process::Command::new(ffmpeg)
                .args([
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-protocol_whitelist",
                    "file",
                    "-ss",
                    at,
                    "-f",
                    "mov",
                    "-i",
                ])
                .arg(&input)
                .args([
                    "-an",
                    "-sn",
                    "-dn",
                    "-frames:v",
                    "1",
                    "-update",
                    "1",
                    "-vf",
                    "scale=480:-2",
                    "-q:v",
                    "5",
                    "-y",
                ])
                .arg(&out),
            POSTER_LIMIT,
            &|| false,
        );
        match ran {
            Ok((true, _, _)) if out.is_file() => return Ok(out),
            Ok(_) => {}
            Err(e) => transient = Some(e),
        }
    }
    Err(match transient {
        Some(e) => Miss::Transient(e),
        None => Miss::NoFrame("ffmpeg made no poster".into()),
    })
}

#[cfg(test)]
pub(crate) fn granted(token: &str) -> bool {
    STORE
        .lock()
        .map(|s| s.iter().any(|(t, _)| t == token))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The store is process-wide and tests run in parallel: the eviction test
    /// fills it past `CAP`, so every test that grants takes this lock.
    static STORE_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A minimal `ftyp` box with `brand`, then `body`.
    fn iso(brand: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 16];
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(body);
        v
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    /// A temp folder whose own name isn't hidden (tempfile's `.tmpXXXX` is).
    fn plain_dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("media-").tempdir().unwrap()
    }

    fn canon(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().canonicalize().unwrap()
    }

    fn get(token: &str, range: Option<&str>) -> Response<Vec<u8>> {
        let mut b = Request::builder().uri(format!("hollow-media://localhost/{token}"));
        if let Some(r) = range {
            b = b.header("Range", r);
        }
        answer("main", &b.body(Vec::new()).unwrap())
    }

    fn header<'a>(r: &'a Response<Vec<u8>>, name: &str) -> &'a str {
        r.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    #[test]
    fn ranges_parse_like_http() {
        assert_eq!(parse_range(Some("bytes=0-1"), 10), Ok(Some((0, 1))));
        assert_eq!(parse_range(Some("bytes=4-"), 10), Ok(Some((4, 9))));
        assert_eq!(parse_range(Some("bytes=-3"), 10), Ok(Some((7, 9))));
        assert_eq!(parse_range(Some("bytes=-500"), 10), Ok(Some((0, 9))));
        assert_eq!(parse_range(Some("bytes=2-500"), 10), Ok(Some((2, 9))));
        assert_eq!(parse_range(Some("bytes=10-"), 10), Err(Unsatisfiable));
        assert_eq!(parse_range(Some("bytes=5-2"), 10), Err(Unsatisfiable));
        assert_eq!(parse_range(Some("bytes=1-2, 5-6"), 10), Ok(Some((1, 2))));
        assert_eq!(parse_range(Some("bytes=x-y"), 10), Ok(None));
        assert_eq!(parse_range(Some("items=0-1"), 10), Ok(None));
        assert_eq!(parse_range(None, 10), Ok(None));
    }

    #[test]
    fn sniff_takes_iso_video_and_refuses_the_rest() {
        assert_eq!(sniff(&iso(b"isom", b""), "mp4"), Ok("video/mp4"));
        assert_eq!(sniff(&iso(b"isom", b""), "mov"), Ok("video/quicktime"));
        assert_eq!(sniff(&iso(b"qt  ", b""), "m4v"), Ok("video/x-m4v"));
        assert_eq!(
            sniff(&[0, 0, 0, 8, b'm', b'd', b'a', b't'], "mov"),
            Ok("video/quicktime")
        );
        for brand in [b"heic", b"avif", b"mif1"] {
            assert_eq!(
                sniff(&iso(brand, b""), "mp4"),
                Err(MediaRefusal::UnsupportedContainer)
            );
        }
        let ebml = [0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01];
        assert_eq!(sniff(&ebml, "mp4"), Err(MediaRefusal::UnsupportedContainer));
        assert_eq!(
            sniff(b"hello world, not a video", "mp4"),
            Err(MediaRefusal::UnsupportedContainer)
        );
        assert_eq!(
            sniff(&iso(b"isom", b""), "webm"),
            Err(MediaRefusal::UnsupportedContainer)
        );
    }

    #[test]
    fn admit_refuses_what_it_should() {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = canon(&vault);
        let roots = vec![root.clone()];
        let video = iso(b"isom", &[1; 64]);
        write(&root.join("ok/clip.mp4"), &video);
        write(&canon(&outside).join("far.mp4"), &video);
        write(&root.join(".hidden/clip.mp4"), &video);
        write(&root.join("secrets/clip.mp4"), &video);
        write(&root.join("ok/.clip.mp4"), &video);
        write(&root.join("ok/clip.mkv"), &video);
        write(
            &root.join("ok/notes.mp4"),
            b"just some text in a file named mp4",
        );
        write(
            &root.join("ok/mkv.mp4"),
            &[0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01],
        );
        write(&root.join("ok/photo.mp4"), &iso(b"heic", &[0; 32]));
        write(&root.join("ok/empty.mp4"), b"");
        #[cfg(unix)]
        std::os::unix::fs::symlink(canon(&outside).join("far.mp4"), root.join("ok/link.mp4"))
            .unwrap();
        #[cfg(unix)]
        {
            let fifo =
                std::ffi::CString::new(root.join("ok/pipe.mp4").to_string_lossy().as_bytes())
                    .unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        }

        let refused = |p: &str| admit(p, &roots).err();
        let at = |rel: &str| root.join(rel).to_string_lossy().into_owned();
        assert_eq!(
            refused("ok/clip.mp4"),
            Some(MediaRefusal::OutsideRoots),
            "relative"
        );
        assert_eq!(
            refused(&canon(&outside).join("far.mp4").to_string_lossy()),
            Some(MediaRefusal::OutsideRoots)
        );
        #[cfg(unix)]
        assert_eq!(
            refused(&at("ok/link.mp4")),
            Some(MediaRefusal::OutsideRoots),
            "symlink out"
        );
        let dotdot = format!(
            "{}/ok/../../{}/far.mp4",
            root.display(),
            canon(&outside).file_name().unwrap().to_string_lossy()
        );
        assert_eq!(refused(&dotdot), Some(MediaRefusal::OutsideRoots), "..");
        assert_eq!(refused(&at(".hidden/clip.mp4")), Some(MediaRefusal::Hidden));
        assert_eq!(refused(&at("secrets/clip.mp4")), Some(MediaRefusal::Hidden));
        assert_eq!(refused(&at("ok/.clip.mp4")), Some(MediaRefusal::Hidden));
        assert_eq!(refused(&at("ok/clip.mkv")), Some(MediaRefusal::NotVideo));
        assert_eq!(
            refused(&at("ok/notes.mp4")),
            Some(MediaRefusal::UnsupportedContainer)
        );
        assert_eq!(
            refused(&at("ok/mkv.mp4")),
            Some(MediaRefusal::UnsupportedContainer)
        );
        assert_eq!(
            refused(&at("ok/photo.mp4")),
            Some(MediaRefusal::UnsupportedContainer)
        );
        assert_eq!(
            refused(&at("ok/empty.mp4")),
            Some(MediaRefusal::UnsupportedContainer)
        );
        assert_eq!(refused(&at("ok/gone.mp4")), Some(MediaRefusal::Missing));
        assert_eq!(refused(&at("ok")), Some(MediaRefusal::NotVideo), "a folder");
        #[cfg(unix)]
        assert_eq!(
            refused(&at("ok/pipe.mp4")),
            Some(MediaRefusal::NotVideo),
            "a FIFO, without blocking"
        );
        assert_eq!(refused("/tmp/x\0.mp4"), Some(MediaRefusal::OutsideRoots));
        // /tmp is not a root unless it is given as one.
        let tmp = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile_in("/tmp")
            .unwrap();
        std::fs::write(tmp.path(), &video).unwrap();
        assert_eq!(
            refused(&tmp.path().to_string_lossy()),
            Some(MediaRefusal::OutsideRoots)
        );
    }

    #[test]
    fn admit_accepts_secret_looking_names_and_isom_movs() {
        let vault = tempfile::tempdir().unwrap();
        let root = canon(&vault);
        write(
            &root.join("outputs/token-flow-demo.mp4"),
            &iso(b"isom", &[2; 32]),
        );
        write(&root.join("outputs/clip.mov"), &iso(b"isom", &[2; 32]));
        write(&root.join("outputs/clip.m4v"), &iso(b"M4V ", &[2; 32]));
        let roots = vec![root.clone()];
        let a = admit(
            &root.join("outputs/token-flow-demo.mp4").to_string_lossy(),
            &roots,
        )
        .unwrap();
        assert_eq!(a.mime, "video/mp4");
        assert_eq!(
            admit(&root.join("outputs/clip.mov").to_string_lossy(), &roots)
                .unwrap()
                .mime,
            "video/quicktime"
        );
        assert_eq!(
            admit(&root.join("outputs/clip.m4v").to_string_lossy(), &roots)
                .unwrap()
                .mime,
            "video/x-m4v"
        );
    }

    #[test]
    fn roots_add_the_chat_folder_only_for_its_conversation() {
        let vault = tempfile::tempdir().unwrap();
        let chat = plain_dir();
        let v = canon(&vault);
        let c = canon(&chat);
        crate::session_store::start_turn(
            &v,
            "s-media-roots",
            "claude",
            &c.to_string_lossy(),
            "run-media-roots",
            "Render",
        )
        .unwrap();
        let without = roots(&v, None);
        assert_eq!(without.first(), Some(&v));
        assert!(!without.contains(&c));
        let with = roots(&v, Some("s-media-roots"));
        assert!(with.contains(&c), "{with:?}");
        assert!(!roots(&v, Some("s-media-other")).contains(&c));
        write(&c.join("render.mp4"), &iso(b"isom", &[3; 32]));
        assert!(admit(&c.join("render.mp4").to_string_lossy(), &with).is_ok());
        assert_eq!(
            admit(&c.join("render.mp4").to_string_lossy(), &without).err(),
            Some(MediaRefusal::OutsideRoots)
        );
    }

    #[test]
    fn a_connected_project_or_worktree_root_admits_its_files() {
        // `roots` adds them from the manifest and the binding store; here the
        // root list is given, as `media_grant` passes it.
        let project = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let roots = vec![canon(&project), canon(&worktree)];
        for dir in [&project, &worktree] {
            write(&canon(dir).join("out/demo.mp4"), &iso(b"isom", &[4; 32]));
            assert!(admit(&canon(dir).join("out/demo.mp4").to_string_lossy(), &roots).is_ok());
        }
    }

    #[test]
    fn chat_roots_refuse_home_slash_and_hidden() {
        let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
        assert!(!chat_root_ok(Path::new("/"), home.as_deref()));
        assert!(!chat_root_ok(Path::new("/Users"), home.as_deref()));
        assert!(!chat_root_ok(Path::new("/private"), home.as_deref()));
        if let Some(h) = &home {
            assert!(!chat_root_ok(h, Some(h)));
            assert!(!chat_root_ok(h.parent().unwrap(), Some(h)), "above home");
        }
        let dir = plain_dir();
        std::fs::create_dir_all(dir.path().join(".cache/x")).unwrap();
        assert!(!chat_root_ok(&dir.path().join(".cache/x"), home.as_deref()));
        assert!(chat_root_ok(dir.path(), home.as_deref()));
    }

    #[test]
    fn serve_answers_bounded_206_chunks() {
        let _g = lock();
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let path = root.join("big.mp4");
        // A 3 GiB sparse file that still sniffs as video.
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&iso(b"isom", &[7; 100])).unwrap();
        f.set_len(3 * 1024 * 1024 * 1024).unwrap();
        drop(f);
        let g = grant(admit(&path.to_string_lossy(), &[root]).unwrap());
        assert_eq!(g.token.len(), 32);
        assert!(g
            .token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let len = 3u64 * 1024 * 1024 * 1024;

        let r = get(&g.token, None);
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.body().len() as u64, MAX_CHUNK, "no Range reads one chunk");
        assert_eq!(
            header(&r, "Content-Range"),
            format!("bytes 0-{}/{len}", MAX_CHUNK - 1)
        );

        let r = get(&g.token, Some("bytes=4-11"));
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.body(), b"ftypisom");
        assert_eq!(header(&r, "Content-Range"), format!("bytes 4-11/{len}"));
        assert_eq!(header(&r, "Content-Length"), "8");
        assert_eq!(header(&r, "Content-Type"), "video/mp4");
        assert_eq!(header(&r, "Accept-Ranges"), "bytes");
        assert_eq!(header(&r, "X-Content-Type-Options"), "nosniff");
        assert_eq!(header(&r, "Cache-Control"), "no-store");
        assert_eq!(header(&r, "Referrer-Policy"), "no-referrer");
        assert!(r.headers().get("Access-Control-Allow-Origin").is_none());

        let r = get(&g.token, Some("bytes=100-"));
        assert_eq!(r.body().len() as u64, MAX_CHUNK, "clamped");
        assert_eq!(
            header(&r, "Content-Range"),
            format!("bytes 100-{}/{len}", 100 + MAX_CHUNK - 1)
        );

        let r = get(&g.token, Some(&format!("bytes={len}-")));
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(header(&r, "Content-Range"), format!("bytes */{len}"));

        let head = Request::builder()
            .method("HEAD")
            .uri(format!("hollow-media://localhost/{}", g.token))
            .body(Vec::new())
            .unwrap();
        let r = answer("main", &head);
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.body().is_empty());
        assert_eq!(header(&r, "Content-Length"), len.to_string());
        release(&g.token);
    }

    #[test]
    fn serve_refuses_other_webviews_unknown_tokens_and_changed_files() {
        let _g = lock();
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let path = root.join("clip.mp4");
        write(&path, &iso(b"isom", &[1; 64]));
        let roots = vec![root.clone()];
        let g = grant(admit(&path.to_string_lossy(), &roots).unwrap());
        let req = Request::builder()
            .uri(format!("hollow-media://localhost/{}", g.token))
            .body(Vec::new())
            .unwrap();
        assert_eq!(answer("browser-1", &req).status(), StatusCode::FORBIDDEN);
        assert_eq!(answer("md-preview-2", &req).status(), StatusCode::FORBIDDEN);
        assert_eq!(
            get("0123456789abcdef0123456789abcdef", None).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(&g.token, Some("bytes=0-3")).status(),
            StatusCode::PARTIAL_CONTENT
        );

        // Grown: 409.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[0; 8])
            .unwrap();
        assert_eq!(get(&g.token, None).status(), StatusCode::CONFLICT);

        // Same length, new mtime: 409.
        let g2 = grant(admit(&path.to_string_lossy(), &roots).unwrap());
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        drop(f);
        assert_eq!(get(&g2.token, None).status(), StatusCode::CONFLICT);

        // Swapped by rename: 410.
        let g3 = grant(admit(&path.to_string_lossy(), &roots).unwrap());
        write(&root.join("other.mp4"), &iso(b"isom", &[9; 80]));
        std::fs::rename(root.join("other.mp4"), &path).unwrap();
        assert_eq!(get(&g3.token, None).status(), StatusCode::GONE);

        // Swapped for a FIFO or a link: 410 at once, never a blocked read.
        #[cfg(unix)]
        {
            let g4 = grant(admit(&path.to_string_lossy(), &roots).unwrap());
            std::fs::remove_file(&path).unwrap();
            let fifo = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert_eq!(get(&g4.token, None).status(), StatusCode::GONE);
            std::fs::remove_file(&path).unwrap();
            write(&root.join("real.mp4"), &iso(b"isom", &[9; 80]));
            std::os::unix::fs::symlink(root.join("real.mp4"), &path).unwrap();
            assert_eq!(get(&g4.token, None).status(), StatusCode::GONE);
            release(&g4.token);
        }
        for t in [g.token, g2.token, g3.token] {
            release(&t);
        }
    }

    #[test]
    fn concurrent_requests_queue_instead_of_failing() {
        let _g = lock();
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let path = root.join("clip.mp4");
        let mut bytes = iso(b"isom", &[]);
        bytes.resize(6 * 1024 * 1024, 5);
        write(&path, &bytes);
        let g = grant(admit(&path.to_string_lossy(), &[root]).unwrap());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let statuses: Vec<StatusCode> = rt.block_on(async {
            let calls = (0..5).map(|i| {
                let req = Request::builder()
                    .uri(format!("hollow-media://localhost/{}", g.token))
                    .header("Range", format!("bytes={}-", i * 1024 * 1024))
                    .body(Vec::new())
                    .unwrap();
                tokio::spawn(queued("main".into(), req))
            });
            let mut out = Vec::new();
            for c in calls.collect::<Vec<_>>() {
                out.push(c.await.unwrap().status());
            }
            out
        });
        assert_eq!(statuses, vec![StatusCode::PARTIAL_CONTENT; 5]);
        release(&g.token);
    }

    #[test]
    fn grants_evict_the_least_recently_used() {
        let _g = lock();
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let path = root.join("clip.mp4");
        write(&path, &iso(b"isom", &[1; 64]));
        let roots = vec![root];
        let first = grant(admit(&path.to_string_lossy(), &roots).unwrap());
        let second = grant(admit(&path.to_string_lossy(), &roots).unwrap());
        let mut rest = Vec::new();
        for i in 0..CAP {
            if i == CAP / 2 {
                // Playing: each request touches the grant.
                assert_eq!(
                    get(&first.token, Some("bytes=0-1")).status(),
                    StatusCode::PARTIAL_CONTENT
                );
            }
            rest.push(grant(admit(&path.to_string_lossy(), &roots).unwrap()));
        }
        assert!(granted(&first.token), "a recently used grant stays");
        assert!(!granted(&second.token), "the least recently used goes");
        release(&first.token);
        assert!(!granted(&first.token));
        for g in rest {
            release(&g.token);
        }
    }

    fn tiny_video(root: &Path, name: &str) -> PathBuf {
        let video = root.join(name);
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../tests/fixtures/media/tiny.mp4"
            ),
            &video,
        )
        .unwrap();
        video
    }

    fn is_jpeg(path: &Path) -> bool {
        std::fs::read(path).is_ok_and(|b| b.starts_with(&[0xFF, 0xD8]))
    }

    /// QuickLook alone (no ffmpeg fallback) makes a JPEG of the fixture.
    /// Bounded to 5 s like the app: a cold QuickLook on a headless runner
    /// could time out, which is then a transient miss, not a frameless file.
    #[cfg(target_os = "macos")]
    #[test]
    fn quicklook_makes_a_jpeg_poster() {
        let dir = plain_dir();
        let video = tiny_video(&canon(&dir), "clip.mp4");
        let work = canon(&dir).join("work");
        std::fs::create_dir_all(&work).unwrap();
        let jpg = quicklook_jpeg(&video, &work).unwrap();
        assert!(is_jpeg(&jpg));
    }

    /// ffmpeg alone, when it is installed here.
    #[test]
    fn ffmpeg_makes_a_jpeg_poster_when_installed() {
        let Some(ffmpeg) = crate::process_capture::find_ffmpeg() else {
            return;
        };
        let dir = plain_dir();
        let video = tiny_video(&canon(&dir), "clip.mp4");
        let work = canon(&dir).join("work");
        std::fs::create_dir_all(&work).unwrap();
        let jpg = ffmpeg_jpeg(&ffmpeg, &video, &work).unwrap();
        assert!(is_jpeg(&jpg));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_poster_is_a_small_cached_jpeg_and_a_re_render_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let video = tiny_video(&root, "clip.mp4");
        let cache = root.join("cache").join("posters");
        let roots = std::slice::from_ref(&root);
        let file = admit(&video.to_string_lossy(), roots).unwrap();
        let url = poster(&file, &cache).unwrap();
        assert!(
            url.starts_with("data:image/jpeg;base64,"),
            "{}",
            &url[..40.min(url.len())]
        );
        assert!(url.len() < 210 * 1024);
        let jpgs = || -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(&cache)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".jpg"))
                .collect();
            v.sort();
            v
        };
        let first = jpgs();
        assert_eq!(first.len(), 1);
        assert_eq!(std::fs::read_dir(cache.join("tmp")).unwrap().count(), 0);
        assert_eq!(cached_poster(&file, &cache), Some(Ok(url.clone())));
        // Re-rendered in place (same inode, new mtime): a new poster, the old one gone.
        let f = std::fs::File::options().write(true).open(&video).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(120))
            .unwrap();
        drop(f);
        let again = admit(&video.to_string_lossy(), roots).unwrap();
        assert!(poster(&again, &cache).is_ok());
        let second = jpgs();
        assert_eq!(second.len(), 1);
        assert_ne!(second, first);
    }

    #[test]
    fn a_file_with_no_frame_is_remembered_and_leaves_no_work() {
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let video = root.join("broken.mp4");
        write(&video, &iso(b"isom", &[0; 256]));
        let cache = root.join("posters");
        let file = admit(&video.to_string_lossy(), std::slice::from_ref(&root)).unwrap();
        // Cache semantics do not depend on QuickLook finishing before its
        // deadline. Real renderer integration is covered by the tests above.
        let result = poster_with(&file, &cache, |_, _| Err(Miss::NoFrame("no frame".into())));
        assert_eq!(result, Err("no frame".into()));
        assert_eq!(std::fs::read_dir(cache.join("tmp")).unwrap().count(), 0);
        let key = poster_key(&file.id).unwrap();
        assert!(cache.join(format!("{key}.none")).is_file());
        // Known: answered from the mark, without running anything.
        assert!(matches!(cached_poster(&file, &cache), Some(Err(_))));
        assert!(poster_with(&file, &cache, |_, _| panic!(
            "cached miss must skip rendering"
        ))
        .is_err());
    }

    #[test]
    fn a_transient_poster_failure_leaves_no_mark_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let root = canon(&dir);
        let video = tiny_video(&root, "clip.mp4");
        let cache = root.join("posters");
        let file = admit(&video.to_string_lossy(), std::slice::from_ref(&root)).unwrap();
        for _ in 0..2 {
            let result = poster_with(&file, &cache, |_, _| {
                Err(Miss::Transient("timed out".into()))
            });
            assert_eq!(result, Err("timed out".into()));
            assert_eq!(cached_poster(&file, &cache), None);
            assert_eq!(std::fs::read_dir(cache.join("tmp")).unwrap().count(), 0);
        }
    }
}
