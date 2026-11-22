//! Companion apps, phase T0: the applier for signed device operations
//! arriving through `.hollow-sync/` in the vault (the local / iCloud road).
//! The contract is `docs/COMPANION_PROTOCOL.md`; the design is the vault
//! workspace's `outputs/companion-apps-2026-10/design.md`.
//!
//! Hollow stays the only writer of vault Markdown. A paired device (the Mac
//! menu-bar app now; the web app and phones later) writes a write-once,
//! signed op file into `.hollow-sync/inbox/<deviceId>/<opId>.json`; this
//! module verifies the signature against the key stored OUTSIDE the vault
//! (`<support>/Companion/devices.json`), applies the op once (the applied-op
//! ledger `<support>/Companion/applied.db` dedups by opId) through
//! [`task_broker::dispatch_companion`] (the agents' validation, hash checks
//! and tracker edit lock, under a short companion lease), merges a stale
//! edit per field, sends a field both sides changed
//! to Waiting on you as a `companion_conflict` card, and writes a receipt.
//! An op whose signed `at` is more than 14 days old is held instead (its
//! exact envelope kept in the ledger's database) behind a `companion_old_op`
//! card: **Apply it** applies it once then, **Discard it** answers it refused.
//!
//! **Trust.** An op is the person acting, so it may do what agents may not.
//! The signature is the boundary: an agent can write a file under
//! `.hollow-sync/` (Hollow's own write gates refuse it, see
//! `run_guard::is_protected_path`, but a CLI's own tools can't be stopped),
//! yet it can't sign as a paired device. Pairing needs a code shown only in
//! Hollow's Settings and kept only in memory.

use crate::task_broker::{self, CompanionRefusal};
use crate::types::Task;
use crate::vault_tasks;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Duration, Local, SecondsFormat, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

/// The vault-root folder devices and Hollow exchange files through.
pub(crate) const SYNC_DIR: &str = ".hollow-sync";
/// Raised after a pass that changed something (Settings, Devices).
pub(crate) const EVENT: &str = "companion-changed";
pub(crate) const CONFLICT_KIND: &str = "companion_conflict";
/// An op sent more than [`OLD_OP_DAYS`] days before Hollow saw it: held
/// until the person applies or discards it (design §3.6).
pub(crate) const OLD_OP_KIND: &str = "companion_old_op";
/// Ops whose signed `at` is older than this many days ask first.
pub(crate) const OLD_OP_DAYS: i64 = 14;
const SOURCE: &str = "companion";
const MAX_ENVELOPE_BYTES: u64 = 64 * 1024;
const MAX_PAIR_BYTES: u64 = 4 * 1024;
const CODE_LEN: usize = 8;
const CODE_TTL_MINUTES: i64 = 10;
const CODE_MAX_FAILURES: u32 = 5;
const RECEIPT_KEEP_DAYS: u64 = 30;
const SCAN_SECS: u64 = 60;
/// Re-reads of a task whose hash moved between the merge and the write.
const MERGE_ATTEMPTS: usize = 3;
const COMMENT_MAX: usize = 2_000;
const NAME_MAX: usize = 80;
const PLATFORMS: [&str; 5] = ["macos", "ios", "ipados", "watchos", "web"];
const UPDATE_FIELDS: [&str; 6] = [
    "title",
    "priority",
    "due",
    "dueTime",
    "labels",
    "description",
];
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

static DEVICE_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{2,63}$").unwrap());
static OP_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9A-HJKMNP-TV-Z]{26}$").unwrap());
/// One pass at a time (watcher, timer, a command's kick).
static PASS: Mutex<()> = Mutex::new(());
static PAIRING: Mutex<Option<OpenCode>> = Mutex::new(None);
static KICK: Mutex<Option<mpsc::Sender<()>>> = Mutex::new(None);
/// Every read-modify-write of `devices.json` (pairing, revoke, last-op
/// stamps), so a revoke can't be undone by a pass that read the list first.
static DEVICES: Mutex<()> = Mutex::new(());

// ── Store (outside the vault) ────────────────────────────────────────────

/// Hollow's companion state outside the vault: paired keys and the
/// applied-op ledger. Tests use a temp folder.
#[derive(Clone, Debug)]
pub(crate) struct Store {
    root: PathBuf,
}

/// A verified op held for the person's confirm (see [`OLD_OP_KIND`]).
struct Held {
    kind: String,
    envelope: String,
    /// It came (at least once) as an inbox file, so its answer is a receipt
    /// file there too.
    via_inbox: bool,
    /// It came (at least once) from the backend, so its answer is owed to
    /// the daemon (the `outbox` table; `companion_bridge` sends it).
    via_backend: bool,
}

/// The road an envelope came by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Road {
    /// `.hollow-sync/inbox/` (receipts are files beside it).
    Inbox,
    /// The `companion_bridge` long-poll (receipts go back to the daemon).
    Backend,
}

/// A paired device as Hollow trusts it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub public_key: String,
    pub paired_at: String,
    #[serde(default)]
    pub last_op_at: Option<String>,
}

/// The ledger's schema version in `PRAGMA user_version`, stamped by
/// `Store::ledger` (raised, never lowered). 0 while every change is
/// additive; raise it with a migration an older build would misread, so Go
/// back sees it (`update_client::goback::known_schema`, row U6).
pub(crate) const LEDGER_SCHEMA_VERSION: i64 = 0;

/// A failure of the applied-op ledger (`applied.db`), in words Settings and
/// the snackbar can show as they are. The raw error stays last, after ": ",
/// so `plainError` turns "database is locked" and the like into a sentence.
fn ledger_err(e: impl std::fmt::Display) -> String {
    format!("Couldn't use Hollow's record of changes already applied from your devices: {e}")
}

impl Store {
    pub(crate) fn for_app() -> Option<Self> {
        crate::instance::support_dir().map(|dir| Self::at(dir.join("Companion")))
    }

    pub(crate) fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn devices_path(&self) -> PathBuf {
        self.root.join("devices.json")
    }

    pub(crate) fn devices(&self) -> Vec<Device> {
        fs::read_to_string(self.devices_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn device(&self, id: &str) -> Option<Device> {
        self.devices().into_iter().find(|d| d.device_id == id)
    }

    pub(crate) fn save_devices(&self, devices: &[Device]) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("companion store: {e}"))?;
        let text = serde_json::to_string_pretty(devices).map_err(|e| e.to_string())?;
        crate::vault::atomic_write_private(&self.devices_path(), text.as_bytes())
    }

    fn touch_device(&self, id: &str, at: &str) {
        let _devices = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
        let mut devices = self.devices();
        if let Some(device) = devices.iter_mut().find(|d| d.device_id == id) {
            device.last_op_at = Some(at.to_string());
            let _ = self.save_devices(&devices);
        }
    }

    fn ledger(&self) -> Result<rusqlite::Connection, String> {
        fs::create_dir_all(&self.root).map_err(ledger_err)?;
        let conn = rusqlite::Connection::open(self.root.join("applied.db")).map_err(ledger_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(ledger_err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS applied (
                device_id TEXT NOT NULL,
                op_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                receipt TEXT NOT NULL,
                at TEXT NOT NULL,
                PRIMARY KEY (device_id, op_id)
            );
            CREATE TABLE IF NOT EXISTS held (
                device_id TEXT NOT NULL,
                op_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                envelope TEXT NOT NULL,
                via_inbox INTEGER NOT NULL DEFAULT 0,
                via_backend INTEGER NOT NULL DEFAULT 0,
                at TEXT NOT NULL,
                PRIMARY KEY (device_id, op_id)
            );
            CREATE TABLE IF NOT EXISTS outbox (
                device_id TEXT NOT NULL,
                op_id TEXT NOT NULL,
                at TEXT NOT NULL,
                PRIMARY KEY (device_id, op_id)
            );",
        )
        .map_err(ledger_err)?;
        // A ledger from an H7 build before its review fixes has `held`
        // without `via_backend`.
        if conn
            .prepare("SELECT via_backend FROM held LIMIT 0")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE held ADD COLUMN via_backend INTEGER NOT NULL DEFAULT 0",
            )
            .map_err(ledger_err)?;
        }
        crate::hollow_databases::stamp_schema_version(&conn, LEDGER_SCHEMA_VERSION)
            .map_err(ledger_err)?;
        Ok(conn)
    }

    /// The stored receipt of a verified op this device already sent. Only
    /// signed ops are recorded, so a forged file can't claim an opId.
    fn seen(&self, device_id: &str, op_id: &str) -> Result<Option<Receipt>, String> {
        let conn = self.ledger()?;
        let mut stmt = conn
            .prepare("SELECT receipt FROM applied WHERE device_id = ?1 AND op_id = ?2")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([device_id, op_id]).map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => {
                let text: String = row.get(0).map_err(|e| e.to_string())?;
                Ok(serde_json::from_str(&text).ok())
            }
            None => Ok(None),
        }
    }

    /// Files this device's ops made since `since` (RFC 3339 UTC): notes
    /// created and conflict copies saved. For the daily limit.
    pub(crate) fn files_made_since(&self, device_id: &str, since: &str) -> Result<i64, String> {
        self.ledger()?
            .query_row(
                "SELECT COUNT(*) FROM applied WHERE device_id = ?1 AND at >= ?2
                 AND ((kind IN ('task.create', 'note.create') AND status = 'applied')
                      OR (kind = 'note.edit' AND status = 'conflict'))",
                [device_id, since],
                |r| r.get(0),
            )
            .map_err(ledger_err)
    }

    /// Keep a verified op that waits for the person (an old op), with its
    /// exact envelope text, so Apply it re-verifies the signed original and
    /// never the card's copy. A second road's delivery only notes its road.
    fn hold(
        &self,
        device_id: &str,
        op_id: &str,
        kind: &str,
        envelope: &str,
        road: Road,
        now: DateTime<Local>,
    ) -> Result<(), String> {
        let at = now
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let (inbox, backend) = (road == Road::Inbox, road == Road::Backend);
        self.ledger()?
            .execute(
                "INSERT INTO held (device_id, op_id, kind, envelope, via_inbox, via_backend, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (device_id, op_id) DO UPDATE
                 SET via_inbox = MAX(via_inbox, excluded.via_inbox),
                     via_backend = MAX(via_backend, excluded.via_backend)",
                rusqlite::params![device_id, op_id, kind, envelope, inbox, backend, at],
            )
            .map(|_| ())
            .map_err(ledger_err)
    }

    /// Note that a held op came again by `road` (so its answer goes that way
    /// too), without touching the kept envelope.
    fn note_road(&self, device_id: &str, op_id: &str, road: Road) -> Result<(), String> {
        let (inbox, backend) = (road == Road::Inbox, road == Road::Backend);
        self.ledger()?
            .execute(
                "UPDATE held SET via_inbox = MAX(via_inbox, ?3),
                 via_backend = MAX(via_backend, ?4)
                 WHERE device_id = ?1 AND op_id = ?2",
                rusqlite::params![device_id, op_id, inbox, backend],
            )
            .map(|_| ())
            .map_err(ledger_err)
    }

    /// A held op, when this device's op waits for the person.
    fn held(&self, device_id: &str, op_id: &str) -> Result<Option<Held>, String> {
        let conn = self.ledger()?;
        let sql = "SELECT kind, envelope, via_inbox, via_backend FROM held
                   WHERE device_id = ?1 AND op_id = ?2";
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let mut rows = stmt.query([device_id, op_id]).map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(Held {
                kind: row.get(0).map_err(|e| e.to_string())?,
                envelope: row.get(1).map_err(|e| e.to_string())?,
                via_inbox: row.get(2).map_err(|e| e.to_string())?,
                via_backend: row.get(3).map_err(|e| e.to_string())?,
            })),
            None => Ok(None),
        }
    }

    /// Settle a decided held op, in one transaction: when the backend
    /// handed it over, owe the daemon its receipt (the `outbox`), then
    /// forget the held copy. The daemon may have given up on it meanwhile
    /// (it expires `note.*`/`workspace.append` ops after 14 days and never
    /// hands them out again), so the receipt is sent, not polled for.
    fn settle(
        &self,
        device_id: &str,
        op_id: &str,
        owe_backend: bool,
        at: &str,
    ) -> Result<(), String> {
        let mut conn = self.ledger()?;
        let tx = conn.transaction().map_err(ledger_err)?;
        if owe_backend {
            tx.execute(
                "INSERT OR IGNORE INTO outbox (device_id, op_id, at) VALUES (?1, ?2, ?3)",
                [device_id, op_id, at],
            )
            .map_err(ledger_err)?;
        }
        tx.execute(
            "DELETE FROM held WHERE device_id = ?1 AND op_id = ?2",
            [device_id, op_id],
        )
        .map_err(ledger_err)?;
        tx.commit().map_err(ledger_err)
    }

    /// Forget every op this device has held (it was revoked).
    fn forget_held(&self, device_id: &str) -> Result<usize, String> {
        self.ledger()?
            .execute("DELETE FROM held WHERE device_id = ?1", [device_id])
            .map_err(ledger_err)
    }

    /// The backend was removed: drop every receipt still owed to it, and
    /// forget that held ops came from it (deciding one later owes nothing).
    /// One transaction. A ledger that was never made is left unmade.
    pub(crate) fn forget_backend(&self) -> Result<(), String> {
        if !self.root.join("applied.db").exists() {
            return Ok(());
        }
        let mut conn = self.ledger()?;
        let tx = conn.transaction().map_err(ledger_err)?;
        tx.execute_batch("DELETE FROM outbox; UPDATE held SET via_backend = 0;")
            .map_err(ledger_err)?;
        tx.commit().map_err(ledger_err)
    }

    /// The ledger receipts owed to the backend (held ops decided since it
    /// handed them over), oldest first, at most `limit`.
    pub(crate) fn owed_receipts(&self, limit: usize) -> Result<Vec<Receipt>, String> {
        let conn = self.ledger()?;
        let sql = "SELECT a.receipt FROM outbox o
                   JOIN applied a ON a.device_id = o.device_id AND a.op_id = o.op_id
                   ORDER BY o.at, o.op_id LIMIT ?1";
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0));
        let mut owed = Vec::new();
        for text in rows.map_err(|e| e.to_string())? {
            let text = text.map_err(|e| e.to_string())?;
            match serde_json::from_str(&text) {
                Ok(receipt) => owed.push(receipt),
                Err(e) => eprintln!("[companion] an owed receipt is unreadable: {e}"),
            }
        }
        Ok(owed)
    }

    /// The backend stored this receipt: it is owed no longer.
    pub(crate) fn receipt_delivered(&self, device_id: &str, op_id: &str) -> Result<(), String> {
        self.ledger()?
            .execute(
                "DELETE FROM outbox WHERE device_id = ?1 AND op_id = ?2",
                [device_id, op_id],
            )
            .map(|_| ())
            .map_err(ledger_err)
    }

    fn record(&self, kind: &str, receipt: &Receipt) -> Result<(), String> {
        let text = serde_json::to_string(receipt).map_err(|e| e.to_string())?;
        self.ledger()?
            .execute(
                "INSERT OR IGNORE INTO applied (op_id, device_id, kind, status, receipt, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    receipt.op_id,
                    receipt.device_id,
                    kind,
                    receipt.status,
                    text,
                    receipt.at
                ],
            )
            .map(|_| ())
            .map_err(ledger_err)
    }
}

// ── Pairing ──────────────────────────────────────────────────────────────

struct OpenCode {
    code: String,
    expires: DateTime<Utc>,
    failures: u32,
}

/// The pairing code Settings shows (`XXXX-XXXX`).
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub expires_at: String,
}

fn display_code(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

/// Open a fresh pairing code for [`CODE_TTL_MINUTES`] (replacing any open
/// one). Kept in memory only.
pub(crate) fn open_pairing(now: DateTime<Utc>) -> PairingCode {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let code: String = (0..CODE_LEN)
        .map(|_| CROCKFORD[rng.gen_range(0..32)] as char)
        .collect();
    set_pairing(&code, now)
}

fn set_pairing(code: &str, now: DateTime<Utc>) -> PairingCode {
    let expires = now + Duration::minutes(CODE_TTL_MINUTES);
    *PAIRING.lock().unwrap_or_else(|e| e.into_inner()) = Some(OpenCode {
        code: code.to_string(),
        expires,
        failures: 0,
    });
    PairingCode {
        code: display_code(code),
        expires_at: expires.to_rfc3339_opts(SecondsFormat::Secs, true),
    }
}

pub(crate) fn close_pairing() {
    *PAIRING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The open code, if it hasn't expired.
pub(crate) fn open_code(now: DateTime<Utc>) -> Option<PairingCode> {
    let guard = PAIRING.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .filter(|c| c.expires > now)
        .map(|c| PairingCode {
            code: display_code(&c.code),
            expires_at: c.expires.to_rfc3339_opts(SecondsFormat::Secs, true),
        })
}

/// A code as typed: upper case, dashes and whitespace removed.
pub(crate) fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

fn proof_message(device_id: &str, public_key: &str) -> String {
    format!("hollow-pair-v1\n{device_id}\n{public_key}")
}

/// `base64(HMAC-SHA256(code, "hollow-pair-v1\n" + deviceId + "\n" + publicKey))`.
pub(crate) fn pair_proof(code: &str, device_id: &str, public_key: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, normalize_code(code).as_bytes());
    let tag = ring::hmac::sign(&key, proof_message(device_id, public_key).as_bytes());
    B64.encode(tag.as_ref())
}

fn proof_matches(code: &str, device_id: &str, public_key: &str, proof: &str) -> bool {
    let Ok(tag) = B64.decode(proof.trim()) else {
        return false;
    };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, code.as_bytes());
    ring::hmac::verify(&key, proof_message(device_id, public_key).as_bytes(), &tag).is_ok()
}

/// A 65-byte uncompressed P-256 point, base64.
fn valid_public_key(raw: &str) -> bool {
    B64.decode(raw.trim())
        .is_ok_and(|bytes| bytes.len() == 65 && bytes[0] == 0x04)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PairRequest {
    v: u32,
    device_id: String,
    name: String,
    platform: String,
    public_key: String,
    #[allow(dead_code)]
    at: String,
    proof: String,
}

/// Decide one pairing request. Ok(name) when paired.
fn decide_pairing(store: &Store, req: &PairRequest, now: DateTime<Utc>) -> Result<String, String> {
    decide_pairing_with(store, req, now, false)
}

/// `always_code`: a device already paired with this key must still prove the
/// open code (the backend road, where anyone who can reach the daemon could
/// replay a known device's id and public key; the inbox road needs vault
/// write access, so a known key re-pairs without a code there).
fn decide_pairing_with(
    store: &Store,
    req: &PairRequest,
    now: DateTime<Utc>,
    always_code: bool,
) -> Result<String, String> {
    let _devices = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    if req.v != 1 {
        return Err(format!("unsupported pairing version {}", req.v));
    }
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.contains('\n') {
        return Err(format!(
            "the device name must be one line of 1 to {NAME_MAX} characters"
        ));
    }
    if !PLATFORMS.contains(&req.platform.as_str()) {
        return Err(format!("unknown platform `{}`", req.platform));
    }
    if !valid_public_key(&req.public_key) {
        return Err("the public key must be a base64 uncompressed P-256 point".into());
    }
    let mut devices = store.devices();
    let mut repair = false;
    if let Some(known) = devices.iter().find(|d| d.device_id == req.device_id) {
        if known.public_key == req.public_key.trim() {
            if !always_code {
                return Ok(known.name.clone());
            }
            repair = true;
        } else {
            return Err(
                "this device id is already paired with another key; revoke it in Hollow first"
                    .into(),
            );
        }
    }
    {
        let mut guard = PAIRING.lock().unwrap_or_else(|e| e.into_inner());
        let Some(open) = guard.as_mut().filter(|c| c.expires > now) else {
            return Err("No pairing code is open in Hollow. Open Settings → Devices → Pair a device, then try again.".into());
        };
        if !proof_matches(
            &open.code,
            &req.device_id,
            req.public_key.trim(),
            &req.proof,
        ) {
            open.failures += 1;
            if open.failures >= CODE_MAX_FAILURES {
                *guard = None;
                return Err("The code doesn't match, and too many tries closed it. Open a new code in Hollow.".into());
            }
            return Err("The code doesn't match the one Hollow shows.".into());
        }
        *guard = None;
    }
    if repair {
        return Ok(name.to_string());
    }
    devices.push(Device {
        device_id: req.device_id.clone(),
        name: name.to_string(),
        platform: req.platform.clone(),
        public_key: req.public_key.trim().to_string(),
        paired_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
        last_op_at: None,
    });
    store.save_devices(&devices)?;
    Ok(name.to_string())
}

fn write_device_status(
    vault: &Path,
    device_id: &str,
    status: &str,
    name: Option<&str>,
    platform: Option<&str>,
    reason: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let path = safe_dir(vault, &["devices"], true)?.join(format!("{device_id}.json"));
    let body = json!({
        "v": 1, "deviceId": device_id, "name": name, "platform": platform,
        "status": status, "reason": reason,
        "at": now.to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    crate::vault::atomic_write(
        &path,
        serde_json::to_string_pretty(&body).unwrap().as_bytes(),
    )
}

/// Every pairing request in `.hollow-sync/pair/`, decided and removed.
fn process_pairing(vault: &Path, store: &Store, now: DateTime<Utc>) -> usize {
    let Ok(dir) = safe_dir(vault, &["pair"], false) else {
        return 0;
    };
    let mut decided = 0;
    for (path, stem) in json_files(&dir) {
        if !DEVICE_ID.is_match(&stem) {
            let _ = fs::remove_file(&path);
            continue;
        }
        let parsed = read_bounded(&path, MAX_PAIR_BYTES).and_then(|text| {
            serde_json::from_str::<PairRequest>(&text)
                .map_err(|e| format!("malformed pairing request: {e}"))
        });
        let outcome = parsed.and_then(|req| {
            if req.device_id != stem {
                return Err("the file name and deviceId differ".into());
            }
            decide_pairing(store, &req, now).map(|name| (name, req.platform))
        });
        let written = match &outcome {
            Ok((name, platform)) => write_device_status(
                vault,
                &stem,
                "paired",
                Some(name),
                Some(platform),
                None,
                now,
            ),
            Err(reason) => {
                write_device_status(vault, &stem, "refused", None, None, Some(reason), now)
            }
        };
        if let Err(error) = written {
            eprintln!("[companion] could not answer pairing request {stem}: {error}");
            continue;
        }
        let _ = fs::remove_file(&path);
        decided += 1;
    }
    decided
}

/// Revoke a device: its key is deleted, so its later ops are refused, and
/// its held old ops go with their cards (Apply it could never verify them).
pub(crate) fn revoke(vault: &Path, store: &Store, device_id: &str) -> Result<(), String> {
    revoke_bound(vault, store, device_id, |mutate| mutate())
}

/// [`revoke`] with its changes run through `bind` once the pass lock is
/// held: the COO's action binds them to its run's History lease there
/// (`Lease::Own`). Taking History first and the pass lock second would
/// deadlock against a pass, which holds the pass lock while an Apply takes
/// History.
pub(crate) fn revoke_bound<'a>(
    vault: &'a Path,
    store: &'a Store,
    device_id: &'a str,
    bind: impl FnOnce(Box<dyn FnOnce() -> Result<(), String> + 'a>) -> Result<(), String>,
) -> Result<(), String> {
    // The pass lock first, in a pass's order (it takes `DEVICES` inside), so
    // no Apply it runs while the device's held ops go.
    let _pass = pass_lock();
    bind(Box::new(move || {
        {
            let _devices = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
            let mut devices = store.devices();
            let before = devices.len();
            devices.retain(|d| d.device_id != device_id);
            if devices.len() == before {
                return Err(format!("no paired device {device_id}"));
            }
            store.save_devices(&devices)?;
        }
        forget_old_ops(vault, store, device_id);
        write_device_status(vault, device_id, "revoked", None, None, None, Utc::now())
    }))
}

/// Decide a pairing request that came through the backend (the same
/// request a device would write to `.hollow-sync/pair/`). Ok(name) when
/// paired; Err is the reason the device is told.
pub(crate) fn decide_remote_pairing(
    store: &Store,
    device_id: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    if text.len() as u64 > MAX_PAIR_BYTES {
        return Err(format!("the request is over {MAX_PAIR_BYTES} bytes"));
    }
    let req: PairRequest =
        serde_json::from_str(text).map_err(|e| format!("malformed pairing request: {e}"))?;
    if req.device_id != device_id || !DEVICE_ID.is_match(device_id) {
        return Err("the request's deviceId doesn't match".into());
    }
    decide_pairing_with(store, &req, now, true)
}

// ── Envelopes, receipts ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Envelope {
    pub v: u32,
    pub device_id: String,
    pub op_id: String,
    pub payload: String,
    pub sig: String,
}

/// One field both sides changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct FieldConflict {
    pub field: String,
    pub base: Value,
    pub device: Value,
    pub mac: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub v: u32,
    pub op_id: String,
    pub device_id: String,
    pub at: String,
    pub status: String,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub content_hash: Option<String>,
    pub applied: Vec<String>,
    pub conflicts: Vec<FieldConflict>,
    pub task: Option<Value>,
    pub reason: Option<String>,
}

impl Receipt {
    fn new(op_id: &str, device_id: &str, now: DateTime<Local>, status: &str) -> Self {
        Self {
            v: 1,
            op_id: op_id.into(),
            device_id: device_id.into(),
            at: now
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            status: status.into(),
            reference: None,
            content_hash: None,
            applied: Vec::new(),
            conflicts: Vec::new(),
            task: None,
            reason: None,
        }
    }

    fn refused(
        op_id: &str,
        device_id: &str,
        now: DateTime<Local>,
        reason: impl Into<String>,
    ) -> Self {
        let mut receipt = Self::new(op_id, device_id, now, "refused");
        receipt.reason = Some(reason.into());
        receipt
    }

    /// The Review cards this op's apply raised: one per conflicting field,
    /// except a repeating task's refused finish (it carries a reason and
    /// gets no card) and a note's conflict copy (no field list). A repeat
    /// counts them again, so a pass that lost its Review refresh (a
    /// cancelled bridge cycle) still gets one.
    pub(crate) fn review_cards(&self) -> usize {
        if self.reason.is_some() {
            0
        } else {
            self.conflicts.len()
        }
    }
}

/// Verify `sig` (base64 r‖s) over the exact payload bytes with `public_key`
/// (base64 uncompressed P-256).
pub(crate) fn verify(public_key: &str, payload: &str, sig: &str) -> bool {
    let (Ok(key), Ok(sig)) = (B64.decode(public_key.trim()), B64.decode(sig.trim())) else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
        .verify(payload.as_bytes(), &sig)
        .is_ok()
}

fn write_receipt(vault: &Path, receipt: &Receipt) -> Result<(), String> {
    let dir = safe_dir(vault, &["receipts", &receipt.device_id], true)?;
    let text = serde_json::to_string_pretty(receipt).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(
        &dir.join(format!("{}.json", receipt.op_id)),
        text.as_bytes(),
    )
}

// ── A pass ───────────────────────────────────────────────────────────────

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    pub applied: usize,
    pub refused: usize,
    /// Ops left in the inbox because the vault was busy.
    pub waiting: usize,
    pub paired: usize,
    /// Old ops now waiting for the person's confirm (no receipt yet).
    pub held: usize,
    pub review_changed: usize,
}

impl Summary {
    fn changed(&self) -> bool {
        self.applied + self.refused + self.paired + self.held + self.review_changed > 0
    }
}

enum Outcome {
    Done {
        receipt: Receipt,
        review_changed: usize,
    },
    /// An old op held for the person's confirm: no receipt yet.
    Held {
        review_changed: usize,
    },
    Waiting,
}

/// Decide pairing requests, then apply every inbox op (per device, in opId
/// order; a device's ops after a busy one wait with it), then prune old
/// receipts. Never panics on a bad file: each one gets a refused receipt
/// when it is addressable, and is dropped otherwise.
pub(crate) fn process(vault: &Path, store: &Store, now: DateTime<Local>) -> Summary {
    let _pass = pass_lock();
    let mut summary = Summary {
        paired: process_pairing(vault, store, now.with_timezone(&Utc)),
        ..Default::default()
    };
    let inbox = safe_dir(vault, &["inbox"], false).ok();
    for (device_dir, device_id) in inbox.as_deref().map(dirs).unwrap_or_default() {
        for (path, op_id) in json_files(&device_dir) {
            if !DEVICE_ID.is_match(&device_id) || !OP_ID.is_match(&op_id) {
                eprintln!(
                    "[companion] dropped an unaddressable inbox file: {}",
                    path.display()
                );
                let _ = fs::remove_file(&path);
                summary.refused += 1;
                continue;
            }
            match handle(vault, store, &device_id, &op_id, &path, now) {
                Ok(Outcome::Waiting) => {
                    summary.waiting += 1;
                    break;
                }
                Ok(Outcome::Held { review_changed }) => {
                    summary.held += 1;
                    summary.review_changed += review_changed;
                }
                Ok(Outcome::Done {
                    receipt,
                    review_changed,
                }) => {
                    if receipt.status == "refused" {
                        summary.refused += 1;
                    } else {
                        summary.applied += 1;
                    }
                    summary.review_changed += review_changed;
                }
                Err(error) => {
                    // The receipt or ledger couldn't be written: keep the op
                    // for the next pass rather than lose it.
                    eprintln!("[companion] {op_id}: {error}");
                    summary.waiting += 1;
                    break;
                }
            }
        }
    }
    prune_receipts(vault);
    summary
}

/// One inbox file. Err only when the outcome couldn't be recorded.
fn handle(
    vault: &Path,
    store: &Store,
    device_id: &str,
    op_id: &str,
    path: &Path,
    now: DateTime<Local>,
) -> Result<Outcome, String> {
    // A repeat delivery of a verified op: answer with the stored receipt
    // (rewritten only if the device already removed it), apply nothing.
    if let Some(receipt) = store.seen(device_id, op_id)? {
        if !receipt_path(vault, device_id, op_id).is_some_and(|p| p.exists()) {
            note_receipt_error(write_receipt(vault, &receipt), op_id);
        }
        let _ = fs::remove_file(path);
        let review_changed = receipt.review_cards();
        return Ok(Outcome::Done {
            receipt,
            review_changed,
        });
    }
    // A copy of a held op: the signed original is kept, so this one is
    // never opened (a damaged or forged copy can't answer it `refused`).
    if let Some(review_changed) = held_again(store, device_id, op_id, Road::Inbox)? {
        let _ = fs::remove_file(path);
        return Ok(Outcome::Held { review_changed });
    }
    let outcome = match read_bounded(path, MAX_ENVELOPE_BYTES) {
        Ok(text) => deliver(vault, store, device_id, op_id, &text, Road::Inbox, now)?,
        Err(reason) => Delivery::Refused(Receipt::refused(op_id, device_id, now, reason)),
    };
    match outcome {
        Delivery::Waiting => Ok(Outcome::Waiting),
        // Kept outside the vault (with its envelope) until the person
        // decides; the device hears nothing until then.
        Delivery::Held { review_changed } => {
            let _ = fs::remove_file(path);
            Ok(Outcome::Held { review_changed })
        }
        Delivery::Repeat(receipt) => {
            note_receipt_error(write_receipt(vault, &receipt), op_id);
            let _ = fs::remove_file(path);
            let review_changed = receipt.review_cards();
            Ok(Outcome::Done {
                receipt,
                review_changed,
            })
        }
        Delivery::Refused(receipt) => {
            note_receipt_error(write_receipt(vault, &receipt), op_id);
            let _ = fs::remove_file(path);
            Ok(Outcome::Done {
                receipt,
                review_changed: 0,
            })
        }
        Delivery::Applied {
            receipt,
            review_changed,
        } => {
            note_receipt_error(write_receipt(vault, &receipt), op_id);
            let _ = fs::remove_file(path);
            Ok(Outcome::Done {
                receipt,
                review_changed,
            })
        }
    }
}

/// What one delivered envelope came to, on either road.
#[derive(Debug)]
pub(crate) enum Delivery {
    /// Already in the ledger (the other road, or a resend): its stored
    /// receipt; nothing applied.
    Repeat(Receipt),
    /// Not verified (unpaired, bad signature, malformed): a receipt but no
    /// ledger row, so a damaged or forged copy can't claim the opId.
    Refused(Receipt),
    /// Verified and recorded in the ledger (applied, partial, conflict, or
    /// refused by validation).
    Applied {
        receipt: Receipt,
        review_changed: usize,
    },
    /// Verified but sent more than [`OLD_OP_DAYS`] days ago: held (with
    /// its envelope) behind an [`OLD_OP_KIND`] card, and no receipt until
    /// the person applies or discards it. A held op delivered again is held
    /// again without opening the copy (its card counted again, so a lost
    /// refresh is made up).
    Held { review_changed: usize },
    /// The vault is busy: nothing done; deliver it again later.
    Waiting,
}

/// Verify and apply one envelope's exact text, once per `(deviceId, opId)`
/// across both roads: the `.hollow-sync/inbox/` files and the backend
/// (`companion_bridge`). The caller holds [`pass_lock`]. Err only when the
/// ledger (or an old op's card) couldn't be read or written.
pub(crate) fn deliver(
    vault: &Path,
    store: &Store,
    device_id: &str,
    op_id: &str,
    text: &str,
    road: Road,
    now: DateTime<Local>,
) -> Result<Delivery, String> {
    if let Some(receipt) = store.seen(device_id, op_id)? {
        return Ok(Delivery::Repeat(receipt));
    }
    if let Some(review_changed) = held_again(store, device_id, op_id, road)? {
        return Ok(Delivery::Held { review_changed });
    }
    let (device, payload) = match open_envelope_text(store, device_id, op_id, text) {
        Ok(opened) => opened,
        Err(reason) => {
            return Ok(Delivery::Refused(Receipt::refused(
                op_id, device_id, now, reason,
            )))
        }
    };
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Design §3.6: an op sent long ago (a device offline for weeks, or a
    // backlog) asks first. Both roads come through here, so both get it.
    if sent_long_ago(&payload, now) {
        // The card first: a later copy of a held op is never opened, so a
        // held row must already have its card. A failed write leaves the op
        // to be delivered again, like a failed ledger write (the card is
        // one per op).
        enqueue_old_op(vault, &device, op_id, &payload, now)
            .map_err(|e| format!("old-op card: {e}"))?;
        store.hold(device_id, op_id, &kind, text, road, now)?;
        return Ok(Delivery::Held { review_changed: 1 });
    }
    match apply_payload(vault, store, &device, op_id, &payload, now) {
        Outcome::Waiting => Ok(Delivery::Waiting),
        Outcome::Held { review_changed } => Ok(Delivery::Held { review_changed }),
        Outcome::Done {
            receipt,
            review_changed,
        } => {
            record_applied(store, device_id, &kind, &receipt)?;
            Ok(Delivery::Applied {
                receipt,
                review_changed,
            })
        }
    }
}

/// An op already held, delivered again by `road`: the copy is not opened
/// (the kept original is what Apply it verifies), only its road is noted so
/// the answer goes that way too. Its card is counted again, so a pass that
/// lost its Review refresh still gets one. None when the op isn't held.
fn held_again(
    store: &Store,
    device_id: &str,
    op_id: &str,
    road: Road,
) -> Result<Option<usize>, String> {
    if store.held(device_id, op_id)?.is_none() {
        return Ok(None);
    }
    store.note_road(device_id, op_id, road)?;
    Ok(Some(1))
}

/// Record a verified op's outcome in the ledger. The ledger first: once it
/// holds the op, a failed receipt write can never get the op applied a
/// second time.
fn record_applied(
    store: &Store,
    device_id: &str,
    kind: &str,
    receipt: &Receipt,
) -> Result<(), String> {
    store.record(kind, receipt)?;
    if receipt.status != "refused" {
        store.touch_device(device_id, &receipt.at);
    }
    Ok(())
}

/// The op's signed `at` is more than [`OLD_OP_DAYS`] days before `now`. A
/// future `at` (a fast device clock) is not old.
fn sent_long_ago(payload: &Map<String, Value>, now: DateTime<Local>) -> bool {
    let Some(at) = sent_at(payload) else {
        return false;
    };
    now.with_timezone(&Utc) - at > Duration::days(OLD_OP_DAYS)
}

fn sent_at(payload: &Map<String, Value>) -> Option<DateTime<Utc>> {
    let at = payload.get("at").and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Serialises applier work across the inbox pass and the backend bridge.
pub(crate) fn pass_lock() -> std::sync::MutexGuard<'static, ()> {
    PASS.lock().unwrap_or_else(|e| e.into_inner())
}

fn receipt_path(vault: &Path, device_id: &str, op_id: &str) -> Option<PathBuf> {
    safe_dir(vault, &["receipts", device_id], false)
        .ok()
        .map(|dir| dir.join(format!("{op_id}.json")))
}

fn note_receipt_error(result: Result<(), String>, op_id: &str) {
    if let Err(error) = result {
        eprintln!("[companion] {op_id}: receipt not written: {error}");
    }
}

/// Verify and parse an envelope's text. Err is the refusal reason.
fn open_envelope_text(
    store: &Store,
    device_id: &str,
    op_id: &str,
    text: &str,
) -> Result<(Device, Map<String, Value>), String> {
    if text.len() as u64 > MAX_ENVELOPE_BYTES {
        return Err(format!("the op is over {MAX_ENVELOPE_BYTES} bytes"));
    }
    let env: Envelope =
        serde_json::from_str(text).map_err(|e| format!("malformed op envelope: {e}"))?;
    if env.v != 1 {
        return Err(format!("unsupported op version {}", env.v));
    }
    if env.device_id != device_id || env.op_id != op_id {
        return Err("the envelope's deviceId/opId don't match its file".into());
    }
    let device = store
        .device(device_id)
        .ok_or("This device isn't paired with Hollow (or was revoked).")?;
    if !verify(&device.public_key, &env.payload, &env.sig) {
        return Err("The op's signature doesn't verify; nothing was applied.".into());
    }
    let payload: Map<String, Value> =
        serde_json::from_str(&env.payload).map_err(|e| format!("malformed op payload: {e}"))?;
    if payload.get("v").and_then(Value::as_u64) != Some(1)
        || payload.get("opId").and_then(Value::as_str) != Some(op_id)
        || payload.get("deviceId").and_then(Value::as_str) != Some(device_id)
    {
        return Err("the payload's v/opId/deviceId don't match the envelope".into());
    }
    if payload
        .get("at")
        .and_then(Value::as_str)
        .is_none_or(|at| DateTime::parse_from_rfc3339(at).is_err())
    {
        return Err("the payload needs an RFC 3339 `at`".into());
    }
    Ok((device, payload))
}

const COMMON: [&str; 5] = ["v", "opId", "deviceId", "at", "kind"];

fn reject_unknown(payload: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match payload
        .keys()
        .find(|k| !COMMON.contains(&k.as_str()) && !allowed.contains(&k.as_str()))
    {
        Some(key) => Err(format!("unknown key `{key}` for this kind")),
        None => Ok(()),
    }
}

fn required_str<'a>(payload: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

/// The op kinds [`apply_payload`] applies. The bridge declares exactly these
/// to the backend (`X-Hollow-Kinds`), which hands this Hollow no other kind:
/// an unknown kind would be refused here and its op id burned in the ledger.
pub(crate) const APPLIED_KINDS: [&str; 8] = [
    "task.create",
    "task.update",
    "task.set_status",
    "task.comment",
    "note.create",
    "note.edit",
    "note.append",
    "workspace.append",
];

/// Notes and workspace lines (overnight 10-12/13, H5). `pub(crate)` for the
/// phone API's quick note (`notes::create_for_phone`, lane I3.5).
#[path = "companion_notes.rs"]
pub(crate) mod notes;

fn apply_payload(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Outcome {
    let refused = |reason: String| Outcome::Done {
        receipt: Receipt::refused(op_id, &device.device_id, now, reason),
        review_changed: 0,
    };
    let kind = payload.get("kind").and_then(Value::as_str).unwrap_or("");
    let result = match kind {
        "task.create" => apply_create(vault, store, device, op_id, payload, now),
        "task.update" | "task.set_status" => apply_merge(vault, device, op_id, payload, now),
        "task.comment" => apply_comment(vault, device, op_id, payload, now),
        "note.create" => notes::note_create(vault, store, device, op_id, payload, now),
        "note.edit" => notes::note_edit(vault, store, device, op_id, payload, now),
        "note.append" => notes::note_append(vault, store, device, op_id, payload, now),
        "workspace.append" => notes::workspace_append(vault, store, device, op_id, payload, now),
        other => Err(Step::Refused(format!("unknown op kind `{other}`"))),
    };
    match result {
        Ok(outcome) => outcome,
        Err(Step::Busy(reason)) => {
            eprintln!("[companion] {op_id} waits: {reason}");
            Outcome::Waiting
        }
        Err(Step::Refused(reason)) => refused(reason),
    }
}

enum Step {
    Busy(String),
    Refused(String),
}

impl From<CompanionRefusal> for Step {
    fn from(value: CompanionRefusal) -> Self {
        match value {
            CompanionRefusal::Busy(m) => Step::Busy(m),
            CompanionRefusal::Refused(m) => Step::Refused(m),
        }
    }
}

fn dispatch(
    vault: &Path,
    device: &Device,
    op_id: &str,
    op: &str,
    args: Value,
    now: DateTime<Local>,
) -> Result<Value, CompanionRefusal> {
    task_broker::dispatch_companion(
        vault,
        &device.device_id,
        &device.name,
        op_id,
        op,
        &args,
        now,
    )
}

/// The receipt's view of a task as saved.
fn task_view(task: &Task) -> Value {
    json!({
        "title": task.title,
        "status": task.status,
        "priority": task.priority,
        "due": task.due,
        "dueTime": task.due_time,
        "recurrence": task.recurrence,
    })
}

fn fill_task(vault: &Path, receipt: &mut Receipt, rel: &str) {
    receipt.reference = Some(format!("vault:{rel}"));
    if let Some(task) = vault_tasks::get(vault, rel) {
        receipt.content_hash = Some(task.content_hash.clone());
        receipt.task = Some(task_view(&task));
    }
}

fn rel_of(reference: &str) -> String {
    reference.trim().trim_start_matches("vault:").to_string()
}

fn apply_create(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    const KEYS: [&str; 10] = [
        "title",
        "status",
        "priority",
        "due",
        "dueTime",
        "durationMin",
        "recurrence",
        "labels",
        "description",
        "projectUrl",
    ];
    reject_unknown(payload, &KEYS).map_err(Step::Refused)?;
    required_str(payload, "title").map_err(Step::Refused)?;
    // A repeat of an op that already made its task isn't a new file.
    let repeat = vault_tasks::fingerprint_for_key(&format!("companion:{}", op_id.trim()))
        .is_some_and(|fp| vault_tasks::fingerprint_index(vault).contains_key(&fp));
    if !repeat {
        notes::within_daily_files(store, device, now)?;
    }
    let mut args = Map::new();
    for key in KEYS {
        if let Some(value) = payload.get(key).filter(|v| !v.is_null()) {
            args.insert(key.into(), value.clone());
        }
    }
    args.insert("idempotencyKey".into(), json!(op_id));
    let result = dispatch(
        vault,
        device,
        op_id,
        "create_task",
        Value::Object(args),
        now,
    )?;
    let mut receipt = Receipt::new(op_id, &device.device_id, now, "applied");
    let reference = result
        .get("ref")
        .and_then(Value::as_str)
        .unwrap_or_default();
    receipt.applied = vec!["create".into()];
    fill_task(vault, &mut receipt, &rel_of(reference));
    Ok(Outcome::Done {
        receipt,
        review_changed: 0,
    })
}

fn apply_comment(
    vault: &Path,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    reject_unknown(payload, &["ref", "text"]).map_err(Step::Refused)?;
    let reference = required_str(payload, "ref").map_err(Step::Refused)?;
    let text = required_str(payload, "text").map_err(Step::Refused)?;
    if text.chars().count() > COMMENT_MAX {
        return Err(Step::Refused(format!(
            "a comment is at most {COMMENT_MAX} characters"
        )));
    }
    let rel = vault_tasks::resolve_task_ref(vault, reference).map_err(Step::Refused)?;
    dispatch(
        vault,
        device,
        op_id,
        "add_comment",
        json!({ "ref": format!("vault:{rel}"), "body": text, "idempotencyKey": op_id }),
        now,
    )?;
    let mut receipt = Receipt::new(op_id, &device.device_id, now, "applied");
    receipt.applied = vec!["comment".into()];
    fill_task(vault, &mut receipt, &rel);
    Ok(Outcome::Done {
        receipt,
        review_changed: 0,
    })
}

// ── Per-field merge ──────────────────────────────────────────────────────

/// A field's value as compared: trimmed text, empty for null, priority as
/// P1–P4, labels as a newline-joined list. Err for a value of the wrong type.
fn norm(field: &str, value: &Value) -> Result<String, String> {
    match (field, value) {
        (_, Value::Null) => Ok(String::new()),
        ("labels", Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| "labels must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|labels| {
                labels
                    .into_iter()
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        ("labels", _) => Err("labels must be an array of strings".into()),
        ("priority", Value::String(s)) => {
            let s = s.trim();
            Ok(vault_tasks::normalize_priority(s).unwrap_or_else(|| s.to_string()))
        }
        ("status", Value::String(s)) => Ok(s.trim().to_lowercase()),
        (_, Value::String(s)) => Ok(s.trim().to_string()),
        (field, _) => Err(format!("`{field}` must be a string or null")),
    }
}

/// The task's current value of a field, as JSON.
fn current(task: &Task, field: &str) -> Value {
    match field {
        "title" => json!(task.title),
        "status" => json!(task.status),
        "priority" => json!(task.priority),
        "due" => json!(task.due),
        "dueTime" => json!(task.due_time),
        "labels" => json!(task.labels),
        "description" => json!(task.description),
        _ => Value::Null,
    }
}

/// The broker's argument for writing `to` into `field` (empty clears).
fn broker_value(field: &str, to: &Value) -> Value {
    match (field, to) {
        ("labels", Value::Null) => json!([]),
        (_, Value::Null) => json!(""),
        _ => to.clone(),
    }
}

/// `(field, from, to)` triples of a `task.update` or `task.set_status`.
fn requested_fields(payload: &Map<String, Value>) -> Result<Vec<(String, Value, Value)>, String> {
    if payload.get("kind").and_then(Value::as_str) == Some("task.set_status") {
        reject_unknown(payload, &["ref", "baseHash", "from", "to"])?;
        required_str(payload, "to")?;
        return Ok(vec![(
            "status".into(),
            payload.get("from").cloned().unwrap_or(Value::Null),
            payload["to"].clone(),
        )]);
    }
    reject_unknown(payload, &["ref", "baseHash", "fields"])?;
    let fields = payload
        .get("fields")
        .and_then(Value::as_object)
        .filter(|f| !f.is_empty())
        .ok_or("`fields` must be a non-empty object")?;
    fields
        .iter()
        .map(|(field, change)| {
            if !UPDATE_FIELDS.contains(&field.as_str()) {
                return Err(format!(
                    "`{field}` can't be changed from a companion (title, priority, due, dueTime, labels, description; status has its own op)"
                ));
            }
            let change = change
                .as_object()
                .filter(|c| c.keys().all(|k| k == "from" || k == "to"))
                .ok_or_else(|| format!("`fields.{field}` must be {{\"from\", \"to\"}}"))?;
            let from = change.get("from").cloned().unwrap_or(Value::Null);
            let to = change.get("to").cloned().unwrap_or(Value::Null);
            norm(field, &from)?;
            norm(field, &to)?;
            Ok((field.clone(), from, to))
        })
        .collect()
}

fn apply_merge(
    vault: &Path,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    let requested = requested_fields(payload).map_err(Step::Refused)?;
    let reference = required_str(payload, "ref").map_err(Step::Refused)?;
    let base_hash = required_str(payload, "baseHash")
        .map_err(Step::Refused)?
        .trim()
        .to_lowercase();
    let rel = vault_tasks::resolve_task_ref(vault, reference).map_err(Step::Refused)?;
    let is_status = requested.iter().any(|(f, _, _)| f == "status");

    for _attempt in 0..MERGE_ATTEMPTS {
        let task = vault_tasks::get(vault, &rel)
            .ok_or_else(|| Step::Refused(format!("{rel} is not a readable task")))?;
        let unchanged = task.content_hash == base_hash;
        // Finishing a repeating task rolls it forward instead of finishing,
        // so its status reads `from` again afterwards: a second device's
        // completion (or a retry) would roll it twice. It applies only on
        // the exact bytes the device saw.
        let rolls = is_status
            && task
                .recurrence
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
            && requested
                .iter()
                .any(|(_, _, to)| to.as_str().is_some_and(vault_tasks::is_terminal_status));
        let mut write: Vec<(String, Value)> = Vec::new();
        let mut conflicts: Vec<FieldConflict> = Vec::new();
        for (field, from, to) in &requested {
            let now_value = current(&task, field);
            let cur = norm(field, &now_value).unwrap_or_default();
            let want = norm(field, to).map_err(Step::Refused)?;
            if cur == want {
                continue;
            }
            if unchanged || (!rolls && cur == norm(field, from).map_err(Step::Refused)?) {
                write.push((field.clone(), to.clone()));
            } else {
                conflicts.push(FieldConflict {
                    field: field.clone(),
                    base: from.clone(),
                    device: to.clone(),
                    mac: now_value,
                });
            }
        }
        if !write.is_empty() {
            let result = if is_status {
                dispatch(
                    vault,
                    device,
                    op_id,
                    "set_status",
                    json!({ "ref": format!("vault:{rel}"), "status": write[0].1, "expectedHash": task.content_hash }),
                    now,
                )
            } else {
                let fields: Map<String, Value> = write
                    .iter()
                    .map(|(f, to)| (f.clone(), broker_value(f, to)))
                    .collect();
                dispatch(
                    vault,
                    device,
                    op_id,
                    "update_task",
                    json!({ "ref": format!("vault:{rel}"), "expectedHash": task.content_hash, "fields": fields }),
                    now,
                )
            };
            match result {
                // The file moved between the read and the write: merge again,
                // and after the last try let the op wait for the next pass.
                Err(CompanionRefusal::Refused(m)) if m.starts_with("TASK_CONFLICT") => continue,
                Err(other) => return Err(other.into()),
                Ok(_) => {}
            }
        }
        let status = match (write.is_empty(), conflicts.is_empty()) {
            (_, true) => "applied",
            (true, false) => "conflict",
            (false, false) => "partial",
        };
        let mut receipt = Receipt::new(op_id, &device.device_id, now, status);
        receipt.applied = write.iter().map(|(f, _)| f.clone()).collect();
        fill_task(vault, &mut receipt, &rel);
        let mut review_changed = 0;
        if rolls && !conflicts.is_empty() {
            // No card: "use the device's value" would roll it forward again.
            receipt.reason = Some(
                "This task repeats and changed after you saw it, so it wasn't finished again. Check it on the Mac.".into(),
            );
            receipt.conflicts = conflicts;
            return Ok(Outcome::Done {
                receipt,
                review_changed,
            });
        }
        for conflict in &conflicts {
            match enqueue_conflict(vault, device, op_id, &rel, &task.title, conflict) {
                Ok(()) => review_changed += 1,
                Err(error) => eprintln!("[companion] {op_id}: conflict card: {error}"),
            }
        }
        receipt.conflicts = conflicts;
        return Ok(Outcome::Done {
            receipt,
            review_changed,
        });
    }
    Err(Step::Busy(
        "the task kept changing while the op applied".into(),
    ))
}

// ── Conflict cards ───────────────────────────────────────────────────────

fn field_label(field: &str) -> &str {
    match field {
        "dueTime" => "due time",
        other => other,
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::Null => "(empty)".into(),
        Value::String(s) if s.trim().is_empty() => "(empty)".into(),
        Value::String(s) => s.trim().to_string(),
        Value::Array(items) if items.is_empty() => "(empty)".into(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

fn enqueue_conflict(
    vault: &Path,
    device: &Device,
    op_id: &str,
    rel: &str,
    task_title: &str,
    conflict: &FieldConflict,
) -> Result<(), String> {
    let label = field_label(&conflict.field);
    crate::review_queue::enqueue(
        vault,
        crate::review_queue::NewReviewItem {
            kind: CONFLICT_KIND.into(),
            fingerprint: format!("companion-conflict:{op_id}:{}", conflict.field),
            payload: json!({
                "title": format!("“{task_title}”: {label} changed on {} and on the Mac", device.name),
                "body": format!(
                    "On {}: {}. On the Mac: {}. Before both: {}.",
                    device.name, shown(&conflict.device), shown(&conflict.mac), shown(&conflict.base)
                ),
                "ref": format!("vault:{rel}"),
                "field": conflict.field,
                "base": conflict.base,
                "device": conflict.device,
                "mac": conflict.mac,
                "opId": op_id,
                "deviceId": device.device_id,
                "deviceName": device.name,
                "taskTitle": task_title,
            }),
            risk: "local_write".into(),
            source: SOURCE.into(),
            source_path: Some(rel.to_string()),
        },
    )
    .map(|_| ())
}

/// **Use the device's value** on a `companion_conflict` card (the person's
/// click; `commands::resolve_review_item`). Re-checked against the task as
/// it is now: if the Mac's side changed again since the card, nothing is
/// written and the card returns to pending with that reason.
pub(crate) fn accept_conflict(
    vault: &Path,
    store: &Store,
    item: &crate::review_queue::ReviewItem,
    now: DateTime<Local>,
) -> Result<(), String> {
    // The card lives in `.review-queue.json`, which a run can write, so only
    // its ids are taken from it: the field and both values come from the
    // signed op's receipt in the ledger (outside the vault).
    let p = &item.payload;
    let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default();
    let (field, device_id, op_id) = (text("field").to_string(), text("deviceId"), text("opId"));
    if !(UPDATE_FIELDS.contains(&field.as_str()) || field == "status")
        || !DEVICE_ID.is_match(device_id)
        || !OP_ID.is_match(op_id)
    {
        return Err(
            "This card doesn't name a companion change Hollow recorded; reject it instead.".into(),
        );
    }
    let receipt = store
        .seen(device_id, op_id)?
        .ok_or("Hollow has no record of this device change; reject the card instead.")?;
    let conflict = receipt
        .conflicts
        .iter()
        .find(|c| c.field == field)
        .cloned()
        .ok_or("Hollow's record of this change has no conflict on that field; reject the card instead.")?;
    // A repeating task's refused finish carries a reason and never got a
    // card; accepting one would roll the task forward a second time.
    if receipt.reason.is_some() {
        return Err("Hollow didn't make a card for this change; reject it instead.".into());
    }
    let reference = receipt.reference.clone().unwrap_or_default();
    let device = store
        .device(device_id)
        .ok_or("That device was revoked, so its value can't be used; reject the card instead.")?;
    crate::review_queue::claim(vault, &item.id, None)?;
    let applied = (|| -> Result<(), String> {
        let rel = vault_tasks::resolve_task_ref(vault, &reference)?;
        let task =
            vault_tasks::get(vault, &rel).ok_or_else(|| format!("{rel} is not a readable task"))?;
        let cur = norm(&field, &current(&task, &field))?;
        if cur == norm(&field, &conflict.device)? {
            return Ok(());
        }
        if cur != norm(&field, &conflict.mac)? {
            return Err("The task changed again since this card was made; nothing was changed. Edit it on the Mac.".into());
        }
        let args = if field == "status" {
            json!({ "ref": format!("vault:{rel}"), "status": conflict.device, "expectedHash": task.content_hash })
        } else {
            json!({ "ref": format!("vault:{rel}"), "expectedHash": task.content_hash,
                    "fields": { field.clone(): broker_value(&field, &conflict.device) } })
        };
        let op = if field == "status" {
            "set_status"
        } else {
            "update_task"
        };
        dispatch(vault, &device, op_id, op, args, now)
            .map(|_| ())
            .map_err(|e| match e {
                CompanionRefusal::Busy(m) | CompanionRefusal::Refused(m) => m,
            })
    })();
    match applied {
        Ok(()) => crate::review_queue::accept(vault, &item.id).map(|_| ()),
        Err(error) => {
            let _ = crate::review_queue::fail(vault, &item.id, &error);
            Err(error)
        }
    }
}

// ── Old ops (design §3.6) ────────────────────────────────────────────────

/// What the device hears when the person discards an old op.
const DISCARDED: &str =
    "You chose not to apply this change: it was sent more than 14 days before Hollow saw it.";
const VAULT_BUSY: &str = "A run is using the vault, so nothing was applied. Try again shortly.";
/// Apply it on a card whose ids or op details no longer match the held op
/// (card text lives in the vault, where an agent can edit it).
const CARD_CHANGED: &str = "This card changed; discard it.";
/// Why a revoked device's old-op cards close.
const REVOKED: &str = "Superseded: the device that sent this change was unpaired.";

/// The plain name of an op's target: a task's title, else the file's name.
fn target_name(vault: &Path, reference: &str) -> String {
    let rel = rel_of(reference);
    let task = vault_tasks::resolve_task_ref(vault, reference)
        .ok()
        .and_then(|rel| vault_tasks::get(vault, &rel));
    match task {
        Some(task) => format!("“{}”", task.title.trim()),
        None => {
            let name = rel.rsplit('/').next().unwrap_or_default();
            format!("“{}”", name.trim_end_matches(".md"))
        }
    }
}

/// A workspace's plain name: its brief's `title`, else its id.
fn workspace_name(vault: &Path, id: &str) -> String {
    let text = crate::workspaces::folder(vault, id)
        .and_then(|dir| read_bounded(&dir.join("WORKSPACE.md"), MAX_ENVELOPE_BYTES))
        .unwrap_or_default();
    let (front, _) = crate::frontmatter::parse_map(&text);
    let title = front.get("title").map_or("", |title| title.trim());
    format!("“{}”", if title.is_empty() { id } else { title })
}

/// What an op would do, in a few plain words for its card.
fn op_summary(vault: &Path, p: &Map<String, Value>) -> String {
    let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default();
    let target = || target_name(vault, text("ref"));
    match text("kind") {
        "task.create" => format!("New task “{}”", text("title").trim()),
        "task.update" => format!("Changes to {}", target()),
        "task.set_status" => format!("{} set to “{}”", target(), text("to").trim()),
        "task.comment" => format!("A comment on {}", target()),
        "note.create" => format!("New note “{}”", text("title").trim()),
        "note.edit" => format!("An edit to {}", target()),
        "note.append" => format!("An addition to {}", target()),
        "workspace.append" => {
            let file = match text("file") {
                "brief" => "brief",
                _ => "plan",
            };
            let workspace = workspace_name(vault, text("workspace"));
            format!("A line for the {file} of {workspace}")
        }
        _ => "A change".into(),
    }
}

/// The op's own words (a comment, an added line, a new note's body), on one
/// line and cut to a preview.
fn op_text(p: &Map<String, Value>) -> Option<String> {
    let key = match p.get("kind").and_then(Value::as_str) {
        Some("note.create") => "body",
        _ => "text",
    };
    let text = p.get(key).and_then(Value::as_str)?;
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let text = words.join(" ");
    let mut preview: String = text.chars().take(160).collect();
    if preview.len() < text.len() {
        preview.push('…');
    }
    Some(preview)
}

/// The one card an old op gets: its fingerprint names the device and op.
fn old_op_fingerprint(device_id: &str, op_id: &str) -> String {
    format!("companion-old-op:{device_id}:{op_id}")
}

/// Ask the person about an old op: a `companion_old_op` card that names the
/// op's ids only (the op itself stays in the ledger's database, outside the
/// vault). One card per op, however often it is delivered.
fn enqueue_old_op(
    vault: &Path,
    device: &Device,
    op_id: &str,
    p: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<(), String> {
    let sent = sent_at(p).unwrap_or_else(|| now.with_timezone(&Utc));
    let days = (now.with_timezone(&Utc) - sent).num_days();
    let what = op_summary(vault, p);
    let date = sent.with_timezone(&Local).format("%b %-d");
    let mut body = format!(
        "{} sent this on {date}, {days} days ago, and Hollow is only seeing it now. \
         A change this old waits for you: Apply it makes it now; Discard it drops it.",
        device.name
    );
    if let Some(words) = op_text(p) {
        body = format!("{body} It says: “{words}”");
    }
    let reference = p.get("ref").and_then(Value::as_str).map(rel_of);
    crate::review_queue::enqueue(
        vault,
        crate::review_queue::NewReviewItem {
            kind: OLD_OP_KIND.into(),
            fingerprint: old_op_fingerprint(&device.device_id, op_id),
            payload: json!({
                "title": format!("{what}, sent from {} {days} days ago", device.name),
                "body": body,
                "opId": op_id,
                "opKind": p.get("kind"),
                "deviceId": device.device_id,
                "deviceName": device.name,
                "sentAt": p.get("at"),
                "ageDays": days,
                "ref": p.get("ref"),
            }),
            risk: "local_write".into(),
            source: SOURCE.into(),
            source_path: reference,
        },
    )
    .map(|_| ())
}

/// The ids an old-op card names (only ids: the op comes from the ledger).
/// Its fingerprint must name the same ones.
fn old_op_ids(item: &crate::review_queue::ReviewItem) -> Result<(String, String), String> {
    let p = &item.payload;
    let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default();
    let (device_id, op_id) = (text("deviceId"), text("opId"));
    if item.kind != OLD_OP_KIND || !DEVICE_ID.is_match(device_id) || !OP_ID.is_match(op_id) {
        return Err("This card doesn't name a change Hollow kept; discard it instead.".into());
    }
    // The review queue stores fingerprints lowercased (`review_queue::enqueue`).
    if item.fingerprint != old_op_fingerprint(device_id, op_id).to_lowercase() {
        return Err(CARD_CHANGED.into());
    }
    Ok((device_id.to_string(), op_id.to_string()))
}

/// The card still describes the held op: the kind and signed `at` it was
/// raised with match the kept envelope's.
fn card_matches(item: &crate::review_queue::ReviewItem, held: &Held) -> Result<(), String> {
    let signed: Map<String, Value> = serde_json::from_str::<Envelope>(&held.envelope)
        .ok()
        .and_then(|env| serde_json::from_str(&env.payload).ok())
        .unwrap_or_default();
    let same = |card_key: &str, op_key: &str| item.payload.get(card_key) == signed.get(op_key);
    if same("opKind", "kind") && same("sentAt", "at") {
        Ok(())
    } else {
        Err(CARD_CHANGED.into())
    }
}

/// **Apply it** on a `companion_old_op` card (the person's click;
/// `commands::resolve_review_item`). The card must still name and describe
/// the held op. The op is the held envelope (outside the vault), verified
/// again against the device's key and applied through the same path and
/// ledger as a fresh op, so it applies once: a second click, or the op
/// arriving again, finds the recorded receipt. A refusal by validation is
/// final, so the card closes with its reason; any other error returns the
/// card to pending with it.
pub(crate) fn accept_old_op(
    vault: &Path,
    store: &Store,
    item: &crate::review_queue::ReviewItem,
    now: DateTime<Local>,
) -> Result<(), String> {
    let (device_id, op_id) = old_op_ids(item)?;
    let _pass = pass_lock();
    if let Some(held) = store.held(&device_id, &op_id)? {
        card_matches(item, &held)?;
    }
    crate::review_queue::claim(vault, &item.id, None)?;
    match apply_held(vault, store, &device_id, &op_id, now) {
        Ok(receipt) if receipt.status == "refused" => {
            let reason = refused_message(&receipt);
            if let Err(error) = crate::review_queue::refuse(vault, &item.id, &reason) {
                eprintln!("[companion] {op_id}: card not closed: {error}");
            }
            Err(reason)
        }
        Ok(_) => crate::review_queue::accept(vault, &item.id).map(|_| ()),
        Err(error) => {
            let _ = crate::review_queue::fail(vault, &item.id, &error);
            Err(error)
        }
    }
}

/// Apply a held op once. Ok is its receipt, decided now or before; Err
/// leaves it held for another try.
fn apply_held(
    vault: &Path,
    store: &Store,
    device_id: &str,
    op_id: &str,
    now: DateTime<Local>,
) -> Result<Receipt, String> {
    let held = store.held(device_id, op_id)?;
    if let Some(receipt) = store.seen(device_id, op_id)? {
        // Already decided: a second click, or a stop before the held copy
        // was settled (so settle it now).
        if let Some(held) = &held {
            answer_held(vault, store, held, &receipt)?;
        }
        return Ok(receipt);
    }
    let held = held.ok_or("Hollow no longer has this change; discard the card.")?;
    let (device, payload) = open_envelope_text(store, device_id, op_id, &held.envelope)
        .map_err(|reason| format!("Hollow can't apply this change: {reason}"))?;
    match apply_payload(vault, store, &device, op_id, &payload, now) {
        Outcome::Waiting | Outcome::Held { .. } => Err(VAULT_BUSY.into()),
        Outcome::Done { receipt, .. } => {
            record_applied(store, device_id, &held.kind, &receipt)?;
            answer_held(vault, store, &held, &receipt)?;
            Ok(receipt)
        }
    }
}

fn refused_message(receipt: &Receipt) -> String {
    let reason = receipt.reason.as_deref().unwrap_or("no reason given.");
    format!("Hollow couldn't apply this change: {reason}")
}

/// Answer a decided held op on each road it came by. An inbox copy gets
/// its receipt file. A backend copy gets an `outbox` row that
/// `companion_bridge` posts as a receipt: the daemon may have expired the op
/// meanwhile and would never hand it out again, so polling can't answer it.
/// The held copy goes in the same ledger transaction as that row.
fn answer_held(vault: &Path, store: &Store, held: &Held, receipt: &Receipt) -> Result<(), String> {
    let op_id = &receipt.op_id;
    if held.via_inbox {
        note_receipt_error(write_receipt(vault, receipt), op_id);
    }
    store.settle(&receipt.device_id, op_id, held.via_backend, &receipt.at)
}

/// The op a discarded old-op card answers: the one it was raised for, read
/// from its fingerprint (`old_op_fingerprint`), which an edit to the card's
/// details doesn't change. So Discard on a card an agent edited still
/// answers that op rather than leaving it held with no card, and a details
/// edit can't point Discard at another op. (A raw edit of the queue file
/// could; the run guard flags it, and a discard can only refuse.) `None` for
/// a card with no such fingerprint: it is just rejected.
fn discard_target(item: &crate::review_queue::ReviewItem) -> Option<(String, String)> {
    if item.kind != OLD_OP_KIND {
        return None;
    }
    // The queue lowercases fingerprints; op ids are uppercase (their
    // alphabet has no I, L, O or U, so this is lossless).
    item.fingerprint
        .strip_prefix("companion-old-op:")
        .and_then(|rest| rest.split_once(':'))
        .map(|(device, op)| (device.to_string(), op.to_ascii_uppercase()))
        .filter(|(device, op)| DEVICE_ID.is_match(device) && OP_ID.is_match(op))
}

/// **Discard it** on a `companion_old_op` card: the held op it was raised
/// for ([`discard_target`]) is answered `refused` (the device stops waiting)
/// and never applied, then the card is rejected. A card that names no held
/// op is simply rejected.
pub(crate) fn discard_old_op(
    vault: &Path,
    store: &Store,
    item: &crate::review_queue::ReviewItem,
    now: DateTime<Local>,
) -> Result<crate::review_queue::ReviewItem, String> {
    if !matches!(item.status.as_str(), "pending" | "snoozed") {
        return Err(format!("review item {} is {}", item.id, item.status));
    }
    if let Some((device_id, op_id)) = discard_target(item) {
        let _pass = pass_lock();
        if let Some(held) = store.held(&device_id, &op_id)? {
            let receipt = match store.seen(&device_id, &op_id)? {
                Some(receipt) => receipt,
                None => {
                    let receipt = Receipt::refused(&op_id, &device_id, now, DISCARDED);
                    store.record(&held.kind, &receipt)?;
                    receipt
                }
            };
            answer_held(vault, store, &held, &receipt)?;
        }
    }
    crate::review_queue::reject(vault, &item.id)
}

/// A revoked device's held ops go (they could never verify again) and their
/// cards close as superseded.
fn forget_old_ops(vault: &Path, store: &Store, device_id: &str) {
    if let Err(error) = store.forget_held(device_id) {
        eprintln!("[companion] {device_id}: held ops not removed: {error}");
    }
    let prefix = old_op_fingerprint(device_id, "");
    let cards = crate::review_queue::list(vault, Some(OLD_OP_KIND)).unwrap_or_default();
    for card in cards {
        let live = matches!(card.status.as_str(), "pending" | "snoozed");
        let mine = live && card.fingerprint.starts_with(&prefix);
        if !mine {
            continue;
        }
        if let Err(error) = crate::review_queue::supersede(vault, &card.id, REVOKED) {
            eprintln!("[companion] {}: card not closed: {error}", card.id);
        }
    }
}

// ── Files ────────────────────────────────────────────────────────────────

/// `vault/.hollow-sync/<parts…>` when every component is a real folder (no
/// symlink anywhere: a link could aim Hollow's deletes and writes outside
/// the vault). With `create`, missing components are made one at a time.
fn safe_dir(vault: &Path, parts: &[&str], create: bool) -> Result<PathBuf, String> {
    let mut dir = vault.to_path_buf();
    for part in std::iter::once(SYNC_DIR).chain(parts.iter().copied()) {
        if part.is_empty() || part == "." || part == ".." || part.contains('/') {
            return Err(format!("bad sync folder name `{part}`"));
        }
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(format!("{} is not a plain folder", dir.display())),
            Err(e) if create && e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                if !fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
                    return Err(format!("{} is not a plain folder", dir.display()));
                }
            }
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        }
    }
    Ok(dir)
}

/// Read a regular file of at most `max` bytes without following a symlink
/// or blocking on a FIFO swapped in after the listing.
fn read_bounded(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| format!("unreadable: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > max {
        return Err(format!("the file is over {max} bytes"));
    }
    let mut text = String::new();
    file.take(max + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("unreadable: {e}"))?;
    if text.len() as u64 > max {
        return Err(format!("the file is over {max} bytes"));
    }
    Ok(text)
}

/// Visible subfolders of `dir`, sorted, with their names.
fn dirs(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (!name.starts_with('.')).then(|| (e.path(), name))
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Visible `*.json` files of `dir`, sorted by name, with their stems. Dot
/// files (a device's temp file mid-write) are skipped.
fn json_files(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".json")?.to_string();
            (!name.starts_with('.')).then(|| (e.path(), stem))
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn prune_receipts(vault: &Path) {
    let keep = std::time::Duration::from_secs(RECEIPT_KEEP_DAYS * 86_400);
    let Ok(receipts) = safe_dir(vault, &["receipts"], false) else {
        return;
    };
    for (dir, _) in dirs(&receipts) {
        for (path, _) in json_files(&dir) {
            let old = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > keep);
            if old {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Ops waiting in the inbox (Settings shows the count).
pub(crate) fn waiting_ops(vault: &Path) -> usize {
    let Ok(inbox) = safe_dir(vault, &["inbox"], false) else {
        return 0;
    };
    dirs(&inbox)
        .iter()
        .map(|(dir, _)| json_files(dir).len())
        .sum()
}

// ── Running inside Hollow ────────────────────────────────────────────────

/// Ask the background loop for a pass now (a new pairing code, say).
pub(crate) fn kick() {
    if let Some(tx) = KICK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(());
    }
}

/// Start the applier: a pass at launch (ops sent while Hollow was quit),
/// then on every change under `.hollow-sync/` and every [`SCAN_SECS`]
/// (iCloud doesn't always raise file events promptly).
pub(crate) fn start(app: impl Into<crate::core_host::Core>, vault: PathBuf) {
    let app = app.into();
    let Some(store) = Store::for_app() else {
        eprintln!("[companion] no support folder; the companion applier is off");
        return;
    };
    let (tx, rx) = mpsc::channel::<()>();
    *KICK.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
    let spawned = std::thread::Builder::new()
        .name("companion-inbox".into())
        .spawn(move || {
            let mut watcher: Option<notify::RecommendedWatcher> = None;
            loop {
                if watcher.is_none() && safe_dir(&vault, &[], false).is_ok() {
                    watcher = watch(&vault.join(SYNC_DIR), tx.clone());
                }
                let summary = process(&vault, &store, Local::now());
                if summary.changed() {
                    let _ = app.emit(EVENT, &summary);
                }
                if summary.review_changed > 0 {
                    let _ = app.emit(task_broker::REVIEW_EVENT, summary.review_changed);
                }
                match rx.recv_timeout(std::time::Duration::from_secs(SCAN_SECS)) {
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    _ => {
                        // Debounce a burst (a device writing temp + rename).
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        while rx.try_recv().is_ok() {}
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("[companion] could not start the applier: {error}");
    }
}

fn watch(dir: &Path, tx: mpsc::Sender<()>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    // Only device writes wake a pass; Hollow's own receipts and device
    // answers would otherwise wake it again.
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let from_device = event.is_ok_and(|e| {
            e.paths.iter().any(|p| {
                p.components()
                    .any(|c| c.as_os_str() == "inbox" || c.as_os_str() == "pair")
            })
        });
        if from_device {
            let _ = tx.send(());
        }
    })
    .ok()?;
    watcher.watch(dir, notify::RecursiveMode::Recursive).ok()?;
    Some(watcher)
}

/// What Settings, Devices shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    pub devices: Vec<Device>,
    pub pairing: Option<PairingCode>,
    pub waiting: usize,
}

fn status(vault: &Path, store: &Store) -> CompanionStatus {
    CompanionStatus {
        devices: store.devices(),
        pairing: open_code(Utc::now()),
        waiting: waiting_ops(vault),
    }
}

fn app_store() -> Result<Store, String> {
    Store::for_app().ok_or_else(|| "Hollow's support folder is unavailable.".into())
}

/// Settings' reading, the open code included (the window only; an agent
/// gets [`for_agent`]'s).
pub(crate) fn status_now(vault: &Path) -> Result<CompanionStatus, String> {
    Ok(status(vault, &app_store()?))
}

#[tauri::command]
pub fn companion_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<CompanionStatus, String> {
    status_now(&state.config.vault_path)
}

/// Open a pairing code: creates `.hollow-sync/pair/` so a device can answer.
/// The person's click returns the code to Settings; the COO's action
/// ([`agent_pairing_start`]) never sees it.
pub(crate) fn pairing_start(vault: &Path) -> Result<PairingCode, String> {
    for sub in ["pair", "inbox", "devices", "receipts"] {
        safe_dir(vault, &[sub], true)?;
    }
    let code = open_pairing(Utc::now());
    kick();
    Ok(code)
}

#[tauri::command]
pub fn companion_pairing_start(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<PairingCode, String> {
    pairing_start(&state.config.vault_path)
}

pub(crate) fn pairing_cancel() {
    close_pairing();
}

#[tauri::command]
pub fn companion_pairing_cancel() {
    pairing_cancel();
}

/// Revoke a paired device and read the list again. Off the main thread:
/// [`revoke`] waits for the pass lock, which a pass holds while it applies a
/// batch, and the window would freeze meanwhile.
pub(crate) async fn revoke_device(
    vault: PathBuf,
    device_id: String,
) -> Result<CompanionStatus, String> {
    let store = app_store()?;
    tauri::async_runtime::spawn_blocking(move || -> Result<CompanionStatus, String> {
        revoke(&vault, &store, device_id.trim())?;
        Ok(status(&vault, &store))
    })
    .await
    .map_err(|e| format!("revoke: {e}"))?
}

#[tauri::command]
pub async fn companion_revoke_device(
    state: tauri::State<'_, crate::commands::AppState>,
    device_id: String,
) -> Result<CompanionStatus, String> {
    revoke_device(state.config.vault_path.clone(), device_id).await
}

/// The COO's `companion.revoke_device` (`Lease::Own`): the pass lock first,
/// then the change inside the caller's own History lease, the order a pass
/// takes them in. Off the main thread like the window's.
pub(crate) async fn agent_revoke_device(
    vault: PathBuf,
    device_id: String,
    caller: crate::app_actions::Caller,
) -> Result<AgentCompanionStatus, String> {
    let store = app_store()?;
    tauri::async_runtime::spawn_blocking(move || -> Result<AgentCompanionStatus, String> {
        revoke_bound(&vault, &store, device_id.trim(), |mutate| {
            crate::app_broker::live_write(&vault, &caller, mutate)
        })?;
        Ok(for_agent(status(&vault, &store)))
    })
    .await
    .map_err(|_| "Revoking the device stopped before it finished.".to_string())?
}

// ── The COO's actions (L13) ──────────────────────────────────────────────
//
// A pairing code lets a device act as the person, so what an agent gets
// back never holds one: whether a code is open and until when, and the
// paired devices. The code itself shows only in Hollow's Settings.

/// Companion devices as the COO sees them: never the open code.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompanionStatus {
    pub devices: Vec<Device>,
    /// A pairing code is open now (Settings shows it to the person).
    pub pairing_open: bool,
    pub pairing_expires_at: Option<String>,
    pub waiting: usize,
}

/// `status` without its pairing code.
pub(crate) fn for_agent(status: CompanionStatus) -> AgentCompanionStatus {
    AgentCompanionStatus {
        devices: status.devices,
        pairing_open: status.pairing.is_some(),
        pairing_expires_at: status.pairing.map(|p| p.expires_at),
        waiting: status.waiting,
    }
}

/// What the COO's pairing start returns: that a code is open, never the code.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPairingStarted {
    pub started: bool,
    pub expires_at: String,
    pub note: String,
}

/// The COO opens a pairing code (after the person's card): the code shows
/// in Hollow's Settings, which is told to read it again, and only there.
pub(crate) fn agent_pairing_start(
    vault: &Path,
    app: Option<&tauri::AppHandle>,
) -> Result<AgentPairingStarted, String> {
    let code = pairing_start(vault)?;
    if let Some(app) = app {
        use tauri::Emitter;
        let _ = app.emit(EVENT, ());
    }
    Ok(AgentPairingStarted {
        started: true,
        expires_at: code.expires_at,
        note: "A pairing code is open for 10 minutes. The person reads it in Hollow's Settings (Devices, Companion devices) and types it on the device; agents never see it.".into(),
    })
}

/// The COO closes the open pairing code (it only narrows access).
pub(crate) fn agent_pairing_cancel(
    vault: &Path,
    app: Option<&tauri::AppHandle>,
) -> Result<AgentCompanionStatus, String> {
    pairing_cancel();
    if let Some(app) = app {
        use tauri::Emitter;
        let _ = app.emit(EVENT, ());
    }
    status_now(vault).map(for_agent)
}

// ── Fixtures for the companion apps ──────────────────────────────────────

const FIXTURE_TODAY: &str = "2026-10-12";

const FIXTURE_TASKS: [(&str, &str); 6] = [
    ("tasks/call-jordan.md", "---\ntitle: Call Jordan about the variance report\nstatus: Not started\npriority: P2\ndue: \"2026-10-12\"\ndue_time: \"16:00\"\nlabels: \"call, claims\"\ncreated: \"2026-10-10T14:00:00Z\"\ntags: [tasks, status/not-started, priority/p2]\n---\n\nAsk about the September numbers.\n\n- [ ] Pull the report\n- [x] Book the call\n"),
    ("tasks/renew-parking-permits.md", "---\ntitle: \"Renew parking permits: Lot 4\"\nstatus: In progress\npriority: P1\ndue: \"2026-10-09\"\nduration_min: 45\nproject: Operations\nproject_url: \"vault:projects/operations.md\"\n---\n\nOverdue since Friday.\n"),
    ("tasks/weekly-review.md", "---\ntitle: Weekly review\nstatus: Not started\npriority: P3\ndue: \"2026-10-12\"\nrecurrence: every friday\nsection: Rituals\n---\n"),
    ("tasks/notion-34bbaaaaaaaaaaaaaaaaaaaaaaaec27b.md", "---\ntitle: \"Synced from Notion\"\nstatus: \"In progress\"\npriority: \"High\"\ndue: \"2026-10-20\"\ndue_end: \"\"\ndue_time_zone: \"\"\ndescription: \"Remote\"\nnotion_project_ids: \"[]\"\nproject_url: \"\"\nproject: \"\"\nnotion_id: \"34bbaaaaaaaaaaaaaaaaaaaaaaaec27b\"\nnotion_url: \"https://www.notion.so/34bbaaaaaaaaaaaaaaaaaaaaaaaec27b\"\n---\n\nBody.\n"),
    ("tasks/no-date.md", "---\ntitle: Someday maybe\nstatus: Not started\nlabels:\n  - ideas\n  - later\n---\n\nNo due date.\n"),
    ("tasks/finished.md", "---\ntitle: Finished thing\nstatus: Done\npriority: P4\ndue: \"2026-10-01\"\n---\n"),
];

const FIXTURE_QUICK_ADD: [&str; 14] = [
    "Call Jordan tomorrow 4pm p1 #Claims @call",
    "Weekly review every friday",
    "Send the report in 3 days p2",
    "Dentist oct 15 at 9:30",
    "Pay invoice 10/20",
    "Plan offsite for 1h 30m next monday",
    "Lunch with Ana noon @people",
    "Ship v2 #\"Big project\" @\"deep work\"",
    "Standup daily 9am",
    "Budget review 15 nov p3",
    "Clean desk tonight",
    "Water plants every weekday",
    "Quarterly taxes in 2 weeks",
    "Just a title",
];

fn task_fixture(task: &Task, rel: &str, content: &str) -> Value {
    json!({
        "file": rel,
        "content": content,
        "parse": task_json(task, rel),
    })
}

/// A task as devices see it: the fixtures' `parse` shape, also what the
/// backend mirror holds (`companion_bridge`). A tracker mirror also carries
/// `statusType`, `trackerKey` and `trackerUrl` from `task.tracker`, the same
/// source and spelling as the phone API's `/v1/tasks` rows
/// (`docs/phone-api/tracker.md`); a local task omits them.
pub(crate) fn task_json(task: &Task, rel: &str) -> Value {
    let mut row = json!({
            "ref": format!("vault:{rel}"),
            "title": task.title,
            "status": task.status,
            "priority": task.priority,
            "due": task.due,
            "dueTime": task.due_time,
            "durationMin": task.duration_min,
            "labels": task.labels,
            "recurrence": task.recurrence,
            "projectUrl": task.project_url,
            "section": task.section,
            "description": task.description,
            "subtasks": task.subtasks.iter().map(|s| json!({"text": s.text, "done": s.done})).collect::<Vec<_>>(),
            "contentHash": task.content_hash,
            "trackerLinked": task.tracker.is_some(),
            "notionLinked": task.tracker.is_some() || task.notion_id.is_some(),
    });
    if let (Some(m), Some(fields)) = (task.tracker.as_ref(), row.as_object_mut()) {
        if let Some(kind) = m.status_type {
            fields.insert("statusType".into(), json!(kind));
        }
        if let Some(key) = &m.key {
            fields.insert("trackerKey".into(), json!(key));
        }
        if let Some(url) = &m.url {
            fields.insert("trackerUrl".into(), json!(url));
        }
    }
    row
}

/// A device key for fixtures and tests: `(pkcs8, base64 public key)`.
pub(crate) fn test_key() -> (Vec<u8>, String) {
    use ring::signature::KeyPair;
    let rng = ring::rand::SystemRandom::new();
    let alg = &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref(), &rng).unwrap();
    let public = B64.encode(pair.public_key().as_ref());
    (pkcs8.as_ref().to_vec(), public)
}

pub(crate) fn test_sign(pkcs8: &[u8], payload: &str) -> String {
    let rng = ring::rand::SystemRandom::new();
    let alg = &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng).unwrap();
    B64.encode(pair.sign(&rng, payload.as_bytes()).unwrap().as_ref())
}

/// A signed envelope for `payload` (whose `v`, `opId`, `deviceId`, `at` are
/// filled in). `at` defaults to now: an op sent more than 14 days ago is
/// held for a confirm, so a fixed date would turn every case into a card.
pub(crate) fn test_envelope(
    pkcs8: &[u8],
    device_id: &str,
    op_id: &str,
    mut payload: Map<String, Value>,
) -> Envelope {
    payload.insert("v".into(), json!(1));
    payload.insert("opId".into(), json!(op_id));
    payload.insert("deviceId".into(), json!(device_id));
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    payload.entry("at").or_insert(json!(now));
    let text = serde_json::to_string(&payload).unwrap();
    Envelope {
        v: 1,
        device_id: device_id.into(),
        op_id: op_id.into(),
        sig: test_sign(pkcs8, &text),
        payload: text,
    }
}

pub(crate) fn drop_op(vault: &Path, env: &Envelope) -> PathBuf {
    let dir = vault.join(SYNC_DIR).join("inbox").join(&env.device_id);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.json", env.op_id));
    fs::write(&path, serde_json::to_string(env).unwrap()).unwrap();
    path
}

fn read_tasks(vault: &Path) -> Vec<Value> {
    let mut files: Vec<(String, String)> = fs::read_dir(vault.join("tasks"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            Some((format!("tasks/{name}"), fs::read_to_string(e.path()).ok()?))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|(file, content)| json!({"file": file, "content": content}))
        .collect()
}

/// A temp folder removed on drop (`tempfile` is a dev-only dependency).
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("hollow-companion-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `hollow companion-fixtures <dir>`: golden fixtures produced by the real
/// parser, quick-add parser, pairing proof, signature check and applier (in
/// a temporary vault), for the companion repo's Swift / TypeScript tests.
pub fn export_fixtures(out: &Path, commit: &str) -> Result<PathBuf, String> {
    let today = chrono::NaiveDate::parse_from_str(FIXTURE_TODAY, "%Y-%m-%d").unwrap();
    let temp = TempDir::new()?;
    let vault = temp.0.join("vault");
    fs::create_dir_all(vault.join("tasks")).map_err(|e| e.to_string())?;
    let mut tasks = Vec::new();
    for (rel, content) in FIXTURE_TASKS {
        fs::write(vault.join(rel), content).map_err(|e| e.to_string())?;
        let task =
            vault_tasks::get(&vault, rel).ok_or_else(|| format!("fixture {rel} didn't parse"))?;
        tasks.push(task_fixture(&task, rel, content));
    }
    let quick_add: Vec<Value> = FIXTURE_QUICK_ADD
        .iter()
        .map(|input| json!({ "input": input, "today": FIXTURE_TODAY, "output": crate::quick_add::parse(input, today) }))
        .collect();
    let (pkcs8, public) = test_key();
    let device_id = "mac-0a1b2c3d";
    let pairing = vec![json!({
        "code": "7K3M-Q9TX", "deviceId": device_id, "publicKey": public,
        "proof": pair_proof("7K3M-Q9TX", device_id, &public),
    })];
    let payload = r#"{"v":1,"opId":"01JA9M3Q8Z5X7Y2W4V6T8S0R1P","deviceId":"mac-0a1b2c3d","at":"2026-10-12T20:41:07.123Z","kind":"task.comment","ref":"vault:tasks/call-jordan.md","text":"Left a voicemail — café ☕"}"#;
    let sig = test_sign(&pkcs8, payload);
    let signatures = vec![
        json!({ "publicKey": public, "payload": payload, "sig": sig, "valid": verify(&public, payload, &sig) }),
        json!({ "publicKey": public, "payload": payload.replace("voicemail", "message"), "sig": sig, "valid": false }),
    ];

    // Op cases through the real applier.
    let store = Store::at(temp.0.join("store"));
    store.save_devices(&[Device {
        device_id: device_id.into(),
        name: "Fixture Mac".into(),
        platform: "macos".into(),
        public_key: public.clone(),
        paired_at: "2026-10-12T20:00:00Z".into(),
        last_op_at: None,
    }])?;
    let now = Local::now();
    let mut ops = Vec::new();
    let mut seq = 0u32;
    let mut case = |name: &str,
                    payload: Value,
                    sign: bool,
                    edit: Option<(&str, &str)>|
     -> Result<(), String> {
        seq += 1;
        let op_id = format!("01JA9M3Q8Z5X7Y2W4V6T8S{seq:04}");
        if let Some((rel, text)) = edit {
            fs::write(vault.join(rel), text).map_err(|e| e.to_string())?;
        }
        let before = read_tasks(&vault);
        let mut env = test_envelope(
            &pkcs8,
            device_id,
            &op_id,
            payload.as_object().cloned().unwrap(),
        );
        if !sign {
            let (other, _) = test_key();
            env.sig = test_sign(&other, &env.payload);
        }
        drop_op(&vault, &env);
        process(&vault, &store, now);
        let receipt: Value = serde_json::from_str(
            &fs::read_to_string(
                vault
                    .join(SYNC_DIR)
                    .join("receipts")
                    .join(device_id)
                    .join(format!("{op_id}.json")),
            )
            .map_err(|e| format!("{name}: no receipt: {e}"))?,
        )
        .map_err(|e| e.to_string())?;
        ops.push(json!({ "name": name, "before": before, "envelope": env, "receipt": receipt, "after": read_tasks(&vault) }));
        Ok(())
    };
    let hash = |rel: &str| {
        vault_tasks::get(&vault, rel)
            .map(|t| t.content_hash)
            .unwrap_or_default()
    };
    case(
        "create",
        json!({ "kind": "task.create", "title": "Send the report", "priority": "P2", "due": "2026-10-15", "labels": ["work"] }),
        true,
        None,
    )?;
    case(
        "complete",
        json!({ "kind": "task.set_status", "ref": "vault:tasks/call-jordan.md", "baseHash": hash("tasks/call-jordan.md"), "from": "Not started", "to": "Done" }),
        true,
        None,
    )?;
    case(
        "complete-recurring-rolls-forward",
        json!({ "kind": "task.set_status", "ref": "vault:tasks/weekly-review.md", "baseHash": hash("tasks/weekly-review.md"), "from": "Not started", "to": "Done" }),
        true,
        None,
    )?;
    let stale = hash("tasks/renew-parking-permits.md");
    case(
        "update-hash-moved-no-overlap",
        json!({ "kind": "task.update", "ref": "vault:tasks/renew-parking-permits.md", "baseHash": stale, "fields": { "due": { "from": "2026-10-09", "to": "2026-10-14" } } }),
        true,
        Some((
            "tasks/renew-parking-permits.md",
            &FIXTURE_TASKS[1].1.replace("priority: P1", "priority: P2"),
        )),
    )?;
    let stale = hash("tasks/renew-parking-permits.md");
    case(
        "update-overlap-conflict",
        json!({ "kind": "task.update", "ref": "vault:tasks/renew-parking-permits.md", "baseHash": stale, "fields": { "due": { "from": "2026-10-14", "to": "2026-10-16" } } }),
        true,
        Some((
            "tasks/renew-parking-permits.md",
            &FIXTURE_TASKS[1]
                .1
                .replace("priority: P1", "priority: P2")
                .replace("2026-10-09", "2026-10-21"),
        )),
    )?;
    case(
        "comment",
        json!({ "kind": "task.comment", "ref": "vault:tasks/no-date.md", "text": "Maybe next quarter" }),
        true,
        None,
    )?;
    case(
        "bad-signature-refused",
        json!({ "kind": "task.set_status", "ref": "vault:tasks/no-date.md", "baseHash": hash("tasks/no-date.md"), "from": "Not started", "to": "Done" }),
        false,
        None,
    )?;
    case(
        "invalid-priority-refused",
        json!({ "kind": "task.update", "ref": "vault:tasks/no-date.md", "baseHash": hash("tasks/no-date.md"), "fields": { "priority": { "from": null, "to": "urgent" } } }),
        true,
        None,
    )?;

    let fixtures = json!({
        "v": 1, "hollowCommit": commit, "today": FIXTURE_TODAY,
        "tasks": tasks, "quickAdd": quick_add, "pairing": pairing, "signatures": signatures, "ops": ops,
    });
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let path = out.join("fixtures.json");
    fs::write(&path, serde_json::to_string_pretty(&fixtures).unwrap())
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// The canonical vault path when it is under the temp folder (the debug
/// end-to-end commands refuse any other vault).
#[cfg(debug_assertions)]
pub(crate) fn temp_vault_only(vault: &Path) -> Result<PathBuf, String> {
    let vault = fs::canonicalize(vault).map_err(|e| e.to_string())?;
    let temp = fs::canonicalize(std::env::temp_dir()).map_err(|e| e.to_string())?;
    let private_tmp = Path::new("/private/tmp");
    if !vault.starts_with(&temp) && !vault.starts_with(private_tmp) {
        return Err(format!(
            "refusing {}: only a vault under the temp folder",
            vault.display()
        ));
    }
    Ok(vault)
}

/// Open a given pairing code (debug end-to-end commands only).
#[cfg(debug_assertions)]
pub(crate) fn open_pairing_with(code: &str) {
    set_pairing(&normalize_code(code), Utc::now());
}

/// Tests that open or close the one global pairing code take this, so they
/// can't close each other's code.
#[cfg(test)]
pub(crate) fn pairing_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `hollow companion-apply <vault> <store> [--pair-code CODE]` (debug builds
/// only): one applier pass over a TEMPORARY vault, for end-to-end tests of
/// the companion apps. Refuses any vault outside the temp folder.
#[cfg(debug_assertions)]
pub fn apply_once_cli(
    vault: &Path,
    store: &Path,
    pair_code: Option<&str>,
) -> Result<Summary, String> {
    let vault = temp_vault_only(vault)?;
    if let Some(code) = pair_code {
        open_pairing_with(code);
    }
    Ok(process(
        &vault,
        &Store::at(store.to_path_buf()),
        Local::now(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = "mac-11223344";

    fn setup() -> (tempfile::TempDir, PathBuf, Store, Vec<u8>, String) {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("tasks")).unwrap();
        let store = Store::at(dir.path().join("store"));
        let (pkcs8, public) = test_key();
        store
            .save_devices(&[Device {
                device_id: DEVICE.into(),
                name: "Test Mac".into(),
                platform: "macos".into(),
                public_key: public.clone(),
                paired_at: "2026-10-12T20:00:00Z".into(),
                last_op_at: None,
            }])
            .unwrap();
        (dir, vault, store, pkcs8, public)
    }

    fn put(vault: &Path, rel: &str, text: &str) {
        fs::create_dir_all(vault.join(rel).parent().unwrap()).unwrap();
        fs::write(vault.join(rel), text).unwrap();
    }

    fn receipt(vault: &Path, op_id: &str) -> Receipt {
        let path = vault
            .join(SYNC_DIR)
            .join("receipts")
            .join(DEVICE)
            .join(format!("{op_id}.json"));
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn op(n: u32) -> String {
        format!("01JA9M3Q8Z5X7Y2W4V6T8S{n:04}")
    }

    fn send(vault: &Path, store: &Store, pkcs8: &[u8], n: u32, payload: Value) -> Receipt {
        let env = test_envelope(pkcs8, DEVICE, &op(n), payload.as_object().cloned().unwrap());
        let path = drop_op(vault, &env);
        process(vault, store, Local::now());
        assert!(!path.exists(), "the inbox file is removed once decided");
        receipt(vault, &op(n))
    }

    const LOCAL: &str = "---\ntitle: Call Jordan\nstatus: Not started\npriority: P2\ndue: \"2026-10-12\"\n---\n\nBody.\n";

    fn hash(vault: &Path, rel: &str) -> String {
        vault_tasks::get(vault, rel).unwrap().content_hash
    }

    #[test]
    fn a_create_delivered_twice_makes_one_task() {
        let (_d, v, store, key, _) = setup();
        let payload = json!({ "kind": "task.create", "title": "Send the report", "priority": "p1", "labels": ["work"] });
        let first = send(&v, &store, &key, 1, payload.clone());
        assert_eq!(first.status, "applied", "{first:?}");
        let rel = rel_of(first.reference.as_deref().unwrap());
        let text = fs::read_to_string(v.join(&rel)).unwrap();
        assert!(
            text.contains("origin: \"companion:mac-11223344\""),
            "{text}"
        );
        assert!(text.contains("priority: P1"), "{text}");
        // The same op again (both roads, or a retry): the stored receipt, no twin.
        let again = send(&v, &store, &key, 1, payload);
        assert_eq!(again, first);
        assert_eq!(fs::read_dir(v.join("tasks")).unwrap().count(), 1);
    }

    #[test]
    fn a_lost_ledger_still_creates_once_through_the_fingerprint() {
        let (d, v, store, key, _) = setup();
        let payload = json!({ "kind": "task.create", "title": "Once only" });
        send(&v, &store, &key, 1, payload.clone());
        fs::remove_file(d.path().join("store").join("applied.db")).unwrap();
        let again = send(&v, &store, &key, 1, payload);
        assert_eq!(again.status, "applied");
        assert_eq!(fs::read_dir(v.join("tasks")).unwrap().count(), 1);
    }

    #[test]
    fn a_moved_hash_without_overlap_applies_the_device_field() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let base = hash(&v, "tasks/call.md");
        // The Mac changes the priority after the device read the file.
        put(&v, "tasks/call.md", &LOCAL.replace("P2", "P3"));
        let r = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.update", "ref": "vault:tasks/call.md", "baseHash": base,
            "fields": { "due": { "from": "2026-10-12", "to": "2026-10-14" } } }),
        );
        assert_eq!(r.status, "applied", "{r:?}");
        assert_eq!(r.applied, vec!["due"]);
        let task = vault_tasks::get(&v, "tasks/call.md").unwrap();
        assert_eq!(task.due.as_deref(), Some("2026-10-14"));
        assert_eq!(
            task.priority.as_deref(),
            Some("P3"),
            "the Mac's change survives"
        );
        assert_eq!(r.content_hash.as_deref(), Some(task.content_hash.as_str()));
    }

    #[test]
    fn an_overlap_applies_the_rest_and_raises_one_card_per_field() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let base = hash(&v, "tasks/call.md");
        put(
            &v,
            "tasks/call.md",
            &LOCAL.replace("2026-10-12", "2026-10-20"),
        );
        let r = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.update", "ref": "vault:tasks/call.md", "baseHash": base,
            "fields": { "due": { "from": "2026-10-12", "to": "2026-10-14" }, "title": { "from": "Call Jordan", "to": "Call Jordan back" } } }),
        );
        assert_eq!(r.status, "partial", "{r:?}");
        assert_eq!(r.applied, vec!["title"]);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].field, "due");
        assert_eq!(r.conflicts[0].mac, json!("2026-10-20"));
        let task = vault_tasks::get(&v, "tasks/call.md").unwrap();
        assert_eq!(
            task.due.as_deref(),
            Some("2026-10-20"),
            "a conflict writes nothing"
        );
        assert_eq!(task.title, "Call Jordan back");
        let cards = crate::review_queue::list(&v, Some(CONFLICT_KIND)).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].source, "companion");
        // Use the device's value.
        accept_conflict(&v, &store, &cards[0], Local::now()).unwrap();
        assert_eq!(
            vault_tasks::get(&v, "tasks/call.md")
                .unwrap()
                .due
                .as_deref(),
            Some("2026-10-14")
        );
        let card = &crate::review_queue::list(&v, Some(CONFLICT_KIND)).unwrap()[0];
        assert_eq!(card.status, "accepted");
    }

    #[test]
    fn accepting_a_stale_card_writes_nothing() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let base = hash(&v, "tasks/call.md");
        put(
            &v,
            "tasks/call.md",
            &LOCAL.replace("2026-10-12", "2026-10-20"),
        );
        send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.update", "ref": "vault:tasks/call.md", "baseHash": base,
            "fields": { "due": { "from": "2026-10-12", "to": "2026-10-14" } } }),
        );
        put(
            &v,
            "tasks/call.md",
            &LOCAL.replace("2026-10-12", "2026-10-30"),
        );
        let card = crate::review_queue::list(&v, Some(CONFLICT_KIND))
            .unwrap()
            .remove(0);
        assert!(accept_conflict(&v, &store, &card, Local::now())
            .unwrap_err()
            .contains("changed again"));
        assert_eq!(
            vault_tasks::get(&v, "tasks/call.md")
                .unwrap()
                .due
                .as_deref(),
            Some("2026-10-30")
        );
    }

    #[test]
    fn completing_a_recurring_task_rolls_it_forward() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/review.md", "---\ntitle: Weekly review\nstatus: Not started\ndue: \"2026-10-09\"\nrecurrence: every friday\n---\n");
        let r = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.set_status", "ref": "vault:tasks/review.md",
            "baseHash": hash(&v, "tasks/review.md"), "from": "Not started", "to": "Done" }),
        );
        assert_eq!(r.status, "applied", "{r:?}");
        let task = r.task.unwrap();
        assert_ne!(
            task["status"],
            json!("Done"),
            "a recurring task never finishes"
        );
        assert_ne!(task["due"], json!("2026-10-09"), "its due date moved");
    }

    #[test]
    fn a_status_already_set_is_applied_without_a_write() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", &LOCAL.replace("Not started", "Done"));
        let before = fs::read_to_string(v.join("tasks/call.md")).unwrap();
        let r = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.set_status", "ref": "vault:tasks/call.md",
            "baseHash": "0".repeat(64), "from": "Not started", "to": "done" }),
        );
        assert_eq!(r.status, "applied");
        assert!(r.applied.is_empty());
        assert_eq!(fs::read_to_string(v.join("tasks/call.md")).unwrap(), before);
    }

    /// Notion two-way sync is gone (2026-11-10): a task it once linked (its
    /// `notion_id`, a stale ledger) takes any status, as a local task does.
    #[test]
    fn a_task_imported_from_notion_takes_any_status() {
        let (_d, v, store, key, _) = setup();
        let id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        put(&v, ".notion-sync.json", "{not json");
        put(&v, "tasks/synced.md", &format!("---\ntitle: \"Synced\"\nstatus: \"Active\"\npriority: \"Medium\"\ndue: \"2026-10-01\"\ndescription: \"Remote\"\nnotion_id: \"{id}\"\nnotion_url: \"https://www.notion.so/{id}\"\n---\n\nBody.\n"));
        let base = hash(&v, "tasks/synced.md");
        let ok = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.set_status", "ref": "vault:tasks/synced.md", "baseHash": base, "from": "Active", "to": "Complete" }),
        );
        assert_eq!(ok.status, "applied", "{ok:?}");
        assert_eq!(
            vault_tasks::get(&v, "tasks/synced.md").unwrap().status,
            "Complete"
        );
    }

    #[test]
    fn unsigned_unknown_and_agent_written_ops_are_refused() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let before = fs::read_to_string(v.join("tasks/call.md")).unwrap();
        let payload = json!({ "kind": "task.set_status", "ref": "vault:tasks/call.md", "baseHash": hash(&v, "tasks/call.md"), "from": "Not started", "to": "Done" });
        // Signed by a key Hollow never paired (what an agent could produce).
        let (agent_key, _) = test_key();
        let mut env = test_envelope(
            &agent_key,
            DEVICE,
            &op(1),
            payload.as_object().cloned().unwrap(),
        );
        drop_op(&v, &env);
        process(&v, &store, Local::now());
        assert_eq!(receipt(&v, &op(1)).status, "refused");
        // No signature at all.
        env = test_envelope(&key, DEVICE, &op(2), payload.as_object().cloned().unwrap());
        env.sig = String::new();
        drop_op(&v, &env);
        // A device that never paired.
        let stranger = test_envelope(
            &key,
            "mac-99999999",
            &op(3),
            payload.as_object().cloned().unwrap(),
        );
        drop_op(&v, &stranger);
        // A payload edited after signing.
        let mut tampered =
            test_envelope(&key, DEVICE, &op(4), payload.as_object().cloned().unwrap());
        tampered.payload = tampered.payload.replace("Done", "Blocked");
        drop_op(&v, &tampered);
        let summary = process(&v, &store, Local::now());
        assert_eq!(summary.refused, 3, "{summary:?}");
        assert_eq!(receipt(&v, &op(2)).status, "refused");
        assert_eq!(receipt(&v, &op(4)).status, "refused");
        assert!(v
            .join(SYNC_DIR)
            .join("receipts/mac-99999999")
            .join(format!("{}.json", op(3)))
            .exists());
        assert_eq!(fs::read_to_string(v.join("tasks/call.md")).unwrap(), before);
    }

    #[test]
    fn a_busy_vault_leaves_ops_waiting_in_order() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let first = test_envelope(
            &key,
            DEVICE,
            &op(1),
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "one" })
                .as_object()
                .cloned()
                .unwrap(),
        );
        let second = test_envelope(
            &key,
            DEVICE,
            &op(2),
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "two" })
                .as_object()
                .cloned()
                .unwrap(),
        );
        drop_op(&v, &first);
        drop_op(&v, &second);
        let lease = crate::automated_history::background_write_lease(&v).unwrap();
        let summary = process(&v, &store, Local::now());
        assert_eq!((summary.applied, summary.waiting), (0, 1), "{summary:?}");
        assert_eq!(waiting_ops(&v), 2);
        drop(lease);
        let summary = process(&v, &store, Local::now());
        assert_eq!(summary.applied, 2, "{summary:?}");
        let text = fs::read_to_string(v.join("tasks/call.md")).unwrap();
        assert!(
            text.find("one").unwrap() < text.find("two").unwrap(),
            "{text}"
        );
        assert!(text.contains("Test Mac (companion)"), "{text}");
    }

    #[test]
    fn unknown_kinds_and_keys_are_refused() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        assert_eq!(
            send(
                &v,
                &store,
                &key,
                1,
                json!({ "kind": "calendar.delete", "title": "x" })
            )
            .status,
            "refused"
        );
        let r = send(
            &v,
            &store,
            &key,
            2,
            json!({ "kind": "task.update", "ref": "vault:tasks/call.md", "baseHash": hash(&v, "tasks/call.md"),
            "fields": { "status": { "from": "Not started", "to": "Done" } } }),
        );
        assert!(r.reason.unwrap().contains("status has its own op"));
        let r = send(
            &v,
            &store,
            &key,
            3,
            json!({ "kind": "task.create", "title": "x", "notion": true }),
        );
        assert!(r.reason.unwrap().contains("unknown key"));
    }

    #[test]
    fn pairing_needs_the_open_code_and_closes_it() {
        let _serial = pairing_test_lock();
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("vault");
        let store = Store::at(dir.path().join("store"));
        let (_, public) = test_key();
        let write = |code: &str| {
            let req = json!({ "v": 1, "deviceId": "mac-aabbccdd", "name": "Menu bar", "platform": "macos",
                "publicKey": public, "at": "2026-10-12T21:00:00Z", "proof": pair_proof(code, "mac-aabbccdd", &public) });
            put(&v, ".hollow-sync/pair/mac-aabbccdd.json", &req.to_string());
        };
        let status = || -> Value {
            serde_json::from_str(
                &fs::read_to_string(v.join(".hollow-sync/devices/mac-aabbccdd.json")).unwrap(),
            )
            .unwrap()
        };
        close_pairing();
        write("AAAA-BBBB");
        process(&v, &store, Local::now());
        assert_eq!(status()["status"], "refused");
        let open = open_pairing(Utc::now());
        write("WRONGCOD");
        process(&v, &store, Local::now());
        assert_eq!(status()["status"], "refused");
        assert!(store.devices().is_empty());
        write(&open.code.to_lowercase());
        process(&v, &store, Local::now());
        assert_eq!(status()["status"], "paired", "{}", status());
        assert_eq!(store.devices()[0].public_key, public);
        assert!(open_code(Utc::now()).is_none(), "a code pairs one device");
        assert!(!v.join(".hollow-sync/pair/mac-aabbccdd.json").exists());
        revoke(&v, &store, "mac-aabbccdd").unwrap();
        assert!(store.devices().is_empty());
        assert_eq!(status()["status"], "revoked");
    }

    #[test]
    fn five_wrong_proofs_close_the_code() {
        let _serial = pairing_test_lock();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("store"));
        let (_, public) = test_key();
        let open = open_pairing(Utc::now());
        let req = |code: &str| PairRequest {
            v: 1,
            device_id: "mac-aabbccdd".into(),
            name: "x".into(),
            platform: "macos".into(),
            public_key: public.clone(),
            at: String::new(),
            proof: pair_proof(code, "mac-aabbccdd", &public),
        };
        for _ in 0..CODE_MAX_FAILURES {
            assert!(decide_pairing(&store, &req("ZZZZZZZZ"), Utc::now()).is_err());
        }
        assert!(decide_pairing(&store, &req(&open.code), Utc::now())
            .unwrap_err()
            .contains("No pairing code"));
    }

    #[test]
    fn a_symlinked_sync_folder_is_never_followed() {
        let (d, v, store, key, _) = setup();
        // An agent points `.hollow-sync/pair` and `inbox` at folders outside
        // the vault (Hollow's own support folder, say).
        let outside = d.path().join("outside");
        fs::create_dir_all(outside.join("mac-11223344")).unwrap();
        fs::write(outside.join("budgets.json"), "{}").unwrap();
        fs::write(
            outside.join("mac-11223344").join(format!("{}.json", op(1))),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(v.join(SYNC_DIR)).unwrap();
        std::os::unix::fs::symlink(&outside, v.join(SYNC_DIR).join("pair")).unwrap();
        std::os::unix::fs::symlink(&outside, v.join(SYNC_DIR).join("inbox")).unwrap();
        std::os::unix::fs::symlink(&outside, v.join(SYNC_DIR).join("receipts")).unwrap();
        let summary = process(&v, &store, Local::now());
        assert_eq!(summary, Summary::default());
        assert!(outside.join("budgets.json").exists());
        assert!(outside
            .join("mac-11223344")
            .join(format!("{}.json", op(1)))
            .exists());
        // A receipt can't be written through a linked folder either.
        let r = Receipt::refused(&op(2), DEVICE, Local::now(), "x");
        assert!(write_receipt(&v, &r).is_err());
        let _ = key;
    }

    #[test]
    fn a_forged_card_writes_nothing() {
        let (_d, v, store, _key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let before = fs::read_to_string(v.join("tasks/call.md")).unwrap();
        // A run writes its own card into the queue: a field devices can't
        // send, and an op Hollow never recorded.
        for (field, op_id) in [("recurrence", op(1)), ("due", op(2))] {
            let card = crate::review_queue::enqueue(
                &v,
                crate::review_queue::NewReviewItem {
                    kind: CONFLICT_KIND.into(),
                    fingerprint: format!("forged:{field}"),
                    payload: json!({ "ref": "vault:tasks/call.md", "field": field, "device": "every day",
                        "mac": null, "opId": op_id, "deviceId": DEVICE }),
                    risk: "local_write".into(),
                    source: SOURCE.into(),
                    source_path: Some("tasks/call.md".into()),
                },
            )
            .unwrap();
            assert!(accept_conflict(&v, &store, &card, Local::now()).is_err());
        }
        assert_eq!(fs::read_to_string(v.join("tasks/call.md")).unwrap(), before);
    }

    #[test]
    fn a_repeating_task_finished_elsewhere_is_not_rolled_twice() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/review.md", "---\ntitle: Weekly review\nstatus: Not started\ndue: \"2026-10-09\"\nrecurrence: every friday\n---\n");
        let base = hash(&v, "tasks/review.md");
        // The Mac (or another device) finishes it first: due rolls forward,
        // status reads "Not started" again.
        vault_tasks::set_status_with_recurrence(
            &v,
            "tasks/review.md",
            "Done",
            Local::now().date_naive(),
            None,
        )
        .unwrap();
        let rolled = vault_tasks::get(&v, "tasks/review.md").unwrap().due;
        let r = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.set_status", "ref": "vault:tasks/review.md",
            "baseHash": base, "from": "Not started", "to": "Done" }),
        );
        assert_eq!(r.status, "conflict", "{r:?}");
        assert!(r.reason.unwrap().contains("repeats"));
        assert_eq!(vault_tasks::get(&v, "tasks/review.md").unwrap().due, rolled);
        assert!(crate::review_queue::list(&v, Some(CONFLICT_KIND))
            .unwrap()
            .is_empty());
        // A forged card naming that refusal can't roll it either.
        let card = crate::review_queue::enqueue(
            &v,
            crate::review_queue::NewReviewItem {
                kind: CONFLICT_KIND.into(),
                fingerprint: "forged:status".into(),
                payload: json!({ "field": "status", "opId": op(1), "deviceId": DEVICE }),
                risk: "local_write".into(),
                source: SOURCE.into(),
                source_path: Some("tasks/review.md".into()),
            },
        )
        .unwrap();
        assert!(accept_conflict(&v, &store, &card, Local::now()).is_err());
        assert_eq!(vault_tasks::get(&v, "tasks/review.md").unwrap().due, rolled);
    }

    #[test]
    fn a_junk_file_cannot_claim_an_op_id() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        // An agent drops junk under the device's opId before the real op.
        put(
            &v,
            &format!(".hollow-sync/inbox/{DEVICE}/{}.json", op(1)),
            "{not json",
        );
        process(&v, &store, Local::now());
        assert_eq!(receipt(&v, &op(1)).status, "refused");
        let real = send(
            &v,
            &store,
            &key,
            1,
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "real" }),
        );
        assert_eq!(real.status, "applied", "{real:?}");
        assert!(fs::read_to_string(v.join("tasks/call.md"))
            .unwrap()
            .contains("real"));
    }

    #[test]
    fn the_sync_folder_is_protected_from_hollows_own_agent_writes() {
        assert!(crate::run_guard::is_protected_path(
            ".hollow-sync/inbox/mac-1234/01JA9M3Q8Z5X7Y2W4V6T8S0001.json"
        ));
        assert!(crate::run_guard::is_protected_path(".hollow-sync"));
        assert!(!crate::run_guard::is_protected_path("tasks/call.md"));
    }

    #[test]
    fn signature_and_proof_vectors_round_trip() {
        let (key, public) = test_key();
        let payload = "{\"v\":1,\"text\":\"café\"}";
        let sig = test_sign(&key, payload);
        assert_eq!(B64.decode(&sig).unwrap().len(), 64);
        assert!(verify(&public, payload, &sig));
        assert!(!verify(&public, "{\"v\":1}", &sig));
        assert_eq!(
            pair_proof("ab12-cd34", "mac-1", "k"),
            pair_proof("AB12CD34", "mac-1", "k")
        );
    }

    #[test]
    fn fixtures_export_runs_every_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_fixtures(dir.path(), "test").unwrap();
        let fx: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(fx["tasks"].as_array().unwrap().len(), FIXTURE_TASKS.len());
        let status = |name: &str| {
            fx["ops"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["name"] == name)
                .unwrap()["receipt"]["status"]
                .clone()
        };
        assert_eq!(status("create"), "applied");
        assert_eq!(status("update-hash-moved-no-overlap"), "applied");
        assert_eq!(status("update-overlap-conflict"), "conflict");
        assert_eq!(status("bad-signature-refused"), "refused");
        assert_eq!(status("invalid-priority-refused"), "refused");
        assert_eq!(fx["signatures"][0]["valid"], true);
    }

    /// `payload` signed as sent `days` before `now`.
    fn aged(payload: Value, now: DateTime<Local>, days: i64) -> Map<String, Value> {
        let sent = now.with_timezone(&Utc) - Duration::days(days);
        let at = sent.to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut payload = payload.as_object().cloned().unwrap();
        payload.insert("at".into(), json!(at));
        payload
    }

    fn times_said(vault: &Path, words: &str) -> usize {
        let text = fs::read_to_string(vault.join("tasks/call.md")).unwrap();
        text.matches(words).count()
    }

    fn old_cards(vault: &Path) -> Vec<crate::review_queue::ReviewItem> {
        crate::review_queue::list(vault, Some(OLD_OP_KIND)).unwrap()
    }

    #[test]
    fn an_op_sent_15_days_ago_waits_for_a_confirm_then_applies_once() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let comment =
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "Old news" });
        let env = test_envelope(&key, DEVICE, &op(1), aged(comment, now, 15));
        let path = drop_op(&v, &env);
        let summary = process(&v, &store, now);
        assert_eq!(summary.held, 1, "{summary:?}");
        assert_eq!(summary.applied, 0, "{summary:?}");
        assert_eq!(summary.review_changed, 1, "{summary:?}");
        assert!(!path.exists(), "the held op leaves the inbox");
        assert_eq!(times_said(&v, "Old news"), 0, "not applied");
        // No receipt until the person decides.
        let receipts = v.join(SYNC_DIR).join("receipts").join(DEVICE);
        assert!(!receipts.join(format!("{}.json", op(1))).exists());
        let cards = old_cards(&v);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].source, "companion");
        let title = cards[0].payload["title"].as_str().unwrap();
        assert!(title.contains("“Call Jordan”"), "{title}");
        assert!(title.contains("15 days ago"), "{title}");

        // Delivered again (a resend, or the other road): held, still one card.
        drop_op(&v, &env);
        assert_eq!(process(&v, &store, now).held, 1);
        assert_eq!(old_cards(&v).len(), 1);
        assert_eq!(times_said(&v, "Old news"), 0);

        // Apply it: applied once, and the device gets its receipt.
        accept_old_op(&v, &store, &cards[0], now).unwrap();
        assert_eq!(times_said(&v, "Old news"), 1);
        assert_eq!(receipt(&v, &op(1)).status, "applied");
        assert_eq!(old_cards(&v)[0].status, "accepted");

        // A second click, the ledger path itself, and a later copy apply
        // nothing again.
        assert!(accept_old_op(&v, &store, &cards[0], now).is_err());
        apply_held(&v, &store, DEVICE, &op(1), now).unwrap();
        drop_op(&v, &env);
        let repeat = process(&v, &store, now);
        assert_eq!(repeat.applied, 1, "answered from the ledger");
        assert_eq!(times_said(&v, "Old news"), 1);
    }

    #[test]
    fn an_op_sent_13_days_ago_applies_at_once() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let comment =
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "Recent" });
        let env = test_envelope(&key, DEVICE, &op(1), aged(comment, now, 13));
        drop_op(&v, &env);
        let summary = process(&v, &store, now);
        assert_eq!((summary.applied, summary.held), (1, 0), "{summary:?}");
        assert_eq!(receipt(&v, &op(1)).status, "applied");
        assert_eq!(times_said(&v, "Recent"), 1);
        assert!(old_cards(&v).is_empty());
    }

    #[test]
    fn discarding_an_old_op_answers_refused_and_never_applies_it() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let comment =
            json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": "Stale" });
        let env = test_envelope(&key, DEVICE, &op(1), aged(comment, now, 30));
        drop_op(&v, &env);
        process(&v, &store, now);
        let card = old_cards(&v).remove(0);
        let rejected = discard_old_op(&v, &store, &card, now).unwrap();
        assert_eq!(rejected.status, "rejected");
        let answer = receipt(&v, &op(1));
        assert_eq!(answer.status, "refused");
        assert!(answer.reason.unwrap_or_default().contains("14 days"));
        drop_op(&v, &env);
        let repeat = process(&v, &store, now);
        assert_eq!(repeat.refused, 1, "answered from the ledger");
        assert_eq!(times_said(&v, "Stale"), 0);
    }

    fn comment_on_call(text: &str) -> Value {
        json!({ "kind": "task.comment", "ref": "vault:tasks/call.md", "text": text })
    }

    fn old_card(vault: &Path, op_id: &str) -> crate::review_queue::ReviewItem {
        let mut cards = old_cards(vault);
        cards.retain(|card| card.payload["opId"] == op_id);
        cards.remove(0)
    }

    #[test]
    fn a_decided_old_op_is_answered_on_each_road_it_came_by() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        // Only as an inbox file: a receipt file, nothing owed to the backend.
        let payload = aged(comment_on_call("Inbox only"), now, 20);
        drop_op(&v, &test_envelope(&key, DEVICE, &op(1), payload));
        process(&v, &store, now);
        accept_old_op(&v, &store, &old_card(&v, &op(1)), now).unwrap();
        assert_eq!(receipt(&v, &op(1)).status, "applied");
        assert!(store.owed_receipts(10).unwrap().is_empty());

        // From the backend, then as an inbox file too: both get the answer.
        let payload = aged(comment_on_call("Both roads"), now, 20);
        let env = test_envelope(&key, DEVICE, &op(2), payload);
        let text = serde_json::to_string(&env).unwrap();
        let held = deliver(&v, &store, DEVICE, &op(2), &text, Road::Backend, now).unwrap();
        assert!(matches!(held, Delivery::Held { .. }), "{held:?}");
        drop_op(&v, &env);
        assert_eq!(process(&v, &store, now).held, 1);
        discard_old_op(&v, &store, &old_card(&v, &op(2)), now).unwrap();
        assert_eq!(receipt(&v, &op(2)).status, "refused");
        let owed = store.owed_receipts(10).unwrap();
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].op_id, op(2));
        assert_eq!(owed[0].status, "refused");
        assert!(store.held(DEVICE, &op(2)).unwrap().is_none());
        // The bridge forgets it once the backend stored it.
        store.receipt_delivered(DEVICE, &op(2)).unwrap();
        assert!(store.owed_receipts(10).unwrap().is_empty());
        assert_eq!(times_said(&v, "Both roads"), 0);
    }

    #[test]
    fn a_damaged_copy_of_a_held_op_is_held_without_a_refusal() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let payload = aged(comment_on_call("Kept as signed"), now, 20);
        let env = test_envelope(&key, DEVICE, &op(1), payload);
        let text = serde_json::to_string(&env).unwrap();
        let held = deliver(&v, &store, DEVICE, &op(1), &text, Road::Backend, now).unwrap();
        assert!(matches!(held, Delivery::Held { .. }), "{held:?}");
        // A copy edited after signing, on either road: held, never refused.
        let mut forged = env.clone();
        forged.payload = forged.payload.replace("Kept as signed", "Forged");
        let path = drop_op(&v, &forged);
        let summary = process(&v, &store, now);
        assert_eq!((summary.held, summary.refused), (1, 0), "{summary:?}");
        assert!(!path.exists());
        let receipts = v.join(SYNC_DIR).join("receipts").join(DEVICE);
        assert!(!receipts.join(format!("{}.json", op(1))).exists());
        let copy = serde_json::to_string(&forged).unwrap();
        let again = deliver(&v, &store, DEVICE, &op(1), &copy, Road::Backend, now).unwrap();
        assert!(matches!(again, Delivery::Held { .. }), "{again:?}");
        // Apply it applies the signed original; the inbox copy gets a receipt.
        accept_old_op(&v, &store, &old_card(&v, &op(1)), now).unwrap();
        assert_eq!(times_said(&v, "Kept as signed"), 1);
        assert_eq!(times_said(&v, "Forged"), 0);
        assert_eq!(receipt(&v, &op(1)).status, "applied");
    }

    #[test]
    fn apply_it_refuses_a_card_that_no_longer_describes_its_op() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let payload = aged(comment_on_call("Old news"), now, 20);
        drop_op(&v, &test_envelope(&key, DEVICE, &op(1), payload));
        let payload = aged(comment_on_call("Other news"), now, 30);
        drop_op(&v, &test_envelope(&key, DEVICE, &op(2), payload));
        process(&v, &store, now);
        let card = old_card(&v, &op(1));
        // Pointed at the other held op: its ids edited, its fingerprint not.
        let mut moved = card.clone();
        moved.payload["opId"] = json!(op(2));
        // The fingerprint edited too, but the details are still the first op's.
        let mut renamed = moved.clone();
        renamed.fingerprint = format!("companion-old-op:{DEVICE}:{}", op(2));
        // Its kind or its sending time edited.
        let mut rekinded = card.clone();
        rekinded.payload["opKind"] = json!("task.set_status");
        let mut redated = card.clone();
        redated.payload["sentAt"] = json!("2026-01-01T00:00:00.000Z");
        for changed in [moved, renamed, rekinded, redated] {
            let error = accept_old_op(&v, &store, &changed, now).unwrap_err();
            assert_eq!(error, CARD_CHANGED);
        }
        assert_eq!(times_said(&v, "Old news"), 0);
        assert_eq!(times_said(&v, "Other news"), 0);
        assert_eq!(old_card(&v, &op(1)).status, "pending");
        // The card as Hollow raised it still applies.
        accept_old_op(&v, &store, &card, now).unwrap();
        assert_eq!(times_said(&v, "Old news"), 1);
        assert_eq!(times_said(&v, "Other news"), 0);
    }

    #[test]
    fn discarding_an_edited_card_still_answers_the_op_it_was_raised_for() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        for (n, text) in [(1, "First"), (2, "Second"), (3, "Third")] {
            let payload = aged(comment_on_call(text), now, 20);
            drop_op(&v, &test_envelope(&key, DEVICE, &op(n), payload));
        }
        assert_eq!(process(&v, &store, now).held, 3);
        // Its details edited: the card no longer describes its op.
        let mut rekinded = old_card(&v, &op(1));
        rekinded.payload["opKind"] = json!("task.set_status");
        rekinded.payload["sentAt"] = json!("2026-01-01T00:00:00.000Z");
        // Its ids edited to name another held op, its fingerprint not: the
        // op it was raised for is answered, the one it now names isn't.
        let mut moved = old_card(&v, &op(2));
        moved.payload["opId"] = json!(op(3));
        for (n, edited) in [(1, rekinded), (2, moved)] {
            let rejected = discard_old_op(&v, &store, &edited, now).unwrap();
            assert_eq!(rejected.status, "rejected");
            let answer = receipt(&v, &op(n));
            assert_eq!(answer.status, "refused");
            assert!(answer.reason.unwrap_or_default().contains("14 days"));
            assert!(store.held(DEVICE, &op(n)).unwrap().is_none(), "op {n}");
        }
        // The op no card was discarded for still waits.
        assert!(store.held(DEVICE, &op(3)).unwrap().is_some());
        assert_eq!(old_card(&v, &op(3)).status, "pending");
        for text in ["First", "Second", "Third"] {
            assert_eq!(times_said(&v, text), 0, "{text}");
        }
    }

    #[test]
    fn revoking_a_device_drops_its_held_ops_and_closes_their_cards() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let payload = aged(comment_on_call("Never"), now, 20);
        drop_op(&v, &test_envelope(&key, DEVICE, &op(1), payload));
        process(&v, &store, now);
        assert!(store.held(DEVICE, &op(1)).unwrap().is_some());
        revoke(&v, &store, DEVICE).unwrap();
        assert!(store.held(DEVICE, &op(1)).unwrap().is_none());
        let card = old_card(&v, &op(1));
        assert_eq!(card.status, "rejected");
        let reason = card.error.clone().unwrap_or_default();
        assert!(reason.starts_with("Superseded:"), "{reason}");
        assert!(accept_old_op(&v, &store, &card, now).is_err());
        assert_eq!(times_said(&v, "Never"), 0);
    }

    #[test]
    fn an_old_op_refused_on_apply_closes_its_card_with_the_reason() {
        let (_d, v, store, key, _) = setup();
        put(&v, "tasks/call.md", LOCAL);
        let now = Local::now();
        let payload = aged(comment_on_call("Too late"), now, 20);
        drop_op(&v, &test_envelope(&key, DEVICE, &op(1), payload));
        process(&v, &store, now);
        // The task is gone by the time the person applies it.
        fs::remove_file(v.join("tasks/call.md")).unwrap();
        let error = accept_old_op(&v, &store, &old_card(&v, &op(1)), now).unwrap_err();
        assert!(error.contains("No task at tasks/call.md"), "{error}");
        let closed = old_card(&v, &op(1));
        assert_eq!(closed.status, "rejected", "not back to pending");
        assert_eq!(closed.error.as_deref(), Some(error.as_str()));
        assert_eq!(receipt(&v, &op(1)).status, "refused");
        assert!(store.held(DEVICE, &op(1)).unwrap().is_none());
    }

    #[test]
    fn an_old_op_card_names_its_workspace_and_previews_a_new_notes_body() {
        const BRIEF: &str = "---\ntitle: Launch plan\n---\n\n# Launch\n";
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        put(v, "workspaces/launch-1/WORKSPACE.md", BRIEF);
        let mut line = Map::new();
        line.insert("kind".into(), json!("workspace.append"));
        line.insert("workspace".into(), json!("launch-1"));
        line.insert("file".into(), json!("plan"));
        line.insert("text".into(), json!("Book the venue"));
        let summary = op_summary(v, &line);
        assert_eq!(summary, "A line for the plan of “Launch plan”");
        assert_eq!(op_text(&line).as_deref(), Some("Book the venue"));
        // A workspace Hollow can't read is named by its id.
        let mut brief = line.clone();
        brief.insert("file".into(), json!("brief"));
        brief.insert("workspace".into(), json!("gone-2"));
        let summary = op_summary(v, &brief);
        assert_eq!(summary, "A line for the brief of “gone-2”");
        let mut note = Map::new();
        note.insert("kind".into(), json!("note.create"));
        note.insert("title".into(), json!("Ideas"));
        note.insert("body".into(), json!("First line\n\n  second   line\n"));
        assert_eq!(op_text(&note).as_deref(), Some("First line second line"));
    }

    #[test]
    fn the_declared_kinds_are_exactly_the_applied_ones() {
        // Every declared kind reaches its handler (a malformed payload, not
        // "unknown op kind"); one more kind would be refused as unknown.
        let vault = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::at(store_dir.path().to_path_buf());
        let device = Device {
            device_id: "web-11223344".into(),
            name: "t".into(),
            platform: "web".into(),
            public_key: String::new(),
            paired_at: String::new(),
            last_op_at: None,
        };
        let now = Local::now();
        let reason = |kind: &str| {
            let payload: Map<String, Value> =
                serde_json::from_value(json!({ "kind": kind, "bogus": 1 })).unwrap();
            match apply_payload(
                vault.path(),
                &store,
                &device,
                "01JA9M3Q8Z5X7Y2W4V6T8S0R1P",
                &payload,
                now,
            ) {
                Outcome::Done { receipt, .. } => receipt.reason.unwrap_or_default(),
                Outcome::Waiting | Outcome::Held { .. } => String::new(),
            }
        };
        for kind in APPLIED_KINDS {
            assert!(!reason(kind).contains("unknown op kind"), "{kind}");
        }
        assert!(reason("calendar.delete").contains("unknown op kind"));
    }

    #[test]
    fn a_tracker_mirror_row_carries_status_type_key_and_url_and_a_local_row_does_not() {
        // Same fields, spelling and source (`task.tracker`) as the phone
        // API's `/v1/tasks` rows, so a custom completed state ("Shipped")
        // reads as done and Open in Linear shows on the web too.
        let (_d, v, ..) = setup();
        put(&v, "tasks/tracked.md", "---\ntitle: A tracker fixture\nstatus: Shipped\nstatus_type: completed\npriority: High\ntracker: linear\ntracker_id: fixture-issue\ntracker_key: ENG-99\ntracker_url: https://example.invalid/ENG-99\n---\n\nBody.\n");
        put(&v, "tasks/local.md", LOCAL);
        let tracked = task_json(
            &vault_tasks::get(&v, "tasks/tracked.md").unwrap(),
            "tasks/tracked.md",
        );
        assert_eq!(tracked["status"], "Shipped");
        assert_eq!(tracked["statusType"], "completed");
        assert_eq!(tracked["trackerKey"], "ENG-99");
        assert_eq!(tracked["trackerUrl"], "https://example.invalid/ENG-99");
        assert_eq!(tracked["trackerLinked"], true);
        let local = task_json(
            &vault_tasks::get(&v, "tasks/local.md").unwrap(),
            "tasks/local.md",
        );
        assert_eq!(local["trackerLinked"], false);
        for field in ["statusType", "trackerKey", "trackerUrl"] {
            assert!(
                local.get(field).is_none(),
                "a local row has no {field}: {local}"
            );
        }
    }

    #[test]
    fn a_ledger_folder_that_cannot_be_made_reads_plainly() {
        // The store's folder is a file, so the ledger's folder can't be made.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        fs::write(&root, "not a folder").unwrap();
        let Err(err) = Store::at(root).ledger() else {
            panic!("the ledger opened inside a file");
        };
        assert!(
            err.starts_with(
                "Couldn't use Hollow's record of changes already applied from your devices: "
            ),
            "{err}"
        );
        assert!(!err.contains("companion store"), "{err}");
    }
}
