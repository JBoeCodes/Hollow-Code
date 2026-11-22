//! Saved chat canvases (Canvas C3 · S2, 2026-10-04).
//!
//! A `hollow-canvas` fence names its canvas with a stable `id`
//! ([`crate::canvas_fence::valid_canvas_id`]). This store keeps every canvas
//! the chat shows, so it survives a restart, travels with the vault and keeps
//! its history: files the workspace owns, compare-and-swap saves, at most
//! [`MAX_REVISIONS`] versions (the pattern the removed whiteboard store used).
//!
//! **Where.** In a workspace (Quick chats included), `outputs/canvases/`:
//! - `<id>.canvas.html`: the current revision's fence body exactly as the
//!   agent wrote it (`<hc-canvas …>…</hc-canvas>`), not the served document.
//!   It is a copy for people, Results and agents to read; Hollow renders from
//!   the revision store below and rewrites this copy on each new revision.
//! - `<id>.canvas.json`, the sidecar and the commit point (`hollowCanvas` 1),
//!   pretty-printed with the title near the top:
//!   `{ hollowCanvas, id, title, kit, session, fenceSha256, createdAt,
//!      updatedAt, revision, state, stateSavedAt, revisions, retired? }`.
//! - `.revisions/<id>/<fenceSha256>.canvas.html`: every kept revision's body,
//!   the current one included, content-addressed. The folder is dot-prefixed,
//!   so the Results walk, the vault watcher and agents' listings skip it and
//!   `outputs/` shows one canvas as two files, not thirty-one.
//!
//! Outside a workspace the same layout lives in app data, keyed by the
//! conversation: `<app data>/canvases/<sessionKey>/` (owner-only files). At most [`MAX_APP_SESSIONS`] conversations are kept;
//! the least recently saved are pruned when a new one is added. Each pruned
//! body is first remembered in `<app data>/canvases/.pruned` (a hash of
//! conversation and body, newest [`MAX_PRUNED`]), so re-rendering that
//! conversation's transcript is refused with [`RETIRED`] and recreates
//! nothing.
//!
//! **Revisions.** [`record`] takes a fence body, re-validates it with
//! `canvas_fence::validate` (the same rules as `src/canvas/fence.ts`) and
//! hashes it. A body whose SHA-256 is any kept revision's is a no-op that
//! reports which revision it is (re-rendering a transcript, or an earlier
//! fence of the same id, never adds a version). A new body adds a revision;
//! past [`MAX_REVISIONS`] the oldest are dropped from the sidecar and their
//! HTML deleted from the revision store, but their number and hash move to
//! `retired` (newest [`MAX_RETIRED`]). A retired body is a no-op too
//! (`retired`), so scrolling back through a long chat never rolls the canvas
//! back or writes a file. Known bodies are looked up before any folder is
//! created. `revisions` lists every kept revision, oldest first; the last is
//! current.
//!
//! **State.** `state` is the canvas's small UI state (bridge `state`, at most
//! [`MAX_STATE_BYTES`] of compact JSON, `LIMITS.stateBytes` in
//! `src/canvasBridge.ts`). It belongs to the canvas, not to a revision, so a
//! new revision carries it over; whether it still fits the new inputs is the
//! renderer's call.
//!
//! **Compare-and-swap.** Every write names the SHA-256 of the sidecar bytes
//! it last read (`expectedSha256`; none for "I saw no canvas"). Anything else
//! that wrote the sidecar since (another window, another revision, an agent's
//! file tools, a sync) makes the save fail with [`CHANGED`] and writes
//! nothing. Writes go temp file → full sync → rename → folder sync
//! (`vault::atomic_write`, `_private` in app data); a new sidecar is
//! published with an exclusive hard link, so it never replaces a file. The
//! order is revision HTML, then the visible copy, then the sidecar, so a
//! crash leaves the previous sidecar in charge. If the visible copy was
//! edited outside Hollow, the edited file is moved to
//! `.revisions/<id>/outside-<sha256>.canvas.html` before Hollow rewrites it,
//! so nothing is lost; it is not listed as a revision.
//!
//! **Paths.** Ids are `[a-z0-9-]{1,64}`, so a file name can't leave its
//! folder. Every folder on the way (`outputs`, `canvases`, `.revisions`, the
//! canvas's revision folder, and in app data `canvases/<sessionKey>`) must be
//! a real folder, never a symbolic link, and a symlinked sidecar or revision
//! file is refused. In a workspace the sidecar and the visible copy are also
//! checked with `run_guard::is_protected_target`, so a write can never land
//! on `me/` or an instruction file. Nothing here writes WORKSPACE.md,
//! PLAN.md or SOURCES.md, and reading never writes a file.
//!
//! **Retired curated views.** The curated section canvases (Canvas C6 · K1)
//! were removed in the COO rebuild. Their files (`_plan.canvas.html` /
//! `.canvas.json` and the other [`RETIRED_CURATED`] ids) may remain in older
//! workspaces: they are not listed, not Results and not Activity, and a
//! `section` attribute on a fence is now an ordinary unknown attribute. An
//! older sidecar's `curated` field is kept as unknown data.
//!
//! `LOCK` makes each read-compare-write atomic in-process. Writers that take
//! no Hollow lock (an agent's file tools, Obsidian) are caught by the
//! compare-and-swap.

use crate::canvas_fence;
use crate::workspaces;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A SHA-256 digest as 64 lowercase hex characters (a fence's or body's).
fn valid_sha256_hex(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub const CHANGED: &str = "CANVAS_CHANGED";
pub const INVALID: &str = "CANVAS_INVALID";
/// A body from a conversation whose app-data canvases were pruned.
pub const RETIRED: &str = "CANVAS_RETIRED";
const FORMAT: u64 = 1;
pub const MAX_REVISIONS: usize = 30;
/// Compact JSON, as the bridge measures it (`LIMITS.stateBytes`).
pub const MAX_STATE_BYTES: usize = 16 * 1024;
/// The sidecar: 30 short revision entries plus pretty-printed state.
pub const MAX_SIDECAR_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_APP_SESSIONS: usize = 500;
/// Revisions that aged out of [`MAX_REVISIONS`] and are still recognised.
pub const MAX_RETIRED: usize = 500;
/// Pruned app-data `(conversation, body)` pairs still recognised.
pub const MAX_PRUNED: usize = 8192;
const MAX_LIST: usize = 500;
const MAX_KIT_CHARS: usize = 32;
const MAX_TURN_ID_CHARS: usize = 128;
const OUTPUTS: &str = "outputs";
const CANVASES: &str = "canvases";
const REVISIONS: &str = ".revisions";
/// In `<app data>/canvases/`: one hex key per line, see [`pruned_key`].
const PRUNED: &str = ".pruned";

/// Read-compare-write must not interleave between two saves of one canvas.
static LOCK: Mutex<()> = Mutex::new(());

/// One kept revision (oldest first in `revisions`; the last is current).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasRevision {
    pub revision: u64,
    /// SHA-256 of the fence body; names its HTML in the revision store.
    pub fence_sha256: String,
    pub title: String,
    pub saved_at: String,
    #[serde(default)]
    pub ask: bool,
    #[serde(default)]
    pub height: Option<u32>,
    /// The conversation that emitted it.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Unknown fields survive a round trip.
    #[serde(flatten)]
    #[cfg_attr(test, ts(skip))]
    pub extra: Map<String, Value>,
}

/// A revision that aged out: only its number and hash are kept, so
/// re-rendering its fence is recognised and writes nothing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RetiredRevision {
    revision: u64,
    fence_sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct CanvasFile {
    hollow_canvas: u64,
    id: String,
    title: String,
    #[serde(default)]
    kit: Option<String>,
    #[serde(default)]
    session: Option<String>,
    fence_sha256: String,
    created_at: String,
    updated_at: String,
    revision: u64,
    #[serde(default)]
    state: Value,
    #[serde(default)]
    state_saved_at: Option<String>,
    revisions: Vec<CanvasRevision>,
    /// Oldest first, at most [`MAX_RETIRED`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retired: Vec<RetiredRevision>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// One saved canvas as the chat and panel see it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    #[cfg_attr(test, ts(as = "crate::ts_bindings::CanvasScope"))]
    pub scope: String,
    /// Set for a workspace canvas.
    pub workspace_id: Option<String>,
    /// Set for an app-data canvas: the conversation that keys the store.
    pub session_key: Option<String>,
    pub id: String,
    pub title: String,
    pub kit: Option<String>,
    /// The conversation that emitted the current revision.
    pub session: Option<String>,
    pub fence_sha256: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub ask: bool,
    pub height: Option<u32>,
    /// The current revision's fence body, from the revision store.
    pub html: String,
    /// Of the sidecar bytes: the compare-and-swap token for the next save.
    pub sha256: String,
    /// Vault-relative path of the visible copy (workspace canvases only).
    pub path: Option<String>,
    /// Workspace-relative path of the visible copy (workspace canvases only).
    pub file: Option<String>,
    /// The visible copy is missing or differs from the current revision.
    pub html_changed: bool,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub state: Value,
    pub state_saved_at: Option<String>,
    /// Oldest first; the last is current.
    pub revisions: Vec<CanvasRevision>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasSummary {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub updated_at: Option<String>,
    pub fence_sha256: Option<String>,
    pub path: Option<String>,
    pub file: Option<String>,
    /// Set when the sidecar could not be read as a canvas.
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasRecorded {
    /// `created`, `revised`, `unchanged` (the body is a kept revision), or
    /// `retired` (an earlier revision that aged out; nothing was written and
    /// its HTML is no longer kept, so render the fence itself).
    #[cfg_attr(test, ts(as = "crate::ts_bindings::CanvasRecordOutcome"))]
    pub outcome: String,
    /// The revision this body is (for `unchanged` and `retired`, an earlier
    /// one).
    pub matched_revision: u64,
    /// The canvas as saved now (for `retired`, its current revision).
    pub canvas: Canvas,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasStateSaved {
    pub sha256: String,
    pub state_saved_at: Option<String>,
    pub revision: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasRevisionHtml {
    pub revision: u64,
    pub fence_sha256: String,
    pub title: String,
    pub html: String,
}

/// Which store: a workspace's `outputs/canvases/`, or the app-data store of
/// one conversation. With a workspace, `session_key` is only recorded as
/// where a revision came from.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scope<'a> {
    pub workspace_id: Option<&'a str>,
    pub session_key: Option<&'a str>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn invalid(msg: impl std::fmt::Display) -> String {
    format!("{INVALID}: {msg}")
}

fn retired() -> String {
    format!("{RETIRED}: this conversation's saved canvases were cleared to save space; the canvas is shown from the chat and not saved again")
}

fn changed() -> String {
    format!("{CHANGED}: this canvas was changed elsewhere since it was read; reopen it to see the newest version")
}

fn check_id(id: &str) -> Result<(), String> {
    if crate::canvas_fence::valid_canvas_id(id) {
        Ok(())
    } else {
        Err(invalid(
            "a canvas id is 1–64 lowercase letters, digits or hyphens",
        ))
    }
}

/// The store ids of the retired curated section views (Canvas C6 · K1,
/// removed in the COO rebuild). Their files may remain in older workspaces;
/// nothing reads or writes them any more.
const RETIRED_CURATED: [&str; 4] = ["_overview", "_brief", "_plan", "_sources"];

fn check_session(key: &str) -> Result<(), String> {
    if crate::agent_sessions::is_safe_session_key(key) {
        Ok(())
    } else {
        Err(invalid("the session key is not a Hollow conversation key"))
    }
}

fn check_turn(turn: Option<&str>) -> Result<(), String> {
    match turn {
        Some(t)
            if t.is_empty()
                || t.chars().count() > MAX_TURN_ID_CHARS
                || t.chars().any(char::is_control) =>
        {
            Err(invalid(format!(
                "turnId must be 1–{MAX_TURN_ID_CHARS} characters without control characters"
            )))
        }
        _ => Ok(()),
    }
}

fn check_kit(kit: Option<&str>) -> Result<(), String> {
    match kit {
        Some(k)
            if k.is_empty()
                || k.len() > MAX_KIT_CHARS
                || !k.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'@' | b'.' | b'-')
                }) =>
        {
            Err(invalid(format!(
                "kit must be 1–{MAX_KIT_CHARS} of a-z, 0-9, @, . or -"
            )))
        }
        _ => Ok(()),
    }
}

fn check_state(state: &Value) -> Result<(), String> {
    let len = serde_json::to_vec(state).map_err(invalid)?.len();
    if len > MAX_STATE_BYTES {
        return Err(invalid(format!(
            "canvas state is {len} bytes; the limit is {} KB",
            MAX_STATE_BYTES / 1024
        )));
    }
    Ok(())
}

/// Where one store lives.
struct Store {
    dir: PathBuf,
    private: bool,
    vault: Option<PathBuf>,
    /// `(id, vault-relative workspace folder)` for a workspace store.
    workspace: Option<(String, String)>,
    session_key: Option<String>,
    /// The folder was created by this call (app-data pruning runs then).
    fresh: bool,
}

/// `base` joined with `parts`, each a real folder (never a symbolic link).
/// With `create`, missing folders are made; without, `Ok(None)` if one is
/// missing, so a read never writes.
fn real_dir(base: &Path, parts: &[&str], create: bool) -> Result<Option<(PathBuf, bool)>, String> {
    let mut at = base.to_path_buf();
    let mut made = false;
    for part in parts {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "{} is a symbolic link; canvases are only saved in real folders",
                    at.display()
                )))
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(invalid(format!("{} is not a folder", at.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                match std::fs::create_dir(&at) {
                    Ok(()) => made = true,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        if std::fs::symlink_metadata(&at)
                            .map(|m| !m.is_dir() || m.file_type().is_symlink())
                            .unwrap_or(true)
                        {
                            return Err(invalid(format!("{} is not a folder", at.display())));
                        }
                    }
                    Err(e) => return Err(format!("Create {}: {e}", at.display())),
                }
            }
            Err(e) => return Err(format!("Read {}: {e}", at.display())),
        }
    }
    Ok(Some((at, made)))
}

/// The store for `scope`, or `None` when nothing was ever saved there (and
/// `create` is false).
fn resolve(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    create: bool,
) -> Result<Option<Store>, String> {
    if let Some(key) = scope.session_key {
        check_session(key)?;
    }
    if let Some(id) = scope.workspace_id {
        let workspace = workspaces::summary(vault, id)?;
        let folder = workspaces::folder(vault, id)?;
        let Some((dir, fresh)) = real_dir(&folder, &[OUTPUTS, CANVASES], create)? else {
            return Ok(None);
        };
        return Ok(Some(Store {
            dir,
            private: false,
            vault: Some(vault.to_path_buf()),
            workspace: Some((workspace.id, workspace.path)),
            session_key: None,
            fresh,
        }));
    }
    let key = scope
        .session_key
        .ok_or_else(|| invalid("name a workspace or a conversation"))?;
    if create {
        std::fs::create_dir_all(app_data).map_err(|e| format!("Create app data: {e}"))?;
    } else if !app_data.is_dir() {
        return Ok(None);
    }
    let Some((dir, fresh)) = real_dir(app_data, &[CANVASES, key], create)? else {
        return Ok(None);
    };
    Ok(Some(Store {
        dir,
        private: true,
        vault: None,
        workspace: None,
        session_key: Some(key.to_string()),
        fresh,
    }))
}

impl Store {
    fn sidecar(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.canvas.json"))
    }

    fn visible(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.canvas.html"))
    }

    /// `outputs/canvases/<name>`, relative to the workspace folder.
    fn file(&self, name: &str) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|_| format!("{OUTPUTS}/{CANVASES}/{name}"))
    }

    fn vault_rel(&self, name: &str) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|(_, path)| format!("{path}/{OUTPUTS}/{CANVASES}/{name}"))
    }

    fn revisions_dir(&self, id: &str, create: bool) -> Result<Option<PathBuf>, String> {
        Ok(real_dir(&self.dir, &[REVISIONS, id], create)?.map(|(p, _)| p))
    }

    /// Fail closed if a file this canvas writes is a protected vault path.
    fn check_protected(&self, id: &str) -> Result<(), String> {
        let Some(vault) = &self.vault else {
            return Ok(());
        };
        for name in [format!("{id}.canvas.json"), format!("{id}.canvas.html")] {
            if let Some(rel) = self.vault_rel(&name) {
                if crate::run_guard::is_protected_target(vault, &rel) {
                    return Err(invalid(format!("{rel} is a protected path")));
                }
            }
        }
        Ok(())
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if self.private {
            crate::vault::atomic_write_private(path, bytes)
        } else {
            crate::vault::atomic_write(path, bytes)
        }
        .map_err(|e| format!("Save canvas: {e}"))
    }

    /// [`Store::write`] only while `path` still holds `seen` (`None`: no
    /// file), checked again just before the publish
    /// ([`crate::vault::replace_if_unchanged`], findings Q-R27 and Q-R28):
    /// a write that landed since is kept and the save is refused with
    /// [`CHANGED`], which callers reread and retry.
    fn write_if_unchanged(
        &self,
        path: &Path,
        seen: Option<&[u8]>,
        bytes: &[u8],
    ) -> Result<(), String> {
        match crate::vault::replace_if_unchanged(path, seen, bytes, self.private)
            .map_err(|e| format!("Save canvas: {e}"))?
        {
            crate::vault::Replace::Done => Ok(()),
            crate::vault::Replace::Changed(_) => Err(changed()),
        }
    }
}

/// The bytes of a regular file, `None` when absent. A symbolic link or a
/// folder at `path` is refused.
fn read_plain(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Read canvas: {e}")),
        Ok(m) if m.file_type().is_symlink() => {
            return Err(invalid(format!("{} is a symbolic link", path.display())))
        }
        Ok(m) if !m.is_file() => return Err(invalid(format!("{} is not a file", path.display()))),
        Ok(m) if m.len() > limit as u64 => {
            return Err(invalid(format!("{} is too large", path.display())))
        }
        Ok(_) => {}
    }
    std::fs::read(path)
        .map(Some)
        .map_err(|e| format!("Read canvas: {e}"))
}

fn parse(bytes: &[u8], id: &str) -> Result<CanvasFile, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("the canvas file is not JSON ({e})")))?;
    match value.get("hollowCanvas").map(Value::as_u64) {
        None => return Err(invalid("the file is not a Hollow canvas")),
        Some(Some(FORMAT)) => {}
        Some(_) => return Err(invalid("this canvas was written by a newer Hollow")),
    }
    let file: CanvasFile = serde_json::from_value(value)
        .map_err(|e| invalid(format!("the canvas file is unreadable ({e})")))?;
    if file.id != id {
        return Err(invalid(format!(
            "the canvas file names id {:?}, not {id:?}",
            file.id
        )));
    }
    let Some(last) = file.revisions.last() else {
        return Err(invalid("the canvas file has no revisions"));
    };
    if !valid_sha256_hex(&file.fence_sha256)
        || last.fence_sha256 != file.fence_sha256
        || last.revision != file.revision
        || file
            .revisions
            .iter()
            .any(|r| !valid_sha256_hex(&r.fence_sha256))
        || file
            .retired
            .iter()
            .any(|r| !valid_sha256_hex(&r.fence_sha256))
    {
        return Err(invalid("the canvas file's revisions don't match"));
    }
    check_state(&file.state)?;
    Ok(file)
}

fn read_sidecar(store: &Store, id: &str) -> Result<Option<(Vec<u8>, CanvasFile)>, String> {
    let Some(bytes) = read_plain(&store.sidecar(id), MAX_SIDECAR_BYTES)? else {
        return Ok(None);
    };
    let file = parse(&bytes, id)?;
    Ok(Some((bytes, file)))
}

/// The SHA-256 of whatever is at `path` (a link is never followed).
fn hash_existing(path: &Path) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Read canvas: {e}")),
        Ok(m) if !m.is_file() => Ok(Some(String::new())),
        Ok(_) => {
            let mut f = std::fs::File::open(path).map_err(|e| format!("Read canvas: {e}"))?;
            let mut h = Sha256::new();
            std::io::copy(&mut f, &mut h).map_err(|e| format!("Read canvas: {e}"))?;
            Ok(Some(hex(&h.finalize())))
        }
    }
}

/// The body of the revision whose fence hashes to `fence`: from the revision
/// store, or from the visible copy when it still holds exactly that body.
fn revision_html(store: &Store, id: &str, fence: &str) -> Result<String, String> {
    let limit = canvas_fence::MAX_BODY_BYTES;
    let mut candidates = Vec::new();
    if let Some(dir) = store.revisions_dir(id, false)? {
        candidates.push(dir.join(format!("{fence}.canvas.html")));
    }
    candidates.push(store.visible(id));
    for path in candidates {
        if let Ok(Some(bytes)) = read_plain(&path, limit) {
            if sha(&bytes) == fence {
                if let Ok(text) = String::from_utf8(bytes) {
                    return Ok(text);
                }
            }
        }
    }
    Err(invalid(
        "this revision's HTML is missing from the canvas store",
    ))
}

fn wire(store: &Store, bytes: &[u8], file: CanvasFile) -> Result<Canvas, String> {
    let html = revision_html(store, &file.id, &file.fence_sha256)?;
    let name = format!("{}.canvas.html", file.id);
    let html_changed =
        hash_existing(&store.visible(&file.id))?.as_deref() != Some(&file.fence_sha256);
    let current = file.revisions.last().cloned();
    Ok(Canvas {
        scope: if store.workspace.is_some() {
            "workspace"
        } else {
            "session"
        }
        .into(),
        workspace_id: store.workspace.as_ref().map(|(id, _)| id.clone()),
        session_key: store.session_key.clone(),
        path: store.vault_rel(&name),
        file: store.file(&name),
        id: file.id,
        title: file.title,
        kit: file.kit,
        session: file.session,
        fence_sha256: file.fence_sha256,
        created_at: file.created_at,
        updated_at: file.updated_at,
        revision: file.revision,
        ask: current.as_ref().is_some_and(|r| r.ask),
        height: current.and_then(|r| r.height),
        html,
        sha256: sha(bytes),
        html_changed,
        state: file.state,
        state_saved_at: file.state_saved_at,
        revisions: file.revisions,
    })
}

/// Pretty JSON within [`MAX_SIDECAR_BYTES`], oldest revisions dropped past
/// [`MAX_REVISIONS`] (and remembered in `retired`). Returns the bytes and the
/// dropped revisions' hashes.
fn encode(file: &mut CanvasFile) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut dropped = Vec::new();
    while file.revisions.len() > MAX_REVISIONS {
        let gone = file.revisions.remove(0);
        if !file
            .retired
            .iter()
            .any(|r| r.fence_sha256 == gone.fence_sha256)
        {
            file.retired.push(RetiredRevision {
                revision: gone.revision,
                fence_sha256: gone.fence_sha256.clone(),
            });
        }
        dropped.push(gone.fence_sha256);
    }
    if file.retired.len() > MAX_RETIRED {
        let excess = file.retired.len() - MAX_RETIRED;
        file.retired.drain(..excess);
    }
    let mut text = serde_json::to_vec_pretty(&*file).map_err(|e| e.to_string())?;
    text.push(b'\n');
    if text.len() > MAX_SIDECAR_BYTES {
        return Err(invalid(format!(
            "the canvas file would exceed {} MiB",
            MAX_SIDECAR_BYTES / 1024 / 1024
        )));
    }
    Ok((text, dropped))
}

/// Publish `bytes` at `path` only if nothing is there: a synced temp file
/// hard-linked into place, so a new sidecar never replaces a file, even one
/// another process creates at the same moment. `Ok(false)` when it exists.
fn publish_new(path: &Path, bytes: &[u8], private: bool) -> Result<bool, String> {
    let dir = path.parent().ok_or("a canvas file has a folder")?;
    let tmp = dir.join(format!(
        ".canvas-new.{}.{:x}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Save canvas: {e}"));
    }
    let result = match std::fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(format!("Save canvas: {e}")),
    };
    let _ = std::fs::remove_file(&tmp);
    if result == Ok(true) {
        crate::vault::flush_dir(dir);
    }
    result
}

/// Write `body` as revision `fence` in the store, unless it is already there.
fn put_revision(store: &Store, rev_dir: &Path, fence: &str, body: &[u8]) -> Result<(), String> {
    let path = rev_dir.join(format!("{fence}.canvas.html"));
    if hash_existing(&path)?.as_deref() == Some(fence) {
        return Ok(());
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(invalid(format!("{} is not a file", path.display())));
    }
    store.write(&path, body)
}

/// Rewrite the visible copy. If it holds anything but a kept revision (an
/// edit made outside Hollow), move it into the revision folder first.
fn put_visible(
    store: &Store,
    id: &str,
    rev_dir: &Path,
    known: &[&str],
    body: &[u8],
) -> Result<(), String> {
    let path = store.visible(id);
    // What the replace below expects to find: the bytes read here, or no
    // file once an outside edit was moved away.
    let mut seen: Option<Vec<u8>> = None;
    if let Ok(m) = std::fs::symlink_metadata(&path) {
        if m.is_dir() {
            return Err(invalid(format!("{} is a folder", path.display())));
        }
        let (keep, bytes) = if m.file_type().is_symlink() {
            (
                Some(format!("outside-link-{:x}", rand::random::<u64>())),
                None,
            )
        } else if !m.is_file() {
            (Some("outside-".to_string()), None)
        } else {
            let bytes = std::fs::read(&path).map_err(|e| format!("Read canvas: {e}"))?;
            let h = sha(&bytes);
            let keep = (!known.contains(&h.as_str())).then(|| format!("outside-{h}"));
            (keep, Some(bytes))
        };
        match keep {
            None => seen = bytes,
            Some(name) => {
                let to = rev_dir.join(format!("{name}.canvas.html"));
                match crate::vault::rename_no_replace(&path, &to) {
                    Ok(()) => {}
                    // The same bytes were kept before, so they may go.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => seen = bytes,
                    Err(e) => return Err(format!("Keep the edited canvas copy: {e}")),
                }
            }
        }
    }
    store.write_if_unchanged(&path, seen.as_deref(), body)
}

/// The tombstone of one pruned body: SHA-256 of `<sessionKey>\n<fenceSha256>`,
/// so the same body in another conversation is unaffected.
fn pruned_key(session_key: &str, fence: &str) -> String {
    sha(format!("{session_key}\n{fence}").as_bytes())
}

fn read_pruned(canvases: &Path) -> Result<Vec<String>, String> {
    let Some(bytes) = read_plain(&canvases.join(PRUNED), (MAX_PRUNED + 64) * 65)? else {
        return Ok(Vec::new());
    };
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| valid_sha256_hex(l))
        .map(str::to_string)
        .collect())
}

/// Whether `fence` was a body of the pruned conversation `session_key`.
fn was_pruned(app_data: &Path, session_key: &str, fence: &str) -> Result<bool, String> {
    let canvases = app_data.join(CANVASES);
    if !canvases.is_dir() {
        return Ok(false);
    }
    let key = pruned_key(session_key, fence);
    Ok(read_pruned(&canvases)?.contains(&key))
}

/// Every body a conversation folder holds (kept and retired revisions), as
/// tombstone keys.
fn tombstones(dir: &Path, session_key: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".canvas.json") else {
            continue;
        };
        let Ok(Some(bytes)) = read_plain(&entry.path(), MAX_SIDECAR_BYTES) else {
            continue;
        };
        let Ok(file) = parse(&bytes, id) else {
            continue;
        };
        let hashes = file
            .revisions
            .iter()
            .map(|r| &r.fence_sha256)
            .chain(file.retired.iter().map(|r| &r.fence_sha256));
        out.extend(hashes.map(|h| pruned_key(session_key, h)));
    }
    out
}

/// Keep the newest `keep` conversation folders in the app-data store (by
/// modification time); never `except`. The pruned bodies are remembered in
/// `.pruned` first (newest [`MAX_PRUNED`]), so re-rendering that
/// conversation later writes nothing; if that file can't be written, nothing
/// is pruned.
fn prune_sessions(canvases: &Path, keep: usize, except: &Path) {
    let Ok(entries) = std::fs::read_dir(canvases) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                && crate::agent_sessions::is_safe_session_key(&e.file_name().to_string_lossy())
        })
        .filter_map(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(|t| (t, e.path()))
        })
        .collect();
    if dirs.len() <= keep {
        return;
    }
    dirs.sort();
    let excess = dirs.len() - keep;
    let doomed: Vec<PathBuf> = dirs
        .into_iter()
        .map(|(_, p)| p)
        .filter(|p| p != except)
        .take(excess)
        .collect();
    let Ok(mut pruned) = read_pruned(canvases) else {
        return;
    };
    for dir in &doomed {
        let key = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for t in tombstones(dir, &key) {
            if !pruned.contains(&t) {
                pruned.push(t);
            }
        }
    }
    if pruned.len() > MAX_PRUNED {
        let excess = pruned.len() - MAX_PRUNED;
        pruned.drain(..excess);
    }
    let mut text = pruned.join("\n");
    text.push('\n');
    if crate::vault::atomic_write_private(&canvases.join(PRUNED), text.as_bytes()).is_err() {
        return;
    }
    for dir in doomed {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// One saved canvas, or `None` when it was never saved. Never writes.
pub fn read(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    id: &str,
) -> Result<Option<Canvas>, String> {
    check_id(id)?;
    let Some(store) = resolve(vault, app_data, scope, false)? else {
        return Ok(None);
    };
    let Some((bytes, file)) = read_sidecar(&store, id)? else {
        return Ok(None);
    };
    wire(&store, &bytes, file).map(Some)
}

/// One kept revision's fence body.
pub fn read_revision(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    id: &str,
    revision: u64,
) -> Result<CanvasRevisionHtml, String> {
    check_id(id)?;
    let missing = || "This canvas no longer exists.".to_string();
    let store = resolve(vault, app_data, scope, false)?.ok_or_else(missing)?;
    let (_, file) = read_sidecar(&store, id)?.ok_or_else(missing)?;
    let r = file
        .revisions
        .iter()
        .find(|r| r.revision == revision)
        .ok_or_else(|| format!("Revision {revision} of this canvas is no longer kept."))?;
    Ok(CanvasRevisionHtml {
        revision,
        fence_sha256: r.fence_sha256.clone(),
        title: r.title.clone(),
        html: revision_html(&store, id, &r.fence_sha256)?,
    })
}

/// Every saved canvas in a store, newest first.
pub fn list(vault: &Path, app_data: &Path, scope: Scope) -> Result<Vec<CanvasSummary>, String> {
    let Some(store) = resolve(vault, app_data, scope, false)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&store.dir).map_err(|e| format!("List canvases: {e}"))?;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".canvas.json") else {
            continue;
        };
        if name.starts_with('.') || RETIRED_CURATED.contains(&id) {
            continue;
        }
        let html = format!("{id}.canvas.html");
        let summary = match check_id(id).and_then(|_| read_sidecar(&store, id)) {
            Ok(Some((_, f))) => CanvasSummary {
                id: f.id,
                title: f.title,
                revision: f.revision,
                updated_at: Some(f.updated_at),
                fence_sha256: Some(f.fence_sha256),
                path: store.vault_rel(&html),
                file: store.file(&html),
                error: None,
            },
            Ok(None) => continue,
            Err(e) => CanvasSummary {
                id: id.into(),
                title: name.clone(),
                revision: 0,
                updated_at: None,
                fence_sha256: None,
                path: store.vault_rel(&name),
                file: store.file(&name),
                error: Some(e),
            },
        };
        out.push(summary);
        if out.len() >= MAX_LIST {
            break;
        }
    }
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

/// A workspace canvas file's id and role from its workspace-relative path
/// (`outputs/canvases/<id>.canvas.html` or `.canvas.json`, nothing nested),
/// for Results (Canvas C3 · S3). `None` for any other path.
pub(crate) fn workspace_canvas_file(rel: &str) -> Option<(&str, CanvasFileRole)> {
    let name = rel.strip_prefix(&format!("{OUTPUTS}/{CANVASES}/"))?;
    let (id, role) = if let Some(id) = name.strip_suffix(".canvas.html") {
        (id, CanvasFileRole::Visible)
    } else {
        (name.strip_suffix(".canvas.json")?, CanvasFileRole::Sidecar)
    };
    if RETIRED_CURATED.contains(&id) {
        return Some((id, CanvasFileRole::Curated));
    }
    crate::canvas_fence::valid_canvas_id(id).then_some((id, role))
}

/// Which of a canvas's two visible files a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CanvasFileRole {
    /// `<id>.canvas.html`: the current revision's body, the Result itself.
    Visible,
    /// `<id>.canvas.json`: the sidecar, never a Result of its own.
    Sidecar,
    /// Either file of a retired curated section view (`_plan.canvas.html` /
    /// `.canvas.json`, Canvas C6 · K1) left in an older workspace: not a
    /// deliverable, so never a Result and not Activity.
    Curated,
}

/// A saved canvas whose current revision is an ask-canvas (Canvas C4 A3):
/// what the "needs you" query lists while its answer isn't sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AskHead {
    pub id: String,
    pub title: String,
    pub revision: u64,
    /// The conversation that emitted the current revision.
    pub session: Option<String>,
    pub saved_at: String,
}

/// Whether JSON `bytes` hold `"ask"` followed by `: true` (any spacing):
/// cheap enough to skip parsing every plain canvas's sidecar.
fn mentions_ask_true(bytes: &[u8]) -> bool {
    let key = b"\"ask\"";
    let skip = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        i
    };
    (0..bytes.len().saturating_sub(key.len() - 1))
        .filter(|&i| bytes[i..].starts_with(key))
        .any(|i| {
            let colon = skip(i + key.len());
            bytes.get(colon) == Some(&b':') && bytes[skip(colon + 1)..].starts_with(b"true")
        })
}

/// What tells a sidecar's bytes apart without reading them: its size,
/// modified time and, on Unix, its inode (a save is a rename, so a new
/// inode). An edit in place that keeps all three is not seen until the
/// file changes again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    ino: u64,
}

impl FileStamp {
    pub(crate) fn of(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        let ino = std::os::unix::fs::MetadataExt::ino(meta);
        #[cfg(not(unix))]
        let ino = 0;
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            ino,
        }
    }
}

/// Each store folder's sidecars as last read, by file name (Canvas C6 P3).
/// Waiting on you lists every ask-canvas on each refresh; a sidecar whose
/// [`FileStamp`] hasn't changed is not read again. A folder's entry is
/// rebuilt from what each listing saw, so a deleted sidecar falls out.
#[derive(Default)]
pub(crate) struct AskCache {
    dirs: std::collections::HashMap<
        PathBuf,
        std::collections::HashMap<String, (FileStamp, Option<AskHead>)>,
    >,
    /// Sidecars read (not answered from the cache), for tests and the
    /// measurement.
    pub(crate) reads: usize,
}

static ASK_CACHE: std::sync::LazyLock<Mutex<AskCache>> =
    std::sync::LazyLock::new(|| Mutex::new(AskCache::default()));

/// The ask head of the sidecar at `path` (canvas `id`), read and parsed:
/// `None` when it is a link, too large, damaged, from a newer Hollow, or
/// its current revision isn't an ask. A sidecar with no `"ask": true`
/// anywhere is skipped before parsing.
fn read_ask_head(path: &Path, id: &str) -> Option<AskHead> {
    let bytes = read_plain(path, MAX_SIDECAR_BYTES).ok().flatten()?;
    if !mentions_ask_true(&bytes) {
        return None;
    }
    let file = parse(&bytes, id).ok()?;
    let head = file.revisions.last().filter(|r| r.ask)?;
    Some(AskHead {
        id: file.id.clone(),
        title: head.title.clone(),
        revision: head.revision,
        session: head.session.clone(),
        saved_at: head.saved_at.clone(),
    })
}

/// Newest saved first (an unreadable time last), ties by id.
fn newest_heads_first(heads: &mut [AskHead]) {
    let ms = |at: &str| {
        chrono::DateTime::parse_from_rfc3339(at.trim())
            .ok()
            .map(|t| t.timestamp_millis())
    };
    heads.sort_by_cached_key(|h| (std::cmp::Reverse(ms(&h.saved_at)), h.id.clone()));
}

/// Every canvas in the store folder `dir` whose current revision is an
/// ask-canvas, read without writing, the newest [`MAX_LIST`] (sorted before
/// the cut, newest first). `Ok(empty)` when the folder is missing; `Err`
/// when it is a symbolic link or can't be listed. A sidecar that is
/// damaged, a link or from a newer Hollow is skipped, as the chat renders
/// that canvas unsaved. Unchanged sidecars come from `cache`.
fn ask_heads_cached(dir: &Path, cache: &mut AskCache) -> Result<Vec<AskHead>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            cache.dirs.remove(dir);
            return Ok(Vec::new());
        }
        Err(e) => return Err(format!("List canvases: {e}")),
    };
    let mut known = cache.dirs.remove(dir).unwrap_or_default();
    let mut seen = std::collections::HashMap::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".canvas.json") else {
            continue;
        };
        if check_id(id).is_err() {
            continue;
        }
        // The entry's own metadata: a symbolic link is not followed.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let stamp = FileStamp::of(&meta);
        let head = match known.remove(&name) {
            Some((was, head)) if was == stamp => head,
            _ => {
                cache.reads += 1;
                read_ask_head(&dir.join(&name), id)
            }
        };
        seen.insert(name, (stamp, head));
    }
    let mut out: Vec<AskHead> = seen.values().filter_map(|(_, h)| h.clone()).collect();
    cache.dirs.insert(dir.to_path_buf(), seen);
    newest_heads_first(&mut out);
    out.truncate(MAX_LIST);
    Ok(out)
}

/// [`ask_heads_cached`] with the process-wide cache.
fn ask_heads_in(dir: &Path) -> Result<Vec<AskHead>, String> {
    let mut cache = ASK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    ask_heads_cached(dir, &mut cache)
}

/// The ask-canvases at their current revision in the workspace folder
/// `workspace` (`outputs/canvases/`, every folder on the way a real one).
pub(crate) fn workspace_ask_heads(workspace: &Path) -> Result<Vec<AskHead>, String> {
    match real_dir(workspace, &[OUTPUTS, CANVASES], false)? {
        Some((dir, _)) => ask_heads_in(&dir),
        None => Ok(Vec::new()),
    }
}

/// The ask-canvases at their current revision in every conversation's
/// app-data store (`<app data>/canvases/<sessionKey>/`), by conversation key.
pub(crate) fn app_data_ask_heads(app_data: &Path) -> Result<Vec<(String, Vec<AskHead>)>, String> {
    let Some((root, _)) = real_dir(app_data, &[CANVASES], false)? else {
        return Ok(Vec::new());
    };
    let entries = std::fs::read_dir(&root).map_err(|e| format!("List canvases: {e}"))?;
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let key = entry.file_name().to_string_lossy().to_string();
        if key.starts_with('.') || check_session(&key).is_err() {
            continue;
        }
        let Ok(Some((dir, _))) = real_dir(&root, &[key.as_str()], false) else {
            continue;
        };
        let heads = ask_heads_in(&dir)?;
        if !heads.is_empty() {
            out.push((key, heads));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The title and current revision of the canvas `id` in a workspace's
/// `outputs/canvases/` folder (`canvases`), read from its sidecar without
/// writing anything. `None` when the sidecar is missing, a symbolic link,
/// damaged or from a newer Hollow.
pub(crate) fn sidecar_title_and_revision(canvases: &Path, id: &str) -> Option<(String, u64)> {
    let bytes = read_plain(
        &canvases.join(format!("{id}.canvas.json")),
        MAX_SIDECAR_BYTES,
    )
    .ok()
    .flatten()?;
    let file = parse(&bytes, id).ok()?;
    Some((file.title, file.revision))
}

/// The kept revisions of canvas `id` in a workspace's `outputs/canvases/`
/// (oldest first: number, when it was saved and the conversation that
/// emitted it), read from its sidecar without writing anything, for a
/// workspace's Activity (Canvas C5b · Q3). `None` as for
/// [`sidecar_title_and_revision`].
pub(crate) fn sidecar_revisions(
    canvases: &Path,
    id: &str,
) -> Option<Vec<(u64, String, Option<String>)>> {
    let bytes = read_plain(
        &canvases.join(format!("{id}.canvas.json")),
        MAX_SIDECAR_BYTES,
    )
    .ok()
    .flatten()?;
    let file = parse(&bytes, id).ok()?;
    Some(
        file.revisions
            .into_iter()
            .map(|r| (r.revision, r.saved_at, r.session))
            .collect(),
    )
}

/// Save a finished fence body: create the canvas, add a revision, or (when
/// the body is a kept or retired revision) change nothing. `expected_sha256`
/// is the sidecar the caller last read (`None`: it saw no canvas); a
/// different one is refused with [`CHANGED`] unless the body is already
/// known. A body of an app-data conversation that was pruned is refused with
/// [`RETIRED`], also without writing.
#[allow(clippy::too_many_arguments)]
pub fn record(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    body: &str,
    turn_id: Option<&str>,
    kit: Option<&str>,
    expected_sha256: Option<&str>,
    now: &str,
) -> Result<CanvasRecorded, String> {
    let valid = canvas_fence::validate(body).map_err(|e| format!("{INVALID}: {e}"))?;
    check_turn(turn_id)?;
    check_kit(kit)?;
    let root = valid.root;
    let id = root.id.as_str();
    let fence = sha(body.as_bytes());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Look before creating anything: a body the store already knows (a
    // transcript re-rendered, however old) writes no file.
    let existing = resolve(vault, app_data, scope, false)?;
    let current = match &existing {
        Some(store) => read_sidecar(store, id)?,
        None => None,
    };
    if let (Some(store), Some((bytes, file))) = (&existing, &current) {
        let matched = if let Some(kept) = file.revisions.iter().find(|r| r.fence_sha256 == fence) {
            Some((kept.revision, "unchanged"))
        } else {
            file.retired
                .iter()
                .find(|r| r.fence_sha256 == fence)
                .map(|r| (r.revision, "retired"))
        };
        if let Some((matched_revision, outcome)) = matched {
            return Ok(CanvasRecorded {
                outcome: outcome.into(),
                matched_revision,
                canvas: wire(store, bytes, file.clone())?,
            });
        }
    }
    if let (None, Some(key)) = (scope.workspace_id, scope.session_key) {
        if was_pruned(app_data, key, &fence)? {
            return Err(retired());
        }
    }
    // A stale token is refused before any folder is created.
    let seen = current.as_ref().map(|(bytes, _)| sha(bytes));
    let seen_sidecar = current.as_ref().map(|(bytes, _)| bytes.clone());
    if expected_sha256 != seen.as_deref() {
        return Err(changed());
    }
    let store = match existing {
        Some(store) => store,
        None => resolve(vault, app_data, scope, true)?
            .ok_or_else(|| "the canvas store could not be created".to_string())?,
    };
    store.check_protected(id)?;
    let entry = CanvasRevision {
        revision: 1,
        fence_sha256: fence.clone(),
        title: root.title.clone(),
        saved_at: now.into(),
        ask: root.ask,
        height: root.height,
        session: scope.session_key.map(str::to_string),
        turn_id: turn_id.map(str::to_string),
        extra: Map::new(),
    };
    let (mut file, outcome) = match current {
        Some((_, file)) => (file, "revised"),
        None => (
            CanvasFile {
                hollow_canvas: FORMAT,
                id: id.into(),
                title: String::new(),
                kit: None,
                session: None,
                fence_sha256: String::new(),
                created_at: now.into(),
                updated_at: now.into(),
                revision: 0,
                state: Value::Null,
                state_saved_at: None,
                revisions: Vec::new(),
                retired: Vec::new(),
                extra: Map::new(),
            },
            "created",
        ),
    };
    file.revision += 1;
    file.revisions.push(CanvasRevision {
        revision: file.revision,
        ..entry
    });
    file.title = root.title;
    file.fence_sha256 = fence.clone();
    file.session = scope.session_key.map(str::to_string);
    if kit.is_some() {
        file.kit = kit.map(str::to_string);
    }
    file.updated_at = now.into();
    let (text, dropped) = encode(&mut file)?;
    let rev_dir = store
        .revisions_dir(id, true)?
        .ok_or_else(|| "the revision folder could not be created".to_string())?;
    put_revision(&store, &rev_dir, &fence, body.as_bytes())?;
    let known: Vec<&str> = file
        .revisions
        .iter()
        .map(|r| r.fence_sha256.as_str())
        .chain(dropped.iter().map(String::as_str))
        .collect();
    put_visible(&store, id, &rev_dir, &known, body.as_bytes())?;
    if outcome == "created" {
        if !publish_new(&store.sidecar(id), &text, store.private)? {
            return Err(changed());
        }
    } else {
        store.write_if_unchanged(&store.sidecar(id), seen_sidecar.as_deref(), &text)?;
    }
    for gone in &dropped {
        if !file.revisions.iter().any(|r| &r.fence_sha256 == gone) {
            let _ = std::fs::remove_file(rev_dir.join(format!("{gone}.canvas.html")));
        }
    }
    if store.fresh && store.private {
        if let Some(parent) = store.dir.parent() {
            prune_sessions(parent, MAX_APP_SESSIONS, &store.dir);
        }
    }
    let matched_revision = file.revision;
    Ok(CanvasRecorded {
        outcome: outcome.into(),
        matched_revision,
        canvas: wire(&store, &text, file)?,
    })
}

/// Save a finished turn's canvases (question cards, 2026-10-14). Until then
/// only a window watching the reply stream saved its fences, so a canvas
/// from a turn nobody watched (an unattended check-in, a background chat)
/// was never saved: its ask-canvas could never be answered ("Hollow didn't
/// take the answers"; `canvas-bridge-refusals.jsonl` showed `not-current`
/// on every such check-in). `bodies` are [`canvas_fence::reply_canvas_bodies`].
///
/// The k-th fence of an id in the reply is left alone when the store holds
/// at least k revisions of it from this turn that this call didn't make: a
/// watching window saved them, maybe after its stray-tag repair changed the
/// bytes (saving the unrepaired bytes too would make the window's own
/// revision look stale). A window that left mid-reply saved only the first
/// versions, so the later ones are still saved here. Invalid fences are skipped; one
/// canvas that can't be saved doesn't stop the others. Returns how many
/// fences were saved or already known.
pub fn record_reply(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    bodies: &[String],
    turn_id: &str,
    now: &str,
) -> usize {
    // Per id: fences of it seen in this reply, and revisions this call saved.
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut ours: std::collections::HashMap<String, usize> = Default::default();
    let mut saved = 0;
    for body in bodies {
        let Ok(valid) = canvas_fence::validate(body) else {
            continue;
        };
        let id = valid.root.id.clone();
        let k = {
            let n = seen.entry(id.clone()).or_insert(0);
            *n += 1;
            *n
        };
        let mine = ours.get(&id).copied().unwrap_or(0);
        let read = || -> Option<(String, usize)> {
            let store = resolve(vault, app_data, scope, false).ok()??;
            let (bytes, file) = read_sidecar(&store, &id).ok()??;
            let this_turn = file
                .revisions
                .iter()
                .filter(|r| r.turn_id.as_deref() == Some(turn_id))
                .count();
            Some((sha(&bytes), this_turn))
        };
        let attempt = || {
            let current = read();
            if current
                .as_ref()
                .is_some_and(|(_, this_turn)| this_turn.saturating_sub(mine) >= k)
            {
                return Ok(None);
            }
            record(
                vault,
                app_data,
                scope,
                body,
                Some(turn_id),
                Some("kit@1"),
                current.as_ref().map(|(token, _)| token.as_str()),
                now,
            )
            .map(Some)
        };
        let result = match attempt() {
            Err(e) if e.starts_with(CHANGED) => attempt(),
            other => other,
        };
        match result {
            Ok(Some(rec)) => {
                saved += 1;
                if rec.outcome != "unchanged" && rec.outcome != "retired" {
                    *ours.entry(id).or_insert(0) += 1;
                }
            }
            Ok(None) => saved += 1,
            Err(_) => {}
        }
    }
    saved
}

/// Save the canvas's UI state if the sidecar is still the version the caller
/// read; otherwise refuse with [`CHANGED`] and write nothing. `null` clears it.
pub fn save_state(
    vault: &Path,
    app_data: &Path,
    scope: Scope,
    id: &str,
    state: Value,
    expected_sha256: &str,
    now: &str,
) -> Result<CanvasStateSaved, String> {
    check_id(id)?;
    check_state(&state)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let missing = || "This canvas no longer exists.".to_string();
    let store = resolve(vault, app_data, scope, false)?.ok_or_else(missing)?;
    store.check_protected(id)?;
    let (bytes, mut file) = read_sidecar(&store, id)?.ok_or_else(missing)?;
    let current = sha(&bytes);
    if current != expected_sha256 {
        return Err(changed());
    }
    if file.state == state {
        return Ok(CanvasStateSaved {
            sha256: current,
            state_saved_at: file.state_saved_at,
            revision: file.revision,
        });
    }
    file.state = state;
    file.state_saved_at = Some(now.into());
    let (text, _) = encode(&mut file)?;
    store.write_if_unchanged(&store.sidecar(id), Some(bytes.as_slice()), &text)?;
    Ok(CanvasStateSaved {
        sha256: sha(&text),
        state_saved_at: file.state_saved_at,
        revision: file.revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T0: &str = "2026-10-04T10:00:00+00:00";
    const T1: &str = "2026-10-04T10:01:00+00:00";
    const T2: &str = "2026-10-04T10:02:00+00:00";
    const S1: &str = "s-alpha-1";
    const S2: &str = "s-beta-2";

    fn body(id: &str, title: &str, inner: &str) -> String {
        format!("<hc-canvas id=\"{id}\" title=\"{title}\">\n{inner}\n</hc-canvas>\n")
    }

    struct Env {
        vault: tempfile::TempDir,
        app: tempfile::TempDir,
        a: workspaces::Workspace,
        b: workspaces::Workspace,
    }

    fn env() -> Env {
        let vault = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        let a = workspaces::create(vault.path(), "Alpha", "One").unwrap();
        let b = workspaces::create(vault.path(), "Beta", "Two").unwrap();
        Env { vault, app, a, b }
    }

    impl Env {
        fn ws(&self) -> Scope<'_> {
            Scope {
                workspace_id: Some(&self.a.id),
                session_key: Some(S1),
            }
        }
        fn record(
            &self,
            scope: Scope,
            b: &str,
            expected: Option<&str>,
            now: &str,
        ) -> Result<CanvasRecorded, String> {
            record(
                self.vault.path(),
                self.app.path(),
                scope,
                b,
                Some("turn-1"),
                None,
                expected,
                now,
            )
        }
        fn read(&self, scope: Scope, id: &str) -> Option<Canvas> {
            read(self.vault.path(), self.app.path(), scope, id).unwrap()
        }
        fn save_state(
            &self,
            scope: Scope,
            id: &str,
            state: Value,
            expected: &str,
        ) -> Result<CanvasStateSaved, String> {
            save_state(
                self.vault.path(),
                self.app.path(),
                scope,
                id,
                state,
                expected,
                T1,
            )
        }
        fn dir(&self) -> PathBuf {
            workspaces::folder(self.vault.path(), &self.a.id)
                .unwrap()
                .join("outputs/canvases")
        }
    }

    /// Question cards, 2026-10-14: the 08:00 check-in's ask-canvas was
    /// never saved because no window watched its turn, so Send was refused
    /// (`not-current`) and Alex had to copy his answers by hand.
    #[test]
    fn a_finished_turn_saves_its_canvases_whether_or_not_a_window_watched() {
        let e = env();
        let (v, a) = (e.vault.path(), e.app.path());
        let ask = "<hc-canvas id=\"checkin\" title=\"Check-in\" ask>\n<hc-ask>\n<hc-choice name=\"a\" label=\"Install?\">\n* Do it\nNot now\n</hc-choice>\n</hc-ask>\n</hc-canvas>";
        let reply = format!(
            "Two choices.\n\n```hollow-canvas\n{ask}\n```\n\nMore.\n```hollow-canvas\n<hc-canvas id=\"cut\" title=\"Cut\">\n"
        );
        let bodies = crate::canvas_fence::reply_canvas_bodies(&reply);
        assert_eq!(
            bodies,
            vec![ask.to_string()],
            "an unclosed fence is not saved"
        );

        // Nobody watched: the turn's end saves it under its turn.
        assert_eq!(record_reply(v, a, e.ws(), &bodies, "run-1", T0), 1);
        let c = e.read(e.ws(), "checkin").unwrap();
        assert_eq!((c.revision, c.html.as_str(), c.ask), (1, ask, true));
        assert_eq!(c.revisions[0].turn_id.as_deref(), Some("run-1"));
        // The same reply again changes nothing.
        record_reply(v, a, e.ws(), &bodies, "run-1", T1);
        assert_eq!(e.read(e.ws(), "checkin").unwrap().revision, 1);

        // A watching window saved its repaired bytes for run-2 first: the
        // as-written fence is left alone, so the window's revision stays current.
        let repaired = ask.replace("Not now", "Not now (repaired)");
        let token = e.read(e.ws(), "checkin").unwrap().sha256;
        record(
            v,
            a,
            e.ws(),
            &repaired,
            Some("run-2"),
            None,
            Some(&token),
            T1,
        )
        .unwrap();
        let written = ask.replace("Not now", "Not now <raw>");
        record_reply(v, a, e.ws(), &[written], "run-2", T2);
        let c = e.read(e.ws(), "checkin").unwrap();
        assert_eq!((c.revision, c.html.as_str()), (2, repaired.as_str()));

        // A window saved run-5's first version and then left: the second is
        // still saved, the first left as the window wrote it.
        let (w1, w2) = (
            ask.replace("Do it", "Do it 5a"),
            ask.replace("Do it", "Do it 5b"),
        );
        let token = e.read(e.ws(), "checkin").unwrap().sha256;
        record(v, a, e.ws(), &w1, Some("run-5"), None, Some(&token), T2).unwrap();
        record_reply(
            v,
            a,
            e.ws(),
            &[w1.replace("5a", "5a <raw>"), w2.clone()],
            "run-5",
            T2,
        );
        let c = e.read(e.ws(), "checkin").unwrap();
        assert_eq!((c.revision, c.html.as_str()), (4, w2.as_str()));

        // Two versions of one canvas in one unwatched reply: both kept, the later current.
        let (v3, v4) = (
            ask.replace("Do it", "Do it 3"),
            ask.replace("Do it", "Do it 4"),
        );
        assert_eq!(
            record_reply(v, a, e.ws(), &[v3, v4.clone()], "run-3", T2),
            2
        );
        let c = e.read(e.ws(), "checkin").unwrap();
        assert_eq!((c.revision, c.html.as_str()), (6, v4.as_str()));

        // A conversation outside any workspace saves to its own store; an
        // invalid fence is skipped without stopping the rest.
        let own = Scope {
            workspace_id: None,
            session_key: Some(S2),
        };
        let fine = body("solo", "Solo", "<p>ok</p>");
        assert_eq!(
            record_reply(v, a, own, &["<p>no root</p>".into(), fine], "run-4", T0),
            1
        );
        assert_eq!(e.read(own, "solo").unwrap().revision, 1);
    }

    #[test]
    fn a_canvas_is_saved_in_the_workspace_with_its_sidecar_and_revision_store() {
        let e = env();
        let v1 = body("release-status", "Release status", "<p>29 checks</p>");
        let saved = e.record(e.ws(), &v1, None, T0).unwrap();
        assert_eq!(saved.outcome, "created");
        assert_eq!(saved.matched_revision, 1);
        let c = saved.canvas;
        assert_eq!(c.scope, "workspace");
        assert_eq!(
            c.file.as_deref(),
            Some("outputs/canvases/release-status.canvas.html")
        );
        assert_eq!(
            c.path.as_deref(),
            Some(format!("{}/outputs/canvases/release-status.canvas.html", e.a.path).as_str())
        );
        assert_eq!(c.html, v1, "the fence body exactly as written");
        assert_eq!(c.fence_sha256, sha(v1.as_bytes()));
        assert_eq!(c.title, "Release status");
        assert_eq!(c.session.as_deref(), Some(S1));
        assert!(!c.html_changed);
        let dir = e.dir();
        assert_eq!(
            std::fs::read_to_string(dir.join("release-status.canvas.html")).unwrap(),
            v1
        );
        let sidecar = std::fs::read(dir.join("release-status.canvas.json")).unwrap();
        assert_eq!(c.sha256, sha(&sidecar));
        let json: Value = serde_json::from_slice(&sidecar).unwrap();
        assert_eq!(json["hollowCanvas"], 1);
        assert_eq!(json["id"], "release-status");
        assert_eq!(json["revisions"][0]["turnId"], "turn-1");
        let text = String::from_utf8(sidecar).unwrap();
        assert!(
            text.find("\"title\"").unwrap() < 120,
            "title near the top, for Results' head read"
        );
        assert!(dir
            .join(format!(
                ".revisions/release-status/{}.canvas.html",
                c.fence_sha256
            ))
            .is_file());
        // outputs/canvases holds exactly the two visible files.
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|d| d.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["release-status.canvas.html", "release-status.canvas.json"]
        );
        // Read back, and listed.
        assert_eq!(e.read(e.ws(), "release-status").unwrap(), c);
        let listed = list(e.vault.path(), e.app.path(), e.ws()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Release status");
        // Another workspace has none, and reading wrote nothing there.
        let other = Scope {
            workspace_id: Some(&e.b.id),
            session_key: None,
        };
        assert!(e.read(other, "release-status").is_none());
        assert!(!workspaces::folder(e.vault.path(), &e.b.id)
            .unwrap()
            .join("outputs/canvases")
            .exists());
    }

    #[test]
    fn the_same_body_is_a_no_op_and_a_new_body_is_a_revision() {
        let e = env();
        let v1 = body("plan", "Plan", "<p>one</p>");
        let v2 = body("plan", "Plan, revised", "<p>two</p>");
        let first = e.record(e.ws(), &v1, None, T0).unwrap().canvas;
        // Same body: nothing written, whatever token the caller holds.
        let again = e.record(e.ws(), &v1, None, T1).unwrap();
        assert_eq!(again.outcome, "unchanged");
        assert_eq!(again.canvas.sha256, first.sha256);
        let second = e.record(e.ws(), &v2, Some(&first.sha256), T1).unwrap();
        assert_eq!(second.outcome, "revised");
        assert_eq!(second.matched_revision, 2);
        let c = second.canvas;
        assert_eq!(c.revision, 2);
        assert_eq!(c.title, "Plan, revised");
        assert_eq!(c.html, v2);
        assert_eq!(c.revisions.len(), 2);
        assert_eq!(c.revisions[0].fence_sha256, sha(v1.as_bytes()));
        assert_eq!(c.created_at, T0);
        assert_eq!(c.updated_at, T1);
        // Re-rendering the earlier fence names it, and adds nothing.
        let earlier = e.record(e.ws(), &v1, Some(&c.sha256), T2).unwrap();
        assert_eq!(earlier.outcome, "unchanged");
        assert_eq!(earlier.matched_revision, 1);
        assert_eq!(earlier.canvas.revision, 2);
        assert_eq!(
            read_revision(e.vault.path(), e.app.path(), e.ws(), "plan", 1)
                .unwrap()
                .html,
            v1
        );
        assert!(read_revision(e.vault.path(), e.app.path(), e.ws(), "plan", 9).is_err());
    }

    #[test]
    fn stale_saves_are_refused_and_nothing_is_overwritten() {
        let e = env();
        let v1 = body("plan", "Plan", "<p>one</p>");
        let first = e.record(e.ws(), &v1, None, T0).unwrap().canvas;
        // Two creators: the second saw no canvas, but one exists now.
        let err = e
            .record(e.ws(), &body("plan", "Plan", "<p>other</p>"), None, T1)
            .unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        let v2 = body("plan", "Plan", "<p>two</p>");
        let second = e
            .record(e.ws(), &v2, Some(&first.sha256), T1)
            .unwrap()
            .canvas;
        // A third save from the first version's token is stale.
        let err = e
            .record(
                e.ws(),
                &body("plan", "Plan", "<p>three</p>"),
                Some(&first.sha256),
                T2,
            )
            .unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        let err = e
            .save_state(e.ws(), "plan", json!({"tab": 2}), &first.sha256)
            .unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        let now = e.read(e.ws(), "plan").unwrap();
        assert_eq!(now.sha256, second.sha256);
        assert_eq!(now.html, v2);
        assert!(now.state.is_null());
        // An outside write to the sidecar is a change too.
        let path = e.dir().join("plan.canvas.json");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push('\n');
        std::fs::write(&path, &text).unwrap();
        assert!(e
            .save_state(e.ws(), "plan", json!(1), &second.sha256)
            .unwrap_err()
            .starts_with(CHANGED));
        // A token for a canvas that is gone.
        assert!(e
            .record(e.ws(), &body("gone", "Gone", ""), Some(&second.sha256), T2)
            .unwrap_err()
            .starts_with(CHANGED));
        // ... refused before any folder is created.
        let other = Scope {
            workspace_id: Some(&e.b.id),
            session_key: None,
        };
        assert!(e
            .record(other, &body("gone", "Gone", ""), Some(&second.sha256), T2)
            .unwrap_err()
            .starts_with(CHANGED));
        assert!(!workspaces::folder(e.vault.path(), &e.b.id)
            .unwrap()
            .join("outputs/canvases")
            .exists());
    }

    #[test]
    fn concurrent_saves_from_one_token_let_exactly_one_win() {
        let e = std::sync::Arc::new(env());
        let base = e
            .record(e.ws(), &body("race", "Race", "<p>0</p>"), None, T0)
            .unwrap()
            .canvas
            .sha256;
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let e = e.clone();
                let base = base.clone();
                std::thread::spawn(move || {
                    if i % 2 == 0 {
                        e.record(
                            e.ws(),
                            &body("race", "Race", &format!("<p>new {i}</p>")),
                            Some(&base),
                            T1,
                        )
                        .map(|_| ())
                    } else {
                        e.save_state(e.ws(), "race", json!({ "i": i }), &base)
                            .map(|_| ())
                    }
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert!(results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e.starts_with(CHANGED)));
        let c = e.read(e.ws(), "race").unwrap();
        assert_eq!(c.revisions.len() + usize::from(!c.state.is_null()), 2);
    }

    #[test]
    fn revisions_are_capped_and_the_oldest_pruned_from_the_store() {
        let e = env();
        let mut token: Option<String> = None;
        let mut bodies = Vec::new();
        for i in 0..(MAX_REVISIONS + 5) {
            let b = body("long", "Long", &format!("<p>{i}</p>"));
            token = Some(
                e.record(e.ws(), &b, token.as_deref(), T0)
                    .unwrap()
                    .canvas
                    .sha256,
            );
            bodies.push(b);
        }
        let c = e.read(e.ws(), "long").unwrap();
        assert_eq!(c.revisions.len(), MAX_REVISIONS);
        assert_eq!(c.revision, (MAX_REVISIONS + 5) as u64);
        assert_eq!(c.revisions[0].revision, 6, "the newest are kept");
        assert_eq!(c.html, *bodies.last().unwrap());
        let store = e.dir().join(".revisions/long");
        let files = std::fs::read_dir(&store).unwrap().count();
        assert_eq!(files, MAX_REVISIONS, "pruned revisions' HTML is deleted");
        assert!(!store
            .join(format!("{}.canvas.html", sha(bodies[0].as_bytes())))
            .exists());
        assert_eq!(
            read_revision(e.vault.path(), e.app.path(), e.ws(), "long", 6)
                .unwrap()
                .html,
            bodies[5]
        );
        // The pruned five are remembered by number and hash only.
        let json: Value =
            serde_json::from_slice(&std::fs::read(e.dir().join("long.canvas.json")).unwrap())
                .unwrap();
        let retired = json["retired"].as_array().unwrap();
        assert_eq!(retired.len(), 5);
        assert_eq!(retired[0]["revision"], 1);
        assert_eq!(retired[0]["fenceSha256"], sha(bodies[0].as_bytes()));
        assert_eq!(retired[4]["revision"], 5);
        // The retired list is capped too, oldest dropped first.
        let (_, mut file) = {
            let store = resolve(e.vault.path(), e.app.path(), e.ws(), false)
                .unwrap()
                .unwrap();
            read_sidecar(&store, "long").unwrap().unwrap()
        };
        file.retired = (0..MAX_RETIRED as u64)
            .map(|n| RetiredRevision {
                revision: n,
                fence_sha256: format!("{n:064x}"),
            })
            .collect();
        let first_kept = file.revisions[0].fence_sha256.clone();
        file.revisions.push(CanvasRevision {
            revision: 99,
            ..file.revisions[0].clone()
        });
        encode(&mut file).unwrap();
        assert_eq!(file.retired.len(), MAX_RETIRED);
        assert_eq!(file.retired[0].revision, 1);
        assert_eq!(file.retired.last().unwrap().fence_sha256, first_kept);
    }

    /// Every file under `dir`, with its bytes (dot folders included).
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push((path.clone(), std::fs::read(&path).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn re_recording_an_aged_out_revision_writes_nothing() {
        // Review of S2: scrolling back through a long chat re-renders fences
        // whose revisions have aged out of the kept 30. That must not roll
        // the canvas back to the old body or write any file.
        let e = env();
        let mut token: Option<String> = None;
        let mut bodies = Vec::new();
        for i in 0..(MAX_REVISIONS + 2) {
            let b = body("long", "Long", &format!("<p>{i}</p>"));
            token = Some(
                e.record(e.ws(), &b, token.as_deref(), T0)
                    .unwrap()
                    .canvas
                    .sha256,
            );
            bodies.push(b);
        }
        let before = snapshot(&e.dir());
        let current = e.read(e.ws(), "long").unwrap();
        for (n, old) in bodies.iter().take(2).enumerate() {
            // Whatever token the renderer holds (the current one, none).
            for expected in [token.as_deref(), None] {
                let again = e.record(e.ws(), old, expected, T2);
                assert_eq!(
                    snapshot(&e.dir()),
                    before,
                    "body {n}, token {expected:?}: no file was written ({again:?})"
                );
                let again = again.unwrap();
                assert_eq!(again.outcome, "retired", "body {n}");
                assert_eq!(again.matched_revision, n as u64 + 1);
                assert_eq!(again.canvas, current, "the current canvas, untouched");
            }
        }
        assert_eq!(snapshot(&e.dir()), before, "no file was written");
        // A genuinely new body still revises.
        let next = body("long", "Long", "<p>new</p>");
        let r = e.record(e.ws(), &next, token.as_deref(), T2).unwrap();
        assert_eq!(r.outcome, "revised");
        assert_eq!(r.canvas.revision, MAX_REVISIONS as u64 + 3);
        // ... and the bodies that aged out before it still write nothing.
        let before = snapshot(&e.dir());
        assert_eq!(
            e.record(e.ws(), &bodies[2], None, T2).unwrap().outcome,
            "retired"
        );
        assert_eq!(snapshot(&e.dir()), before);
    }

    #[test]
    fn re_recording_into_a_pruned_conversation_writes_nothing() {
        // Review of S2: a conversation whose app-data folder was pruned must
        // not be recreated by re-rendering its old fences.
        let e = env();
        let s1 = Scope {
            workspace_id: None,
            session_key: Some(S1),
        };
        let s2 = Scope {
            workspace_id: None,
            session_key: Some(S2),
        };
        let old = body("scratch", "Scratch", "<p>a</p>");
        let first = e.record(s1, &old, None, T0).unwrap().canvas;
        e.record(
            s1,
            &body("scratch", "Scratch", "<p>b</p>"),
            Some(&first.sha256),
            T1,
        )
        .unwrap();
        e.record(s2, &body("other", "Other", "<p>x</p>"), None, T1)
            .unwrap();
        let canvases = e.app.path().join(CANVASES);
        std::fs::File::open(canvases.join(S1))
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();
        prune_sessions(&canvases, 1, &canvases.join(S2));
        assert!(
            !canvases.join(S1).exists(),
            "the older conversation was pruned"
        );
        let before = snapshot(&canvases);
        for b in [old.clone(), body("scratch", "Scratch", "<p>b</p>")] {
            let err = e.record(s1, &b, None, T2).unwrap_err();
            assert!(err.starts_with(RETIRED), "{err}");
        }
        assert!(!canvases.join(S1).exists(), "nothing was recreated");
        assert_eq!(snapshot(&canvases), before, "no file was written");
        // A new canvas in that conversation (a live turn) is saved again.
        let fresh = e
            .record(s1, &body("scratch", "Scratch", "<p>new</p>"), None, T2)
            .unwrap();
        assert_eq!(fresh.outcome, "created");
        // The same body in another conversation is not affected.
        assert_eq!(e.record(s2, &old, None, T2).unwrap().outcome, "created");
    }

    #[test]
    fn state_is_capped_saved_with_cas_and_carried_to_new_revisions() {
        let e = env();
        let c = e
            .record(e.ws(), &body("calc", "Calc", "<input>"), None, T0)
            .unwrap()
            .canvas;
        // The cap is on compact JSON bytes, as the bridge measures it.
        let at_cap = json!("x".repeat(MAX_STATE_BYTES - 2));
        assert_eq!(serde_json::to_vec(&at_cap).unwrap().len(), MAX_STATE_BYTES);
        let over = json!("é".repeat(MAX_STATE_BYTES / 2));
        let err = e.save_state(e.ws(), "calc", over, &c.sha256).unwrap_err();
        assert!(err.starts_with(INVALID), "{err}");
        let saved = e
            .save_state(e.ws(), "calc", at_cap.clone(), &c.sha256)
            .unwrap();
        assert_eq!(saved.state_saved_at.as_deref(), Some(T1));
        let small = json!({ "slider": 40, "tab": "costs" });
        let saved = e
            .save_state(e.ws(), "calc", small.clone(), &saved.sha256)
            .unwrap();
        // Unchanged state writes nothing.
        assert_eq!(
            e.save_state(e.ws(), "calc", small.clone(), &saved.sha256)
                .unwrap(),
            saved
        );
        let revised = e
            .record(
                e.ws(),
                &body("calc", "Calc", "<input><output>"),
                Some(&saved.sha256),
                T2,
            )
            .unwrap()
            .canvas;
        assert_eq!(revised.revision, 2);
        assert_eq!(
            revised.state, small,
            "state carries over to the new revision"
        );
        // Null clears it.
        let cleared = e
            .save_state(e.ws(), "calc", Value::Null, &revised.sha256)
            .unwrap();
        assert!(e.read(e.ws(), "calc").unwrap().state.is_null());
        assert_ne!(cleared.sha256, revised.sha256);
        // State for a canvas that was never saved.
        assert!(e
            .save_state(e.ws(), "nope", json!(1), &cleared.sha256)
            .unwrap_err()
            .contains("no longer exists"));
    }

    #[test]
    fn invalid_fences_ids_and_inputs_are_refused_before_anything_is_written() {
        let e = env();
        for bad in [
            String::new(),
            "<p>no root</p>".to_string(),
            body("Bad Id", "x", ""),
            body("ok", "", ""),
            "<hc-canvas id=\"open\" title=\"Open\">".to_string(),
            format!(
                "<hc-canvas id=\"big\" title=\"Big\">{}</hc-canvas>",
                "x".repeat(canvas_fence::MAX_BODY_BYTES)
            ),
            body("ok", "Two", "</hc-canvas><hc-canvas id=\"b\" title=\"b\">"),
        ] {
            let err = e.record(e.ws(), &bad, None, T0).unwrap_err();
            assert!(err.starts_with(INVALID), "{bad:.60}: {err}");
        }
        let good = body("ok", "Ok", "");
        for (turn, kit) in [
            (Some(""), None),
            (Some("t\u{7}"), None),
            (None, Some("Kit 1")),
            (None, Some("")),
        ] {
            let err = record(
                e.vault.path(),
                e.app.path(),
                e.ws(),
                &good,
                turn,
                kit,
                None,
                T0,
            )
            .unwrap_err();
            assert!(err.starts_with(INVALID), "{turn:?} {kit:?}: {err}");
        }
        for id in ["../x", "Plan", "", &"x".repeat(65), "a/b", ".hidden"] {
            assert!(read(e.vault.path(), e.app.path(), e.ws(), id)
                .unwrap_err()
                .starts_with(INVALID));
            assert!(e
                .save_state(e.ws(), id, json!(1), "x")
                .unwrap_err()
                .starts_with(INVALID));
        }
        // Path traversal through the scope.
        for ws in ["../etc", "..", "a/b"] {
            let scope = Scope {
                workspace_id: Some(ws),
                session_key: None,
            };
            assert!(e.record(scope, &good, None, T0).is_err(), "{ws}");
        }
        for key in ["../x", "S-UPPER", "nope", "s-a/b", ""] {
            let scope = Scope {
                workspace_id: None,
                session_key: Some(key),
            };
            assert!(
                e.record(scope, &good, None, T0)
                    .unwrap_err()
                    .starts_with(INVALID),
                "{key}"
            );
        }
        assert!(e
            .record(Scope::default(), &good, None, T0)
            .unwrap_err()
            .starts_with(INVALID));
        assert!(!e.dir().exists(), "nothing was created");
        assert!(!e.app.path().join(CANVASES).exists());
        let k = record(
            e.vault.path(),
            e.app.path(),
            e.ws(),
            &good,
            None,
            Some("kit@1"),
            None,
            T0,
        )
        .unwrap();
        assert_eq!(k.canvas.kit.as_deref(), Some("kit@1"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folders_and_files_are_refused_and_markdown_is_never_written() {
        use std::os::unix::fs::symlink;
        let e = env();
        let ws = workspaces::folder(e.vault.path(), &e.a.id).unwrap();
        let brief = std::fs::read(ws.join("WORKSPACE.md")).unwrap();
        let good = body("workspace", "W", "");
        // outputs/canvases → the workspace folder (where WORKSPACE.md lives).
        std::fs::create_dir_all(ws.join("outputs")).unwrap();
        symlink(&ws, ws.join("outputs/canvases")).unwrap();
        assert!(e
            .record(e.ws(), &good, None, T0)
            .unwrap_err()
            .starts_with(INVALID));
        std::fs::remove_file(ws.join("outputs/canvases")).unwrap();
        // outputs/canvases → me/ (a protected folder).
        let me = e.vault.path().join("me");
        std::fs::create_dir_all(&me).unwrap();
        symlink(&me, ws.join("outputs/canvases")).unwrap();
        assert!(e
            .record(e.ws(), &good, None, T0)
            .unwrap_err()
            .starts_with(INVALID));
        assert_eq!(std::fs::read_dir(&me).unwrap().count(), 0);
        std::fs::remove_file(ws.join("outputs/canvases")).unwrap();
        // A symlinked sidecar, and a symlinked revision folder.
        std::fs::create_dir_all(ws.join("outputs/canvases")).unwrap();
        symlink(
            ws.join("WORKSPACE.md"),
            ws.join("outputs/canvases/workspace.canvas.json"),
        )
        .unwrap();
        assert!(e
            .record(e.ws(), &good, None, T0)
            .unwrap_err()
            .starts_with(INVALID));
        assert!(read(e.vault.path(), e.app.path(), e.ws(), "workspace").is_err());
        std::fs::remove_file(ws.join("outputs/canvases/workspace.canvas.json")).unwrap();
        symlink(&ws, ws.join("outputs/canvases/.revisions")).unwrap();
        assert!(e
            .record(e.ws(), &good, None, T0)
            .unwrap_err()
            .starts_with(INVALID));
        std::fs::remove_file(ws.join("outputs/canvases/.revisions")).unwrap();
        assert_eq!(std::fs::read(ws.join("WORKSPACE.md")).unwrap(), brief);
        for name in ["PLAN.md", "SOURCES.md"] {
            let before = std::fs::read(ws.join(name)).ok();
            e.record(e.ws(), &good, None, T0).ok();
            assert_eq!(std::fs::read(ws.join(name)).ok(), before, "{name}");
        }
        // With real folders it saves, and only under outputs/canvases.
        assert!(e.read(e.ws(), "workspace").is_some());
        assert_eq!(std::fs::read(ws.join("WORKSPACE.md")).unwrap(), brief);
    }

    #[test]
    fn protected_targets_are_refused() {
        let e = env();
        let store = resolve(e.vault.path(), e.app.path(), e.ws(), true)
            .unwrap()
            .unwrap();
        assert!(store.check_protected("plan").is_ok());
        // The same check a write gate uses, failing closed on a vault that
        // can't be resolved.
        let gone = Store {
            vault: Some(e.vault.path().join("missing-vault")),
            ..store
        };
        assert!(gone
            .check_protected("plan")
            .unwrap_err()
            .contains("protected"));
    }

    #[test]
    fn an_outside_edit_of_the_visible_copy_is_kept_before_a_new_revision() {
        let e = env();
        let v1 = body("note", "Note", "<p>1</p>");
        let c = e.record(e.ws(), &v1, None, T0).unwrap().canvas;
        let visible = e.dir().join("note.canvas.html");
        std::fs::write(&visible, "<p>edited by hand</p>").unwrap();
        let now = e.read(e.ws(), "note").unwrap();
        assert!(now.html_changed);
        assert_eq!(now.html, v1, "rendering uses the revision store");
        assert_eq!(
            now.sha256, c.sha256,
            "the visible copy is not the CAS token"
        );
        let v2 = body("note", "Note", "<p>2</p>");
        let r = e.record(e.ws(), &v2, Some(&c.sha256), T1).unwrap().canvas;
        assert!(!r.html_changed);
        assert_eq!(std::fs::read_to_string(&visible).unwrap(), v2);
        let kept = e.dir().join(format!(
            ".revisions/note/outside-{}.canvas.html",
            sha(b"<p>edited by hand</p>")
        ));
        assert_eq!(
            std::fs::read_to_string(kept).unwrap(),
            "<p>edited by hand</p>"
        );
        // A missing visible copy is reported and rewritten by the next revision.
        std::fs::remove_file(&visible).unwrap();
        assert!(e.read(e.ws(), "note").unwrap().html_changed);
    }

    #[test]
    fn unknown_fields_survive_and_a_newer_format_is_refused() {
        let e = env();
        let c = e
            .record(e.ws(), &body("f", "F", "<p>1</p>"), None, T0)
            .unwrap()
            .canvas;
        let path = e.dir().join("f.canvas.json");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["futureField"] = json!({ "kept": true });
        value["revisions"][0]["futureRev"] = json!(7);
        let text = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &text).unwrap();
        let saved = e.save_state(e.ws(), "f", json!(1), &sha(&text)).unwrap();
        let r = e
            .record(e.ws(), &body("f", "F", "<p>2</p>"), Some(&saved.sha256), T1)
            .unwrap()
            .canvas;
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["futureField"], json!({ "kept": true }));
        assert_eq!(after["revisions"][0]["futureRev"], 7);
        assert_eq!(r.revisions[0].extra["futureRev"], 7);
        assert_ne!(c.sha256, r.sha256);
        value["hollowCanvas"] = json!(2);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(read(e.vault.path(), e.app.path(), e.ws(), "f")
            .unwrap_err()
            .contains("newer Hollow"));
        // A damaged sidecar is refused, never overwritten.
        std::fs::write(&path, b"{not json").unwrap();
        assert!(e
            .record(e.ws(), &body("f", "F", "<p>3</p>"), None, T2)
            .unwrap_err()
            .starts_with(INVALID));
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
        let listed = list(e.vault.path(), e.app.path(), e.ws()).unwrap();
        assert!(listed[0].error.is_some());
    }

    #[test]
    fn outside_a_workspace_canvases_live_in_app_data_keyed_by_conversation() {
        let e = env();
        let s1 = Scope {
            workspace_id: None,
            session_key: Some(S1),
        };
        let s2 = Scope {
            workspace_id: None,
            session_key: Some(S2),
        };
        let v1 = body("scratch", "Scratch", "<p>a</p>");
        let c = e.record(s1, &v1, None, T0).unwrap().canvas;
        assert_eq!(c.scope, "session");
        assert_eq!(c.session_key.as_deref(), Some(S1));
        assert!(c.path.is_none() && c.file.is_none());
        let dir = e.app.path().join("canvases").join(S1);
        assert!(dir.join("scratch.canvas.json").is_file());
        assert!(dir.join("scratch.canvas.html").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["scratch.canvas.json", "scratch.canvas.html"] {
                let mode = std::fs::metadata(dir.join(name))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{name}");
            }
        }
        // Another conversation's store is separate; nothing reached the vault.
        assert!(e.read(s2, "scratch").is_none());
        e.record(s2, &body("scratch", "Other", "<p>b</p>"), None, T0)
            .unwrap();
        assert_eq!(e.read(s1, "scratch").unwrap().title, "Scratch");
        assert!(!e
            .vault
            .path()
            .join("workspaces")
            .join(&e.a.id)
            .join("outputs/canvases")
            .exists());
        let saved = e
            .save_state(s1, "scratch", json!({ "x": 1 }), &c.sha256)
            .unwrap();
        let r = e
            .record(
                s1,
                &body("scratch", "Scratch", "<p>c</p>"),
                Some(&saved.sha256),
                T1,
            )
            .unwrap()
            .canvas;
        assert_eq!(r.state, json!({ "x": 1 }));
        assert_eq!(list(e.vault.path(), e.app.path(), s1).unwrap().len(), 1);
    }

    #[test]
    fn the_app_data_store_prunes_the_least_recently_saved_conversations() {
        let canvases = tempfile::tempdir().unwrap();
        let keys: Vec<String> = (0..4).map(|i| format!("s-{i}")).collect();
        for (i, key) in keys.iter().enumerate() {
            let dir = canvases.path().join(key);
            std::fs::create_dir_all(&dir).unwrap();
            let when = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_000_000 + i as u64 * 10);
            std::fs::File::open(&dir)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        std::fs::create_dir_all(canvases.path().join("not-a-session")).unwrap();
        // The newest is always kept, even when it is the oldest on disk.
        prune_sessions(canvases.path(), 2, &canvases.path().join("s-0"));
        let mut left: Vec<String> = std::fs::read_dir(canvases.path())
            .unwrap()
            .map(|d| d.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left, [".pruned", "not-a-session", "s-0", "s-3"]);
    }

    #[test]
    fn quick_chats_canvases_save_into_its_outputs() {
        let vault = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        workspaces::ensure_quick_chats(vault.path()).unwrap();
        let scope = Scope {
            workspace_id: Some(workspaces::QUICK_CHATS_ID),
            session_key: Some(S1),
        };
        let c = record(
            vault.path(),
            app.path(),
            scope,
            &body("scratch", "Scratch", ""),
            None,
            None,
            None,
            T0,
        )
        .unwrap()
        .canvas;
        assert_eq!(
            c.path.as_deref(),
            Some("workspaces/quick-chats/outputs/canvases/scratch.canvas.html")
        );
        assert!(vault.path().join(c.path.unwrap()).is_file());
    }

    /// A sidecar whose one revision is an ask-canvas saved at `saved_at`,
    /// written straight to `dir` (fast enough for hundreds).
    fn write_ask_sidecar(dir: &Path, id: &str, saved_at: &str, ask: bool) {
        let fence = format!("{:064x}", 7);
        let file = json!({
            "hollowCanvas": 1, "id": id, "title": format!("Ask {id}"), "session": S1,
            "fenceSha256": fence, "createdAt": saved_at, "updatedAt": saved_at, "revision": 1,
            "revisions": [{
                "revision": 1, "fenceSha256": fence, "title": format!("Ask {id}"),
                "savedAt": saved_at, "ask": ask, "session": S1,
            }],
        });
        std::fs::write(
            dir.join(format!("{id}.canvas.json")),
            serde_json::to_vec_pretty(&file).unwrap(),
        )
        .unwrap();
    }

    /// `n` seconds after 2026-10-04T10:00:00Z.
    fn at_second(n: usize) -> String {
        (chrono::DateTime::parse_from_rfc3339("2026-10-04T10:00:00+00:00").unwrap()
            + chrono::Duration::seconds(n as i64))
        .to_rfc3339()
    }

    #[test]
    fn ask_heads_keep_the_newest_when_a_store_holds_more_than_the_cap() {
        // C6 P3: the cap used to apply in directory order, before sorting,
        // so a newer ask past the first 500 entries was never seen.
        let dir = tempfile::tempdir().unwrap();
        let n = MAX_LIST + 100;
        // Ids are a permutation of the save order, so neither creation nor
        // name order is time order.
        let id_of = |i: usize| format!("ask-{:04}", (i * 7919) % n);
        for i in 0..n {
            write_ask_sidecar(dir.path(), &id_of(i), &at_second(i), true);
        }
        let heads = ask_heads_in(dir.path()).unwrap();
        assert_eq!(heads.len(), MAX_LIST);
        let mut got: Vec<&str> = heads.iter().map(|h| h.id.as_str()).collect();
        got.sort();
        let mut want: Vec<String> = (n - MAX_LIST..n).map(id_of).collect();
        want.sort();
        assert_eq!(got, want, "the cap kept other asks than the newest 500");
        assert_eq!(heads[0].id, id_of(n - 1), "newest first");
    }

    #[test]
    fn ask_heads_reread_only_the_sidecars_that_changed() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            write_ask_sidecar(
                dir.path(),
                &format!("ask-{i:02}"),
                &at_second(i),
                i % 2 == 0,
            );
        }
        let mut cache = AskCache::default();
        let first = ask_heads_cached(dir.path(), &mut cache).unwrap();
        assert_eq!(first.len(), 10);
        assert_eq!(cache.reads, 20, "the first listing reads every sidecar");

        // Nothing changed: nothing is read, and the answer is the same.
        let again = ask_heads_cached(dir.path(), &mut cache).unwrap();
        assert_eq!(again, first);
        assert_eq!(cache.reads, 20, "an unchanged sidecar was read again");

        // One plain canvas becomes an ask (a new revision) and one ask is
        // deleted: only the changed sidecar is read.
        write_ask_sidecar(dir.path(), "ask-01", &at_second(100), true);
        std::fs::remove_file(dir.path().join("ask-04.canvas.json")).unwrap();
        let changed = ask_heads_cached(dir.path(), &mut cache).unwrap();
        assert_eq!(cache.reads, 21, "more than the changed sidecar was read");
        assert_eq!(
            changed[0].id, "ask-01",
            "the change is picked up, newest first"
        );
        assert!(
            !changed.iter().any(|h| h.id == "ask-04"),
            "a deleted ask stays listed"
        );
        assert_eq!(changed.len(), 10);

        // A folder that is gone lists nothing and forgets its sidecars.
        let gone = dir.path().join("missing");
        assert!(ask_heads_cached(&gone, &mut cache).unwrap().is_empty());
    }

    #[test]
    #[ignore = "measurement: cargo test --lib measure_ask_heads -- --ignored --nocapture"]
    fn measure_ask_heads_on_a_large_store() {
        // C6 P3: 40 stores x 250 sidecars (10,000), one in ten an ask.
        let root = tempfile::tempdir().unwrap();
        let dirs: Vec<PathBuf> = (0..40)
            .map(|d| {
                let dir = root.path().join(format!("store-{d:02}"));
                std::fs::create_dir_all(&dir).unwrap();
                for i in 0..250 {
                    write_ask_sidecar(
                        &dir,
                        &format!("canvas-{i:03}"),
                        &at_second(d * 250 + i),
                        i % 10 == 0,
                    );
                }
                dir
            })
            .collect();
        let mut cache = AskCache::default();
        let time = |cache: &mut AskCache| {
            let start = std::time::Instant::now();
            let n: usize = dirs
                .iter()
                .map(|d| ask_heads_cached(d, cache).unwrap().len())
                .sum();
            (start.elapsed(), n)
        };
        let (cold, n) = time(&mut cache);
        let mut warm = Vec::new();
        for _ in 0..5 {
            warm.push(time(&mut cache).0);
        }
        warm.sort();
        println!(
            "ask heads, 10,000 sidecars ({n} asks): cold (every sidecar read) {cold:?}; warm (stamps only) median {:?}; reads {}",
            warm[2], cache.reads
        );
    }
}
