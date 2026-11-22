//! Companion apps, phase W1: Hollow as the *applier* client of the backend
//! (`hollow-companiond`, in the separate `hollow-companion` repo; contract in
//! that repo's `docs/BACKEND_API.md`). The second road for device ops, next
//! to the `.hollow-sync/inbox/` files of [`companion_inbox`].
//!
//! - Long-polls the backend for ops and pairing requests. Each op's exact
//!   envelope text goes through [`companion_inbox::deliver`]: the same
//!   verification, task-broker path and applied-op ledger as the inbox, so
//!   an op that arrives on both roads applies once. Receipts go back to the
//!   backend (not into the vault).
//! - Decides backend pairing requests with the same in-memory code as the
//!   inbox road ([`companion_inbox::decide_remote_pairing`]); Hollow stays
//!   the trust root and pushes the paired-device list to the backend.
//! - Pushes a read-only mirror: task files under `tasks/` (path, hash, bytes
//!   and the devices' parse) and project titles/status. Nothing else from
//!   the vault ever leaves (decision D5; the backend refuses other paths
//!   too).
//!
//! - Bridge generation 2 (overnight 10-12/13): every call says
//!   `X-Hollow-Bridge: 2` and `X-Hollow-Kinds` (exactly the op kinds
//!   [`companion_inbox::APPLIED_KINDS`] applies; the daemon hands no other),
//!   and a separate loop copies the documents [`companion_docs`] allows
//!   (scope personal by default) to `/api/v2/applier/docs`, incrementally by
//!   hash. A daemon without v2 (404, or its web app answering) just gets no
//!   documents; a document failure never stops the task mirror or ops.
//!
//! Configured in Settings, Devices (Backend URL + the daemon's
//! applier token), kept in `<support>/Companion/backend.json` (0600). Off
//! when that file is absent.

use crate::companion_docs::{self, HashCache, LocalDoc};
use crate::companion_inbox::{self, Delivery, Store};
use crate::vault_tasks;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const CONFIG_FILE: &str = "backend.json";
const APPLIER_ID_FILE: &str = "applier-id";
/// Long-poll length asked of the backend; the HTTP timeout is longer.
const POLL_WAIT_SECS: u64 = 20;
const HTTP_TIMEOUT: Duration = Duration::from_secs(POLL_WAIT_SECS + 15);
/// How often the mirror and device list are compared with what was pushed
/// (a separate task from the long-poll, so neither waits on the other).
const PUSH_EVERY: Duration = Duration::from_secs(5);
/// Back-off after a failed session, doubling up to the cap.
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// When an op must wait (a run holds the vault).
const BUSY_RETRY: Duration = Duration::from_secs(5);
/// A session that lasted this long resets the back-off.
const SESSION_HEALTHY: Duration = Duration::from_secs(60);
/// Mirror request size; the daemon refuses bodies over 2 MiB.
const MIRROR_BATCH_BYTES: usize = 1024 * 1024;
/// The most Hollow reads of one backend answer.
const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;
/// The generation this bridge declares (`X-Hollow-Bridge`).
const BRIDGE_GENERATION: &str = "2";
/// How often the documents are re-scanned (the task mirror is every 5 s).
const DOCS_EVERY: Duration = Duration::from_secs(15);
/// A failing document push backs off up to this.
const DOCS_RETRY_MAX: Duration = Duration::from_secs(600);
/// A daemon without the document mirror is asked again this often.
const DOCS_RECHECK: Duration = Duration::from_secs(3600);
/// The daemon refuses more than 1,000 documents in a batch.
const DOCS_PER_BATCH: usize = 500;
/// The daemon's own limits for one mirrored task (its `allowlist.rs`).
const MAX_ITEM_BYTES: usize = 256 * 1024;
const MAX_PATH_BYTES: usize = 512;

// ── configuration ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BackendConfig {
    pub url: String,
    pub token: String,
}

fn config_path(store: &Store) -> PathBuf {
    store.root().join(CONFIG_FILE)
}

pub(crate) fn load_config(store: &Store) -> Option<BackendConfig> {
    let text = std::fs::read_to_string(config_path(store)).ok()?;
    let config: BackendConfig = serde_json::from_str(&text).ok()?;
    normalize_url(&config.url).ok()?;
    (!config.token.trim().is_empty()).then_some(config)
}

/// The hosts plain http may go to: this Mac only. `normalize_url` and the
/// not-HTTPS words both ask here, so they can't drift apart (R7).
fn is_this_mac(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// `http://` only to this Mac (127.0.0.1, localhost, [::1]); `https://` to
/// any host (the tailnet name later). Returns the URL without a trailing
/// slash, query or fragment.
pub(crate) fn normalize_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "That isn't a URL.".to_string())?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The URL must not carry a user name or password.".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The URL must not have a query or fragment.".into());
    }
    let host = url.host_str().unwrap_or("");
    match url.scheme() {
        "https" if !host.is_empty() => {}
        "http" if is_this_mac(host) => {}
        "http" => return Err(
            "Plain http is only allowed to this Mac (127.0.0.1, localhost or [::1]); use https otherwise."
                .into(),
        ),
        _ => return Err("The URL must start with http:// or https://.".into()),
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn origin_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
}

/// Save (or, with an empty URL, remove) the backend settings. An empty
/// token keeps the saved one, but only for the same origin: the token is
/// never sent to a host it wasn't pasted for. Removing the backend also
/// drops the receipts still owed to it (`Store::forget_backend`), so they
/// aren't posted to whichever backend is set up next. That comes first: if
/// it fails, the backend stays set up and the removal can be tried again,
/// rather than being half done (review of EH5). Either failure says what
/// state it left, so Turn off can be tried again: forgetting is idempotent,
/// so a second try finishes the job. The first doesn't say "try again"
/// itself: the error it carries, put in plain words, usually ends with one.
pub(crate) fn save_config(store: &Store, url: &str, token: &str) -> Result<(), String> {
    let path = config_path(store);
    if url.trim().is_empty() {
        if let Err(e) = store.forget_backend() {
            return Err(format!(
                "Couldn't remove the backend. Nothing changed. Hollow couldn't clear the replies it still owes that backend: {e}"
            ));
        }
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "Hollow cleared the replies it owed the backend, but couldn't delete the backend's settings file. Choose Turn off again: {e}"
            )),
        };
    }
    let url = normalize_url(url)?;
    let token = resolve_token(
        store,
        &url,
        token,
        "Paste the backend's applier token (hollow-companiond --data <dir> token).",
    )?;
    std::fs::create_dir_all(store.root()).map_err(|e| format!("companion store: {e}"))?;
    let text = serde_json::to_string_pretty(&BackendConfig { url, token }).unwrap();
    crate::vault::atomic_write_private(&path, text.as_bytes())
}

/// The token for `url` (already normalized): the one typed, or, when blank,
/// the saved one, but only for the saved address. Save and Test connection
/// share this rule, so a token is never sent to a host it wasn't pasted for.
/// `missing` is what to say when a blank token has nothing to reuse.
fn resolve_token(store: &Store, url: &str, token: &str, missing: &str) -> Result<String, String> {
    match token.trim() {
        "" => load_config(store)
            .filter(|saved| origin_of(&saved.url) == origin_of(url))
            .map(|c| c.token)
            .ok_or_else(|| missing.to_string()),
        t if t.len() > 256 || t.chars().any(|c| c.is_whitespace() || c.is_control()) => {
            Err("That doesn't look like an applier token.".into())
        }
        t => Ok(t.to_string()),
    }
}

/// This Hollow's id towards the backend, which lets one Hollow at a time be
/// its applier (a dev build pointed at the same daemon is refused).
pub(crate) fn applier_id(store: &Store) -> Result<String, String> {
    let path = store.root().join(APPLIER_ID_FILE);
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if (8..=64).contains(&id.len())
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Ok(id.to_string());
        }
    }
    let mut bytes = [0u8; 12];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "no random source".to_string())?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(store.root()).map_err(|e| format!("companion store: {e}"))?;
    crate::vault::atomic_write_private(&path, id.as_bytes())?;
    Ok(id)
}

// ── status (Settings) ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    /// The configured URL (never the token).
    pub url: Option<String>,
    pub connected: bool,
    pub last_contact_at: Option<String>,
    pub last_error: Option<String>,
    /// The last mirror push that failed (ops still apply meanwhile).
    pub mirror_error: Option<String>,
    pub mirrored_tasks: usize,
    pub last_push_at: Option<String>,
    /// Documents the backend holds (v2); None until a v2 daemon answered.
    pub mirrored_docs: Option<usize>,
    /// The last document push that failed (tasks and ops go on meanwhile).
    pub docs_error: Option<String>,
    /// False when the daemon has no v2 document mirror (an older one).
    pub docs_supported: Option<bool>,
    /// `personal` or `work`, as last pushed.
    pub scope: Option<String>,
    /// `scope.json` couldn't be read: only tasks go up until it is saved.
    pub scope_error: Option<String>,
}

static STATUS: Mutex<Option<BackendStatus>> = Mutex::new(None);
/// Wakes the bridge: settings changed, or a re-push asked.
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
static RECONNECT: AtomicBool = AtomicBool::new(false);
static FULL_PUSH: AtomicBool = AtomicBool::new(false);
/// The document pusher's own "push everything" (it runs on its own loop).
static DOCS_FULL: AtomicBool = AtomicBool::new(false);
/// File hashes by size and mtime, kept across sessions so a reconnect
/// doesn't re-read the whole vault.
static DOCS_CACHE: Mutex<Option<HashCache>> = Mutex::new(None);

fn with_status(f: impl FnOnce(&mut BackendStatus)) {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(BackendStatus::default));
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn status(store: &Store) -> BackendStatus {
    let mut status = STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default();
    status.url = load_config(store).map(|c| c.url);
    if status.url.is_none() {
        status.connected = false;
    }
    status
}

/// Settings changed: drop the session and start again with the new ones.
pub(crate) fn reconnect() {
    RECONNECT.store(true, Ordering::SeqCst);
    FULL_PUSH.store(true, Ordering::SeqCst);
    DOCS_FULL.store(true, Ordering::SeqCst);
    WAKE.notify_waiters();
}

/// A held old op was just applied or discarded: send the receipts owed to
/// the backend now rather than after the current long-poll (each cycle
/// sends them first; see [`send_owed`]).
pub(crate) fn send_owed_now() {
    WAKE.notify_waiters();
}

/// Push every task and project again on the next tick.
pub(crate) fn repush() {
    FULL_PUSH.store(true, Ordering::SeqCst);
    DOCS_FULL.store(true, Ordering::SeqCst);
    WAKE.notify_waiters();
}

// ── the client ───────────────────────────────────────────────────────────

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    applier_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolledOp {
    seq: i64,
    device_id: String,
    op_id: String,
    envelope: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolledPair {
    device_id: String,
    request: String,
    /// Names the exact pending request; echoed with the decision.
    ticket: String,
}

#[derive(Debug, Deserialize)]
struct Poll {
    ops: Vec<PolledOp>,
    pairs: Vec<PolledPair>,
}

/// A failed call: the HTTP status when the backend answered.
struct CallError {
    status: Option<u16>,
    message: String,
}

impl From<CallError> for String {
    fn from(e: CallError) -> Self {
        e.message
    }
}

impl Client {
    fn new(config: &BackendConfig, applier_id: String) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            base: config.url.clone(),
            token: config.token.clone(),
            applier_id,
        })
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, CallError> {
        let mut resp = req
            .bearer_auth(&self.token)
            .header("X-Hollow-Applier", &self.applier_id)
            .header("X-Hollow-Bridge", BRIDGE_GENERATION)
            .header("X-Hollow-Kinds", companion_inbox::APPLIED_KINDS.join(","))
            .send()
            .await
            .map_err(|e| CallError {
                status: None,
                message: unreachable_words(
                    &host_of(&self.base),
                    Unreachable::of(&e, HTTP_TIMEOUT.as_secs()),
                ),
            })?;
        let status = resp.status();
        // A bounded read: a compromised or broken backend can't make Hollow
        // buffer an endless answer (a full poll is ~6.4 MB at most).
        let mut bytes: Vec<u8> = Vec::new();
        let mut broken: Option<String> = None;
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_ANSWER_BYTES as u64)
        {
            broken = Some("The backend's answer was too large.".into());
        }
        while broken.is_none() {
            match resp.chunk().await {
                Ok(Some(chunk)) if bytes.len() + chunk.len() > MAX_ANSWER_BYTES => {
                    broken = Some("The backend's answer was too large.".into());
                }
                Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(e) => {
                    broken = Some(format!(
                        "The backend's answer broke off: {}",
                        e.without_url()
                    ))
                }
            }
        }
        if let Some(message) = broken {
            // A failed answer keeps its status (a 409 is still a 409); a
            // successful one we couldn't read is no answer at all.
            if status.is_success() {
                return Err(CallError {
                    status: None,
                    message,
                });
            }
            bytes.clear();
        }
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            with_status(|s| {
                s.connected = true;
                s.last_contact_at = Some(now_rfc3339());
                s.last_error = None;
            });
            return Ok(body);
        }
        let message = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("no reason given");
        Err(CallError {
            status: Some(status.as_u16()),
            message: match status.as_u16() {
                401 => {
                    "The backend refused the applier token; paste it again in Settings.".to_string()
                }
                // The backend's own words, which could quote the token
                // back: masked before anything stores or shows them.
                code => backend_answered(code, message, &self.token),
            },
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1/applier{path}", self.base)
    }

    async fn put_devices(&self, store: &Store) -> Result<(), String> {
        let devices: Vec<Value> = store
            .devices()
            .into_iter()
            .map(|d| {
                json!({ "deviceId": d.device_id, "name": d.name, "platform": d.platform,
                        "publicKey": d.public_key })
            })
            .collect();
        self.send(
            self.http
                .put(self.url("/devices"))
                .json(&json!({ "devices": devices })),
        )
        .await
        .map(|_| ())
        .map_err(String::from)
    }

    async fn poll(&self, after: i64, wait: u64) -> Result<Poll, String> {
        let body = self
            .send(
                self.http
                    .get(self.url(&format!("/poll?after={after}&wait={wait}"))),
            )
            .await?;
        serde_json::from_value(body).map_err(|e| format!("The backend's poll answer: {e}"))
    }

    /// Hollow's decision on the request it saw. A request that is gone or
    /// was replaced (404/409) is skipped: the next poll shows what's pending.
    async fn post_pair(
        &self,
        pair: &PolledPair,
        decision: &Result<String, String>,
    ) -> Result<(), String> {
        let body = match decision {
            Ok(_) => json!({ "status": "paired", "ticket": pair.ticket }),
            Err(reason) => json!({ "status": "refused", "ticket": pair.ticket, "reason": reason }),
        };
        match self
            .send(
                self.http
                    .post(self.url(&format!("/pair/{}", pair.device_id)))
                    .json(&body),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(CallError {
                status: Some(404 | 409),
                message,
            }) => {
                eprintln!("[companion-bridge] pairing {}: {message}", pair.device_id);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn post_receipts(&self, receipts: &[companion_inbox::Receipt]) -> Result<(), String> {
        if receipts.is_empty() {
            return Ok(());
        }
        self.send(
            self.http
                .post(self.url("/receipts"))
                .json(&json!({ "receipts": receipts })),
        )
        .await
        .map(|_| ())
        .map_err(String::from)
    }

    fn url_v2(&self, path: &str) -> String {
        format!("{}/api/v2/applier{path}", self.base)
    }

    /// What the daemon's document mirror holds (path → hash), or None when
    /// it has no v2 mirror: a 404, or an older daemon whose web app answers
    /// unknown paths with its page (no JSON object).
    async fn doc_index(&self) -> Result<Option<HashMap<String, String>>, String> {
        match self.send(self.http.get(self.url_v2("/docs"))).await {
            Ok(body) => Ok(body.get("docs").and_then(Value::as_object).map(|docs| {
                docs.iter()
                    .filter_map(|(k, v)| v.as_str().map(|h| (k.clone(), h.to_string())))
                    .collect()
            })),
            Err(CallError {
                status: Some(404 | 405),
                ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn put_docs(&self, body: &Value) -> Result<(), String> {
        self.send(self.http.put(self.url_v2("/docs")).json(body))
            .await
            .map(|_| ())
            .map_err(String::from)
    }

    async fn put_mirror(&self, body: &Value) -> Result<(), String> {
        self.send(self.http.put(self.url("/mirror")).json(body))
            .await
            .map(|_| ())
            .map_err(String::from)
    }
}

// ── the mirror ───────────────────────────────────────────────────────────

/// What was last pushed: task path → content hash, the projects and the
/// device list, so a tick sends only what changed.
#[derive(Default)]
pub(crate) struct Pushed {
    tasks: HashMap<String, String>,
    projects: Option<Vec<Value>>,
    devices: Option<Vec<Value>>,
}

/// A task path the mirror may hold: under `tasks/`, no hidden component,
/// and within the daemon's own limits (it refuses a whole batch otherwise).
pub(crate) fn mirrorable(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() >= 2
        && parts[0] == "tasks"
        && rel.ends_with(".md")
        && rel.len() <= MAX_PATH_BYTES
        && !rel.contains('\\')
        && !rel.chars().any(char::is_control)
        && parts
            .iter()
            .all(|p| !p.is_empty() && !p.starts_with('.') && *p != "..")
}

/// One mirror item from the file as it is now (None: gone, not a plain
/// file, over the size limit, or not a task).
fn mirror_item(vault: &Path, rel: &str) -> Option<Value> {
    let full = crate::vault::safe_join(vault, rel).ok()?;
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_ITEM_BYTES as u64 {
        return None;
    }
    let content = std::fs::read_to_string(full).ok()?;
    if content.len() > MAX_ITEM_BYTES {
        return None;
    }
    let mut task = vault_tasks::parse(rel, &content, vault)?;
    task.content_hash = vault_tasks::content_hash(&content);
    Some(json!({
        "path": rel,
        "contentHash": task.content_hash,
        "content": content,
        "task": companion_inbox::task_json(&task, rel),
    }))
}

fn projects_now(vault: &Path) -> Vec<Value> {
    crate::vault_projects::list(vault)
        .into_iter()
        .map(|p| json!({ "ref": p.url, "title": p.name, "status": p.status }))
        .collect()
}

/// Split items into requests of at most [`MIRROR_BATCH_BYTES`].
fn chunk_by_bytes(items: Vec<Value>) -> Vec<Vec<Value>> {
    let mut chunks: Vec<Vec<Value>> = Vec::new();
    let mut size = 0;
    for item in items {
        let len = item.to_string().len();
        if chunks.is_empty() || size + len > MIRROR_BATCH_BYTES {
            chunks.push(Vec::new());
            size = 0;
        }
        size += len;
        chunks.last_mut().unwrap().push(item);
    }
    chunks
}

/// The mirror batches that bring the backend up to date with the vault.
/// `full`: every task, the last batch `complete` with `keep` naming what the
/// earlier batches sent, so the backend drops anything else.
pub(crate) fn mirror_batches(vault: &Path, pushed: &Pushed, full: bool) -> Vec<Value> {
    let now: HashMap<String, String> = vault_tasks::list_parsed(vault)
        .into_iter()
        .filter_map(|t| {
            let rel = t.url.strip_prefix("vault:")?.to_string();
            mirrorable(&rel).then_some((rel, t.content_hash))
        })
        .collect();
    let projects = projects_now(vault);
    let mut changed: Vec<&String> = now
        .iter()
        .filter(|(rel, hash)| full || pushed.tasks.get(*rel) != Some(*hash))
        .map(|(rel, _)| rel)
        .collect();
    changed.sort();
    let items: Vec<Value> = changed
        .into_iter()
        .filter_map(|rel| mirror_item(vault, rel))
        .collect();
    let mut chunks = chunk_by_bytes(items);
    if full {
        let last = chunks.pop().unwrap_or_default();
        let mut batches: Vec<Value> = Vec::new();
        let mut keep: Vec<String> = Vec::new();
        for chunk in chunks {
            keep.extend(
                chunk
                    .iter()
                    .filter_map(|t| t["path"].as_str().map(String::from)),
            );
            batches.push(json!({ "tasks": chunk }));
        }
        batches
            .push(json!({ "complete": true, "tasks": last, "keep": keep, "projects": projects }));
        return batches;
    }
    let mut removed: Vec<&String> = pushed
        .tasks
        .keys()
        .filter(|rel| !now.contains_key(*rel))
        .collect();
    removed.sort();
    let projects_changed = pushed.projects.as_ref() != Some(&projects);
    let mut batches: Vec<Value> = chunks
        .into_iter()
        .map(|chunk| json!({ "tasks": chunk }))
        .collect();
    if !removed.is_empty() || projects_changed {
        batches.push(json!({
            "removed": removed,
            "projects": if projects_changed { Value::from(projects) } else { Value::Null },
        }));
    }
    batches
}

/// Record a batch the backend accepted.
fn note_pushed(pushed: &mut Pushed, batch: &Value) {
    for item in batch["tasks"].as_array().into_iter().flatten() {
        if let (Some(path), Some(hash)) = (item["path"].as_str(), item["contentHash"].as_str()) {
            pushed.tasks.insert(path.to_string(), hash.to_string());
        }
    }
    if batch["complete"] == true {
        // The backend now holds exactly this batch plus `keep`.
        let mut kept: std::collections::HashSet<&str> = batch["keep"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        kept.extend(
            batch["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["path"].as_str()),
        );
        pushed.tasks.retain(|path, _| kept.contains(path.as_str()));
    }
    for path in batch["removed"].as_array().into_iter().flatten() {
        if let Some(path) = path.as_str() {
            pushed.tasks.remove(path);
        }
    }
    if let Some(projects) = batch["projects"].as_array() {
        pushed.projects = Some(projects.clone());
    }
}

fn devices_now(store: &Store) -> Vec<Value> {
    store
        .devices()
        .into_iter()
        .map(|d| json!([d.device_id, d.public_key]))
        .collect()
}

/// Bring the backend's device list and mirror up to date.
async fn push(
    client: &Client,
    vault: &Path,
    store: &Store,
    pushed: &mut Pushed,
    full: bool,
) -> Result<(), String> {
    let devices = devices_now(store);
    if full || pushed.devices.as_ref() != Some(&devices) {
        client.put_devices(store).await?;
        pushed.devices = Some(devices);
    }
    let batches = {
        let vault = vault.to_path_buf();
        let snapshot = Pushed {
            tasks: pushed.tasks.clone(),
            projects: pushed.projects.clone(),
            devices: None,
        };
        tokio::task::spawn_blocking(move || mirror_batches(&vault, &snapshot, full))
            .await
            .map_err(|e| e.to_string())?
    };
    for batch in &batches {
        client.put_mirror(batch).await?;
        note_pushed(pushed, batch);
    }
    if !batches.is_empty() {
        with_status(|s| {
            s.mirrored_tasks = pushed.tasks.len();
            s.last_push_at = Some(now_rfc3339());
        });
    }
    Ok(())
}

// ── the document mirror (v2) ─────────────────────────────────────────────

/// What the daemon's document mirror holds, as far as this session knows.
#[derive(Default)]
pub(crate) struct DocsPushed {
    /// None: not asked yet; Some(false): the daemon has no v2 mirror.
    supported: Option<bool>,
    hashes: HashMap<String, String>,
    /// Scope + personal list last pushed; a change forces a complete push.
    scope_key: String,
    /// (path, hash) the daemon refused (422) this session: left out, so one
    /// odd file can't hold back every document after it (a fixed file has a
    /// new hash and goes again).
    skip: std::collections::HashSet<(String, String)>,
    /// The wait after a failure (doubles up to [`DOCS_RETRY_MAX`]).
    backoff: Option<Duration>,
}

/// Split documents into requests of at most [`MIRROR_BATCH_BYTES`] and
/// [`DOCS_PER_BATCH`] documents.
fn chunk_docs(items: Vec<Value>) -> Vec<Vec<Value>> {
    let mut chunks: Vec<Vec<Value>> = Vec::new();
    let mut size = 0;
    for item in items {
        let len = item.to_string().len();
        let full = chunks
            .last()
            .is_none_or(|c| c.len() >= DOCS_PER_BATCH || size + len > MIRROR_BATCH_BYTES);
        if full {
            chunks.push(Vec::new());
            size = 0;
        }
        size += len;
        chunks.last_mut().unwrap().push(item);
    }
    chunks
}

/// Document batches that bring the daemon from `pushed` to `local`. `full`:
/// a last, document-free batch is `complete`, with `keep` naming every local
/// document, so the daemon drops anything else.
pub(crate) fn docs_batches(
    vault: &Path,
    local: &[LocalDoc],
    pushed: &HashMap<String, String>,
    scope: &str,
    personal: &[String],
    full: bool,
) -> Vec<Value> {
    let changed: Vec<Value> = local
        .iter()
        .filter(|d| pushed.get(&d.path) != Some(&d.hash))
        .filter_map(|d| companion_docs::wire(vault, d))
        .collect();
    let now: std::collections::HashSet<&str> = local.iter().map(|d| d.path.as_str()).collect();
    let mut removed: Vec<&String> = pushed
        .keys()
        .filter(|p| !now.contains(p.as_str()))
        .collect();
    removed.sort();
    let personal = if scope == "work" {
        Vec::new()
    } else {
        personal.to_vec()
    };
    let head =
        |docs: Vec<Value>| json!({ "scope": scope, "personalWorkspaces": personal, "docs": docs });
    let mut batches: Vec<Value> = chunk_docs(changed).into_iter().map(head).collect();
    if full {
        let mut last = head(Vec::new());
        last["complete"] = json!(true);
        last["keep"] = json!(local.iter().map(|d| d.path.as_str()).collect::<Vec<_>>());
        batches.push(last);
    } else if !removed.is_empty() {
        let mut b = head(Vec::new());
        b["removed"] = json!(removed);
        batches.push(b);
    }
    batches
}

/// Record a document batch the daemon accepted.
fn note_docs_pushed(hashes: &mut HashMap<String, String>, batch: &Value) {
    for d in batch["docs"].as_array().into_iter().flatten() {
        if let (Some(p), Some(h)) = (d["path"].as_str(), d["hash"].as_str()) {
            hashes.insert(p.to_string(), h.to_string());
        }
    }
    for p in batch["removed"].as_array().into_iter().flatten() {
        if let Some(p) = p.as_str() {
            hashes.remove(p);
        }
    }
    if batch["complete"] == true {
        let mut kept: std::collections::HashSet<&str> = batch["keep"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        kept.extend(
            batch["docs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["path"].as_str()),
        );
        hashes.retain(|p, _| kept.contains(p.as_str()));
    }
}

/// The document of `batch` a daemon refusal names (its messages quote the
/// path in backticks), as (path, hash). None when the refusal is about the
/// batch itself, not one document.
fn refused_doc(error: &str, batch: &Value) -> Option<(String, String)> {
    if !error.contains("answered 422") {
        return None;
    }
    batch["docs"].as_array()?.iter().find_map(|d| {
        let path = d["path"].as_str()?;
        error.contains(&format!("`{path}`")).then(|| {
            (
                path.to_string(),
                d["hash"].as_str().unwrap_or("").to_string(),
            )
        })
    })
}

/// One document pass: bring the daemon's document mirror up to date.
/// Returns how long to wait before the next one. Never fails the caller;
/// errors go to the status.
async fn push_docs(
    client: &Client,
    vault: &Path,
    store: &Store,
    docs: &mut DocsPushed,
    full: bool,
) -> Duration {
    let result: Result<Duration, String> = async {
        let mut full = full;
        if full || docs.supported != Some(true) {
            match client.doc_index().await? {
                None => {
                    docs.supported = Some(false);
                    with_status(|s| {
                        s.docs_supported = Some(false);
                        s.mirrored_docs = None;
                    });
                    return Ok(DOCS_RECHECK);
                }
                Some(index) => {
                    docs.supported = Some(true);
                    docs.hashes = index;
                    full = true;
                }
            }
        }
        let (vault_buf, store2) = (vault.to_path_buf(), store.clone());
        let mut cache = DOCS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_default();
        let (scope, scope_error, local, personal) = tokio::task::spawn_blocking(move || {
            let (scope, error) = companion_docs::load_scope_checked(&store2, &vault_buf);
            let (local, personal) = companion_docs::collect(&vault_buf, &scope, &mut cache);
            // Put the cache back here, so a pass dropped mid-way keeps it.
            *DOCS_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(cache);
            (scope, error, local, personal)
        })
        .await
        .map_err(|e| e.to_string())?;
        with_status(|s| s.scope_error = scope_error.clone());
        if scope_error.is_some() {
            // Fail closed: nothing but tasks until the person saves a scope.
            return Ok(DOCS_EVERY);
        }
        let key = format!("{}|{}", scope.name(), personal.join(","));
        if key != docs.scope_key {
            full = true;
            docs.skip.clear();
            // Set now: a refusal below must not clear the skips again.
            docs.scope_key = key.clone();
        }
        let local: Vec<LocalDoc> = local
            .into_iter()
            .filter(|d| !docs.skip.contains(&(d.path.clone(), d.hash.clone())))
            .collect();
        let batches = {
            let (vault_buf, pushed, name, personal2) = (
                vault.to_path_buf(),
                docs.hashes.clone(),
                scope.name(),
                personal.clone(),
            );
            tokio::task::spawn_blocking(move || {
                docs_batches(&vault_buf, &local, &pushed, name, &personal2, full)
            })
            .await
            .map_err(|e| e.to_string())?
        };
        for batch in &batches {
            if let Err(error) = client.put_docs(batch).await {
                if let Some(doc) = refused_doc(&error, batch) {
                    if docs.skip.insert(doc) {
                        // Leave that one file out and carry on with the rest soon.
                        docs.supported = None;
                        let error = masked(error, &client.token);
                        with_status(|s| s.docs_error = Some(error));
                        return Ok(DOCS_EVERY);
                    }
                }
                return Err(error);
            }
            note_docs_pushed(&mut docs.hashes, batch);
        }
        let count = docs.hashes.len();
        with_status(|s| {
            s.docs_supported = Some(true);
            s.mirrored_docs = Some(count);
            s.docs_error = None;
            s.scope = Some(scope.name().to_string());
        });
        Ok(DOCS_EVERY)
    }
    .await;
    match result {
        Ok(wait) => {
            docs.backoff = None;
            wait
        }
        Err(error) => {
            eprintln!("[companion-bridge] documents: {error}");
            // Ask the daemon what it holds next time rather than guess.
            docs.supported = None;
            let wait = docs
                .backoff
                .map_or(DOCS_EVERY, |b| (b * 2).min(DOCS_RETRY_MAX));
            docs.backoff = Some(wait);
            let error = masked(error, &client.token);
            with_status(|s| s.docs_error = Some(error));
            wait
        }
    }
}

// ── applying ─────────────────────────────────────────────────────────────

/// What one poll's ops came to.
pub(crate) struct Applied {
    pub receipts: Vec<companion_inbox::Receipt>,
    /// The seq to poll after next time.
    pub cursor: i64,
    /// An op had to wait (busy vault): poll again after a pause.
    pub busy: bool,
    pub review_changed: usize,
}

/// Apply polled ops in order through the inbox's own path. A busy vault
/// stops the batch, and the next poll starts again from that op.
fn apply_ops(vault: &Path, store: &Store, ops: &[PolledOp], after: i64) -> Applied {
    let _pass = companion_inbox::pass_lock();
    let mut out = Applied {
        receipts: Vec::new(),
        cursor: after,
        busy: false,
        review_changed: 0,
    };
    for op in ops {
        match companion_inbox::deliver(
            vault,
            store,
            &op.device_id,
            &op.op_id,
            &op.envelope,
            companion_inbox::Road::Backend,
            Local::now(),
        ) {
            Ok(Delivery::Waiting) => {
                out.busy = true;
                break;
            }
            // An old op waits for the person's confirm: no receipt yet, so
            // the backend keeps it pending. Once it is decided its receipt
            // is owed in the ledger and sent by `send_owed`.
            Ok(Delivery::Held { review_changed }) => out.review_changed += review_changed,
            // A repeat counts the cards its first apply raised: a cycle
            // cancelled after applying never sent its Review refresh.
            Ok(Delivery::Repeat(receipt)) => {
                out.review_changed += receipt.review_cards();
                out.receipts.push(receipt);
            }
            Ok(Delivery::Refused(receipt)) => out.receipts.push(receipt),
            Ok(Delivery::Applied {
                receipt,
                review_changed,
            }) => {
                out.receipts.push(receipt);
                out.review_changed += review_changed;
            }
            Err(error) => {
                // The ledger couldn't be written: try this op again later.
                eprintln!("[companion-bridge] {}: {error}", op.op_id);
                out.busy = true;
                break;
            }
        }
        out.cursor = op.seq;
    }
    out
}

/// Decisions already made, by ticket, so a request that comes back (the
/// answer was lost, or a cancelled cycle never sent it) gets the same answer
/// instead of a second decision against a code the first one used up.
static DECIDED: Mutex<Option<HashMap<String, (Result<String, String>, std::time::Instant)>>> =
    Mutex::new(None);
const DECIDED_KEEP: Duration = Duration::from_secs(15 * 60);

/// Decide the polled pairing requests (each with the open code; see
/// [`companion_inbox::decide_remote_pairing`]).
fn decide_pairs(store: &Store, pairs: &[PolledPair]) -> Vec<Result<String, String>> {
    let mut guard = DECIDED.lock().unwrap_or_else(|e| e.into_inner());
    let decided = guard.get_or_insert_with(HashMap::new);
    decided.retain(|_, (_, at)| at.elapsed() < DECIDED_KEEP);
    pairs
        .iter()
        .map(|p| {
            if let Some((decision, _)) = decided.get(&p.ticket) {
                return decision.clone();
            }
            let decision =
                companion_inbox::decide_remote_pairing(store, &p.device_id, &p.request, Utc::now());
            decided.insert(
                p.ticket.clone(),
                (decision.clone(), std::time::Instant::now()),
            );
            decision
        })
        .collect()
}

/// Receipts owed to the backend per request (each is a few hundred bytes).
const OWED_PER_POST: usize = 100;

/// Post the receipts the ledger owes the backend: held old ops the person
/// decided after the backend handed them over. Not left to a poll: the
/// daemon expires an unapplied `note.*`/`workspace.append` op after 14 days
/// and never hands it out again, while a receipt it is sent replaces its
/// `expired` one. Each is owed until the daemon stored it (a 200); a failure
/// keeps it for the next cycle and doesn't stop the poll. Returns how many
/// went.
async fn send_owed(client: &Client, store: &Store) -> usize {
    let reading = store.clone();
    let owed = tokio::task::spawn_blocking(move || reading.owed_receipts(OWED_PER_POST)).await;
    let owed = match owed {
        Ok(Ok(owed)) => owed,
        Ok(Err(error)) => {
            eprintln!("[companion-bridge] owed receipts: {error}");
            return 0;
        }
        Err(error) => {
            eprintln!("[companion-bridge] owed receipts: {error}");
            return 0;
        }
    };
    if owed.is_empty() {
        return 0;
    }
    if let Err(error) = client.post_receipts(&owed).await {
        eprintln!("[companion-bridge] owed receipts kept for the next cycle: {error}");
        return 0;
    }
    let sent = owed.len();
    let store = store.clone();
    let forgotten = tokio::task::spawn_blocking(move || {
        for receipt in &owed {
            // A row left behind is only sent again: the daemon ignores a
            // receipt it already has.
            if let Err(error) = store.receipt_delivered(&receipt.device_id, &receipt.op_id) {
                eprintln!("[companion-bridge] {}: {error}", receipt.op_id);
            }
        }
    })
    .await;
    if let Err(error) = forgotten {
        eprintln!("[companion-bridge] owed receipts: {error}");
    }
    sent
}

/// What one cycle did.
struct CycleOutcome {
    cursor: i64,
    busy: bool,
    /// Receipts posted this cycle. Only the debug-only `run_once` reports
    /// them; the running bridge doesn't need the count.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    receipts: usize,
}

/// One poll and what follows it, after the receipts owed from decided old
/// ops. `notify(review_changed, changed)` reports a change to Hollow's UI.
async fn cycle(
    client: &Client,
    vault: &Path,
    store: &Store,
    after: i64,
    wait: u64,
    notify: &(dyn Fn(usize, bool) + Sync),
) -> Result<CycleOutcome, String> {
    let owed = send_owed(client, store).await;
    let polled = client.poll(after, wait).await?;
    let mut paired_any = false;
    if !polled.pairs.is_empty() {
        let pairs = polled.pairs;
        let (store2, pairs) = (store.clone(), std::sync::Arc::new(pairs));
        let decisions = {
            let pairs = pairs.clone();
            tokio::task::spawn_blocking(move || decide_pairs(&store2, &pairs))
                .await
                .map_err(|e| e.to_string())?
        };
        paired_any = decisions.iter().any(Result::is_ok);
        if paired_any {
            // The device list first, so the token works as soon as the
            // device hears "paired".
            client.put_devices(store).await?;
        }
        for (pair, decision) in pairs.iter().zip(&decisions) {
            client.post_pair(pair, decision).await?;
        }
    }
    if polled.ops.is_empty() {
        if paired_any {
            notify(0, true);
        }
        return Ok(CycleOutcome {
            cursor: after,
            busy: false,
            receipts: owed,
        });
    }
    let applied = {
        let (vault, store, ops) = (vault.to_path_buf(), store.clone(), polled.ops);
        tokio::task::spawn_blocking(move || apply_ops(&vault, &store, &ops, after))
            .await
            .map_err(|e| e.to_string())?
    };
    client.post_receipts(&applied.receipts).await?;
    notify(
        applied.review_changed,
        paired_any || !applied.receipts.is_empty(),
    );
    Ok(CycleOutcome {
        cursor: applied.cursor,
        busy: applied.busy,
        receipts: owed + applied.receipts.len(),
    })
}

/// One whole bridge pass without waiting: devices, a full mirror, every
/// pending pairing request and op, then the mirror again. For the debug
/// `hollow companion-bridge-once` and tests (debug builds only, like that
/// command).
#[cfg(debug_assertions)]
pub(crate) async fn run_once(
    vault: &Path,
    store: &Store,
    config: &BackendConfig,
) -> Result<Value, String> {
    let client = Client::new(config, applier_id(store)?)?;
    let mut pushed = Pushed::default();
    push(&client, vault, store, &mut pushed, true).await?;
    let mut docs = DocsPushed::default();
    let _ = push_docs(&client, vault, store, &mut docs, true).await;
    let (mut cursor, mut receipts) = (0, 0usize);
    let busy = loop {
        let out = cycle(&client, vault, store, cursor, 0, &|_, _| {}).await?;
        receipts += out.receipts;
        if out.cursor == cursor || out.busy {
            break out.busy;
        }
        cursor = out.cursor;
    };
    push(&client, vault, store, &mut pushed, false).await?;
    let docs_error = STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|s| s.docs_error.clone());
    Ok(
        json!({ "cursor": cursor, "receipts": receipts, "busy": busy,
               "mirroredTasks": pushed.tasks.len(), "mirroredDocs": docs.hashes.len(),
               "docsSupported": docs.supported, "docsError": docs_error }),
    )
}

// ── running inside Hollow ────────────────────────────────────────────────

/// Start the bridge: idle until a backend is configured, then poll, apply
/// and push until Hollow quits. Reconnects with back-off on any error.
pub(crate) fn start(app: impl Into<crate::core_host::Core>, vault: PathBuf) {
    let app = app.into();
    let Some(store) = Store::for_app() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let notify = move |review_changed: usize, changed: bool| {
            if changed {
                let _ = app.emit(companion_inbox::EVENT, json!({ "road": "backend" }));
            }
            if review_changed > 0 {
                let _ = app.emit(crate::task_broker::REVIEW_EVENT, review_changed);
            }
        };
        let mut retry = RETRY_MIN;
        loop {
            RECONNECT.store(false, Ordering::SeqCst);
            let Some(config) = load_config(&store) else {
                with_status(|s| {
                    s.connected = false;
                    s.last_error = None;
                    s.mirror_error = None;
                });
                let _ = tokio::time::timeout(Duration::from_secs(30), WAKE.notified()).await;
                continue;
            };
            with_status(|s| s.connected = false);
            let started = tokio::time::Instant::now();
            let error = match session(&config, &vault, &store, &notify).await {
                Ok(()) => {
                    retry = RETRY_MIN;
                    continue;
                }
                Err(error) => error,
            };
            eprintln!("[companion-bridge] {error}");
            if started.elapsed() >= SESSION_HEALTHY {
                // A session that ran a while: the next try starts quickly. A
                // failure that comes right back keeps backing off.
                retry = RETRY_MIN;
            }
            // Masked with this session's token before it is kept: the status
            // outlives a token change, and an agent reads it.
            let error = masked(error, &config.token);
            with_status(|s| {
                s.connected = false;
                s.last_error = Some(error);
            });
            let _ = tokio::time::timeout(retry, WAKE.notified()).await;
            retry = (retry * 2).min(RETRY_MAX);
        }
    });
}

/// One connected session: the long-poll (ops and pairing) and the mirror
/// pusher run side by side. Ok(()) when settings changed; Err when the poll
/// fails. A mirror failure never stops ops from applying: it is shown in
/// Settings and retried on the next tick.
async fn session(
    config: &BackendConfig,
    vault: &Path,
    store: &Store,
    notify: &(dyn Fn(usize, bool) + Sync),
) -> Result<(), String> {
    let client = Client::new(config, applier_id(store)?)?;
    let applied = tokio::sync::Notify::new();
    let poller = async {
        let mut cursor = 0;
        loop {
            if RECONNECT.load(Ordering::SeqCst) {
                return Ok(());
            }
            let out = tokio::select! {
                r = cycle(&client, vault, store, cursor, POLL_WAIT_SECS, notify) => r?,
                // Re-check RECONNECT. Ops a cancelled cycle already applied
                // come back on the next poll as repeats with their receipts.
                _ = WAKE.notified() => continue,
            };
            if out.cursor != cursor {
                applied.notify_one();
            }
            cursor = out.cursor;
            if out.busy {
                let _ = tokio::time::timeout(BUSY_RETRY, WAKE.notified()).await;
            }
        }
    };
    let pusher = async {
        let mut pushed = Pushed::default();
        let mut full = true;
        let mut wait = PUSH_EVERY;
        loop {
            if RECONNECT.load(Ordering::SeqCst) {
                return Ok::<(), String>(());
            }
            full |= FULL_PUSH.swap(false, Ordering::SeqCst);
            match push(&client, vault, store, &mut pushed, full).await {
                Ok(()) => {
                    full = false;
                    wait = PUSH_EVERY;
                    with_status(|s| s.mirror_error = None);
                }
                Err(error) => {
                    // Back off: a failing full push re-reads every task.
                    eprintln!("[companion-bridge] mirror: {error}");
                    let error = masked(error, &client.token);
                    with_status(|s| s.mirror_error = Some(error));
                    wait = (wait * 2).min(RETRY_MAX);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = applied.notified() => {}
                _ = WAKE.notified() => {}
            }
        }
    };
    // Documents on their own loop: a slow or failing document push never
    // delays the task mirror or the ops.
    let documents = async {
        let mut docs = DocsPushed::default();
        let mut full = true;
        loop {
            if RECONNECT.load(Ordering::SeqCst) {
                return Ok::<(), String>(());
            }
            full |= DOCS_FULL.swap(false, Ordering::SeqCst);
            // Listen before pushing, so a repush during the pass isn't missed.
            let woken = WAKE.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            let wait = push_docs(&client, vault, store, &mut docs, full).await;
            full = false;
            if DOCS_FULL.load(Ordering::SeqCst) || RECONNECT.load(Ordering::SeqCst) {
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = woken => {}
            }
        }
    };
    tokio::select! {
        r = poller => r,
        r = pusher => r,
        r = documents => r,
    }
}

// ── Settings commands (the person's clicks; the COO's actions in L13) ────
//
// Each command calls a plain inner function, which the COO's action calls
// too (behind the person's card for every change). The token goes in only;
// what the COO gets back masks it (`agent_*` below).

fn app_store() -> Result<Store, String> {
    Store::for_app().ok_or_else(|| "Hollow's support folder is unavailable.".into())
}

pub(crate) fn backend_status() -> Result<BackendStatus, String> {
    Ok(status(&app_store()?))
}

#[tauri::command]
pub fn companion_backend_status() -> Result<BackendStatus, String> {
    backend_status()
}

/// Save the backend URL and applier token (an empty URL turns the bridge
/// off; an empty token keeps the saved one for the same origin).
#[tauri::command]
pub fn companion_backend_set(url: String, token: String) -> Result<BackendStatus, String> {
    backend_set(&url, &token)
}

pub(crate) fn backend_set(url: &str, token: &str) -> Result<BackendStatus, String> {
    let store = app_store()?;
    save_config(&store, url, token)?;
    with_status(|s| {
        s.connected = false;
        s.last_error = None;
        s.mirror_error = None;
        // A new address (or token) hasn't been reached yet: never show the
        // old backend's time under the new one's error (B05 review).
        s.last_contact_at = None;
    });
    reconnect();
    Ok(status(&store))
}

/// A claim code as the backend shows it (`K7M3-Q9TX`): 8 Crockford base32
/// characters, case, spaces and dashes ignored.
pub(crate) fn normalize_claim_code(raw: &str) -> Result<String, String> {
    let code: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if code.len() == 8
        && code
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    {
        Ok(code)
    } else {
        Err(
            "That isn't a claim code: 8 letters and digits, as the backend's setup page shows it."
                .into(),
        )
    }
}

/// Trade a claim code for the backend's applier token. The code is shown
/// only to the tailnet user on the backend's `/setup` page (and in its log),
/// so no token is copied by hand.
async fn claim(url: &str, code: &str, applier_id: &str) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http
        .post(format!("{url}/api/v2/applier/claim"))
        .json(&json!({ "code": code, "applierId": applier_id }))
        .send()
        .await
        .map_err(|e| unreachable_words(&host_of(url), Unreachable::of(&e, 15)))?;
    let status = resp.status().as_u16();
    // Small answers only; the guard answers some refusals in plain text.
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("The backend's answer: {}", e.without_url()))?;
    if bytes.len() > 64 * 1024 {
        return Err("The backend's answer was too large.".into());
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let message = body
        .get("error")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            let t: String = text.trim().chars().take(160).collect();
            (!t.is_empty() && !t.starts_with('<'))
                .then(|| format!("The backend answered {status}: {t}"))
        });
    match status {
        200 => {
            let token = body
                .get("token")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if token.is_empty() || token.len() > 256 || token.chars().any(char::is_whitespace) {
                Err("The backend's answer had no usable token.".into())
            } else {
                Ok(token)
            }
        }
        404 | 405 => Err(
            "This backend can't be connected with a code yet; paste its applier token instead."
                .into(),
        ),
        _ => Err(message.unwrap_or_else(|| format!("The backend answered {status}."))),
    }
}

/// Connect to a backend with the claim code its setup page shows: saves the
/// URL and the token it returns (the person's Settings click, or the COO's
/// `companion.backend_claim` after its card).
#[tauri::command]
pub async fn companion_backend_claim(url: String, code: String) -> Result<BackendStatus, String> {
    backend_claim(&url, &code).await
}

pub(crate) async fn backend_claim(url: &str, code: &str) -> Result<BackendStatus, String> {
    let store = app_store()?;
    let url = normalize_url(url)?;
    let code = normalize_claim_code(code)?;
    let id = applier_id(&store)?;
    let token = claim(&url, &code, &id).await?;
    save_config(&store, &url, &token)?;
    with_status(|s| {
        s.connected = false;
        s.last_error = None;
        s.mirror_error = None;
        // A new address (or token) hasn't been reached yet: never show the
        // old backend's time under the new one's error (B05 review).
        s.last_contact_at = None;
    });
    reconnect();
    Ok(status(&store))
}

#[tauri::command]
pub fn companion_backend_repush() -> Result<BackendStatus, String> {
    backend_repush()
}

pub(crate) fn backend_repush() -> Result<BackendStatus, String> {
    let store = app_store()?;
    if load_config(&store).is_none() {
        return Err("No backend is set up.".into());
    }
    repush();
    Ok(status(&store))
}

/// What Settings, Devices, **Test connection** found: one plain
/// sentence, and the daemon's version when it answered (ST5, 2026-10-14).
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackendTest {
    pub ok: bool,
    pub message: String,
    pub version: Option<String>,
}

/// The connection a test checks: the typed URL and token, or the saved
/// connection when the URL is blank. A blank token keeps the saved one only
/// for the saved address, as Save does: a token never goes to a host it
/// wasn't pasted for.
pub(crate) fn test_target(store: &Store, url: &str, token: &str) -> Result<BackendConfig, String> {
    if url.trim().is_empty() {
        return load_config(store).ok_or_else(|| "No backend is set up.".to_string());
    }
    let url = normalize_url(url)?;
    let token = resolve_token(
        store,
        &url,
        token,
        "Paste the backend's applier token to test this address.",
    )?;
    Ok(BackendConfig { url, token })
}

/// The most a test reads of one answer (the daemon's health and status are
/// a few fields); a bounded read like [`Client::send`]'s, so a wrong address
/// that streams can't make Hollow buffer it.
const TEST_ANSWER_BYTES: usize = 64 * 1024;

/// A small JSON answer: its status, and its body or Null when it isn't
/// JSON, is too large or broke off.
async fn small_json(mut resp: reqwest::Response) -> (u16, Value) {
    let status = resp.status().as_u16();
    if resp
        .content_length()
        .is_some_and(|n| n > TEST_ANSWER_BYTES as u64)
    {
        return (status, Value::Null);
    }
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() > TEST_ANSWER_BYTES => {
                return (status, Value::Null)
            }
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(_) => return (status, Value::Null),
        }
    }
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Why a request got no answer at all (B05, overnight 10-16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unreachable {
    /// Nothing came back within this many seconds.
    Timeout(u64),
    /// No connection: nothing listening, no route, or the name didn't resolve.
    Connect,
    /// Something answered, but its secure connection (its HTTPS certificate)
    /// didn't check out. reqwest counts this as a connect error too.
    Certificate,
    /// Something answered, but not over HTTPS: an `https://` address in
    /// front of a plain-HTTP server (rustls: "received corrupt message of
    /// type InvalidContentType"). A connect error too (R6).
    NotHttps,
    Other,
}

impl Unreachable {
    /// What kind of no-answer `e` is, for a client that gives up after
    /// `timeout_secs`.
    fn of(e: &reqwest::Error, timeout_secs: u64) -> Self {
        if e.is_timeout() {
            return Self::Timeout(timeout_secs);
        }
        // Only the causes under reqwest's own words: those end with the URL,
        // and a host named like `tls.example.com` isn't a certificate problem.
        let mut causes = Vec::new();
        let mut source = std::error::Error::source(e);
        while let Some(cause) = source {
            causes.push(cause.to_string());
            source = std::error::Error::source(cause);
        }
        Self::from_causes(&causes, e.is_connect())
    }

    /// The kind for a request that didn't time out, from its error's causes
    /// (outermost first) and whether reqwest called it a connect error.
    /// rustls words these "invalid peer certificate: …", "received fatal
    /// alert: HandshakeFailure" and the like. Only a connect error can be one:
    /// the handshake happens while connecting, and an HTTPS connection that
    /// drops later says "peer closed connection without sending TLS
    /// close_notify" (review of R5), which is the backend going away. A
    /// handshake that ends in EOF ("tls handshake eof": something accepted
    /// the connection and closed it, e.g. a relay whose far side is down)
    /// isn't a certificate problem either.
    fn from_causes(causes: &[String], connect: bool) -> Self {
        let plain = causes.iter().any(|cause| {
            let cause = cause.to_ascii_lowercase();
            // Only this variant means the first bytes weren't TLS at all; other
            // "corrupt message" kinds can come from a TLS server gone wrong.
            cause.contains("invalidcontenttype")
        });
        if connect && plain {
            return Self::NotHttps;
        }
        let secure = causes.iter().any(|cause| {
            let cause = cause.to_ascii_lowercase();
            // The exact tokio-rustls text: a bare "eof" would also match a
            // host such as `theoffice.lan` inside a not-valid-for-name message.
            !cause.contains("handshake eof")
                && [
                    "certificate",
                    "tls",
                    "ssl",
                    "handshake",
                    "received fatal alert",
                    "peer is incompatible",
                    "peer misbehaved",
                ]
                .iter()
                .any(|word| cause.contains(word))
        });
        if connect && secure {
            Self::Certificate
        } else if connect {
            Self::Connect
        } else {
            Self::Other
        }
    }
}

/// The host part of a backend URL, for messages (the text itself when it
/// doesn't parse).
fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| url.to_string())
}

/// A Tailscale name (`*.ts.net`) or a Tailscale address (100.64.0.0/10).
fn on_tailnet(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.ends_with(".ts.net") {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => {
            let [first, second, ..] = ip.octets();
            first == 100 && (second & 0xC0) == 64
        }
        Err(_) => false,
    }
}

/// What Settings says when the backend at `host` didn't answer at all: the
/// live status (`Client::send`), Test connection and Connect all use it. A
/// tailnet address also gets the likeliest cause, Tailscale being off on
/// this Mac (B05: "error sending request" named neither).
pub(crate) fn unreachable_words(host: &str, kind: Unreachable) -> String {
    let mut words = match kind {
        Unreachable::Timeout(secs) => format!(
            "{host} didn't answer within {secs} seconds. Check that the backend is running and this Mac can reach it."
        ),
        Unreachable::Connect | Unreachable::Other => format!(
            "Hollow couldn't reach {host}. Check the address, that the backend is running and that this Mac can reach it."
        ),
        // It did answer, so Tailscale being off isn't the cause. Settings
        // shows "Backend needs attention" for this, not "Can't reach".
        Unreachable::NotHttps => {
            // Plain http is only allowed to this Mac (`normalize_url`), so
            // only there is "use http://" a way out.
            return if is_this_mac(host) {
                format!(
                    "Hollow reached {host} but it didn't answer over HTTPS. If the backend serves plain HTTP, check whether the address should start with http:// instead."
                )
            } else {
                format!(
                    "Hollow reached {host} but it didn't answer over HTTPS. Hollow uses plain HTTP only to this Mac, so the backend needs to serve HTTPS (for example through tailscale serve or a reverse proxy)."
                )
            };
        }
        Unreachable::Certificate => {
            let mut words = format!(
                "Hollow reached {host} but couldn't verify its secure connection (certificate). Check the backend's HTTPS certificate, or the address."
            );
            if host.trim_end_matches('.').to_ascii_lowercase().ends_with(".ts.net") {
                words.push_str(
                    " If the backend uses tailscale serve, check that its certificate has been issued.",
                );
            }
            return words;
        }
    };
    if on_tailnet(host) {
        words.push_str(
            " If the backend is on your tailnet, check that Tailscale is running and connected on this Mac.",
        );
    }
    words
}

/// Check a backend without saving anything or touching the bridge's live
/// status: its health page (is anything there, and is it a companion
/// backend?), then its applier status with the token (does it take it?).
/// The second call is what the bridge itself sends every poll: with a good
/// token it claims or renews this Hollow's applier lease on that backend
/// (90 s), as connecting would. A refused token stops before the lease.
pub(crate) async fn test_backend(config: &BackendConfig, applier_id: &str) -> BackendTest {
    let fail = |message: String| BackendTest {
        ok: false,
        message,
        version: None,
    };
    let host = host_of(&config.url);
    let unreachable = |e: reqwest::Error| unreachable_words(&host, Unreachable::of(&e, 10));
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(http) => http,
        Err(e) => return fail(format!("Hollow couldn't start the test: {e}")),
    };
    let (status, health) = match http
        .get(format!("{}/api/v1/health", config.url))
        .send()
        .await
    {
        Ok(resp) => small_json(resp).await,
        Err(e) => return fail(unreachable(e)),
    };
    if status != 200 {
        // The daemon's guard answers 403/421 to a Mac or address it doesn't allow.
        let reason = health
            .get("error")
            .and_then(Value::as_str)
            .map(|r| format!(" ({r})"))
            .unwrap_or_default();
        return fail(format!(
            "{host} answered {status}{reason}. Check the address, and that this Mac may use that backend."
        ));
    }
    if health.get("ok").and_then(Value::as_bool) != Some(true) {
        return fail(format!(
            "{host} answered, but not as a Hollow companion backend. Check the address."
        ));
    }
    let version = health
        .get("version")
        .and_then(Value::as_str)
        .map(String::from);
    let (status, body) = match http
        .get(format!("{}/api/v1/applier/status", config.url))
        .bearer_auth(&config.token)
        .header("X-Hollow-Applier", applier_id)
        .header("X-Hollow-Bridge", BRIDGE_GENERATION)
        .header("X-Hollow-Kinds", companion_inbox::APPLIED_KINDS.join(","))
        .send()
        .await
    {
        Ok(resp) => small_json(resp).await,
        Err(e) => return fail(unreachable(e)),
    };
    let count = |key: &str| body.get(key).and_then(Value::as_u64).unwrap_or(0);
    let plural = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match status {
        200 => BackendTest {
            ok: true,
            message: format!(
                "Connected: the backend at {host} accepted the token. It has {} and {}.",
                plural(count("devices"), "paired device", "paired devices"),
                plural(count("mirrorTasks"), "shared task", "shared tasks"),
            ),
            version,
        },
        401 => fail(format!(
            "The backend at {host} answered but refused the applier token. Paste it again, or connect with a new code."
        )),
        code => fail(match body.get("error").and_then(Value::as_str) {
            Some(reason) if code == 409 => reason.to_string(),
            Some(reason) => format!("The backend at {host} answered {code}: {reason}"),
            None => format!("The backend at {host} answered {code}."),
        }),
    }
}

/// Test the typed connection, or the saved one when the URL is blank (the
/// person's Settings click; the COO's `companion.backend_test` masks the
/// token in its answer. It saves nothing).
#[tauri::command]
pub async fn companion_backend_test(url: String, token: String) -> Result<BackendTest, String> {
    let store = app_store()?;
    let config = test_target(&store, &url, &token)?;
    let id = applier_id(&store)?;
    Ok(test_backend(&config, &id).await)
}

/// What the backend may copy (Settings, Devices, Backend).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionScope {
    include_work_data: bool,
    seeded_at: Option<String>,
    workspaces: Vec<companion_docs::WorkspaceEntry>,
    /// `scope.json` couldn't be read (saving fixes it).
    error: Option<String>,
}

fn scope_view(vault: &Path, store: &Store) -> CompanionScope {
    let (scope, error) = companion_docs::load_scope_checked(store, vault);
    CompanionScope {
        include_work_data: scope.include_work_data,
        seeded_at: scope.seeded_at.clone(),
        workspaces: companion_docs::workspaces(vault, &scope),
        error,
    }
}

pub(crate) async fn scope_get(vault: PathBuf) -> Result<CompanionScope, String> {
    let store = app_store()?;
    tauri::async_runtime::spawn_blocking(move || scope_view(&vault, &store))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn companion_scope_get(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<CompanionScope, String> {
    scope_get(state.config.vault_path.clone()).await
}

/// Save what the backend copies: work data on or off, and the personal
/// workspaces (`None` keeps the saved list). Blocking.
pub(crate) fn scope_set_in(
    vault: &Path,
    store: &Store,
    include_work_data: bool,
    personal_workspaces: Option<Vec<String>>,
) -> Result<CompanionScope, String> {
    let (mut scope, _) = companion_docs::load_scope_checked(store, vault);
    scope.include_work_data = include_work_data;
    if let Some(personal) = personal_workspaces {
        scope.personal_workspaces = personal;
    }
    if scope.seeded_at.is_none() {
        scope.seeded_at = Some(now_rfc3339());
    }
    companion_docs::save_scope(store, &scope)?;
    Ok(scope_view(vault, store))
}

/// Save the scope off the main thread, then push: the next push applies it
/// (the daemon drops anything that left the scope at once).
pub(crate) async fn scope_set(
    vault: PathBuf,
    include_work_data: bool,
    personal_workspaces: Option<Vec<String>>,
) -> Result<CompanionScope, String> {
    let store = app_store()?;
    let view = tauri::async_runtime::spawn_blocking(move || {
        scope_set_in(&vault, &store, include_work_data, personal_workspaces)
    })
    .await
    .map_err(|e| e.to_string())??;
    repush();
    Ok(view)
}

/// The person's choice of what the backend copies.
#[tauri::command]
pub async fn companion_scope_set(
    state: tauri::State<'_, crate::commands::AppState>,
    include_work_data: bool,
    personal_workspaces: Vec<String>,
) -> Result<CompanionScope, String> {
    scope_set(
        state.config.vault_path.clone(),
        include_work_data,
        Some(personal_workspaces),
    )
    .await
}

// ── The COO's actions (L13) ──────────────────────────────────────────────
//
// The applier token lets a client act as this Hollow, and a claim code
// buys one, so both go in only (`Kind::Secret` on the action) and nothing
// the COO gets back holds them: `BackendStatus` never carries the token,
// and these mask it (and a claim code) even where a backend's answer
// quoted it back in an error or a test's sentence.

/// A backend's refusal in its own words, with the token the call carried
/// masked: made before the error is kept in the status, which outlives a
/// token change (L13 review).
pub(crate) fn backend_answered(code: u16, message: &str, token: &str) -> String {
    masked(format!("The backend answered {code}: {message}"), token)
}

/// `text` with `secret` masked as ••••, whatever its length (a blank
/// secret masks nothing). Any accepted token counts, however short (L13
/// delta review).
pub(crate) fn masked(text: String, secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        return text;
    }
    text.replace(secret, "••••")
}

/// `status` with `token` masked wherever an error quotes it.
pub(crate) fn scrubbed(mut status: BackendStatus, token: &str) -> BackendStatus {
    for field in [
        &mut status.last_error,
        &mut status.mirror_error,
        &mut status.docs_error,
        &mut status.scope_error,
    ] {
        *field = field.take().map(|text| masked(text, token));
    }
    status
}

/// The saved backend's address (a card's destination), if one is set up.
pub(crate) fn saved_backend_url() -> Option<String> {
    app_store()
        .ok()
        .and_then(|store| load_config(&store))
        .map(|c| c.url)
}

/// The backend status the COO reads: the saved token masked.
pub(crate) fn agent_backend_status() -> Result<BackendStatus, String> {
    let store = app_store()?;
    let token = load_config(&store).map(|c| c.token).unwrap_or_default();
    Ok(scrubbed(status(&store), &token))
}

/// Set the backend from the COO (after the person's card).
pub(crate) fn agent_backend_set(url: &str, token: &str) -> Result<BackendStatus, String> {
    backend_set(url, token).map_err(|e| masked(e, token))?;
    agent_backend_status()
}

/// The address a card shows for a typed backend URL: validated first (a
/// user name, password, query or fragment can hold a secret and is
/// refused), then only scheme://host[:port]/path.
pub(crate) fn card_backend_url(raw: &str) -> Result<String, String> {
    let url = normalize_url(raw)?;
    let parsed = reqwest::Url::parse(&url).map_err(|_| "That isn't a URL.".to_string())?;
    let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!(
        "{}://{}{port}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or(""),
        parsed.path().trim_end_matches('/')
    ))
}

/// Host-written claim failures that quote nothing a backend or a person
/// typed; an agent may read these as they are.
const CLAIM_SAFE_ERRORS: [&str; 3] = [
    "The backend's answer had no usable token.",
    "This backend can't be connected with a code yet; paste its applier token instead.",
    "The backend's answer was too large.",
];

/// What the COO reads when a claim fails: fixed words, never the backend's
/// own (it can quote the code back in any form, k7m3q9tx or K7M3-Q9TX).
pub(crate) fn agent_claim_error(error: &str) -> String {
    if CLAIM_SAFE_ERRORS.contains(&error) {
        return format!("{error} Nothing was saved.");
    }
    "The backend didn't trade that claim code for a token, or Hollow couldn't save it (claim-failed); nothing was saved. Check that the code is still current on the backend's setup page and that this Mac can reach the address. Connecting from Hollow's Settings shows the backend's own reason.".into()
}

/// Trade a claim code for the applier token from the COO (after the
/// person's card): the token is saved, never returned, and a failure is
/// one of the fixed sentences above.
pub(crate) async fn agent_backend_claim(url: &str, code: &str) -> Result<BackendStatus, String> {
    // Host-written refusals, which quote neither the address nor the code.
    let url = normalize_url(url)?;
    let code = normalize_claim_code(code).map_err(|_| {
        "That isn't a claim code: 8 letters and digits, as the backend's setup page shows it."
            .to_string()
    })?;
    backend_claim(&url, &code)
        .await
        .map_err(|e| agent_claim_error(&e))?;
    agent_backend_status()
}

/// Re-push the mirror from the COO.
pub(crate) fn agent_backend_repush() -> Result<BackendStatus, String> {
    backend_repush()?;
    agent_backend_status()
}

/// `test` with the token it used masked in every string it returns: the
/// sentence (it can quote the backend's reason), and the backend-controlled
/// version, kept only when it looks like one (at most 40 of letters,
/// digits and `.+_-`) and doesn't hold the token.
pub(crate) fn masked_test(mut test: BackendTest, token: &str) -> BackendTest {
    test.message = masked(test.message, token);
    test.version = test.version.filter(|v| {
        let secret = token.trim();
        !v.is_empty()
            && v.len() <= 40
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
            && (secret.is_empty() || !v.contains(secret))
    });
    test
}

/// Test a connection from the COO: the typed address and token, or the
/// saved connection when the address is blank. Saves nothing.
pub(crate) async fn agent_backend_test(url: &str, token: &str) -> Result<BackendTest, String> {
    let store = app_store()?;
    let config = test_target(&store, url, token).map_err(|e| masked(e, token))?;
    let id = applier_id(&store)?;
    Ok(masked_test(test_backend(&config, &id).await, &config.token))
}

/// The card's lines for a scope change: work data and the personal
/// workspaces, each old → new (the saved `scope.json`; none yet reads as
/// new), and a warning when it widens.
pub(crate) fn scope_change_lines(
    include_work_data: bool,
    personal_workspaces: Option<&[String]>,
) -> Vec<String> {
    let onoff = |on: bool| if on { "on" } else { "off" };
    let list = |ids: &[String]| {
        if ids.is_empty() {
            "none".to_string()
        } else {
            ids.join(", ")
        }
    };
    let mut lines = vec![
        "Change what the homelab backend may copy from this vault (the next push applies it)."
            .to_string(),
    ];
    // Read only: `load_scope_checked` seeds and saves a missing file, which
    // a card (built before anything is allowed) must never do.
    let old: Option<companion_docs::Scope> = app_store()
        .ok()
        .and_then(|store| std::fs::read_to_string(store.root().join("scope.json")).ok())
        .and_then(|text| serde_json::from_str(&text).ok());
    let mut widens = false;
    match &old {
        Some(old) => {
            widens |= include_work_data && !old.include_work_data;
            lines.push(format!(
                "Work data: {} → {}",
                onoff(old.include_work_data),
                onoff(include_work_data)
            ));
            if let Some(new) = personal_workspaces {
                widens |= new.iter().any(|id| !old.personal_workspaces.contains(id));
                lines.push(format!(
                    "Personal workspaces: {} → {}",
                    list(&old.personal_workspaces),
                    list(new)
                ));
            }
        }
        None => {
            widens = true;
            lines.push(format!("Work data: {}", onoff(include_work_data)));
            if let Some(new) = personal_workspaces {
                lines.push(format!("Personal workspaces: {}", list(new)));
            }
        }
    }
    if widens {
        lines.push("This widens it: more of the vault goes to the homelab.".into());
    }
    lines
}

/// `hollow companion-bridge-once <temp vault> <store> <url> <token> [CODE]`
/// (debug builds only): one bridge pass against a backend, for the
/// companion apps' end-to-end tests. Refuses any vault outside the temp
/// folder, like `companion-apply`.
#[cfg(debug_assertions)]
pub fn bridge_once_cli(
    vault: &Path,
    store: &Path,
    url: &str,
    token: &str,
    pair_code: Option<&str>,
) -> Result<Value, String> {
    let vault = companion_inbox::temp_vault_only(vault)?;
    if let Some(code) = pair_code {
        companion_inbox::open_pairing_with(code);
    }
    let config = BackendConfig {
        url: normalize_url(url)?,
        token: token.to_string(),
    };
    let store = Store::at(store.to_path_buf());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(run_once(&vault, &store, &config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion_inbox::{test_envelope, test_key, Device};

    /// A temp folder (`.0` is its path), removed on drop.
    struct TempDir(PathBuf, #[allow(dead_code)] tempfile::TempDir);
    impl TempDir {
        fn new(_tag: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            Self(dir.path().to_path_buf(), dir)
        }
    }

    const DEVICE: &str = "web-11223344";
    const TASK: &str =
        "---\ntitle: Call Jordan\nstatus: Not started\npriority: P2\n---\n\nNotes.\n";

    fn setup(tag: &str) -> (TempDir, TempDir, Store, Vec<u8>) {
        let vault = TempDir::new(&format!("{tag}-vault"));
        let support = TempDir::new(&format!("{tag}-store"));
        std::fs::create_dir_all(vault.0.join("tasks")).unwrap();
        std::fs::write(vault.0.join("tasks/call-jordan.md"), TASK).unwrap();
        let store = Store::at(support.0.clone());
        let (pkcs8, public_key) = test_key();
        store
            .save_devices(&[Device {
                device_id: DEVICE.into(),
                name: "Test browser".into(),
                platform: "web".into(),
                public_key,
                paired_at: "2026-10-12T00:00:00Z".into(),
                last_op_at: None,
            }])
            .unwrap();
        (vault, support, store, pkcs8)
    }

    fn comment_op(pkcs8: &[u8], op_id: &str) -> (String, companion_inbox::Envelope) {
        let env = test_envelope(
            pkcs8,
            DEVICE,
            op_id,
            json!({ "kind": "task.comment", "ref": "vault:tasks/call-jordan.md", "text": "From the web" })
                .as_object()
                .cloned()
                .unwrap(),
        );
        (serde_json::to_string(&env).unwrap(), env)
    }

    fn polled(seq: i64, op_id: &str, text: &str) -> PolledOp {
        PolledOp {
            seq,
            device_id: DEVICE.into(),
            op_id: op_id.into(),
            envelope: text.into(),
        }
    }

    fn comments(vault: &Path) -> usize {
        std::fs::read_to_string(vault.join("tasks/call-jordan.md"))
            .unwrap()
            .matches("From the web")
            .count()
    }

    #[test]
    fn an_op_on_both_roads_applies_once_whichever_comes_first() {
        // Backend first, then the same op as an inbox file.
        let (vault, _s, store, pkcs8) = setup("both-a");
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1A";
        let (text, env) = comment_op(&pkcs8, op);
        let applied = apply_ops(&vault.0, &store, &[polled(1, op, &text)], 0);
        assert_eq!(applied.receipts.len(), 1);
        assert_eq!(applied.receipts[0].status, "applied");
        assert_eq!(applied.cursor, 1);
        assert_eq!(comments(&vault.0), 1);
        companion_inbox::drop_op(&vault.0, &env);
        let summary = companion_inbox::process(&vault.0, &store, Local::now());
        assert_eq!(summary.applied, 1, "answered with the stored receipt");
        assert_eq!(comments(&vault.0), 1, "the inbox copy didn't apply again");

        // Inbox first, then the backend copy.
        let (vault, _s, store, pkcs8) = setup("both-b");
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1B";
        let (text, env) = comment_op(&pkcs8, op);
        companion_inbox::drop_op(&vault.0, &env);
        companion_inbox::process(&vault.0, &store, Local::now());
        assert_eq!(comments(&vault.0), 1);
        let applied = apply_ops(&vault.0, &store, &[polled(7, op, &text)], 0);
        assert_eq!(applied.receipts.len(), 1);
        assert_eq!(applied.receipts[0].status, "applied", "the stored receipt");
        assert_eq!(comments(&vault.0), 1, "the backend copy didn't apply again");
        // And the backend road repeats itself harmlessly.
        apply_ops(&vault.0, &store, &[polled(7, op, &text)], 0);
        assert_eq!(comments(&vault.0), 1);
    }

    #[test]
    fn a_tampered_backend_op_is_refused_and_doesnt_claim_its_op_id() {
        let (vault, _s, store, pkcs8) = setup("tampered");
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1C";
        let (text, _) = comment_op(&pkcs8, op);
        let forged = text.replace("From the web", "From a forger");
        let applied = apply_ops(&vault.0, &store, &[polled(1, op, &forged)], 0);
        assert_eq!(applied.receipts[0].status, "refused");
        assert_eq!(applied.cursor, 1);
        let applied = apply_ops(&vault.0, &store, &[polled(2, op, &text)], 1);
        assert_eq!(
            applied.receipts[0].status, "applied",
            "the genuine op still applies"
        );
        assert_eq!(comments(&vault.0), 1);
    }

    #[test]
    fn a_busy_vault_stops_the_batch_at_the_waiting_op() {
        let (vault, _s, store, pkcs8) = setup("busy");
        let a = "01JA9M3Q8Z5X7Y2W4V6T8S0R1D";
        let b = "01JA9M3Q8Z5X7Y2W4V6T8S0R1E";
        let (ta, _) = comment_op(&pkcs8, a);
        let (tb, _) = comment_op(&pkcs8, b);
        let lease = crate::automated_history::background_write_lease(&vault.0).unwrap();
        let applied = apply_ops(&vault.0, &store, &[polled(3, a, &ta), polled(4, b, &tb)], 2);
        assert!(applied.busy);
        assert_eq!(applied.cursor, 2, "poll again from before the waiting op");
        assert!(applied.receipts.is_empty());
        drop(lease);
        let applied = apply_ops(&vault.0, &store, &[polled(3, a, &ta), polled(4, b, &tb)], 2);
        assert!(!applied.busy);
        assert_eq!(applied.cursor, 4);
        assert_eq!(comments(&vault.0), 2);
    }

    fn envelope_text(pkcs8: &[u8], op_id: &str, payload: Value) -> String {
        let payload = payload.as_object().cloned().unwrap();
        let env = test_envelope(pkcs8, DEVICE, op_id, payload);
        serde_json::to_string(&env).unwrap()
    }

    fn cards(vault: &Path, kind: &str) -> Vec<crate::review_queue::ReviewItem> {
        crate::review_queue::list(vault, Some(kind)).unwrap()
    }

    #[test]
    fn a_repeated_conflict_refreshes_review_again_and_a_repeated_plain_op_does_not() {
        let (vault, _s, store, pkcs8) = setup("repeat-card");
        let task = vault_tasks::get(&vault.0, "tasks/call-jordan.md").unwrap();
        let moved = TASK.replace("title: Call Jordan", "title: Call Jordan soon");
        std::fs::write(vault.0.join("tasks/call-jordan.md"), moved).unwrap();
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1F";
        let update = json!({ "kind": "task.update", "ref": "vault:tasks/call-jordan.md", "baseHash": task.content_hash, "fields": { "title": { "from": "Call Jordan", "to": "Call Jordan back" } } });
        let text = envelope_text(&pkcs8, op, update);
        // The first cycle applied it and raised a card, then was cancelled
        // before it sent its Review refresh.
        let first = apply_ops(&vault.0, &store, &[polled(1, op, &text)], 0);
        assert_eq!(first.receipts[0].status, "conflict");
        assert_eq!(first.review_changed, 1);
        // The next poll hands the same op back: a repeat, which still
        // refreshes Review.
        let again = apply_ops(&vault.0, &store, &[polled(1, op, &text)], 0);
        assert_eq!(again.receipts[0].status, "conflict");
        assert!(again.review_changed >= 1);
        let conflicts = cards(&vault.0, companion_inbox::CONFLICT_KIND);
        assert_eq!(conflicts.len(), 1, "no second card");

        // A plain op repeated refreshes nothing.
        let plain = "01JA9M3Q8Z5X7Y2W4V6T8S0R1G";
        let (text, _) = comment_op(&pkcs8, plain);
        let first = apply_ops(&vault.0, &store, &[polled(2, plain, &text)], 1);
        assert_eq!(first.review_changed, 0);
        let again = apply_ops(&vault.0, &store, &[polled(2, plain, &text)], 1);
        assert_eq!(again.receipts[0].status, "applied");
        assert_eq!(again.review_changed, 0);
    }

    /// A comment op signed as sent 15 days before `now`.
    fn old_comment(pkcs8: &[u8], op_id: &str, now: chrono::DateTime<Local>) -> String {
        let sent = now.with_timezone(&Utc) - chrono::Duration::days(15);
        let at = sent.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let comment = json!({ "kind": "task.comment", "ref": "vault:tasks/call-jordan.md", "text": "From the web", "at": at });
        envelope_text(pkcs8, op_id, comment)
    }

    #[test]
    fn an_old_backend_op_waits_without_a_receipt_until_apply_it() {
        let (vault, _s, store, pkcs8) = setup("old-op");
        let now = Local::now();
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1H";
        let text = old_comment(&pkcs8, op, now);
        let held = apply_ops(&vault.0, &store, &[polled(5, op, &text)], 4);
        assert!(held.receipts.is_empty(), "the backend keeps it pending");
        assert_eq!((held.cursor, held.review_changed), (5, 1));
        assert_eq!(comments(&vault.0), 0);
        assert!(store.owed_receipts(10).unwrap().is_empty());
        let card = cards(&vault.0, companion_inbox::OLD_OP_KIND).remove(0);
        companion_inbox::accept_old_op(&vault.0, &store, &card, now).unwrap();
        assert_eq!(comments(&vault.0), 1);
        let receipts = vault.0.join(".hollow-sync/receipts");
        assert!(!receipts.exists(), "its receipt goes to the backend");
        // Owed to the daemon, which may have expired the op meanwhile and
        // would never hand it out again.
        let owed = store.owed_receipts(10).unwrap();
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].op_id, op);
        assert_eq!(owed[0].status, "applied");
        // A later copy is answered from the ledger.
        let answered = apply_ops(&vault.0, &store, &[polled(5, op, &text)], 0);
        assert_eq!(answered.receipts[0].status, "applied");
        assert_eq!(comments(&vault.0), 1);
    }

    #[test]
    fn a_removal_that_cannot_forget_the_receipts_keeps_the_backend() {
        let support = TempDir::new("forget-fails-store");
        let store = Store::at(support.0.clone());
        save_config(&store, "http://127.0.0.1:8787", "tok-123").unwrap();
        // A ledger that can't be opened: forgetting fails.
        std::fs::create_dir_all(support.0.join("applied.db")).unwrap();
        let err = save_config(&store, "", "").unwrap_err();
        assert!(
            err.starts_with("Couldn't remove the backend. Nothing changed. Hollow couldn't clear the replies it still owes that backend: "),
            "{err}"
        );
        assert!(
            config_path(&store).exists(),
            "still set up, so it can be removed again"
        );
        std::fs::remove_dir(support.0.join("applied.db")).unwrap();
        save_config(&store, "", "").unwrap();
        assert!(!config_path(&store).exists());
    }

    #[test]
    fn a_removal_that_cannot_delete_the_settings_file_says_to_turn_off_again() {
        let support = TempDir::new("delete-fails-store");
        let store = Store::at(support.0.clone());
        // A settings "file" that is a folder with something in it can't be
        // deleted as a file (and isn't missing either).
        std::fs::create_dir_all(config_path(&store).join("stuck")).unwrap();
        let err = save_config(&store, "", "").unwrap_err();
        assert!(
            err.starts_with("Hollow cleared the replies it owed the backend, but couldn't delete the backend's settings file. Choose Turn off again: "),
            "{err}"
        );
        // Trying again once the file can go finishes the job.
        std::fs::remove_dir_all(config_path(&store)).unwrap();
        save_config(&store, "", "").unwrap();
    }

    #[test]
    fn no_answer_reads_plainly_and_names_tailscale_only_for_tailnet_addresses() {
        let reach =
            "Check the address, that the backend is running and that this Mac can reach it.";
        let tailscale = " If the backend is on your tailnet, check that Tailscale is running and connected on this Mac.";
        for host in ["x.tail1234.ts.net", "100.101.1.2", "100.64.0.1"] {
            assert_eq!(
                unreachable_words(host, Unreachable::Connect),
                format!("Hollow couldn't reach {host}. {reach}{tailscale}")
            );
        }
        // Outside 100.64.0.0/10 on either side, a LAN address and a public name.
        for host in ["100.128.0.1", "100.0.0.1", "192.168.0.20", "example.com"] {
            let plain = format!("Hollow couldn't reach {host}. {reach}");
            assert_eq!(unreachable_words(host, Unreachable::Connect), plain);
            assert_eq!(unreachable_words(host, Unreachable::Other), plain);
        }
        let waited = "Check that the backend is running and this Mac can reach it.";
        assert_eq!(
            unreachable_words("192.168.0.20", Unreachable::Timeout(10)),
            format!("192.168.0.20 didn't answer within 10 seconds. {waited}")
        );
        assert_eq!(
            unreachable_words("x.tail1234.ts.net", Unreachable::Timeout(35)),
            format!("x.tail1234.ts.net didn't answer within 35 seconds. {waited}{tailscale}")
        );
        assert_eq!(
            host_of("https://homelab.tail1234.ts.net/"),
            "homelab.tail1234.ts.net"
        );
        assert_eq!(host_of("not a url"), "not a url");
    }

    /// R6: an `https://` address in front of a plain-HTTP server answered,
    /// so it isn't "couldn't reach", and it isn't a certificate problem.
    #[test]
    fn an_https_address_on_a_plain_http_server_says_so() {
        let chain = |causes: &[&str]| causes.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let causes = chain(&[
            "client error (Connect)",
            "received corrupt message of type InvalidContentType",
        ]);
        assert_eq!(
            Unreachable::from_causes(&causes, true),
            Unreachable::NotHttps
        );
        // Only while connecting: the same text later is something else.
        assert_eq!(Unreachable::from_causes(&causes, false), Unreachable::Other);
        for host in ["homelab.tail1234.ts.net", "192.168.0.20"] {
            let words = unreachable_words(host, Unreachable::NotHttps);
            assert!(
                words.starts_with(&format!("Hollow reached {host} but")),
                "{words}"
            );
            // Away from this Mac plain http is refused (`normalize_url`), so
            // the words never send the person there.
            assert!(!words.contains("http://"), "{words}");
            assert!(words.contains("needs to serve HTTPS"), "{words}");
            assert!(!words.contains("Tailscale is running"), "{words}");
            assert!(normalize_url(&format!("http://{host}:8787")).is_err());
        }
        // `host_of` gives the bracketed, compressed form `normalize_url` sees.
        assert_eq!(host_of("https://[0:0:0:0:0:0:0:1]:8787"), "[::1]");
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            let words = unreachable_words(host, Unreachable::NotHttps);
            assert!(words.contains("http://"), "{words}");
            assert!(normalize_url(&format!("http://{host}:8787")).is_ok());
        }
        // Other corrupt-message kinds come from a TLS server gone wrong.
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "client error (Connect)",
                    "received corrupt message of type MissingData"
                ]),
                true
            ),
            Unreachable::Connect
        );
    }

    #[test]
    fn a_bad_certificate_is_not_called_unreachable() {
        let chain = |causes: &[&str]| causes.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        // reqwest counts both as connect errors; only the causes tell them apart.
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "error sending request",
                    "client error (Connect)",
                    "invalid peer certificate: Expired",
                ]),
                true,
            ),
            Unreachable::Certificate
        );
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "client error (Connect)",
                    "received fatal alert: HandshakeFailure"
                ]),
                true,
            ),
            Unreachable::Certificate
        );
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "error sending request",
                    "tcp connect error",
                    "Connection refused (os error 61)",
                ]),
                true,
            ),
            Unreachable::Connect
        );
        // Review of R5: an HTTPS connection dropped after the handshake (the
        // backend restarting mid-poll) is not a certificate problem, and
        // neither is a handshake cut short by EOF.
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "client error (SendRequest)",
                    "connection error",
                    // Shortened on purpose: the real text ends `#unexpected-eof`,
                    // which would pass without the connect check.
                    "peer closed connection without sending TLS close_notify: https://docs.rs/rustls",
                ]),
                false,
            ),
            Unreachable::Other
        );
        assert_eq!(
            Unreachable::from_causes(
                &chain(&["client error (Connect)", "tls handshake eof"]),
                true,
            ),
            Unreachable::Connect
        );
        assert_eq!(
            Unreachable::from_causes(
                &chain(&[
                    "client error (Connect)",
                    "peer is incompatible: Tls12NotOffered"
                ]),
                true,
            ),
            Unreachable::Certificate
        );
        assert_eq!(
            Unreachable::from_causes(
                &chain(&["connection closed before message completed"]),
                false,
            ),
            Unreachable::Other
        );

        let unverified = "but couldn't verify its secure connection (certificate).";
        let checked = "Check the backend's HTTPS certificate, or the address.";
        let serve =
            "If the backend uses tailscale serve, check that its certificate has been issued.";
        for host in ["192.168.0.20", "100.101.1.2"] {
            let words = unreachable_words(host, Unreachable::Certificate);
            assert_eq!(
                words,
                format!("Hollow reached {host} {unverified} {checked}")
            );
            assert!(!words.contains("Tailscale is running"), "{words}");
        }
        let host = "x.tail1234.ts.net";
        let words = unreachable_words(host, Unreachable::Certificate);
        assert_eq!(
            words,
            format!("Hollow reached {host} {unverified} {checked} {serve}")
        );
        assert!(!words.contains("Tailscale is running"), "{words}");
        // Settings' "Can't reach the backend" badge keys on these openings.
        assert!(!words.starts_with("Hollow couldn't reach"));
        assert!(!words.contains("didn't answer within"));
    }

    #[tokio::test]
    async fn the_live_status_says_the_backend_could_not_be_reached() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let client = Client::new(
            &config(&format!("http://127.0.0.1:{port}")),
            "applier-abcdef12".into(),
        )
        .unwrap();
        let err = client.poll(5, 0).await.unwrap_err();
        assert_eq!(
            err,
            "Hollow couldn't reach 127.0.0.1. Check the address, that the backend is running and that this Mac can reach it."
        );
    }

    #[test]
    fn removing_the_backend_forgets_the_receipts_owed_to_it() {
        let (vault, _s, store, pkcs8) = setup("forget-backend");
        let now = Local::now();
        let (decided, waiting) = ("01JA9M3Q8Z5X7Y2W4V6T8S0R1H", "01JA9M3Q8Z5X7Y2W4V6T8S0R2J");
        let ops = [
            polled(5, decided, &old_comment(&pkcs8, decided, now)),
            polled(6, waiting, &old_comment(&pkcs8, waiting, now)),
        ];
        assert_eq!(apply_ops(&vault.0, &store, &ops, 4).cursor, 6);
        let card = |op: &str| {
            cards(&vault.0, companion_inbox::OLD_OP_KIND)
                .into_iter()
                .find(|card| card.payload["opId"] == op)
                .unwrap()
        };
        companion_inbox::accept_old_op(&vault.0, &store, &card(decided), now).unwrap();
        assert_eq!(store.owed_receipts(10).unwrap().len(), 1);
        save_config(&store, "http://127.0.0.1:8787", "tok-123").unwrap();
        save_config(&store, "", "").unwrap();
        assert!(
            store.owed_receipts(10).unwrap().is_empty(),
            "nothing is owed to a removed backend"
        );
        // An op that backend handed over, decided after it went, owes
        // nothing either (the next backend never held it).
        companion_inbox::discard_old_op(&vault.0, &store, &card(waiting), now).unwrap();
        assert!(store.owed_receipts(10).unwrap().is_empty());
        assert_eq!(comments(&vault.0), 1);
    }

    #[test]
    fn backend_pairing_uses_the_code_open_in_hollow() {
        let _serial = companion_inbox::pairing_test_lock();
        let (_vault, _s, store, _) = setup("pair");
        let (_, public_key) = test_key();
        let request = |id: &str, proof: &str| {
            json!({ "v": 1, "deviceId": id, "name": "Safari on iPhone", "platform": "web",
                    "publicKey": public_key, "at": "2026-10-12T23:00:00Z", "proof": proof })
            .to_string()
        };
        // No code open: refused with the reason the device shows.
        companion_inbox::close_pairing();
        let refused = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-aaaaaaaa".into(),
                request: request("web-aaaaaaaa", "AAAA"),
                ticket: "t1".into(),
            }],
        );
        assert!(refused[0].as_ref().unwrap_err().contains("No pairing code"));
        let code = companion_inbox::open_pairing(Utc::now()).code;
        let proof = companion_inbox::pair_proof(
            &companion_inbox::normalize_code(&code),
            "web-aaaaaaaa",
            &public_key,
        );
        // The file name and deviceId must agree.
        let mismatch = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-bbbbbbbb".into(),
                request: request("web-aaaaaaaa", &proof),
                ticket: "t2".into(),
            }],
        );
        assert!(mismatch[0].is_err());
        let paired = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-aaaaaaaa".into(),
                request: request("web-aaaaaaaa", &proof),
                ticket: "t3".into(),
            }],
        );
        assert_eq!(paired[0].as_deref(), Ok("Safari on iPhone"));
        assert!(store
            .devices()
            .iter()
            .any(|d| d.device_id == "web-aaaaaaaa"));
        assert!(
            companion_inbox::open_code(Utc::now()).is_none(),
            "the code works once"
        );
        // Replaying a paired device's id and key over the backend (anyone who
        // can reach the daemon could) still needs a fresh code.
        let replay = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-aaaaaaaa".into(),
                request: request("web-aaaaaaaa", &proof),
                ticket: "t4".into(),
            }],
        );
        assert!(replay[0].is_err(), "{replay:?}");
        let code = companion_inbox::open_pairing(Utc::now()).code;
        let proof = companion_inbox::pair_proof(
            &companion_inbox::normalize_code(&code),
            "web-aaaaaaaa",
            &public_key,
        );
        let again = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-aaaaaaaa".into(),
                request: request("web-aaaaaaaa", &proof),
                ticket: "t5".into(),
            }],
        );
        assert!(again[0].is_ok(), "re-pairing with a new code works");
        // The answer was lost and the same request (ticket) comes back: the
        // same decision, not a refusal against the used-up code.
        assert!(companion_inbox::open_code(Utc::now()).is_none());
        let resent = decide_pairs(
            &store,
            &[PolledPair {
                device_id: "web-aaaaaaaa".into(),
                request: request("web-aaaaaaaa", &proof),
                ticket: "t5".into(),
            }],
        );
        assert!(resent[0].is_ok(), "{resent:?}");
        assert_eq!(
            store
                .devices()
                .iter()
                .filter(|d| d.device_id == "web-aaaaaaaa")
                .count(),
            1
        );
    }

    #[test]
    fn the_mirror_holds_task_files_only_and_sends_what_changed() {
        let (vault, _s, _store, _) = setup("mirror");
        std::fs::create_dir_all(vault.0.join("tasks/.trash")).unwrap();
        std::fs::write(vault.0.join("tasks/.trash/old.md"), TASK).unwrap();
        std::fs::create_dir_all(vault.0.join("library/work")).unwrap();
        std::fs::write(vault.0.join("library/work/plan.md"), TASK).unwrap();
        std::fs::create_dir_all(vault.0.join("me")).unwrap();
        std::fs::write(vault.0.join("me/Identity.md"), "# me").unwrap();
        let mut pushed = Pushed::default();
        let full = mirror_batches(&vault.0, &pushed, true);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0]["complete"], true);
        let paths: Vec<&str> = full[0]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, vec!["tasks/call-jordan.md"]);
        let item = &full[0]["tasks"][0];
        assert_eq!(item["task"]["ref"], "vault:tasks/call-jordan.md");
        assert_eq!(item["task"]["title"], "Call Jordan");
        assert_eq!(item["contentHash"], vault_tasks::content_hash(TASK));
        assert_eq!(item["content"], TASK);
        let text = serde_json::to_string(&full).unwrap();
        assert!(!text.contains("Identity") && !text.contains("library/"));
        note_pushed(&mut pushed, &full[0]);

        // Nothing changed: nothing to send.
        assert!(mirror_batches(&vault.0, &pushed, false).is_empty());
        // An edit and a new file go; a deleted file is removed.
        std::fs::write(
            vault.0.join("tasks/call-jordan.md"),
            TASK.replace("P2", "P1"),
        )
        .unwrap();
        std::fs::write(
            vault.0.join("tasks/new.md"),
            TASK.replace("Call Jordan", "New one"),
        )
        .unwrap();
        let batches = mirror_batches(&vault.0, &pushed, false);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["tasks"].as_array().unwrap().len(), 2);
        for b in &batches {
            note_pushed(&mut pushed, b);
        }
        std::fs::remove_file(vault.0.join("tasks/new.md")).unwrap();
        let batches = mirror_batches(&vault.0, &pushed, false);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["removed"], json!(["tasks/new.md"]));
        assert!(batches[0]["projects"].is_null());
    }

    #[test]
    fn mirrorable_paths() {
        assert!(mirrorable("tasks/a.md"));
        assert!(mirrorable("tasks/archive/a.md"));
        for bad in [
            "tasks/.trash/a.md",
            "tasks/.a.md",
            "me/a.md",
            "library/work/a.md",
            "tasks/a.txt",
            "tasks",
        ] {
            assert!(!mirrorable(bad), "{bad}");
        }
    }

    #[test]
    fn claim_codes_are_normalized() {
        assert_eq!(normalize_claim_code("k7m3-q9tx").unwrap(), "K7M3Q9TX");
        assert_eq!(normalize_claim_code(" K7M3 Q9TX ").unwrap(), "K7M3Q9TX");
        assert!(normalize_claim_code("K7M3-Q9T").is_err());
        assert!(
            normalize_claim_code("K7M3-Q9TI").is_err(),
            "I isn't Crockford"
        );
        assert!(normalize_claim_code("../../x").is_err());
    }

    #[test]
    fn backend_urls_and_settings() {
        assert_eq!(
            normalize_url("http://127.0.0.1:8787/").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert!(normalize_url("http://localhost:8787").is_ok());
        assert!(normalize_url("https://hollow.tail1234.ts.net").is_ok());
        for bad in [
            "http://192.168.0.20:8787",
            "http://hollow.example.com",
            "ftp://127.0.0.1",
            "https://user:pw@host",
            "https://host/?token=x",
            "not a url",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
        let support = TempDir::new("settings");
        let store = Store::at(support.0.clone());
        assert!(load_config(&store).is_none());
        assert!(
            save_config(&store, "http://127.0.0.1:8787", "").is_err(),
            "a token is needed first"
        );
        save_config(&store, "http://127.0.0.1:8787/", "tok-123").unwrap();
        assert_eq!(
            load_config(&store),
            Some(BackendConfig {
                url: "http://127.0.0.1:8787".into(),
                token: "tok-123".into()
            })
        );
        save_config(&store, "http://127.0.0.1:8787", "").unwrap();
        assert_eq!(
            load_config(&store).unwrap().token,
            "tok-123",
            "an empty token keeps the saved one"
        );
        assert!(
            save_config(&store, "https://elsewhere.example", "").is_err(),
            "the token is never sent to a new origin without being pasted again"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path(&store))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let shown = serde_json::to_string(&status(&store)).unwrap();
        assert!(
            !shown.contains("tok-123"),
            "the status never carries the token"
        );
        save_config(&store, "", "").unwrap();
        assert!(load_config(&store).is_none());
    }

    #[test]
    fn refusals_name_the_path_and_batches_cap_their_count() {
        let batch = json!({ "docs": [{ "path": "notes/x.md", "hash": "h1" }, { "path": "notes/y.md", "hash": "h2" }] });
        assert_eq!(
            refused_doc(
                "The backend answered 422: `notes/y.md` is over 256 KiB; send it truncated",
                &batch
            ),
            Some(("notes/y.md".into(), "h2".into()))
        );
        // A refusal about the batch itself names no document: back off instead.
        assert_eq!(
            refused_doc(
                "The backend answered 422: `keep` belongs to a complete batch",
                &batch
            ),
            None
        );
        assert_eq!(
            refused_doc("The backend answered 500: `notes/x.md`", &batch),
            None
        );
        let items: Vec<Value> = (0..1200)
            .map(|i| json!({ "path": format!("notes/{i}.md") }))
            .collect();
        let chunks = chunk_docs(items);
        assert_eq!(
            chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![500, 500, 200]
        );
    }

    #[test]
    fn document_batches_send_what_changed_removals_and_complete_pushes() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let write = |rel: &str, text: &str| {
            let p = v.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("notes/a.md", "a");
        write("notes/b.md", "b");
        write("workspaces/hollow-1/PLAN.md", "plan");
        write("me/Identity.md", "never");
        let scope = companion_docs::Scope {
            include_work_data: false,
            personal_workspaces: vec!["hollow-1".into()],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let personal = vec!["hollow-1".to_string()];
        let local = companion_docs::collect(v, &scope, &mut cache).0;
        let mut pushed: HashMap<String, String> = HashMap::new();
        let batches = docs_batches(v, &local, &pushed, "personal", &personal, false);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["scope"], "personal");
        assert_eq!(batches[0]["personalWorkspaces"], json!(["hollow-1"]));
        let paths: Vec<&str> = batches[0]["docs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec!["notes/a.md", "notes/b.md", "workspaces/hollow-1/PLAN.md"]
        );
        note_docs_pushed(&mut pushed, &batches[0]);
        // Nothing changed: nothing to send.
        assert!(docs_batches(v, &local, &pushed, "personal", &personal, false).is_empty());
        // One edit, one removal.
        write("notes/a.md", "a2");
        std::fs::remove_file(v.join("notes/b.md")).unwrap();
        let local = companion_docs::collect(v, &scope, &mut cache).0;
        let batches = docs_batches(v, &local, &pushed, "personal", &personal, false);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0]["docs"][0]["path"], "notes/a.md");
        assert_eq!(batches[0]["docs"][0]["content"], "a2");
        assert_eq!(batches[1]["removed"], json!(["notes/b.md"]));
        for b in &batches {
            note_docs_pushed(&mut pushed, b);
        }
        assert_eq!(pushed.len(), 2);
        // A full push: complete, keeping what wasn't resent.
        let batches = docs_batches(v, &local, &pushed, "personal", &personal, true);
        let last = batches.last().unwrap();
        assert_eq!(last["complete"], true);
        assert_eq!(
            last["keep"],
            json!(["notes/a.md", "workspaces/hollow-1/PLAN.md"])
        );
        // Work scope never sends a personal list.
        let batches = docs_batches(v, &local, &HashMap::new(), "work", &personal, false);
        assert_eq!(batches[0]["personalWorkspaces"], json!([]));
    }

    #[test]
    fn the_mirror_skips_what_the_backend_would_refuse_and_batches_by_size() {
        let (vault, _s, _store, _) = setup("limits");
        // Over the daemon's 256 KiB, and a backslash in the name: skipped,
        // so they can never get a whole batch refused.
        let big = format!("{TASK}{}", "x".repeat(300 * 1024));
        std::fs::write(vault.0.join("tasks/big.md"), big).unwrap();
        std::fs::write(vault.0.join("tasks/odd\\name.md"), TASK).unwrap();
        let full = mirror_batches(&vault.0, &Pushed::default(), true);
        let text = serde_json::to_string(&full).unwrap();
        assert!(
            !text.contains("tasks/big.md") && !text.contains("odd"),
            "{}",
            &text[..200]
        );
        assert!(!mirrorable("tasks/odd\\name.md"));
        assert!(!mirrorable(&format!("tasks/{}.md", "a".repeat(600))));

        // Many tasks: several requests under the cap, the last one complete
        // and keeping what the earlier ones sent.
        let body = "y".repeat(200 * 1024);
        for n in 0..12 {
            std::fs::write(
                vault.0.join(format!("tasks/t{n:02}.md")),
                format!("---\ntitle: T{n}\nstatus: Not started\n---\n\n{body}\n"),
            )
            .unwrap();
        }
        let full = mirror_batches(&vault.0, &Pushed::default(), true);
        assert!(full.len() >= 3, "{} batches", full.len());
        for batch in &full {
            assert!(batch.to_string().len() <= MIRROR_BATCH_BYTES + 256 * 1024);
        }
        let last = full.last().unwrap();
        assert_eq!(last["complete"], true);
        let sent_before: usize = full[..full.len() - 1]
            .iter()
            .map(|b| b["tasks"].as_array().unwrap().len())
            .sum();
        assert_eq!(last["keep"].as_array().unwrap().len(), sent_before);
        let mut pushed = Pushed::default();
        for batch in &full {
            note_pushed(&mut pushed, batch);
        }
        assert_eq!(pushed.tasks.len(), 13, "12 new tasks and call-jordan");
        assert!(mirror_batches(&vault.0, &pushed, false).is_empty());
    }

    // ── Test connection (ST5) ────────────────────────────────────────────

    use std::io::{Read, Write};

    /// One scripted answer per connection; the requests as received.
    fn serve(responses: Vec<String>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&requests);
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let raw = read_request(&mut stream);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw).into_owned());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base, requests)
    }

    /// One request as received: its head, then the body its Content-Length
    /// names (so a test can read what was posted).
    fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let (mut raw, mut chunk) = (Vec::new(), [0u8; 2048]);
        let mut whole: Option<usize> = None;
        while whole.is_none_or(|whole| raw.len() < whole) {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
            }
            let end = raw.windows(4).position(|w| w == b"\r\n\r\n");
            if let (None, Some(end)) = (whole, end) {
                let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                whole = Some(end + 4 + length);
            }
        }
        raw
    }

    fn answer(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    const HEALTH: &str = r#"{"ok":true,"version":"0.4.2","applierSeenAt":null}"#;

    fn config(url: &str) -> BackendConfig {
        BackendConfig {
            url: url.into(),
            token: "tok-123".into(),
        }
    }

    #[tokio::test]
    async fn a_test_that_works_says_so_plainly_and_sends_the_token_as_the_bridge_does() {
        let (base, requests) = serve(vec![
            answer("200 OK", HEALTH),
            answer(
                "200 OK",
                r#"{"devices":1,"pendingOps":0,"pendingPairs":0,"mirrorTasks":177}"#,
            ),
        ]);
        let result = test_backend(&config(&base), "applier-abcdef12").await;
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.message,
            "Connected: the backend at 127.0.0.1 accepted the token. It has 1 paired device and 177 shared tasks."
        );
        assert_eq!(result.version.as_deref(), Some("0.4.2"));
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /api/v1/health "));
        assert!(
            !requests[0].to_ascii_lowercase().contains("authorization"),
            "health goes without the token"
        );
        let status = requests[1].to_ascii_lowercase();
        assert!(status.starts_with("get /api/v1/applier/status "));
        assert!(status.contains("authorization: bearer tok-123"));
        assert!(status.contains("x-hollow-applier: applier-abcdef12"));
        assert!(status.contains(&format!("x-hollow-bridge: {BRIDGE_GENERATION}")));
        assert!(status.contains("x-hollow-kinds: "));
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_so_the_token_never_moves() {
        let (elsewhere, reached) = serve(vec![answer("200 OK", HEALTH), answer("200 OK", "{}")]);
        let moved = format!("HTTP/1.1 302 Found\r\nLocation: {elsewhere}/api/v1/applier/status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (base, _) = serve(vec![moved]);
        let result = test_backend(&config(&base), "applier-abcdef12").await;
        assert!(!result.ok);
        assert_eq!(
            result.message,
            "127.0.0.1 answered 302. Check the address, and that this Mac may use that backend."
        );
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            reached.lock().unwrap().is_empty(),
            "nothing followed the redirect"
        );
    }

    #[tokio::test]
    async fn a_refused_token_another_applier_and_a_wrong_address_each_read_plainly() {
        let (base, _) = serve(vec![
            answer("200 OK", HEALTH),
            answer(
                "401 Unauthorized",
                r#"{"error":"This route needs the applier token."}"#,
            ),
        ]);
        let refused = test_backend(&config(&base), "applier-abcdef12").await;
        assert!(!refused.ok);
        assert!(
            refused.message.contains("refused the applier token"),
            "{}",
            refused.message
        );

        let busy = r#"{"error":"Another Hollow is this backend's applier. Quit it (or turn its backend off), then wait 90 seconds."}"#;
        let (base, _) = serve(vec![answer("200 OK", HEALTH), answer("409 Conflict", busy)]);
        let other = test_backend(&config(&base), "applier-abcdef12").await;
        assert!(!other.ok);
        assert!(other
            .message
            .starts_with("Another Hollow is this backend's applier."));

        // Something else answers there (a web page, not the daemon).
        let page = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 15\r\nConnection: close\r\n\r\n<html></html>\r\n".to_string();
        let (base, requests) = serve(vec![page]);
        let wrong = test_backend(&config(&base), "applier-abcdef12").await;
        assert!(!wrong.ok);
        assert_eq!(
            wrong.message,
            "127.0.0.1 answered, but not as a Hollow companion backend. Check the address."
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "no token is sent to something that isn't the daemon"
        );

        // The daemon's guard refusing this Mac says why.
        let (base, _) = serve(vec![answer(
            "403 Forbidden",
            r#"{"error":"not this tailnet user"}"#,
        )]);
        let guarded = test_backend(&config(&base), "applier-abcdef12").await;
        assert_eq!(guarded.message, "127.0.0.1 answered 403 (not this tailnet user). Check the address, and that this Mac may use that backend.");
    }

    #[tokio::test]
    async fn nothing_listening_is_a_plain_cannot_reach() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = test_backend(
            &config(&format!("http://127.0.0.1:{port}")),
            "applier-abcdef12",
        )
        .await;
        assert!(!result.ok);
        assert!(
            result
                .message
                .starts_with("Hollow couldn't reach 127.0.0.1."),
            "{}",
            result.message
        );
    }

    const EMPTY_POLL: &str = r#"{"ops":[],"pairs":[]}"#;

    /// One cycle against the backend at `base` (polling after seq 5, without
    /// waiting).
    async fn cycle_at(base: &str, vault: &Path, store: &Store) -> CycleOutcome {
        let client = Client::new(&config(base), "applier-abcdef12".into()).unwrap();
        let out = cycle(&client, vault, store, 5, 0, &|_, _| {}).await;
        out.unwrap()
    }

    #[tokio::test]
    async fn a_decided_backend_op_is_owed_until_the_backend_stores_its_receipt() {
        let (vault, _s, store, pkcs8) = setup("owed");
        let now = Local::now();
        let op = "01JA9M3Q8Z5X7Y2W4V6T8S0R1J";
        let text = old_comment(&pkcs8, op, now);
        apply_ops(&vault.0, &store, &[polled(5, op, &text)], 4);
        let card = cards(&vault.0, companion_inbox::OLD_OP_KIND).remove(0);
        companion_inbox::discard_old_op(&vault.0, &store, &card, now).unwrap();
        assert_eq!(store.owed_receipts(10).unwrap().len(), 1);

        // The daemon fails the post: the receipt stays owed, and the poll
        // still runs.
        let failed = answer("500 Internal Server Error", r#"{"error":"disk full"}"#);
        let (base, requests) = serve(vec![failed, answer("200 OK", EMPTY_POLL)]);
        let out = cycle_at(&base, &vault.0, &store).await;
        assert_eq!(out.receipts, 0);
        let owed = store.owed_receipts(10).unwrap();
        assert_eq!(owed.len(), 1, "kept for the next cycle");
        {
            let requests = requests.lock().unwrap();
            assert!(requests[0].starts_with("POST /api/v1/applier/receipts "));
            assert!(requests[1].contains("/poll?after=5&wait=0"));
        }

        // The daemon stores it (it replaces an `expired` receipt): owed no
        // longer.
        let stored = answer("200 OK", r#"{"stored":1}"#);
        let (base, requests) = serve(vec![stored, answer("200 OK", EMPTY_POLL)]);
        let out = cycle_at(&base, &vault.0, &store).await;
        assert_eq!(out.receipts, 1);
        assert!(store.owed_receipts(10).unwrap().is_empty());
        let posted = requests.lock().unwrap()[0].clone();
        assert!(posted.contains(op), "{posted}");
        assert!(posted.contains(r#""status":"refused""#), "{posted}");

        // Nothing is owed now: the next cycle only polls.
        let (base, requests) = serve(vec![answer("200 OK", EMPTY_POLL)]);
        cycle_at(&base, &vault.0, &store).await;
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /api/v1/applier/poll"));
    }

    #[test]
    fn a_test_uses_the_typed_connection_or_the_saved_one_and_never_moves_a_token() {
        let support = TempDir::new("backend-test");
        let store = Store::at(support.0.clone());
        assert_eq!(
            test_target(&store, "", "").unwrap_err(),
            "No backend is set up."
        );
        save_config(&store, "http://127.0.0.1:8787", "tok-123").unwrap();
        assert_eq!(
            test_target(&store, "", "").unwrap(),
            config("http://127.0.0.1:8787")
        );
        // The saved address with a blank token keeps the saved token…
        assert_eq!(
            test_target(&store, "http://127.0.0.1:8787/", " ").unwrap(),
            config("http://127.0.0.1:8787")
        );
        // …another address needs its own token, and a pasted one is used as typed.
        assert!(test_target(&store, "https://elsewhere.example", "")
            .unwrap_err()
            .starts_with("Paste the backend's applier token"));
        assert_eq!(
            test_target(&store, "https://elsewhere.example", "tok-9")
                .unwrap()
                .token,
            "tok-9"
        );
        assert!(
            test_target(&store, "http://example.com", "tok-9").is_err(),
            "plain http stays local-only"
        );
        assert_eq!(
            test_target(&store, "http://127.0.0.1:8787", "a b").unwrap_err(),
            "That doesn't look like an applier token."
        );
        // Testing saves nothing.
        assert_eq!(
            load_config(&store).unwrap(),
            config("http://127.0.0.1:8787")
        );
    }
}
