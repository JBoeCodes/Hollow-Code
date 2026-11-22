//! Results within a workspace (next-level P05, 2026-10-01).
//!
//! One list that joins a workspace's deliverables (`outputs/**`) with where each came from (the run receipt that last
//! wrote it) and the user's own review of it. Three independent facts, never
//! collapsed into one: the file (path, kind, size, content hash), the checks
//! its originating run recorded (a missing receipt is "not checked", never
//! zero failures), and the review state (draft / ready / accepted, plus
//! `changed` when the file no longer matches the accepted hash).
//!
//! Review records are local and versioned: `{workspace}/.hollow/results.json`
//! (dot-prefixed, so the workspace file walk, the vault watcher and agents'
//! listings skip it). Accepting is compare-and-swap on the content hash the
//! user was looking at, and keeps a snapshot of that exact text in
//! `.hollow/results/<sha256>.txt` (text files up to 2 MiB) so a later edit can
//! be compared with what was accepted. A file's timestamp is never approval,
//! and accepting publishes nothing.
//!
//! Only a person accepts (owner decision 3, merged plan slice 4, B·F05).
//! Every write names its [`Reviewer`]: the user in Hollow's window, or an
//! agent/automation run through `hollow_app`, which may mark a result draft
//! or ready for review and is refused `accepted`. It also cannot change the
//! review of a version the user accepted while those bytes are still on
//! disk; once the file changes it may mark the revision. An acceptance reads the
//! file once and hashes, compares and snapshots that one buffer, so the
//! accepted bytes are the hashed bytes, and records an [`Acceptance`]: the
//! actor, the run that wrote the file and its permission mode, the basis,
//! and the exact hash and size.
//!
//! A saved chat canvas (Canvas C3 · S3, `workspace_canvases.rs`) is one
//! Result of kind `canvas`: its visible copy `outputs/canvases/<id>.canvas.html`
//! is the file reviewed, hashed and accepted, titled from its sidecar, which
//! also gives the current revision ([`ResultCanvas`]). The sidecar
//! (`<id>.canvas.json`) is never listed or reviewed on its own, and the
//! revision store (`.revisions/`) is dot-prefixed, so the walk skips it. A
//! new revision rewrites the visible copy, so an accepted canvas then reads
//! `changed` like any other edited Result.

use crate::workspaces::{self, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const SCHEMA_VERSION: u32 = 1;
/// Files larger than this are listed but not hashed (their review cannot be
/// recorded against an exact version).
const HASH_LIMIT: u64 = 32 * 1024 * 1024;
/// Accepted text up to this size is kept for comparison.
const SNAPSHOT_LIMIT: u64 = 2 * 1024 * 1024;
/// Review history entries kept per result.
const HISTORY_KEEP: usize = 50;
/// The refusal an agent or automation gets for `accepted` (decision 3).
pub const PERSON_ONLY: &str = "RESULT_ACCEPT_PERSON_ONLY: only the user accepts a result. \
    Mark it ready for review and they decide in Results. Nothing changed.";
/// The refusal an agent or automation gets for changing the review of a
/// version the user accepted (decision 3: agents only mark results ready).
pub const ACCEPTED_PERSON_ONLY: &str = "RESULT_ACCEPTED_PERSON_ONLY: the user accepted this \
    version; only they can change that. Nothing changed.";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ReviewEvent {
    pub state: String,
    pub sha256: Option<String>,
    pub at: String,
    #[serde(default)]
    pub note: Option<String>,
    /// `person` | `agent`. Absent on events recorded before it.
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<crate::ts_bindings::ReviewActor>"))]
    pub actor: Option<String>,
    /// The agent run that recorded the event (`actor: agent` only).
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Who records a review. Only [`Reviewer::Person`] may accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reviewer {
    /// The user, in Hollow's own window (the Results view's Tauri command).
    Person,
    /// An agent or automation run (`hollow_app`, MCP, workflow code): it may
    /// only mark a result draft or ready for review.
    Agent { run_id: String },
}

impl Reviewer {
    fn actor(&self) -> &'static str {
        match self {
            Reviewer::Person => "person",
            Reviewer::Agent { .. } => "agent",
        }
    }
}

/// What an acceptance recorded (owner decision 3): who accepted, which run
/// wrote the accepted bytes and in what permission mode, what the person
/// accepted on, and the exact bytes (their hash and size; the text itself
/// is kept as the snapshot when it fits).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ResultAcceptance"))]
#[serde(rename_all = "camelCase")]
pub struct Acceptance {
    /// Always `person`: agents and automations cannot accept.
    #[cfg_attr(test, ts(type = "\"person\""))]
    pub actor: String,
    /// SHA-256 of the exact bytes read, compared and snapshotted.
    pub sha256: String,
    pub bytes: u64,
    /// The run that last wrote the file (its newest receipt), when any.
    #[serde(default)]
    pub run_id: Option<String>,
    /// That run's permission mode, when its turn recorded one.
    #[serde(default)]
    pub mode: Option<String>,
    /// What the person accepted on: the version they had open, and the
    /// run's checks at that moment.
    pub basis: String,
    /// Whether the accepted bytes were kept as a snapshot (text up to 2 MiB).
    pub snapshot: bool,
    pub at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    /// `draft` | `ready` | `accepted`. `changed` is derived, never stored.
    pub state: String,
    #[serde(default)]
    pub accepted_sha256: Option<String>,
    #[serde(default)]
    pub accepted_at: Option<String>,
    #[serde(default)]
    pub ready_at: Option<String>,
    /// A display title the user chose; the file is never renamed.
    #[serde(default)]
    pub title: Option<String>,
    /// The latest acceptance (absent on records accepted before it).
    #[serde(default)]
    pub acceptance: Option<Acceptance>,
    #[serde(default)]
    pub history: Vec<ReviewEvent>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    schema_version: u32,
    /// Keyed by the path relative to the workspace folder (`outputs/x.md`).
    #[serde(default)]
    results: BTreeMap<String, ReviewRecord>,
    /// Unknown fields from a newer Hollow survive a round trip.
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ResultOrigin {
    pub session_key: Option<String>,
    pub run_id: String,
    pub finished_at: String,
    pub status: String,
    pub verification: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ResultChecks {
    /// `None`: no receipt names this file, so nothing is known about checks.
    pub total: Option<usize>,
    pub failed: Option<usize>,
    pub unknown: Option<usize>,
}

/// A canvas Result's canvas (kind `canvas` only), from its sidecar.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ResultCanvas {
    pub id: String,
    /// The current revision. `None` when the sidecar is missing or
    /// unreadable (the file is still listed and reviewable).
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkResult {
    /// Vault-relative path (`workspaces/<id>/outputs/x.md`): the open target.
    pub path: String,
    /// Relative to the workspace folder (`outputs/x.md`).
    pub name: String,
    pub title: String,
    /// `document` | `table` | `image` | `video` | `page` | `review` |
    /// `canvas` | `pdf` | `data` | `file`. (`board` went with whiteboards on
    /// 2026-10-07: a leftover `.board.json` is `data`.)
    pub kind: String,
    pub size: u64,
    pub modified: Option<String>,
    pub sha256: Option<String>,
    /// `draft` | `ready` | `accepted` | `changed`.
    #[cfg_attr(test, ts(as = "crate::ts_bindings::ResultReview"))]
    pub review: String,
    pub accepted_sha256: Option<String>,
    pub accepted_at: Option<String>,
    pub ready_at: Option<String>,
    /// Whether an accepted snapshot exists to compare against.
    pub has_snapshot: bool,
    /// Who accepted the accepted version, on what, from which run.
    pub acceptance: Option<Acceptance>,
    pub history: Vec<ReviewEvent>,
    pub origin: Option<ResultOrigin>,
    pub checks: ResultChecks,
    /// Set for kind `canvas` (Canvas C3 · S3).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub canvas: Option<ResultCanvas>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn store_path(dir: &Path) -> PathBuf {
    dir.join(".hollow").join("results.json")
}
fn snapshot_path(dir: &Path, sha: &str) -> PathBuf {
    dir.join(".hollow")
        .join("results")
        .join(format!("{sha}.txt"))
}

/// The review state. Only a missing file is empty: an unreadable store is
/// an error and stays put, and a damaged one is moved aside
/// (`results.json.corrupt-<time>`) before the error, so a later save can
/// never replace review history with an empty store (B·F08).
fn read_store(dir: &Path) -> Result<Store, String> {
    let store = crate::state_file::read_json::<Store>(&store_path(dir), "Results review state")?;
    Ok(store.unwrap_or_else(|| Store {
        schema_version: SCHEMA_VERSION,
        ..Default::default()
    }))
}

/// The review state for an unattended read (Your work digests): like
/// [`read_store`] but never moves a damaged store aside, because nobody
/// would see the error saying where it went. The user-facing reads (`list`,
/// `list_all`, `set_review`) do the move and report it.
fn read_store_peek(dir: &Path) -> Result<Store, String> {
    let path = store_path(dir);
    match crate::state_file::peek_json::<Store>(&path) {
        Ok(store) => Ok(store.unwrap_or_else(|| Store {
            schema_version: SCHEMA_VERSION,
            ..Default::default()
        })),
        Err(e) => Err(format!(
            "Results review state could not be read ({}): {e}. It was left as is; open Results to see it.",
            path.display()
        )),
    }
}

/// The review records for a read-only query (B·F15): no store is an empty
/// one, but an unreadable or corrupt store is an error, and nothing is
/// written. Setting a corrupt store aside is [`read_store`]'s job, for the
/// views that own the store; a query run every minute must not leave a
/// backup per run, nor hide a workspace's results as an empty list.
fn read_store_strict(dir: &Path) -> Result<Store, String> {
    let path = store_path(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Store {
                schema_version: SCHEMA_VERSION,
                ..Default::default()
            })
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str::<Store>(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_store(dir: &Path, store: &Store) -> Result<(), String> {
    let mut store = store.clone();
    store.schema_version = store.schema_version.max(SCHEMA_VERSION);
    let text = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&store_path(dir), text.as_bytes())
}

/// A page review's capture folder (`outputs/reviews/<slug>/` beside
/// `<slug>.annotations.json`, next-level P15): its PNGs belong to the review,
/// which Results shows as one item, so they are not listed one by one.
pub(crate) fn is_review_capture_dir(path: &Path) -> bool {
    path.is_dir()
        && path.file_name().is_some_and(|n| {
            path.with_file_name(format!("{}.annotations.json", n.to_string_lossy()))
                .is_file()
        })
}

/// Whether a workspace-relative output path is a Result of its own. A saved
/// canvas's sidecar is not: its canvas is listed once, by its visible copy
/// (Canvas C3 · S3). A curated section canvas (`_plan.canvas.*`, C6 · K1) is
/// neither: it is a view of the markdown the panel shows, not a deliverable. Dot-prefixed folders (`.revisions/`) are pruned by the
/// walks themselves.
pub(crate) fn is_listed(rel: &str) -> bool {
    // A brief's archived older entries (the retired Archive older entries…) are
    // the brief's own record, not a deliverable.
    if rel.starts_with(crate::context_compact::ARCHIVE_DIR) {
        return false;
    }
    // A process capture's frames are its note's pictures, not Results.
    if crate::process_capture::is_frame(rel) {
        return false;
    }
    !matches!(
        crate::workspace_canvases::workspace_canvas_file(rel),
        Some((
            _,
            crate::workspace_canvases::CanvasFileRole::Sidecar
                | crate::workspace_canvases::CanvasFileRole::Curated
        ))
    )
}

pub(crate) fn kind_of(name: &str) -> &'static str {
    // A saved chat canvas's visible copy (Canvas C3 · S3); the path shape is
    // the store's, so a `.canvas.html` anywhere else stays a page.
    if matches!(
        crate::workspace_canvases::workspace_canvas_file(name),
        Some((_, crate::workspace_canvases::CanvasFileRole::Visible))
    ) {
        return "canvas";
    }
    let lower = name.to_ascii_lowercase();
    // A page review saved from the side panel browser (next-level P15).
    if lower.ends_with(".annotations.json") {
        return "review";
    }
    match lower.rsplit('.').next().unwrap_or("") {
        "md" | "markdown" | "txt" | "rtf" | "docx" | "doc" => "document",
        "csv" | "tsv" | "xlsx" | "xls" | "numbers" => "table",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" => "image",
        // mp4/m4v/mov play in Results (media.rs); webm stays a file.
        "mp4" | "m4v" | "mov" => "video",
        "html" | "htm" => "page",
        "pdf" => "pdf",
        "json" | "yaml" | "yml" | "toml" | "xml" | "ts" | "tsx" | "js" | "rs" | "py" | "sql"
        | "sh" | "patch" | "diff" => "data",
        _ => "file",
    }
}

/// A human title: a Markdown document's first heading (after frontmatter), a
/// page review's `title`, an HTML `<title>`, else the file name as words.
pub(crate) fn title_of(name: &str, head: &str) -> String {
    let kind = kind_of(name);
    if kind == "document" && (name.ends_with(".md") || name.ends_with(".markdown")) {
        let mut body = head;
        if let Some(rest) = head.strip_prefix("---\n") {
            if let Some(end) = rest.find("\n---") {
                body = &rest[end + 4..];
            }
        }
        if let Some(line) = body.lines().map(str::trim).find(|l| l.starts_with("# ")) {
            let t = line.trim_start_matches('#').trim();
            if !t.is_empty() {
                return t.chars().take(160).collect();
            }
        }
    }
    if kind == "review" {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(head) {
            if let Some(t) = v
                .get("title")
                .and_then(|t| t.as_str())
                .filter(|t| !t.trim().is_empty())
            {
                return t.trim().chars().take(160).collect();
            }
        }
        // A large file's 16 KiB head can be cut mid-JSON, so read the first
        // `title` from the raw text.
        if let Some(rest) = head
            .find("\"title\"")
            .map(|i| head[i + 7..].trim_start())
            .and_then(|r| r.strip_prefix(':'))
        {
            if let Some(Ok(t)) = serde_json::Deserializer::from_str(rest.trim_start())
                .into_iter::<String>()
                .next()
            {
                if !t.trim().is_empty() {
                    return t.trim().chars().take(160).collect();
                }
            }
        }
    }
    if kind == "canvas" {
        // Normally titled from the sidecar; this is the fallback when it is
        // missing or damaged: the root's `title`, even from a cut head.
        if let crate::canvas_fence::CanvasParse::Drawing(root)
        | crate::canvas_fence::CanvasParse::Ready { root, .. } =
            crate::canvas_fence::parse(head, false)
        {
            return root.title.chars().take(160).collect();
        }
    }
    if kind == "page" {
        let lower = head.to_ascii_lowercase();
        if let (Some(a), Some(b)) = (lower.find("<title>"), lower.find("</title>")) {
            if b > a + 7 {
                let t = head[a + 7..b].trim();
                if !t.is_empty() {
                    return t.chars().take(160).collect();
                }
            }
        }
    }
    let file = name.rsplit('/').next().unwrap_or(name);
    // A leftover whiteboard file (removed 2026-10-07) keeps a plain title.
    let stem = file
        .strip_suffix(".board.json")
        .or_else(|| file.strip_suffix(".canvas.html"))
        .unwrap_or_else(|| file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file));
    let words = stem.replace(['-', '_'], " ");
    let words = words.trim();
    let mut chars = words.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => file.to_string(),
    }
}

fn sha_of(path: &Path, size: u64) -> Option<String> {
    if size > HASH_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(hex(&Sha256::digest(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn head_text(path: &Path) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; 16 * 1024];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The newest receipt that names this vault path as its own, for its origin
/// and checks. A receipt that lists it as `shared` (another chat in the
/// same workspace folder changed it, chat concurrency F1, 2026-10-20) is
/// skipped: the origin is the run whose own write named it, or none.
fn origin_for<'a>(
    receipts: &'a [crate::evidence::EvidenceReceipt],
    vault_path: &str,
) -> Option<&'a crate::evidence::EvidenceReceipt> {
    receipts
        .iter()
        .filter(|r| r.files.iter().any(|f| f.path == vault_path && f.is_own()))
        .max_by(|a, b| a.finished_at.cmp(&b.finished_at))
}

fn derive_review(record: Option<&ReviewRecord>, sha: Option<&str>) -> String {
    match record {
        None => "draft".into(),
        Some(r) if r.state == "accepted" => {
            if r.accepted_sha256.as_deref().is_some() && r.accepted_sha256.as_deref() == sha {
                "accepted".into()
            } else {
                "changed".into()
            }
        }
        Some(r) if r.state == "ready" => "ready".into(),
        Some(_) => "draft".into(),
    }
}

fn result_for(
    dir: &Path,
    workspace: &Workspace,
    rel: &str,
    store: &Store,
    receipts: &[crate::evidence::EvidenceReceipt],
) -> Option<WorkResult> {
    result_for_with(dir, workspace, rel, store, receipts, true)
}

/// `hash_all: false` hashes only files with an accepted record (to tell
/// accepted from changed); the rest report `sha256: null`.
fn result_for_with(
    dir: &Path,
    workspace: &Workspace,
    rel: &str,
    store: &Store,
    receipts: &[crate::evidence::EvidenceReceipt],
    hash_all: bool,
) -> Option<WorkResult> {
    let full = dir.join(rel);
    let meta = std::fs::metadata(&full).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let record = store.results.get(rel);
    let sha = if hash_all || record.is_some_and(|r| r.state == "accepted") {
        sha_of(&full, size)
    } else {
        None
    };
    let vault_path = format!("{}/{rel}", workspace.path);
    let origin = origin_for(receipts, &vault_path);
    // A saved canvas: its title and current revision come from the sidecar.
    let canvas = crate::workspace_canvases::workspace_canvas_file(rel)
        .filter(|(_, role)| *role == crate::workspace_canvases::CanvasFileRole::Visible)
        .map(|(id, _)| {
            let side = full.parent().and_then(|canvases| {
                crate::workspace_canvases::sidecar_title_and_revision(canvases, id)
            });
            (id.to_string(), side)
        });
    let title = record
        .and_then(|r| r.title.clone())
        .or_else(|| {
            canvas
                .as_ref()
                .and_then(|(_, side)| side.as_ref())
                .map(|(title, _)| title.chars().take(160).collect())
        })
        .unwrap_or_else(|| title_of(rel, &head_text(&full)));
    let accepted = record.and_then(|r| r.accepted_sha256.clone());
    Some(WorkResult {
        path: vault_path,
        name: rel.to_string(),
        title,
        kind: kind_of(rel).into(),
        size,
        modified: workspaces::modified(&meta),
        review: derive_review(record, sha.as_deref()),
        has_snapshot: accepted
            .as_deref()
            .map(|s| snapshot_path(dir, s).is_file())
            .unwrap_or(false),
        accepted_sha256: accepted,
        accepted_at: record.and_then(|r| r.accepted_at.clone()),
        acceptance: record.and_then(|r| r.acceptance.clone()),
        ready_at: record.and_then(|r| r.ready_at.clone()),
        history: record
            .map(|r| r.history.iter().rev().take(10).cloned().collect())
            .unwrap_or_default(),
        sha256: sha,
        origin: origin.map(|r| ResultOrigin {
            session_key: r.session_key.clone(),
            run_id: r.run_id.clone(),
            finished_at: r.finished_at.clone(),
            status: r.status.clone(),
            verification: r.verification_state.clone(),
        }),
        checks: match origin {
            None => ResultChecks {
                total: None,
                failed: None,
                unknown: None,
            },
            Some(r) => ResultChecks {
                total: Some(r.checks.len()),
                failed: Some(r.checks.iter().filter(|c| c.outcome == "failed").count()),
                unknown: Some(
                    r.checks
                        .iter()
                        .filter(|c| !matches!(c.outcome.as_str(), "passed" | "failed"))
                        .count(),
                ),
            },
        },
        canvas: canvas.map(|(id, side)| ResultCanvas {
            id,
            revision: side.map(|(_, revision)| revision),
        }),
    })
}

/// Every deliverable in `outputs/`, newest first.
pub fn list(root: &Path, id: &str) -> Result<Vec<WorkResult>, String> {
    list_with(root, id, false)
}

/// [`list`] for an agent action or other unattended reader: a damaged review
/// store is an error but stays where it is, so acceptance records never
/// leave the user's Results view with the only message naming the copy
/// going to an agent (B·F08).
pub fn list_unattended(root: &Path, id: &str) -> Result<Vec<WorkResult>, String> {
    list_with(root, id, true)
}

/// [`list`]; with `peek`, a damaged review store is an error but stays where
/// it is (unattended callers, B·F08).
fn list_with(root: &Path, id: &str, peek: bool) -> Result<Vec<WorkResult>, String> {
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let store = {
        let _g = LOCK.lock().map_err(|_| "results lock poisoned")?;
        if peek {
            read_store_peek(&dir)?
        } else {
            read_store(&dir)?
        }
    };
    let receipts = crate::evidence::list(root, None, None, usize::MAX).unwrap_or_default();
    let outputs = dir.join("outputs");
    let mut out = Vec::new();
    if outputs.is_dir() {
        for entry in walkdir::WalkDir::new(&outputs)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name().to_string_lossy().starts_with('.')
                    && !is_review_capture_dir(e.path())
            })
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !is_listed(&rel) {
                continue;
            }
            if let Some(r) = result_for(&dir, &workspace, &rel, &store, &receipts) {
                out.push(r);
            }
        }
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// One Result file as a workspace's Activity sees it (Canvas C5b · Q3): what
/// it is and when the file system says it was born and last written. No
/// hash: Activity says what happened, Results says what is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResultFile {
    /// Relative to the workspace folder (`outputs/x.md`).
    pub rel: String,
    /// Vault-relative (`workspaces/<id>/outputs/x.md`).
    pub path: String,
    pub title: String,
    pub kind: &'static str,
    /// The file's birth time, where the file system keeps one (APFS does).
    pub created: Option<String>,
    pub modified: Option<String>,
    /// A saved canvas's kept revisions, oldest first: `(revision, saved at,
    /// the conversation that emitted it)`. Empty for other kinds and for a
    /// canvas whose sidecar can't be read.
    pub canvas_revisions: Vec<(u64, String, Option<String>)>,
}

/// What Activity reads of a workspace's Results: the files (at most
/// `max_files`, newest first) and every review event the store keeps (up to
/// [`HISTORY_KEEP`] per result, including results whose file is gone).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ResultActivity {
    pub files: Vec<ResultFile>,
    /// More files than `max_files` were found; the oldest were left out.
    pub truncated: bool,
    /// `(rel, title the person gave it, its history oldest first)`.
    pub reviews: Vec<(String, Option<String>, Vec<ReviewEvent>)>,
    /// The review store couldn't be read: the files are still listed, the
    /// review events aren't, and the store is left exactly where it is.
    pub store_error: Option<String>,
}

/// [`ResultActivity`] for workspace `id`. Read-only: a damaged review store
/// is reported, never moved aside (B·F08's unattended rule), because a
/// timeline is viewing and viewing writes nothing.
pub(crate) fn activity(root: &Path, id: &str, max_files: usize) -> Result<ResultActivity, String> {
    let dir = workspaces::folder(root, id)?;
    let workspace = workspaces::summary(root, id)?;
    let (store, store_error) = {
        let _g = LOCK.lock().map_err(|_| "results lock poisoned")?;
        match read_store_peek(&dir) {
            Ok(store) => (store, None),
            Err(e) => (Store::default(), Some(e)),
        }
    };
    let mut files = Vec::new();
    let outputs = dir.join("outputs");
    if outputs.is_dir() {
        for entry in walkdir::WalkDir::new(&outputs)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name().to_string_lossy().starts_with('.')
                    && !is_review_capture_dir(e.path())
            })
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !is_listed(&rel) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            files.push((rel, entry.path().to_path_buf(), meta));
        }
    }
    // Newest first, then the cap, so the files read for a title are bounded.
    files.sort_by(|a, b| {
        workspaces::modified(&b.2)
            .cmp(&workspaces::modified(&a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    let truncated = files.len() > max_files;
    files.truncate(max_files);
    let files = files
        .into_iter()
        .map(|(rel, full, meta)| {
            let record = store.results.get(&rel);
            let canvas_id = crate::workspace_canvases::workspace_canvas_file(&rel)
                .filter(|(_, role)| *role == crate::workspace_canvases::CanvasFileRole::Visible)
                .map(|(cid, _)| cid.to_string());
            let canvases = full.parent();
            let canvas_title = canvas_id
                .as_deref()
                .zip(canvases)
                .and_then(|(cid, dir)| {
                    crate::workspace_canvases::sidecar_title_and_revision(dir, cid)
                })
                .map(|(title, _)| title.chars().take(160).collect::<String>());
            let canvas_revisions = canvas_id
                .as_deref()
                .zip(canvases)
                .and_then(|(cid, dir)| crate::workspace_canvases::sidecar_revisions(dir, cid))
                .unwrap_or_default();
            let title = record
                .and_then(|r| r.title.clone())
                .or(canvas_title)
                .unwrap_or_else(|| title_of(&rel, &head_text(&full)));
            ResultFile {
                path: format!("{}/{rel}", workspace.path),
                kind: kind_of(&rel),
                title,
                created: meta
                    .created()
                    .ok()
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
                modified: workspaces::modified(&meta),
                canvas_revisions,
                rel,
            }
        })
        .collect();
    let reviews = store
        .results
        .into_iter()
        .filter(|(_, r)| !r.history.is_empty())
        .map(|(rel, r)| (rel, r.title, r.history))
        .collect();
    Ok(ResultActivity {
        files,
        truncated,
        reviews,
        store_error,
    })
}

fn relative_output(id: &str, workspace: &Workspace, path: &str) -> Result<String, String> {
    let rel = path
        .strip_prefix(&format!("{}/", workspace.path))
        .unwrap_or(path)
        .trim_start_matches('/');
    if !rel.starts_with("outputs/")
        || rel
            .split('/')
            .any(|seg| seg == ".." || seg.starts_with('.'))
    {
        return Err(format!(
            "{path} is not a result in workspace {id}: results live in outputs/"
        ));
    }
    if rel.starts_with(crate::context_compact::ARCHIVE_DIR) {
        return Err(format!(
            "{path} holds entries moved out of the brief, not a result"
        ));
    }
    if !is_listed(rel) {
        return Err(format!(
            "{path} is a saved canvas's record, not a result: review the canvas (its .canvas.html)"
        ));
    }
    Ok(rel.to_string())
}

/// What an acceptance rests on: the exact version the person had open, and
/// the checks the run that wrote it recorded (none known when no receipt
/// names the file, which is never "zero failures").
fn acceptance_basis(origin: Option<&crate::evidence::EvidenceReceipt>) -> String {
    let checks = match origin {
        None => "not checked (no run record names this file)".to_string(),
        Some(r) if r.checks.is_empty() => "its run recorded no checks".to_string(),
        Some(r) => {
            let failed = r.checks.iter().filter(|c| c.outcome == "failed").count();
            let unknown = r
                .checks
                .iter()
                .filter(|c| !matches!(c.outcome.as_str(), "passed" | "failed"))
                .count();
            format!(
                "{} checks recorded, {failed} failed, {unknown} not confirmed",
                r.checks.len()
            )
        }
    };
    format!("reviewed this exact version; {checks}")
}

/// The permission mode the run that wrote a result recorded on its turn
/// (`None` for an automation run, or a turn from before modes were kept).
fn run_mode(root: &Path, r: &crate::evidence::EvidenceReceipt) -> Option<String> {
    let key = r.session_key.as_deref()?;
    let mode = crate::session_store::turn_permission_for_run(root, key, &r.run_id);
    mode.unwrap_or_default()
}

/// Keep the accepted bytes for later comparison. An existing snapshot is
/// reused only when its bytes still hash to `sha` (the store sits in a
/// folder agents can write).
fn write_snapshot(dir: &Path, sha: &str, bytes: &[u8]) -> Result<(), String> {
    let snap = snapshot_path(dir, sha);
    let intact = std::fs::read(&snap).is_ok_and(|kept| hex(&Sha256::digest(&kept)) == sha);
    if intact {
        return Ok(());
    }
    crate::vault::atomic_write(&snap, bytes)
}

/// Record a review of the exact version the reviewer saw.
///
/// Only a person accepts (owner decision 3): an agent or automation
/// ([`Reviewer::Agent`]) asking for `accepted` is refused
/// (`RESULT_ACCEPT_PERSON_ONLY:`) before anything is read or written; it may
/// mark the result draft or ready for review, but not while the user's
/// acceptance of the current bytes stands (`RESULT_ACCEPTED_PERSON_ONLY:`):
/// only once the file has changed can an agent put the revision up again. `expected_sha256` must match
/// the file now: an edit since the reviewer looked is refused
/// (`RESULT_CHANGED:`), so an approval never lands on content the person did
/// not see. The file is read once, and that buffer is what is compared,
/// hashed and kept as the snapshot.
pub fn set_review(
    root: &Path,
    id: &str,
    path: &str,
    state: &str,
    expected_sha256: Option<&str>,
    note: Option<&str>,
    reviewer: &Reviewer,
) -> Result<WorkResult, String> {
    if !matches!(state, "draft" | "ready" | "accepted") {
        return Err(format!(
            "unknown review state {state}: use draft, ready or accepted"
        ));
    }
    if state == "accepted" && *reviewer != Reviewer::Person {
        return Err(PERSON_ONLY.into());
    }
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let rel = relative_output(id, &workspace, path)?;
    let full = dir.join(&rel);
    let meta = std::fs::metadata(&full).map_err(|_| format!("{rel} no longer exists"))?;
    // One read: the bytes checked against the reviewed hash are the bytes
    // hashed, recorded and snapshotted (B·F05).
    let bytes = if meta.len() <= HASH_LIMIT {
        std::fs::read(&full).ok()
    } else {
        None
    };
    let current = bytes.as_deref().map(|b| hex(&Sha256::digest(b)));
    if state == "accepted" && current.is_none() {
        return Err(format!(
            "{rel} is too large to record an exact accepted version"
        ));
    }
    if let Some(expected) = expected_sha256 {
        if current.as_deref() != Some(expected) {
            return Err(format!(
                "RESULT_CHANGED: {rel} changed since you opened it; review the new version"
            ));
        }
    } else if state == "accepted" {
        return Err("accepting needs the version you reviewed (expectedSha256)".into());
    }
    let receipts = crate::evidence::list(root, None, None, usize::MAX).unwrap_or_default();
    let vault_path = format!("{}/{rel}", workspace.path);
    let origin = origin_for(&receipts, &vault_path);
    let mode = match origin {
        Some(r) if state == "accepted" => run_mode(root, r),
        _ => None,
    };
    let _g = LOCK.lock().map_err(|_| "results lock poisoned")?;
    let mut store = read_store(&dir)?;
    // An agent may put a result up for review, never overturn a person's
    // acceptance of the version still on disk. Once the file has changed
    // (derived `changed`) it may mark the revision draft or ready.
    if matches!(reviewer, Reviewer::Agent { .. })
        && derive_review(store.results.get(&rel), current.as_deref()) == "accepted"
    {
        return Err(ACCEPTED_PERSON_ONLY.into());
    }
    let at = now();
    let record = store.results.entry(rel.clone()).or_default();
    record.state = state.to_string();
    match (state, bytes.as_deref(), current.as_deref()) {
        ("accepted", Some(bytes), Some(sha)) => {
            let snapshot =
                bytes.len() as u64 <= SNAPSHOT_LIMIT && std::str::from_utf8(bytes).is_ok();
            if snapshot {
                write_snapshot(&dir, sha, bytes)?;
            }
            record.accepted_sha256 = Some(sha.to_string());
            record.accepted_at = Some(at.clone());
            record.acceptance = Some(Acceptance {
                actor: reviewer.actor().into(),
                sha256: sha.to_string(),
                bytes: bytes.len() as u64,
                run_id: origin.map(|r| r.run_id.clone()),
                mode,
                basis: acceptance_basis(origin),
                snapshot,
                at: at.clone(),
            });
        }
        ("ready", _, _) => {
            record.ready_at = Some(at.clone());
        }
        _ => {}
    }
    record.history.push(ReviewEvent {
        state: state.into(),
        sha256: current,
        at,
        note: note.map(|n| n.chars().take(500).collect()),
        actor: Some(reviewer.actor().into()),
        run_id: match reviewer {
            Reviewer::Agent { run_id } => Some(run_id.clone()),
            Reviewer::Person => None,
        },
    });
    if record.history.len() > HISTORY_KEEP {
        let drop = record.history.len() - HISTORY_KEEP;
        record.history.drain(0..drop);
    }
    write_store(&dir, &store)?;
    result_for(&dir, &workspace, &rel, &store, &receipts)
        .ok_or_else(|| format!("{rel} no longer exists"))
}

/// The accepted text of a result, for comparing with the current file.
/// `None` when none was kept, or when the kept bytes no longer hash to
/// `sha256` (the accepted text is shown only if it is the accepted text).
pub fn snapshot(root: &Path, id: &str, path: &str, sha256: &str) -> Result<Option<String>, String> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not a content hash".into());
    }
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    relative_output(id, &workspace, path)?;
    Ok(std::fs::read(snapshot_path(&dir, sha256))
        .ok()
        .filter(|kept| hex(&Sha256::digest(kept)).eq_ignore_ascii_case(sha256))
        .and_then(|kept| String::from_utf8(kept).ok()))
}

/// A result's preview (images as data URLs, text capped) and its absolute
/// path for "Open in its app" (next-level P05). Outputs only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPreview {
    pub preview: crate::types::ChatPreviewFile,
    pub absolute: String,
}

pub fn preview(root: &Path, id: &str, path: &str) -> Result<ResultPreview, String> {
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let rel = relative_output(id, &workspace, path)?;
    let full = dir.join(&rel);
    let absolute = full.to_string_lossy().into_owned();
    Ok(ResultPreview {
        preview: crate::chat::preview_file_scoped(&absolute, &[dir.join("outputs")])?,
        absolute,
    })
}

/// A result with its workspace, for Library's cross-workspace views (P06).
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibraryResult {
    pub workspace_id: String,
    pub workspace_title: String,
    #[serde(flatten)]
    pub result: WorkResult,
}

/// Every workspace's results (optionally one kind), newest first, at most
/// `limit`. Hashes only what review state needs. Quick chats are skipped.
pub fn list_all(
    root: &Path,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<LibraryResult>, String> {
    list_all_with(root, kind, limit, false)
}

/// [`list_all`] for an agent action or other unattended reader: a damaged
/// review store is an error but is never moved aside (see
/// [`list_unattended`]).
pub fn list_all_unattended(
    root: &Path,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<LibraryResult>, String> {
    list_all_with(root, kind, limit, true)
}

fn list_all_with(
    root: &Path,
    kind: Option<&str>,
    limit: usize,
    peek: bool,
) -> Result<Vec<LibraryResult>, String> {
    let receipts = crate::evidence::list(root, None, None, usize::MAX).unwrap_or_default();
    let mut out = Vec::new();
    for w in workspaces::list(root)? {
        if workspaces::is_quick_chats(&w.id) {
            continue;
        }
        let Ok(dir) = workspaces::folder(root, &w.id) else {
            continue;
        };
        let outputs = dir.join("outputs");
        if !outputs.is_dir() {
            continue;
        }
        let store = {
            let _g = LOCK.lock().map_err(|_| "results lock poisoned")?;
            if peek {
                read_store_peek(&dir)?
            } else {
                read_store(&dir)?
            }
        };
        for entry in walkdir::WalkDir::new(&outputs)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name().to_string_lossy().starts_with('.')
                    && !is_review_capture_dir(e.path())
            })
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !is_listed(&rel) || kind.is_some_and(|k| kind_of(&rel) != k) {
                continue;
            }
            if let Some(r) = result_for_with(&dir, &w, &rel, &store, &receipts, false) {
                out.push(LibraryResult {
                    workspace_id: w.id.clone(),
                    workspace_title: w.title.clone(),
                    result: r,
                });
            }
        }
    }
    out.sort_by(|a, b| b.result.modified.cmp(&a.result.modified));
    out.truncate(limit.clamp(1, 2000));
    Ok(out)
}

/// A result waiting for the user's review, by its exact path (B·F15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AwaitingResult {
    /// Vault-relative path (`workspaces/<id>/outputs/x.md`): the open target.
    pub path: String,
    pub title: String,
    pub kind: String,
    /// `ready` | `changed`.
    pub review: String,
    /// When it started waiting: marked ready, or (changed) last modified.
    pub since: Option<String>,
}

/// Every result of workspace `w` waiting for the user: marked ready for
/// review, or changed since accepted. Reads the review records, not every
/// output, and hashes only accepted files (to tell accepted from changed).
/// An unreadable or corrupt review store is an error, and nothing is written.
pub(crate) fn awaiting_review(root: &Path, w: &Workspace) -> Result<Vec<AwaitingResult>, String> {
    let dir = workspaces::folder(root, &w.id)?;
    let store = {
        let _g = LOCK.lock().map_err(|_| "results lock poisoned")?;
        read_store_strict(&dir)?
    };
    let mut out = Vec::new();
    for (rel, record) in &store.results {
        if !matches!(record.state.as_str(), "ready" | "accepted")
            || !rel.starts_with("outputs/")
            || !is_listed(rel)
            || rel
                .split('/')
                .any(|seg| seg == ".." || seg.starts_with('.'))
        {
            continue;
        }
        let full = dir.join(rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let sha = if record.state == "accepted" {
            sha_of(&full, meta.len())
        } else {
            None
        };
        let review = derive_review(Some(record), sha.as_deref());
        if review != "ready" && review != "changed" {
            continue;
        }
        out.push(AwaitingResult {
            path: format!("{}/{rel}", w.path),
            title: record
                .title
                .clone()
                .unwrap_or_else(|| title_of(rel, &head_text(&full))),
            kind: kind_of(rel).into(),
            since: if review == "ready" {
                record.ready_at.clone()
            } else {
                workspaces::modified(&meta)
            },
            review,
        });
    }
    Ok(out)
}

// ── Your work digests (next-level P02) ──────────────────────────────────

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LatestResult {
    pub path: String,
    pub title: String,
    pub kind: String,
    pub modified: Option<String>,
    #[cfg_attr(test, ts(as = "crate::ts_bindings::ResultReview"))]
    pub review: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDigest {
    pub workspace_id: String,
    pub reentry: crate::workspace_reentry::WorkspaceReentry,
    pub latest_result: Option<LatestResult>,
    pub result_count: usize,
    /// Results waiting for the user: ready for review, or changed since accepted.
    pub awaiting_review: usize,
    pub last_activity_at: Option<String>,
    /// Why this workspace's results couldn't be counted (a damaged or
    /// unreadable review store). Digests never move the store aside; the
    /// Results view does, and says where (B·F08).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub results_error: Option<String>,
}

/// The re-entry snapshot, latest result and review backlog of a few
/// workspaces at once (pinned and recent rows on Your work). Bounded: at most
/// 24 ids; an unreadable workspace is skipped, not an error for the rest.
pub fn digests(root: &Path, ids: &[String]) -> Result<Vec<WorkspaceDigest>, String> {
    let sessions = crate::session_store::list_sessions(root, 1000)?;
    let mut out = Vec::new();
    for id in ids.iter().take(24) {
        let Ok(workspace) = workspaces::summary(root, id) else {
            continue;
        };
        let Ok(dir) = workspaces::folder(root, id) else {
            continue;
        };
        let dirs = workspaces::session_dirs(&dir, &workspace);
        let mine: Vec<_> = sessions
            .iter()
            .filter(|s| dirs.contains(&workspaces::session_dir(&s.cwd)))
            .cloned()
            .collect();
        let reentry = crate::workspace_reentry::read(root, &workspace, &mine);
        // Unattended (Your work loads it, agents read it): never move a
        // damaged review store aside here, and say why nothing was counted.
        let (results, results_error) = match list_with(root, id, true) {
            Ok(results) => (results, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let awaiting_review = results
            .iter()
            .filter(|r| r.review == "ready" || r.review == "changed")
            .count();
        let last_activity_at = mine.iter().map(|s| s.last_activity_at.clone()).max();
        out.push(WorkspaceDigest {
            workspace_id: id.clone(),
            reentry,
            latest_result: results.first().map(|r| LatestResult {
                path: r.path.clone(),
                title: r.title.clone(),
                kind: r.kind.clone(),
                modified: r.modified.clone(),
                review: r.review.clone(),
            }),
            result_count: results.len(),
            awaiting_review,
            last_activity_at,
            results_error,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces::create(root.path(), "Review", "Make a brief").unwrap();
        (root, w)
    }

    #[test]
    fn a_page_review_lists_once_without_its_capture_images() {
        let dir = crate::test_temp::process_dir("hollow-results-review-");
        let reviews = dir.join("outputs/reviews");
        std::fs::create_dir_all(reviews.join("pricing")).unwrap();
        std::fs::write(
            reviews.join("pricing.annotations.json"),
            r#"{"title":"Pricing page"}"#,
        )
        .unwrap();
        std::fs::write(reviews.join("pricing/c1.png"), b"png").unwrap();
        std::fs::create_dir_all(dir.join("outputs/shots")).unwrap();
        std::fs::write(dir.join("outputs/shots/a.png"), b"png").unwrap();
        assert!(is_review_capture_dir(&reviews.join("pricing")));
        assert!(!is_review_capture_dir(&dir.join("outputs/shots")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn titles_and_kinds_come_from_content_not_timestamps() {
        assert_eq!(
            title_of(
                "outputs/q3-review.md",
                "---\ntitle: x\n---\n\n# Q3 regional review\n"
            ),
            "Q3 regional review"
        );
        // A leftover whiteboard file is data, titled from its name.
        assert_eq!(
            title_of(
                "outputs/release-flow.board.json",
                r#"{"version":2,"title":"Deploy decision"}"#
            ),
            "Release flow"
        );
        assert_eq!(
            title_of(
                "outputs/reviews/big.annotations.json",
                "{\n  \"schema\": 1,\n  \"title\": \"Big review\",\n  \"marks\": [{\"ti"
            ),
            "Big review"
        );
        assert_eq!(
            title_of("outputs/variance_table.csv", "a,b"),
            "Variance table"
        );
        assert_eq!(kind_of("outputs/a.board.json"), "data");
        assert_eq!(
            kind_of("outputs/reviews/pricing.annotations.json"),
            "review"
        );
        assert_eq!(
            title_of(
                "outputs/reviews/pricing.annotations.json",
                r#"{"hollowReview":1,"id":"x","title":"Pricing page"}"#
            ),
            "Pricing page"
        );
        assert_eq!(kind_of("outputs/a.CSV"), "table");
        assert_eq!(kind_of("outputs/a.pdf"), "pdf");
        // Videos play in Results (inline video); webm and mkv stay files.
        for name in ["outputs/a.mp4", "outputs/b.MOV", "outputs/c.m4v"] {
            assert_eq!(kind_of(name), "video", "{name}");
        }
        assert_eq!(kind_of("outputs/a.webm"), "file");
        assert_eq!(kind_of("outputs/a.mkv"), "file");
    }

    #[test]
    fn an_accepted_result_that_changes_needs_a_new_review() {
        let (root, w) = setup();
        let file = Path::new(&w.cwd).join("outputs/review.md");
        std::fs::write(&file, "# Review\n\nDraft one.").unwrap();
        let listed = list(root.path(), &w.id).unwrap();
        assert_eq!(listed.len(), 1);
        let r = &listed[0];
        assert_eq!(
            (r.review.as_str(), r.title.as_str(), r.checks.total),
            ("draft", "Review", None)
        );
        // Accepting needs the version the user saw.
        assert!(set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            None,
            None,
            &Reviewer::Person
        )
        .unwrap_err()
        .contains("expectedSha256"));
        assert!(set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            Some(&"0".repeat(64)),
            None,
            &Reviewer::Person
        )
        .unwrap_err()
        .starts_with("RESULT_CHANGED:"));
        let accepted = set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            r.sha256.as_deref(),
            Some("Looks right"),
            &Reviewer::Person,
        )
        .unwrap();
        assert_eq!(accepted.review, "accepted");
        assert!(accepted.has_snapshot);
        // A later edit is a new, unaccepted revision; the accepted text survives.
        std::fs::write(&file, "# Review\n\nDraft two.").unwrap();
        let changed = &list(root.path(), &w.id).unwrap()[0];
        assert_eq!(changed.review, "changed");
        assert_eq!(changed.accepted_sha256, r.sha256);
        let snap = snapshot(
            root.path(),
            &w.id,
            &changed.path,
            r.sha256.as_deref().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert!(snap.contains("Draft one."));
        // The review store is not a workspace file and not an output.
        let detail = workspaces::read(root.path(), &w.id).unwrap();
        assert!(detail.files.iter().all(|f| !f.name.starts_with(".hollow")));
    }

    /// Owner decision 3 (merged plan slice 4, B·F05): an agent or
    /// automation may mark a result ready, never accept it, and a refused
    /// accept writes nothing.
    #[test]
    fn an_agent_cannot_accept_a_result_only_mark_it_ready() {
        let (root, w) = setup();
        std::fs::write(Path::new(&w.cwd).join("outputs/brief.md"), "# Brief").unwrap();
        let r = list(root.path(), &w.id).unwrap().remove(0);
        let agent = Reviewer::Agent {
            run_id: "run-agent-1".into(),
        };
        let refused = set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            r.sha256.as_deref(),
            Some("The user said it looks fine"),
            &agent,
        )
        .unwrap_err();
        assert!(
            refused.starts_with("RESULT_ACCEPT_PERSON_ONLY:"),
            "{refused}"
        );
        assert!(
            !store_path(Path::new(&w.cwd)).exists(),
            "a refused accept records nothing"
        );
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "draft");
        let ready = set_review(
            root.path(),
            &w.id,
            &r.path,
            "ready",
            r.sha256.as_deref(),
            None,
            &agent,
        )
        .unwrap();
        assert_eq!(ready.review, "ready");
        assert_eq!(ready.acceptance, None);
        assert_eq!(
            (
                ready.history[0].actor.as_deref(),
                ready.history[0].run_id.as_deref()
            ),
            (Some("agent"), Some("run-agent-1"))
        );

        // Once the person accepts, the agent can't overturn it: neither
        // draft nor ready lands while the accepted bytes are on disk.
        let accepted = set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            r.sha256.as_deref(),
            None,
            &Reviewer::Person,
        )
        .unwrap();
        assert_eq!(accepted.review, "accepted");
        for state in ["draft", "ready"] {
            let refused = set_review(
                root.path(),
                &w.id,
                &r.path,
                state,
                r.sha256.as_deref(),
                None,
                &agent,
            )
            .unwrap_err();
            assert!(
                refused.starts_with("RESULT_ACCEPTED_PERSON_ONLY:"),
                "{state}: {refused}"
            );
            assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "accepted");
        }
        let kept = list(root.path(), &w.id).unwrap().remove(0);
        assert_eq!(
            kept.history.len(),
            accepted.history.len(),
            "refusals record nothing"
        );

        // After the file changes, the agent may put the revision up for review.
        std::fs::write(Path::new(&w.cwd).join("outputs/brief.md"), "# Brief v2").unwrap();
        let revised = list(root.path(), &w.id).unwrap().remove(0);
        assert_eq!(revised.review, "changed");
        let ready = set_review(
            root.path(),
            &w.id,
            &revised.path,
            "ready",
            revised.sha256.as_deref(),
            None,
            &agent,
        )
        .unwrap();
        assert_eq!(ready.review, "ready");
    }

    /// The guard that stops an agent overturning an acceptance is agent-only:
    /// the person reopens an accepted, unchanged result from Results ("Move
    /// back to draft", or ready), and once they have, an agent may mark it
    /// ready again.
    #[test]
    fn only_the_person_reopens_an_accepted_result() {
        let (root, w) = setup();
        std::fs::write(Path::new(&w.cwd).join("outputs/memo.md"), "# Memo").unwrap();
        let r = list(root.path(), &w.id).unwrap().remove(0);
        let sha = r.sha256.as_deref();
        let agent = Reviewer::Agent {
            run_id: "run-agent-2".into(),
        };
        let review = |state: &str, who: &Reviewer| {
            set_review(root.path(), &w.id, &r.path, state, sha, None, who)
        };
        for reopened in ["draft", "ready"] {
            assert_eq!(
                review("accepted", &Reviewer::Person).unwrap().review,
                "accepted"
            );
            let refused = review("draft", &agent).unwrap_err();
            assert!(
                refused.starts_with("RESULT_ACCEPTED_PERSON_ONLY:"),
                "{refused}"
            );
            let back = review(reopened, &Reviewer::Person).unwrap();
            assert_eq!(back.review, reopened, "the person reopens it");
            assert_eq!(back.history[0].actor.as_deref(), Some("person"));
            // Reopened, it is no longer the person's standing acceptance.
            assert_eq!(review("ready", &agent).unwrap().review, "ready");
        }
    }

    /// The accepted record names the person, the run that wrote the file,
    /// its permission mode and the basis, and its hash is the hash of the
    /// exact bytes kept; a later edit shows as changed.
    #[test]
    fn acceptance_stores_the_hash_of_the_exact_bytes_and_who_accepted() {
        let (root, w) = setup();
        let bytes = "# Q3 brief\n\nRevenue up 4%.\n";
        let file = Path::new(&w.cwd).join("outputs/q3.md");
        std::fs::write(&file, bytes).unwrap();
        let key = "s-20261002-accept01";
        let ord = crate::session_store::start_turn(
            root.path(),
            key,
            "claude",
            &w.cwd,
            "run-wrote-q3",
            "Write the brief",
        )
        .unwrap();
        crate::session_store::record_turn_permission(root.path(), key, ord, "auto").unwrap();
        crate::evidence::persist_for_test(
            root.path(),
            crate::evidence::EvidenceReceipt {
                id: "r-q3".into(),
                source: "chat".into(),
                run_id: "run-wrote-q3".into(),
                history_run_id: "run-wrote-q3".into(),
                session_key: Some(key.into()),
                status: "success".into(),
                finished_at: "2026-10-02T00:00:00Z".into(),
                files: vec![crate::evidence::ReceiptFile {
                    path: format!("{}/outputs/q3.md", w.path),
                    change_type: "create".into(),
                    attribution: "confirmed".into(),
                    tool: None,
                    additions: None,
                    deletions: None,
                    not_recoverable: false,
                    also_changed_by: Vec::new(),
                    hollow_write: false,
                }],
                checks: vec![crate::evidence::ReceiptCheck {
                    command: "npm test".into(),
                    kind: "test".into(),
                    outcome: "passed".into(),
                    sequence: 1,
                    excerpt: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let expected = hex(&Sha256::digest(bytes.as_bytes()));
        let r = list(root.path(), &w.id).unwrap().remove(0);
        assert_eq!(r.sha256.as_deref(), Some(expected.as_str()));
        let accepted = set_review(
            root.path(),
            &w.id,
            &r.path,
            "accepted",
            Some(&expected),
            None,
            &Reviewer::Person,
        )
        .unwrap();
        let a = accepted.acceptance.clone().expect("an acceptance record");
        assert_eq!(
            (
                a.actor.as_str(),
                a.sha256.as_str(),
                a.bytes,
                a.run_id.as_deref(),
                a.mode.as_deref(),
                a.snapshot
            ),
            (
                "person",
                expected.as_str(),
                bytes.len() as u64,
                Some("run-wrote-q3"),
                Some("auto"),
                true
            )
        );
        assert!(
            a.basis.starts_with("reviewed this exact version")
                && a.basis.contains("1 checks recorded, 0 failed"),
            "{}",
            a.basis
        );
        assert_eq!(accepted.history[0].actor.as_deref(), Some("person"));
        // The kept snapshot is the exact bytes that were hashed.
        let kept = std::fs::read(snapshot_path(Path::new(&w.cwd), &expected)).unwrap();
        assert_eq!(kept, bytes.as_bytes());
        assert_eq!(hex(&Sha256::digest(&kept)), a.sha256);
        // A file changed after acceptance shows as changed; the record stays.
        std::fs::write(&file, "# Q3 brief\n\nRevenue up 40%.\n").unwrap();
        let changed = list(root.path(), &w.id).unwrap().remove(0);
        assert_eq!(changed.review, "changed");
        assert_eq!(changed.acceptance, Some(a));
        // Bytes swapped under the snapshot's name are never shown as accepted.
        std::fs::write(
            snapshot_path(Path::new(&w.cwd), &expected),
            "# Q3 brief\n\nForged.\n",
        )
        .unwrap();
        assert_eq!(
            snapshot(root.path(), &w.id, &changed.path, &expected).unwrap(),
            None
        );
    }

    #[test]
    fn reviews_stay_inside_outputs_and_unknown_fields_survive() {
        let (root, w) = setup();
        std::fs::write(Path::new(&w.cwd).join("outputs/a.md"), "# A").unwrap();
        assert!(set_review(
            root.path(),
            &w.id,
            &format!("{}/WORKSPACE.md", w.path),
            "ready",
            None,
            None,
            &Reviewer::Person
        )
        .unwrap_err()
        .contains("outputs/"));
        assert!(set_review(
            root.path(),
            &w.id,
            &format!("{}/outputs/../WORKSPACE.md", w.path),
            "ready",
            None,
            None,
            &Reviewer::Person
        )
        .is_err());
        let dir = Path::new(&w.cwd).to_path_buf();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        std::fs::write(
            store_path(&dir),
            r#"{"schemaVersion":1,"results":{},"future":{"keep":true}}"#,
        )
        .unwrap();
        set_review(
            root.path(),
            &w.id,
            &format!("{}/outputs/a.md", w.path),
            "ready",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap();
        let text = std::fs::read_to_string(store_path(&dir)).unwrap();
        assert!(text.contains("\"future\""), "{text}");
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "ready");
    }

    #[test]
    fn a_corrupt_store_is_moved_aside_never_overwritten() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        let a = format!("{}/outputs/a.md", w.path);
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        // Not JSON, not even UTF-8: before B·F08 this read as an empty store
        // and the next review replaced it with no copy kept.
        let damaged = b"{\"results\": \xff".to_vec();
        std::fs::write(store_path(&dir), &damaged).unwrap();
        let err = set_review(
            root.path(),
            &w.id,
            &a,
            "ready",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap_err();
        assert!(err.contains("was damaged"), "{err}");
        let kept = crate::state_file::corrupt_copies(&store_path(&dir));
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read(&kept[0]).unwrap(),
            damaged,
            "bytes kept exactly"
        );
        // With the damage set aside, review starts a fresh store.
        set_review(
            root.path(),
            &w.id,
            &a,
            "ready",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap();
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "ready");
        assert_eq!(
            std::fs::read(&kept[0]).unwrap(),
            damaged,
            "the copy is never touched"
        );
    }

    #[test]
    fn a_damaged_store_fails_the_listing_instead_of_reading_empty() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        std::fs::write(store_path(&dir), "{not json").unwrap();
        let err = list(root.path(), &w.id).unwrap_err();
        assert!(err.contains("Results review state was damaged"), "{err}");
        assert_eq!(
            crate::state_file::corrupt_copies(&store_path(&dir)).len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_store_is_surfaced_and_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        let a = format!("{}/outputs/a.md", w.path);
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        set_review(
            root.path(),
            &w.id,
            &a,
            "ready",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap();
        let path = store_path(&dir);
        let before = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // running as root: permissions don't stop the read
        }
        let err = set_review(
            root.path(),
            &w.id,
            &a,
            "draft",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        assert!(list(root.path(), &w.id).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before, "never replaced");
        assert!(crate::state_file::corrupt_copies(&path).is_empty());
    }

    #[test]
    fn agent_reads_never_move_a_damaged_store_aside() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        let damaged = b"{\"results\": \xff".to_vec();
        std::fs::write(store_path(&dir), &damaged).unwrap();
        let err = list_unattended(root.path(), &w.id).unwrap_err();
        assert!(err.contains("left as is"), "{err}");
        let err = list_all_unattended(root.path(), None, 100).unwrap_err();
        assert!(err.contains("left as is"), "{err}");
        assert_eq!(
            std::fs::read(store_path(&dir)).unwrap(),
            damaged,
            "still in place for the user's Results view"
        );
        assert!(crate::state_file::corrupt_copies(&store_path(&dir)).is_empty());
        // The user's own view still moves it aside and says where.
        let err = list(root.path(), &w.id).unwrap_err();
        assert!(err.contains("was moved to"), "{err}");
    }

    #[test]
    fn library_lists_results_across_workspaces_by_kind() {
        let (root, w) = setup();
        let other = workspaces::create(root.path(), "Other", "x").unwrap();
        std::fs::write(Path::new(&w.cwd).join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(Path::new(&other.cwd).join("outputs/data")).unwrap();
        std::fs::write(
            Path::new(&other.cwd).join("outputs/data/flow-export.json"),
            r#"{"title":"Flow"}"#,
        )
        .unwrap();
        let all = list_all(root.path(), None, 100).unwrap();
        assert_eq!(all.len(), 2);
        let data = list_all(root.path(), Some("data"), 100).unwrap();
        assert_eq!(
            (
                data.len(),
                data[0].workspace_title.as_str(),
                data[0].result.title.as_str()
            ),
            (1, "Other", "Flow export")
        );
        assert!(data[0].result.sha256.is_none());
    }

    #[test]
    fn digests_report_the_latest_result_and_review_backlog() {
        let (root, w) = setup();
        std::fs::write(Path::new(&w.cwd).join("outputs/old.md"), "# Old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(Path::new(&w.cwd).join("outputs/new.md"), "# New report").unwrap();
        let first = &list(root.path(), &w.id).unwrap()[0];
        set_review(
            root.path(),
            &w.id,
            &first.path,
            "ready",
            None,
            None,
            &Reviewer::Person,
        )
        .unwrap();
        let d = digests(root.path(), &[w.id.clone(), "missing".into()]).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].result_count, 2);
        assert_eq!(d[0].awaiting_review, 1);
        assert_eq!(d[0].latest_result.as_ref().unwrap().title, "New report");
        assert!(d[0].results_error.is_none());
    }

    #[test]
    fn digests_never_move_a_damaged_store_aside() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        let damaged = b"{\"results\": \xff".to_vec();
        std::fs::write(store_path(&dir), &damaged).unwrap();
        let d = digests(root.path(), std::slice::from_ref(&w.id)).unwrap();
        assert_eq!(d.len(), 1, "the workspace is still listed");
        let err = d[0].results_error.as_deref().unwrap_or_default();
        assert!(
            err.contains("Results review state could not be read"),
            "{err}"
        );
        assert_eq!(d[0].result_count, 0);
        assert_eq!(
            std::fs::read(store_path(&dir)).unwrap(),
            damaged,
            "still in place"
        );
        assert!(crate::state_file::corrupt_copies(&store_path(&dir)).is_empty());
    }

    // ── Saved chat canvases (Canvas C3 · S3) ─────────────────────────────

    fn canvas_body(title: &str, inner: &str) -> String {
        format!("<hc-canvas id=\"release-status\" title=\"{title}\">\n{inner}\n</hc-canvas>\n")
    }

    /// Records a canvas body into the workspace store, as the chat will.
    fn record_canvas(
        root: &Path,
        w: &Workspace,
        body: &str,
        expected: Option<&str>,
    ) -> crate::workspace_canvases::CanvasRecorded {
        let app = root.join("app-data");
        crate::workspace_canvases::record(
            root,
            &app,
            crate::workspace_canvases::Scope {
                workspace_id: Some(&w.id),
                session_key: Some("s-canvas-1"),
            },
            body,
            Some("turn-1"),
            None,
            expected,
            "2026-10-04T10:00:00+00:00",
        )
        .unwrap()
    }

    const CANVAS: &str = "outputs/canvases/release-status.canvas.html";

    #[test]
    fn canvas_paths_are_the_stores_exact_shape() {
        assert_eq!(kind_of(CANVAS), "canvas");
        assert_eq!(kind_of("outputs/canvases/a-1.canvas.html"), "canvas");
        // Anything else stays what its extension says.
        assert_eq!(kind_of("outputs/x.canvas.html"), "page");
        assert_eq!(kind_of("outputs/canvases/deeper/x.canvas.html"), "page");
        assert_eq!(kind_of("outputs/canvases/Bad_Id.canvas.html"), "page");
        assert_eq!(kind_of("outputs/canvases/x.canvas.json"), "data");
        assert!(!is_listed("outputs/canvases/release-status.canvas.json"));
        assert!(is_listed("outputs/canvases/deeper/x.canvas.json"));
        assert!(is_listed("outputs/x.canvas.json"));
        assert!(is_listed(CANVAS));
        // A curated section canvas (C6 · K1) is a panel view, never a Result;
        // an underscore name that is no section stays an ordinary file.
        assert!(!is_listed("outputs/canvases/_plan.canvas.html"));
        assert!(!is_listed("outputs/canvases/_overview.canvas.json"));
        assert!(is_listed("outputs/canvases/_results.canvas.html"));
        // A compacted brief's archive is no Result either.
        assert!(!is_listed(
            "outputs/workspace-archive/2026-10-10-current-context.md"
        ));
        assert!(is_listed("outputs/workspace-archive.md"));
        // A process capture's note is a Result; its frames are not.
        assert!(is_listed("outputs/process-captures/tip-run/capture.md"));
        assert!(!is_listed(
            "outputs/process-captures/tip-run/frames/frame-001.jpg"
        ));
        // Without a readable sidecar the title is the root's, then the id.
        assert_eq!(
            title_of(
                CANVAS,
                "<hc-canvas id=\"release-status\" title=\"Ship &amp; tell\">"
            ),
            "Ship & tell"
        );
        assert_eq!(title_of(CANVAS, "not a canvas"), "Release status");
    }

    #[test]
    fn a_saved_canvas_is_one_result_titled_from_its_sidecar() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        std::fs::write(dir.join("outputs/notes.md"), "# Notes").unwrap();
        let v1 = canvas_body("Ship readiness", "<p>29 checks</p>");
        let saved = record_canvas(root.path(), &w, &v1, None);
        assert_eq!(saved.outcome, "created");
        assert!(dir
            .join("outputs/canvases/release-status.canvas.json")
            .is_file());
        assert!(dir
            .join("outputs/canvases/.revisions/release-status")
            .is_dir());

        let listed = list(root.path(), &w.id).unwrap();
        let names: Vec<&str> = listed.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names.len(),
            2,
            "the canvas once, beside the note: {names:?}"
        );
        assert!(names.contains(&"outputs/notes.md"));
        let c = listed.iter().find(|r| r.kind == "canvas").unwrap();
        assert_eq!(c.name, CANVAS, "the visible copy is the Result");
        assert_eq!(c.path, format!("{}/{CANVAS}", w.path));
        assert_eq!(c.title, "Ship readiness", "the sidecar's title, not the id");
        assert_eq!(
            c.canvas,
            Some(ResultCanvas {
                id: "release-status".into(),
                revision: Some(1)
            })
        );
        assert_eq!(
            c.sha256.as_deref(),
            Some(hex(&Sha256::digest(v1.as_bytes())).as_str()),
            "the hash of the canvas HTML file"
        );
        assert_eq!(c.review, "draft");
        // Library: once, by kind, never the sidecar.
        let all = list_all(root.path(), None, 100).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|r| !r.result.name.ends_with(".canvas.json")));
        let canvases = list_all(root.path(), Some("canvas"), 100).unwrap();
        assert_eq!(canvases.len(), 1);
        assert_eq!(canvases[0].result.title, "Ship readiness");
        assert!(list_all(root.path(), Some("data"), 100).unwrap().is_empty());

        // A new revision: the same one Result, its new title and revision.
        let v2 = canvas_body("Ship readiness, Friday", "<p>31 checks</p>");
        let revised = record_canvas(root.path(), &w, &v2, Some(&saved.canvas.sha256));
        assert_eq!(revised.outcome, "revised");
        let listed = list(root.path(), &w.id).unwrap();
        assert_eq!(listed.len(), 2);
        let c = listed.iter().find(|r| r.kind == "canvas").unwrap();
        assert_eq!(c.title, "Ship readiness, Friday");
        assert_eq!(c.canvas.as_ref().and_then(|c| c.revision), Some(2));

        // A damaged sidecar still lists the canvas, titled from its root,
        // with no revision; the sidecar stays hidden.
        std::fs::write(
            dir.join("outputs/canvases/release-status.canvas.json"),
            "{not json",
        )
        .unwrap();
        let listed = list(root.path(), &w.id).unwrap();
        assert_eq!(listed.len(), 2);
        let c = listed.iter().find(|r| r.kind == "canvas").unwrap();
        assert_eq!(c.title, "Ship readiness, Friday");
        assert_eq!(c.canvas.as_ref().map(|c| c.revision), Some(None));
    }

    #[test]
    fn an_accepted_canvas_reads_changed_after_a_new_revision() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        let v1 = canvas_body("Ship readiness", "<p>29 checks</p>");
        let saved = record_canvas(root.path(), &w, &v1, None);
        let c = list(root.path(), &w.id).unwrap().remove(0);
        let accepted = set_review(
            root.path(),
            &w.id,
            &c.path,
            "accepted",
            c.sha256.as_deref(),
            None,
            &Reviewer::Person,
        )
        .unwrap();
        assert_eq!(accepted.review, "accepted");
        let a = accepted.acceptance.clone().unwrap();
        let v1_sha = hex(&Sha256::digest(v1.as_bytes()));
        assert_eq!(
            (a.actor.as_str(), a.sha256.as_str(), a.bytes, a.snapshot),
            ("person", v1_sha.as_str(), v1.len() as u64, true),
            "the exact bytes of the canvas HTML file"
        );
        assert_eq!(
            std::fs::read(snapshot_path(&dir, &v1_sha)).unwrap(),
            v1.as_bytes()
        );

        // Re-rendering the accepted fence writes nothing: still accepted.
        assert_eq!(
            record_canvas(root.path(), &w, &v1, None).outcome,
            "unchanged"
        );
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "accepted");

        // Recording a revision never touches the review store, and the new
        // revision needs its own review.
        let store_before = std::fs::read(store_path(&dir)).unwrap();
        let v2 = canvas_body("Ship readiness", "<p>31 checks</p>");
        record_canvas(root.path(), &w, &v2, Some(&saved.canvas.sha256));
        assert_eq!(std::fs::read(store_path(&dir)).unwrap(), store_before);
        let changed = list(root.path(), &w.id).unwrap().remove(0);
        assert_eq!(changed.review, "changed");
        assert_eq!(changed.canvas.as_ref().and_then(|c| c.revision), Some(2));
        assert_eq!(changed.acceptance, Some(a), "the acceptance stays");
        assert!(changed.has_snapshot);
        let waiting = awaiting_review(root.path(), &w).unwrap();
        assert_eq!(
            waiting
                .iter()
                .map(|r| (r.kind.as_str(), r.review.as_str()))
                .collect::<Vec<_>>(),
            vec![("canvas", "changed")]
        );
        // Re-rendering the accepted (older) fence doesn't roll it back.
        assert_eq!(
            record_canvas(root.path(), &w, &v1, None).outcome,
            "unchanged"
        );
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "changed");
        // Accepting the new revision records its bytes.
        let again = set_review(
            root.path(),
            &w.id,
            &changed.path,
            "accepted",
            changed.sha256.as_deref(),
            None,
            &Reviewer::Person,
        )
        .unwrap();
        assert_eq!(again.review, "accepted");
        assert_eq!(
            again.acceptance.unwrap().sha256,
            hex(&Sha256::digest(v2.as_bytes()))
        );
    }

    #[test]
    fn an_agent_cannot_accept_a_canvas_and_its_record_is_not_reviewable() {
        let (root, w) = setup();
        let dir = Path::new(&w.cwd).to_path_buf();
        let v1 = canvas_body("Ship readiness", "<p>29 checks</p>");
        record_canvas(root.path(), &w, &v1, None);
        let c = list(root.path(), &w.id).unwrap().remove(0);
        let agent = Reviewer::Agent {
            run_id: "run-agent-1".into(),
        };
        let refused = set_review(
            root.path(),
            &w.id,
            &c.path,
            "accepted",
            c.sha256.as_deref(),
            None,
            &agent,
        )
        .unwrap_err();
        assert!(
            refused.starts_with("RESULT_ACCEPT_PERSON_ONLY:"),
            "{refused}"
        );
        assert!(
            !store_path(&dir).exists(),
            "a refused accept records nothing"
        );

        // Neither the sidecar nor a stored revision is a Result, for anyone.
        let fence = hex(&Sha256::digest(v1.as_bytes()));
        for path in [
            format!("{}/outputs/canvases/release-status.canvas.json", w.path),
            format!(
                "{}/outputs/canvases/.revisions/release-status/{fence}.canvas.html",
                w.path
            ),
        ] {
            let sha = sha_of(
                &dir.join(path.trim_start_matches(&format!("{}/", w.path))),
                0,
            );
            for (state, who) in [("accepted", &Reviewer::Person), ("ready", &agent)] {
                let err = set_review(root.path(), &w.id, &path, state, sha.as_deref(), None, who)
                    .unwrap_err();
                assert!(err.contains("not a result"), "{path}: {err}");
            }
        }
        assert!(!store_path(&dir).exists(), "nothing was recorded");

        // The agent may put the canvas up for review; once the person
        // accepts, the agent can't move it back.
        let ready = set_review(
            root.path(),
            &w.id,
            &c.path,
            "ready",
            c.sha256.as_deref(),
            None,
            &agent,
        )
        .unwrap();
        assert_eq!(
            (ready.review.as_str(), ready.kind.as_str()),
            ("ready", "canvas")
        );
        set_review(
            root.path(),
            &w.id,
            &c.path,
            "accepted",
            c.sha256.as_deref(),
            None,
            &Reviewer::Person,
        )
        .unwrap();
        let err = set_review(
            root.path(),
            &w.id,
            &c.path,
            "draft",
            c.sha256.as_deref(),
            None,
            &agent,
        )
        .unwrap_err();
        assert!(err.starts_with("RESULT_ACCEPTED_PERSON_ONLY:"), "{err}");
        assert_eq!(list(root.path(), &w.id).unwrap()[0].review, "accepted");
    }

    /// Chat concurrency F1 (2026-10-20, red first): a receipt that lists a
    /// file as `shared` (another chat in the folder changed it) is never
    /// its origin, however new: the origin is the run whose own write named
    /// it, or none.
    #[test]
    fn origin_skips_a_receipt_that_lists_the_file_as_shared() {
        let receipt = |id: &str, at: &str, attribution: &str| crate::evidence::EvidenceReceipt {
            id: id.into(),
            run_id: id.into(),
            finished_at: at.into(),
            files: vec![crate::evidence::ReceiptFile {
                path: "workspaces/w1/outputs/report.md".into(),
                change_type: "modify".into(),
                attribution: attribution.into(),
                tool: None,
                additions: None,
                deletions: None,
                not_recoverable: false,
                also_changed_by: Vec::new(),
                hollow_write: false,
            }],
            ..Default::default()
        };
        let path = "workspaces/w1/outputs/report.md";
        let receipts = vec![
            receipt("run-writer", "2026-10-20T10:00:00Z", "confirmed"),
            receipt("run-reviewer", "2026-10-20T11:00:00Z", "shared"),
        ];
        assert_eq!(
            origin_for(&receipts, path).map(|r| r.run_id.as_str()),
            Some("run-writer")
        );
        assert!(origin_for(&receipts[1..], path).is_none());
    }
}
