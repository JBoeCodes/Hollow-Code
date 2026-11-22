//! Saved annotation sets (next-level P15, 2026-10-01).
//!
//! The side panel browser's Annotate marks up a snapshot of a page. **Save
//! to workspace** turns that into a review the workspace owns:
//! `{workspace}/outputs/reviews/<slug>.annotations.json`, with each capture's
//! PNG beside it in `outputs/reviews/<slug>/<capture>.png`, so both show up
//! in Results and travel with the vault.
//!
//! File shape (`hollowAnnotations` 1), pretty-printed JSON:
//! `{ hollowAnnotations, id, title, workspaceId, createdAt, updatedAt,
//!    captures: [{ id, url, title, capturedAt, viewport, imageSize, image,
//!                 marks, parent }],
//!    links: [{ capture, mark, result, status, note, at }] }`.
//! - A capture is a *capture*: the page as it looked at `capturedAt`, in a
//!   `viewport` of CSS pixels, never a live view. `image` is the PNG's path
//!   relative to the workspace folder; `marks` are the user's marks in the
//!   image's own pixels (opaque here, bounded).
//! - "Compare with a newer capture" appends a capture whose `parent` is the
//!   one compared against, and `links` records, per requested change
//!   (`capture` + `mark` index), the capture that shows its result and the
//!   user's verdict: `done`, `notDone` or `unclear`. The newest link per
//!   mark wins.
//! - Unknown top-level fields survive a round trip.
//!
//! Creating never replaces a file; appending is compare-and-swap on the
//! SHA-256 of the JSON the reviewer read ([`CHANGED`]). Both writes are the
//! user's (`user-only` in the command map): a review records the user's own
//! marks and verdicts, so an agent may read it (to act on the requested
//! changes) but not grade its own work into it.

use crate::workspaces::{self, Workspace};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const CHANGED: &str = "REVIEW_CHANGED";
pub const INVALID: &str = "REVIEW_INVALID";
const FORMAT: u64 = 1;
const REVIEWS_DIR: &str = "outputs/reviews";
const SUFFIX: &str = ".annotations.json";
/// One capture's PNG.
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// The JSON (images are separate files).
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CAPTURES: usize = 40;
const MAX_MARKS: usize = 200;
const MAX_MARKS_BYTES: usize = 256 * 1024;
const MAX_TEXT: usize = 2048;
const MAX_LIST: usize = 300;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub captured_at: String,
    pub viewport: Size,
    #[serde(default)]
    pub image_size: Size,
    /// Relative to the workspace folder.
    pub image: String,
    #[serde(default)]
    pub marks: Value,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// The capture the mark was drawn on, and its index there.
    pub capture: String,
    pub mark: u32,
    /// The capture that shows the result.
    pub result: String,
    /// `done`, `notDone` or `unclear`.
    pub status: String,
    #[serde(default)]
    pub note: String,
    pub at: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ReviewFile {
    hollow_annotations: u64,
    id: String,
    title: String,
    workspace_id: String,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    captures: Vec<Capture>,
    #[serde(default)]
    links: Vec<Link>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// A capture as the reviewer sees it: the stored record and its image.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CaptureView {
    #[serde(flatten)]
    pub capture: Capture,
    /// `data:image/png;base64,…`, or null when the PNG is missing.
    pub image_data_url: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSet {
    pub workspace_id: String,
    /// Vault-relative.
    pub path: String,
    /// Relative to the workspace folder.
    pub file: String,
    pub sha256: String,
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub captures: Vec<CaptureView>,
    pub links: Vec<Link>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSetSummary {
    pub path: String,
    pub file: String,
    pub id: Option<String>,
    pub title: String,
    pub updated_at: Option<String>,
    pub url: Option<String>,
    pub captures: usize,
    /// Marks on the first capture (the requested changes).
    pub requested: usize,
    /// Of those, the ones whose newest link says `done`.
    pub done: usize,
    pub error: Option<String>,
}

/// A capture the frontend sends: the record plus its PNG.
#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewCapture {
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub captured_at: String,
    pub viewport: Size,
    #[serde(default)]
    pub image_size: Size,
    #[serde(default)]
    pub marks: Value,
    /// Base64 PNG (no data: prefix needed; one is accepted).
    pub png_base64: String,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewAnnotationSet {
    #[serde(default)]
    pub title: Option<String>,
    pub capture: NewCapture,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewLink {
    pub capture: String,
    pub mark: u32,
    pub status: String,
    #[serde(default)]
    pub note: String,
}

fn invalid(msg: impl std::fmt::Display) -> String {
    format!("{INVALID}: {msg}")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A random UUID v4 (also the tracker create journal's issue ids).
pub(crate) fn new_id() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 60 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "review".into()
    } else {
        out
    }
}

fn clip(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

/// The PNG bytes of a capture, checked.
fn png(base64: &str) -> Result<Vec<u8>, String> {
    let body = base64.trim();
    let body = body.strip_prefix("data:image/png;base64,").unwrap_or(body);
    if body.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
        return Err(invalid(format!(
            "the capture is over {} MiB",
            MAX_IMAGE_BYTES / 1024 / 1024
        )));
    }
    let bytes = STANDARD
        .decode(body)
        .map_err(|e| invalid(format!("the capture is not base64 ({e})")))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid("the capture is not a PNG"));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(invalid(format!(
            "the capture is over {} MiB",
            MAX_IMAGE_BYTES / 1024 / 1024
        )));
    }
    Ok(bytes)
}

fn validate_capture(c: &NewCapture) -> Result<(), String> {
    let url = c.url.trim();
    let lower = url.to_ascii_lowercase();
    if url.len() > MAX_TEXT
        || !(lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower == "about:blank")
    {
        return Err(invalid("a capture's url must be an http(s) address"));
    }
    if chrono::DateTime::parse_from_rfc3339(&c.captured_at).is_err() {
        return Err(invalid("capturedAt must be an RFC 3339 time"));
    }
    if c.viewport.width == 0
        || c.viewport.height == 0
        || c.viewport.width > 20_000
        || c.viewport.height > 20_000
    {
        return Err(invalid("the viewport must be a real size"));
    }
    match &c.marks {
        Value::Null => {}
        Value::Array(items) if items.len() <= MAX_MARKS => {
            if serde_json::to_string(&c.marks).map_err(invalid)?.len() > MAX_MARKS_BYTES {
                return Err(invalid("the marks are too large"));
            }
        }
        _ => {
            return Err(invalid(format!(
                "marks must be a list of at most {MAX_MARKS}"
            )))
        }
    }
    Ok(())
}

struct Located {
    workspace: Workspace,
    dir: PathBuf,
    rel: String,
    full: PathBuf,
}

fn locate(root: &Path, id: &str, path: &str) -> Result<Located, String> {
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let rel = path
        .strip_prefix(&format!("{}/", workspace.path))
        .unwrap_or(path)
        .trim_start_matches('/')
        .to_string();
    let bad = rel
        .split('/')
        .any(|seg| seg.is_empty() || seg == ".." || seg == "." || seg.starts_with('.'));
    if !rel.starts_with("outputs/") || bad || rel.contains('\\') || !rel.ends_with(SUFFIX) {
        return Err(format!(
            "{path} is not a review in workspace {id}: reviews are outputs/**/*{SUFFIX}"
        ));
    }
    let full = dir.join(&rel);
    if let (Ok(real), Ok(base)) = (std::fs::canonicalize(&full), std::fs::canonicalize(&dir)) {
        if !real.starts_with(&base) {
            return Err(format!("{path} leaves workspace {id}"));
        }
    }
    Ok(Located {
        workspace,
        dir,
        rel,
        full,
    })
}

fn parse(bytes: &[u8]) -> Result<ReviewFile, String> {
    if bytes.len() > MAX_FILE_BYTES + 64 * 1024 {
        return Err(invalid("the review file is too large"));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("the review file is not JSON ({e})")))?;
    match value.get("hollowAnnotations").and_then(Value::as_u64) {
        Some(FORMAT) => {}
        Some(_) => return Err(invalid("this review was written by a newer Hollow")),
        None => return Err(invalid("not a Hollow annotation set")),
    }
    serde_json::from_value(value)
        .map_err(|e| invalid(format!("the review file is unreadable ({e})")))
}

fn encode(file: &ReviewFile) -> Result<Vec<u8>, String> {
    let mut text = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
    text.push(b'\n');
    if text.len() > MAX_FILE_BYTES {
        return Err(invalid(format!(
            "the review would exceed {} MiB",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    Ok(text)
}

/// The image path for capture `capture` of the review at `rel`.
fn image_rel(rel: &str, capture: &str) -> String {
    let stem = rel.strip_suffix(SUFFIX).unwrap_or(rel);
    format!("{stem}/{capture}.png")
}

/// An image path from the file, accepted only inside the workspace's outputs/.
fn image_full(dir: &Path, image: &str) -> Option<PathBuf> {
    let bad = image
        .split('/')
        .any(|seg| seg.is_empty() || seg == ".." || seg == "." || seg.starts_with('.'));
    if !image.starts_with("outputs/") || bad || image.contains('\\') || !image.ends_with(".png") {
        return None;
    }
    let full = dir.join(image);
    match (std::fs::canonicalize(&full), std::fs::canonicalize(dir)) {
        (Ok(real), Ok(base)) if real.starts_with(&base) => Some(full),
        _ => None,
    }
}

fn wire(at: &Located, bytes: &[u8], file: ReviewFile) -> AnnotationSet {
    let captures = file
        .captures
        .into_iter()
        .map(|capture| {
            let image_data_url = image_full(&at.dir, &capture.image)
                .and_then(|p| std::fs::read(p).ok())
                .filter(|b| b.len() <= MAX_IMAGE_BYTES)
                .map(|b| format!("data:image/png;base64,{}", STANDARD.encode(b)));
            CaptureView {
                capture,
                image_data_url,
            }
        })
        .collect();
    AnnotationSet {
        workspace_id: at.workspace.id.clone(),
        path: format!("{}/{}", at.workspace.path, at.rel),
        file: at.rel.clone(),
        sha256: sha(bytes),
        id: file.id,
        title: file.title,
        created_at: file.created_at,
        updated_at: file.updated_at,
        captures,
        links: file.links,
    }
}

fn read_bytes(full: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(full).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "This review no longer exists.".to_string()
        } else {
            format!("Read review: {e}")
        }
    })
}

pub fn read(root: &Path, id: &str, path: &str) -> Result<AnnotationSet, String> {
    let at = locate(root, id, path)?;
    let bytes = read_bytes(&at.full)?;
    let file = parse(&bytes)?;
    Ok(wire(&at, &bytes, file))
}

/// Requested changes and how many the newest verdicts say are done.
fn progress(file: &ReviewFile) -> (usize, usize) {
    let Some(first) = file.captures.first() else {
        return (0, 0);
    };
    let requested = first.marks.as_array().map_or(0, Vec::len);
    let done = (0..requested)
        .filter(|i| {
            file.links
                .iter()
                .rev()
                .find(|l| l.capture == first.id && l.mark as usize == *i)
                .is_some_and(|l| l.status == "done")
        })
        .count();
    (requested, done)
}

/// Every `*.annotations.json` in a workspace's `outputs/`, newest first.
pub fn list(root: &Path, id: &str) -> Result<Vec<AnnotationSetSummary>, String> {
    let dir = workspaces::folder(root, id)?;
    let outputs = dir.join("outputs");
    let mut out = Vec::new();
    if !outputs.is_dir() {
        return Ok(out);
    }
    let workspace = workspaces::summary(root, id)?;
    for entry in walkdir::WalkDir::new(&outputs)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() || !entry.file_name().to_string_lossy().ends_with(SUFFIX) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&dir) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let path = format!("{}/{rel}", workspace.path);
        out.push(
            match std::fs::read(entry.path())
                .map_err(|e| e.to_string())
                .and_then(|b| parse(&b))
            {
                Ok(f) => {
                    let (requested, done) = progress(&f);
                    AnnotationSetSummary {
                        path,
                        file: rel,
                        id: Some(f.id.clone()),
                        title: f.title.clone(),
                        updated_at: Some(f.updated_at.clone()),
                        url: f.captures.first().map(|c| c.url.clone()),
                        captures: f.captures.len(),
                        requested,
                        done,
                        error: None,
                    }
                }
                Err(e) => AnnotationSetSummary {
                    title: rel.rsplit('/').next().unwrap_or(&rel).to_string(),
                    path,
                    file: rel,
                    id: None,
                    updated_at: None,
                    url: None,
                    captures: 0,
                    requested: 0,
                    done: 0,
                    error: Some(e),
                },
            },
        );
        if out.len() >= MAX_LIST {
            break;
        }
    }
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.file.cmp(&b.file))
    });
    Ok(out)
}

/// Write `bytes` to `target`, refusing to replace an existing file.
fn write_new(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn capture_record(c: NewCapture, id: &str, image: String, parent: Option<String>) -> Capture {
    Capture {
        id: id.to_string(),
        url: clip(&c.url, MAX_TEXT),
        title: clip(&c.title, 300),
        captured_at: c.captured_at,
        viewport: c.viewport,
        image_size: c.image_size,
        image,
        marks: if c.marks.is_null() {
            Value::Array(Vec::new())
        } else {
            c.marks
        },
        parent,
        extra: Map::new(),
    }
}

/// Save a marked-up capture as a new review in `outputs/reviews/`.
pub fn save(
    root: &Path,
    id: &str,
    input: NewAnnotationSet,
    now: &str,
) -> Result<AnnotationSet, String> {
    validate_capture(&input.capture)?;
    let image = png(&input.capture.png_base64)?;
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let reviews = dir.join(REVIEWS_DIR);
    std::fs::create_dir_all(&reviews).map_err(|e| format!("Create outputs/reviews: {e}"))?;
    if let (Ok(real), Ok(base)) = (std::fs::canonicalize(&reviews), std::fs::canonicalize(&dir)) {
        if !real.starts_with(&base) {
            return Err("outputs/reviews leaves the workspace".into());
        }
    }
    let fallback = input.capture.title.trim().to_string();
    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if fallback.is_empty() {
                "Page review".into()
            } else {
                format!("Review: {fallback}")
            }
        });
    let title = clip(&title, 180);
    let stem = slug(&title);
    let first = capture_record(input.capture, "c1", String::new(), None);
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for n in 1..=500 {
        let name = if n == 1 {
            format!("{stem}{SUFFIX}")
        } else {
            format!("{stem}-{n}{SUFFIX}")
        };
        let rel = format!("{REVIEWS_DIR}/{name}");
        let full = dir.join(&rel);
        if full.exists() || dir.join(rel.strip_suffix(SUFFIX).unwrap_or(&rel)).exists() {
            continue;
        }
        let image_rel = image_rel(&rel, "c1");
        let file = ReviewFile {
            hollow_annotations: FORMAT,
            id: new_id(),
            title: title.clone(),
            workspace_id: id.to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            captures: vec![Capture {
                image: image_rel.clone(),
                ..first.clone()
            }],
            links: Vec::new(),
            extra: Map::new(),
        };
        let bytes = encode(&file)?;
        let image_path = dir.join(&image_rel);
        std::fs::create_dir_all(image_path.parent().unwrap())
            .map_err(|e| format!("Save review: {e}"))?;
        write_new(&image_path, &image).map_err(|e| format!("Save capture: {e}"))?;
        match write_new(&full, &bytes) {
            Ok(()) => {
                let at = Located {
                    workspace,
                    dir,
                    rel,
                    full,
                };
                return Ok(wire(&at, &bytes, file));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&image_path);
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(format!("Save review: {e}"));
            }
        }
    }
    Err(invalid(format!("too many reviews named {stem}")))
}

/// "Compare with a newer capture": append `capture` (its parent is the
/// newest capture so far) and the user's verdict per requested change, if
/// the file is still the version they read.
pub fn add_capture(
    root: &Path,
    id: &str,
    path: &str,
    expected_sha256: &str,
    capture: NewCapture,
    links: Vec<NewLink>,
    now: &str,
) -> Result<AnnotationSet, String> {
    validate_capture(&capture)?;
    let image = png(&capture.png_base64)?;
    let at = locate(root, id, path)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = read_bytes(&at.full)?;
    if sha(&bytes) != expected_sha256 {
        return Err(format!("{CHANGED}: this review was changed elsewhere since you opened it; reopen it to see the newest version"));
    }
    let mut file = parse(&bytes)?;
    if file.captures.len() >= MAX_CAPTURES {
        return Err(invalid(format!(
            "a review keeps at most {MAX_CAPTURES} captures; start a new one"
        )));
    }
    for link in &links {
        let Some(on) = file.captures.iter().find(|c| c.id == link.capture) else {
            return Err(invalid(format!(
                "no capture {} in this review",
                link.capture
            )));
        };
        if link.mark as usize >= on.marks.as_array().map_or(0, Vec::len) {
            return Err(invalid(format!(
                "capture {} has no mark {}",
                link.capture,
                link.mark + 1
            )));
        }
        if !matches!(link.status.as_str(), "done" | "notDone" | "unclear") {
            return Err(invalid("a verdict is done, notDone or unclear"));
        }
    }
    let next = (1..)
        .map(|n| format!("c{n}"))
        .find(|c| !file.captures.iter().any(|x| &x.id == c))
        .unwrap();
    let image_rel = image_rel(&at.rel, &next);
    let image_path = at.dir.join(&image_rel);
    let parent = file.captures.last().map(|c| c.id.clone());
    file.captures
        .push(capture_record(capture, &next, image_rel, parent));
    for link in links {
        file.links.push(Link {
            capture: link.capture,
            mark: link.mark,
            result: next.clone(),
            status: link.status,
            note: clip(&link.note, 500),
            at: now.into(),
            extra: Map::new(),
        });
    }
    file.updated_at = now.into();
    let text = encode(&file)?;
    std::fs::create_dir_all(image_path.parent().unwrap())
        .map_err(|e| format!("Save capture: {e}"))?;
    // A leftover image of a failed earlier append is replaced; the JSON never is.
    crate::vault::atomic_write(&image_path, &image)?;
    crate::vault::atomic_write(&at.full, &text)?;
    Ok(wire(&at, &text, file))
}

// ── Tauri commands ─────────────────────────────────────────────────────

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// The saved annotation sets in a workspace's outputs/.
#[tauri::command]
pub async fn list_annotation_sets(
    id: String,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Vec<AnnotationSetSummary>, String> {
    let root = state.config.vault_path.clone();
    blocking(move || list(&root, &id)).await
}

/// One annotation set with its captures (images as data URLs) and verdicts.
#[tauri::command]
pub async fn read_annotation_set(
    id: String,
    path: String,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<AnnotationSet, String> {
    let root = state.config.vault_path.clone();
    blocking(move || read(&root, &id, &path)).await
}

/// Save a marked-up capture as a new review file.
#[tauri::command]
pub async fn save_annotation_set(
    id: String,
    input: NewAnnotationSet,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<AnnotationSet, String> {
    let root = state.config.vault_path.clone();
    let now = chrono::Utc::now().to_rfc3339();
    blocking(move || save(&root, &id, input, &now)).await
}

/// Append a newer capture and the user's verdicts; refused when the file changed.
#[tauri::command]
pub async fn add_annotation_capture(
    id: String,
    path: String,
    expected_sha256: String,
    capture: NewCapture,
    links: Vec<NewLink>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<AnnotationSet, String> {
    let root = state.config.vault_path.clone();
    let now = chrono::Utc::now().to_rfc3339();
    blocking(move || add_capture(&root, &id, &path, &expected_sha256, capture, links, &now)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T0: &str = "2026-10-01T10:00:00+00:00";
    const T1: &str = "2026-10-01T11:00:00+00:00";
    /// A 1×1 PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

    fn capture(url: &str, marks: Value) -> NewCapture {
        NewCapture {
            url: url.into(),
            title: "Pricing".into(),
            captured_at: T0.into(),
            viewport: Size {
                width: 1280,
                height: 720,
            },
            image_size: Size {
                width: 2560,
                height: 1440,
            },
            marks,
            png_base64: PNG.into(),
        }
    }
    fn marks() -> Value {
        json!([{ "kind": "pin", "x": 10, "y": 20, "note": "Make this blue" }, { "kind": "box", "x": 0, "y": 0, "w": 5, "h": 5, "note": "Too much space" }])
    }
    fn setup() -> (tempfile::TempDir, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces::create(root.path(), "Landing", "Ship it").unwrap();
        (root, w)
    }

    #[test]
    fn a_saved_review_keeps_url_time_viewport_and_its_capture_beside_it() {
        let (root, w) = setup();
        let set = save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: None,
                capture: capture("http://localhost:5173/pricing", marks()),
            },
            T0,
        )
        .unwrap();
        assert_eq!(set.file, "outputs/reviews/review-pricing.annotations.json");
        assert!(set.path.starts_with(&w.path));
        let c = &set.captures[0];
        assert_eq!(
            (
                c.capture.url.as_str(),
                c.capture.captured_at.as_str(),
                c.capture.viewport
            ),
            (
                "http://localhost:5173/pricing",
                T0,
                Size {
                    width: 1280,
                    height: 720
                }
            )
        );
        assert_eq!(c.capture.image, "outputs/reviews/review-pricing/c1.png");
        assert!(c
            .image_data_url
            .as_deref()
            .is_some_and(|d| d.starts_with("data:image/png;base64,")));
        let dir = workspaces::folder(root.path(), &w.id).unwrap();
        assert!(
            dir.join("outputs/reviews/review-pricing/c1.png").is_file(),
            "the capture is a file Results can show"
        );
        // A second save never replaces the first.
        let again = save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: None,
                capture: capture("http://localhost:5173/pricing", json!([])),
            },
            T0,
        )
        .unwrap();
        assert_eq!(
            again.file,
            "outputs/reviews/review-pricing-2.annotations.json"
        );
        assert_eq!(
            read(root.path(), &w.id, &set.path).unwrap().captures[0]
                .capture
                .marks,
            marks()
        );
        let listed = list(root.path(), &w.id).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|s| s.requested == 2
            && s.done == 0
            && s.url.as_deref() == Some("http://localhost:5173/pricing")));
    }

    #[test]
    fn a_newer_capture_links_each_requested_change_to_its_result() {
        let (root, w) = setup();
        let set = save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: Some("Pricing review".into()),
                capture: capture("http://localhost:5173/pricing", marks()),
            },
            T0,
        )
        .unwrap();
        let links = vec![
            NewLink {
                capture: "c1".into(),
                mark: 0,
                status: "done".into(),
                note: String::new(),
            },
            NewLink {
                capture: "c1".into(),
                mark: 1,
                status: "notDone".into(),
                note: "still loose".into(),
            },
        ];
        let mut after = capture("http://localhost:5173/pricing", json!([]));
        after.captured_at = T1.into();
        let next =
            add_capture(root.path(), &w.id, &set.path, &set.sha256, after, links, T1).unwrap();
        assert_eq!(next.captures.len(), 2);
        assert_eq!(next.captures[1].capture.id, "c2");
        assert_eq!(next.captures[1].capture.parent.as_deref(), Some("c1"));
        assert_eq!(next.captures[1].capture.captured_at, T1);
        assert_eq!(
            next.links
                .iter()
                .map(|l| (l.mark, l.result.as_str(), l.status.as_str()))
                .collect::<Vec<_>>(),
            [(0, "c2", "done"), (1, "c2", "notDone")]
        );
        let summary = list(root.path(), &w.id).unwrap();
        assert_eq!(
            (summary[0].requested, summary[0].done, summary[0].captures),
            (2, 1, 2)
        );
        // The old token is stale now: nothing is overwritten.
        let stale = add_capture(
            root.path(),
            &w.id,
            &set.path,
            &set.sha256,
            capture("http://localhost:5173/pricing", json!([])),
            vec![],
            T1,
        )
        .unwrap_err();
        assert!(stale.starts_with(CHANGED));
        // A verdict must name a real mark.
        let bad = add_capture(
            root.path(),
            &w.id,
            &set.path,
            &next.sha256,
            capture("http://localhost:5173/", json!([])),
            vec![NewLink {
                capture: "c1".into(),
                mark: 7,
                status: "done".into(),
                note: String::new(),
            }],
            T1,
        )
        .unwrap_err();
        assert!(bad.starts_with(INVALID));
    }

    #[test]
    fn captures_and_paths_are_checked() {
        let (root, w) = setup();
        let mut not_png = capture("http://localhost:5173/", json!([]));
        not_png.png_base64 = STANDARD.encode(b"GIF89a");
        assert!(save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: None,
                capture: not_png
            },
            T0
        )
        .unwrap_err()
        .starts_with(INVALID));
        assert!(save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: None,
                capture: capture("file:///etc/passwd", json!([]))
            },
            T0
        )
        .is_err());
        let mut no_time = capture("https://example.com/", json!([]));
        no_time.captured_at = "yesterday".into();
        assert!(save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: None,
                capture: no_time
            },
            T0
        )
        .is_err());
        assert!(read(root.path(), &w.id, "outputs/../WORKSPACE.md").is_err());
        assert!(read(root.path(), &w.id, "outputs/reviews/x.json").is_err());
        // A capture path in a hand-edited file can't reach outside outputs/.
        let dir = workspaces::folder(root.path(), &w.id).unwrap();
        assert!(image_full(&dir, "../../secret.png").is_none());
        assert!(image_full(&dir, "WORKSPACE.png").is_none());
    }

    #[test]
    fn unknown_fields_survive_an_append() {
        let (root, w) = setup();
        let set = save(
            root.path(),
            &w.id,
            NewAnnotationSet {
                title: Some("Keep".into()),
                capture: capture("https://example.com/", marks()),
            },
            T0,
        )
        .unwrap();
        let dir = workspaces::folder(root.path(), &w.id).unwrap();
        let full = dir.join(&set.file);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&full).unwrap()).unwrap();
        value["futureField"] = json!({ "kept": true });
        std::fs::write(&full, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let fresh = read(root.path(), &w.id, &set.path).unwrap();
        add_capture(
            root.path(),
            &w.id,
            &set.path,
            &fresh.sha256,
            capture("https://example.com/", json!([])),
            vec![],
            T1,
        )
        .unwrap();
        let after: Value = serde_json::from_slice(&std::fs::read(&full).unwrap()).unwrap();
        assert_eq!(after["futureField"], json!({ "kept": true }));
    }
}
