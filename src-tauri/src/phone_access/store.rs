//! The phone API's record outside the vault and its pairing window.
//!
//! - **The record** (`<support>/Companion/phone.json`, mode 0600): the
//!   iPhone app switch, the paired phones, and the permission modes the
//!   window reports (for new chats, and each chat's own choice). A phone's
//!   token is never
//!   stored, only its SHA-256 (hex) beside the device id, name, model,
//!   paired time and last-seen time, and (v1.1) the APNs device token it
//!   registered for push, which goes with the phone's record. Only a missing file is empty; a file
//!   that can't be read is an error (no phone gets in, and the switch reads
//!   as off), and a damaged one is moved aside only by the person's own
//!   click in Settings ([`crate::state_file`]).
//! - **The pairing window** lives in memory only: an 8-character code from
//!   [`CODE_ALPHABET`], good for [`PAIRING_MINUTES`] minutes or one use; a
//!   new window closes the old one and [`MAX_WRONG_CODES`] wrong codes close
//!   it.
//! - **Machines** (M2, 2026-11-15, `docs/phone-api/machines.md`): another Mac
//!   paired through its own window is a [`Device`] of [`DeviceKind::Machine`]
//!   (`mc-` id, its `machineId`), kept in the record's own `machines` list, not
//!   `devices`. A Hollow from before M2 doesn't know that list, so it lets no
//!   machine token in (and drops the list on its next write: the Mac pairs
//!   again), rather than reading a machine as a phone with a phone's routes.
//!   Fields a later Hollow adds to the record or a device are kept on write.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The record's file name, in Hollow's companion folder.
pub(crate) const FILE: &str = "phone.json";
/// What a pairing code is made of: no 0, 1, I, L or O, so it reads aloud.
pub(crate) const CODE_ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
pub(crate) const CODE_LEN: usize = 8;
pub(crate) const PAIRING_MINUTES: i64 = 10;
pub(crate) const MAX_WRONG_CODES: u32 = 5;
/// A phone's last-seen time is written at most this often.
const LAST_SEEN_WRITE_SECS: i64 = 60;
/// What the record is called in errors.
const WHAT: &str = "Hollow's record of paired phones";

/// The whole record, as stored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    /// Settings, Devices, Companion devices, iPhone app. Off until the person
    /// turns it on; once on, it starts with Hollow.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub devices: Vec<Device>,
    /// The permission mode Settings, Chat gives new chats, as the window
    /// last reported it: a chat a phone starts begins in it, also after a
    /// restart before the window says it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_chat_mode: Option<String>,
    /// The agent `new_chat_mode` is for (the window's preferred agent), so a
    /// phone chat on another agent inherits it from that agent
    /// (`agent::inherited_mode`, Q-R3). None: an older report, read the
    /// most restrictive way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_chat_agent: Option<String>,
    /// Each chat's permission mode as the person chose it in the window
    /// (`[key, mode]`, oldest first, at most [`CHAT_MODES_MAX`]), as the
    /// window last reported its record (`hollow.chat.modes.v1`): a phone's
    /// send runs in the mode the window shows for that chat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat_modes: Vec<(String, String)>,
    /// The window couldn't keep every choice (its storage was full), so a
    /// chat it has no choice for shows Plan there, and a phone's send runs in
    /// Plan too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chat_modes_plan: bool,
    /// M2: the other Macs paired through the machine window (each of
    /// [`DeviceKind::Machine`]). Apart from `devices`, so a Hollow from
    /// before M2 never reads one as a phone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub machines: Vec<Device>,
    /// B4: how many seconds a card waits while a phone has approvals on
    /// (Settings, beside the toggle); None is the default, 5 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_wait_secs: Option<u64>,
    /// Fields a later Hollow adds; kept on write.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Chats whose mode choice the record keeps, at most (the window's own
/// `CHOICE_LIMIT`).
pub(crate) const CHAT_MODES_MAX: usize = 500;

/// What one of the window's mode reports did to the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Report {
    /// Written.
    Kept,
    /// The record already said so: nothing written.
    Same,
    /// The switch is off and no phone (nor Mac) is paired, so nothing uses the modes
    /// and nothing was written: the record lags the window until it reports
    /// again.
    Unused,
}

/// What a paired device is: a phone (the iPhone app, every route it was
/// given) or another Mac (M2: only the machine routes, [`super::routes::ROUTES`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DeviceKind {
    /// A record without `kind` (every one before M2) is a phone.
    #[default]
    Phone,
    Machine,
}

impl DeviceKind {
    fn is_phone(&self) -> bool {
        *self == DeviceKind::Phone
    }

    /// The device id's prefix: `ph-` for a phone, `mc-` for a Mac.
    fn prefix(self) -> &'static str {
        match self {
            DeviceKind::Phone => "ph-",
            DeviceKind::Machine => "mc-",
        }
    }
}

/// One paired phone, or (M2) another Mac.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Device {
    /// `ph-` and 8 hex digits (a Mac: `mc-`).
    pub device_id: String,
    /// M2: a phone unless it says otherwise; a phone's record never says.
    #[serde(default, skip_serializing_if = "DeviceKind::is_phone")]
    pub kind: DeviceKind,
    /// M2: a Mac's own `machineId` (`m-` and 16 hex, its `machine.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
    /// SHA-256 of the bearer token, lower-case hex. The token itself is
    /// never kept.
    pub token_sha256: String,
    pub paired_at: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    /// v1.1: where this phone gets push notifications (`POST /v1/push`);
    /// gone with `DELETE /v1/push`, a revoke, or a token Apple refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushTarget>,
    /// B4: the keys this phone answers approval cards with (enrolled with a
    /// code from the Mac and an App Attest attestation); gone with
    /// `DELETE /v1/approvals/key`, the Mac's Turn off, five bad signatures or
    /// a revoke. An older `phone.json` has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_keys: Option<ApprovalKeys>,
    /// Fields a later Hollow adds; kept on write.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// B4: a phone's enrolled approval keys (`docs/phone-api/approvals.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalKeys {
    /// The approve key: base64url of its X9.63 point (65 bytes); Face ID guards it.
    pub approve_key: String,
    /// The deny key, same encoding; no prompt.
    pub deny_key: String,
    /// The first 8 hex digits of SHA-256(approve point || deny point), as the
    /// phone shows them: `a1b2 c3d4`.
    pub fingerprint: String,
    pub enrolled_at: String,
}

/// v1.1: a phone's APNs registration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushTarget {
    /// The APNs device token, lower-case hex.
    pub token: String,
    /// `production` or `development`: which APNs it belongs to.
    pub environment: String,
    /// The pushes it wants (`turnFinished`, `question`).
    pub events: Vec<String>,
    pub registered_at: String,
}

/// SHA-256 of `token`, lower-case hex.
pub(crate) fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A fresh bearer token: 32 random bytes, base64url without padding (43
/// characters).
pub(crate) fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A device id no paired device has and none revoked in this run had: the
/// kind's prefix (`ph-`, `mc-`) and 8 hex digits.
fn new_device_id(kind: DeviceKind, record: &Record, revoked: &HashSet<String>) -> String {
    loop {
        let mut bytes = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let id = format!(
            "{}{:02x}{:02x}{:02x}{:02x}",
            kind.prefix(),
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3]
        );
        if !revoked.contains(&id) && !record.all_devices().any(|d| d.device_id == id) {
            return id;
        }
    }
}

impl Record {
    /// Every paired device: the phones, then the Macs.
    pub(crate) fn all_devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().chain(self.machines.iter())
    }

    fn all_devices_mut(&mut self) -> impl Iterator<Item = &mut Device> {
        self.devices.iter_mut().chain(self.machines.iter_mut())
    }
}

/// A Mac's `machineId`: `m-` and 16 lower-case hex digits.
pub(crate) fn valid_machine_id(id: &str) -> bool {
    id.len() == 18
        && id.starts_with("m-")
        && id[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Equal without an early exit (the lengths are not secret).
pub(crate) fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, false)
}

/// The record file and an in-memory copy of it. Only this process writes
/// the file, so the copy stays current; a read error is never cached, so a
/// repaired file is read again.
pub(crate) struct Store {
    path: PathBuf,
    state: Mutex<Cached>,
    /// The ids revoked in this run (the revocation generation: an id in
    /// here is never live again). A leaf lock, taken inside `change` and
    /// on its own, never the other way round.
    revoked: Mutex<HashSet<String>>,
}

/// Why [`Store::ensure_live`] refused a phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotLive {
    /// Revoked (or never paired): the 401 an unknown token gets.
    Revoked,
    /// The record can't be read.
    Unreadable(String),
}

/// What a string-carried refusal says when a revoked phone's request reached
/// a commit point inside code that returns `String` errors (the send path).
pub(crate) const REVOKED_TEXT: &str = "This iPhone is no longer paired with Hollow.";

impl std::fmt::Display for NotLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Revoked => f.write_str(REVOKED_TEXT),
            Self::Unreadable(error) => f.write_str(error),
        }
    }
}

#[derive(Default)]
struct Cached {
    record: Option<Record>,
    /// When each phone's last-seen time was last written to the file.
    seen_written: HashMap<String, DateTime<Utc>>,
}

impl Store {
    /// The store in `dir` (created on the first write).
    pub(crate) fn at(dir: &Path) -> Self {
        Self {
            path: dir.join(FILE),
            state: Mutex::new(Cached::default()),
            revoked: Mutex::new(HashSet::new()),
        }
    }

    /// The app's store: `<support>/Companion/phone.json`, beside the
    /// companion devices' own record.
    pub(crate) fn for_app() -> Option<Self> {
        crate::instance::support_dir().map(|dir| Self::at(&dir.join("Companion")))
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The folder the record lives in (`<support>/Companion`), beside the
    /// companion's scope file.
    pub(crate) fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(&self.path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cached> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The record for unattended readers (the server, startup): a damaged
    /// file is left where it is and is an error.
    pub(crate) fn read(&self) -> Result<Record, String> {
        let mut cached = self.lock();
        if let Some(record) = &cached.record {
            return Ok(record.clone());
        }
        let record = crate::state_file::read_json_or::<Record>(
            &self.path,
            WHAT,
            crate::state_file::OnDamage::Leave,
        )?
        .unwrap_or_default();
        cached.record = Some(record.clone());
        Ok(record)
    }

    /// The record for Settings, the screen the person opened: a damaged file
    /// is moved aside and named in the error (no phone gets in until one
    /// pairs again); the next read starts a fresh record.
    pub(crate) fn read_for_settings(&self) -> Result<Record, String> {
        if let Ok(record) = self.read() {
            return Ok(record);
        }
        let mut cached = self.lock();
        let record = crate::state_file::read_json_or::<Record>(
            &self.path,
            WHAT,
            crate::state_file::OnDamage::SetAside,
        )?
        .unwrap_or_default();
        cached.record = Some(record.clone());
        Ok(record)
    }

    /// Read, change and write the record (the person's clicks in Settings,
    /// a pairing). A damaged file is moved aside and named in the error;
    /// the next change starts a fresh record.
    pub(crate) fn change<T>(
        &self,
        f: impl FnOnce(&mut Record) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut cached = self.lock();
        let mut record = crate::state_file::read_json_or::<Record>(
            &self.path,
            WHAT,
            crate::state_file::OnDamage::SetAside,
        )?
        .unwrap_or_default();
        let out = f(&mut record)?;
        write(&self.path, &record)?;
        cached.record = Some(record);
        Ok(out)
    }

    /// The paired device whose token this is (never one revoked in this
    /// run). One from the record's `machines` list is always a
    /// [`DeviceKind::Machine`], whatever its `kind` says.
    pub(crate) fn device_for_token(&self, token: &str) -> Result<Option<Device>, String> {
        let hash = token_hash(token);
        let revoked = self.revoked_ids();
        let record = self.read()?;
        let machines = record.machines.into_iter().map(|mut d| {
            d.kind = DeviceKind::Machine;
            d
        });
        Ok(record
            .devices
            .into_iter()
            .chain(machines)
            .filter(|d| !revoked.contains(&d.device_id))
            .find(|d| same_bytes(d.token_sha256.as_bytes(), hash.as_bytes())))
    }

    fn revoked_ids(&self) -> HashSet<String> {
        self.revoked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The one commit-point check: `Ok` while `device_id` is paired and was
    /// not revoked in this run, else [`NotLive`]. Every route runs it just
    /// before an effect lands, after any wait that can be long (a request
    /// authenticated before a revoke must not commit after it). Reads the
    /// cached record in place: cheap enough for a closure on a blocking
    /// thread.
    pub(crate) fn ensure_live(&self, device_id: &str) -> Result<(), NotLive> {
        if self
            .revoked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(device_id)
        {
            return Err(NotLive::Revoked);
        }
        let cached_answer = self
            .lock()
            .record
            .as_ref()
            .map(|r| r.all_devices().any(|d| d.device_id == device_id));
        let paired = match cached_answer {
            Some(paired) => paired,
            None => self
                .read()
                .map_err(NotLive::Unreadable)?
                .all_devices()
                .any(|d| d.device_id == device_id),
        };
        if paired {
            Ok(())
        } else {
            Err(NotLive::Revoked)
        }
    }

    /// The iPhone app is on and at least one phone is paired: while it is,
    /// the live buffer copies chat events (`live::Live::follow`). A record
    /// that can't be read is neither.
    pub(crate) fn phone_ready(&self) -> bool {
        self.read()
            .is_ok_and(|record| record.enabled && !record.devices.is_empty())
    }

    /// M2: someone may watch a chat live: [`Self::phone_ready`], or another
    /// Mac is paired (whatever the iPhone switch says). The live buffer
    /// copies chat events while this holds (`live::Live::follow`).
    pub(crate) fn viewers_ready(&self) -> bool {
        self.read().is_ok_and(|record| {
            (record.enabled && !record.devices.is_empty()) || !record.machines.is_empty()
        })
    }

    /// M2: another Mac is paired, so the server listens even with the
    /// iPhone switch off. A record that can't be read says no.
    pub(crate) fn machines_paired(&self) -> bool {
        self.read().is_ok_and(|record| !record.machines.is_empty())
    }

    /// Whether `device_id` is still paired.
    pub(crate) fn has_device(&self, device_id: &str) -> Result<bool, String> {
        match self.ensure_live(device_id) {
            Ok(()) => Ok(true),
            Err(NotLive::Revoked) => Ok(false),
            Err(NotLive::Unreadable(error)) => Err(error),
        }
    }

    /// Note that `device_id` was just seen. The copy in memory always moves;
    /// the file at most every [`LAST_SEEN_WRITE_SECS`] seconds per phone.
    /// Returns whether the file was written.
    pub(crate) fn touch(&self, device_id: &str, now: DateTime<Utc>) -> bool {
        let mut cached = self.lock();
        let Some(record) = cached.record.as_mut() else {
            return false;
        };
        let Some(device) = record.all_devices_mut().find(|d| d.device_id == device_id) else {
            return false;
        };
        device.last_seen_at = Some(stamp(now));
        let due = cached
            .seen_written
            .get(device_id)
            .is_none_or(|at| now - *at >= Duration::seconds(LAST_SEEN_WRITE_SECS));
        if !due {
            return false;
        }
        // Every change goes through this store under the same lock, so the
        // copy in memory is what the file holds plus the newer stamps.
        let snapshot = cached.record.clone().unwrap_or_default();
        let written = write(&self.path, &snapshot).is_ok();
        if written {
            cached.seen_written.insert(device_id.to_string(), now);
        }
        written
    }

    /// Pair a phone: a new device with a fresh token, which is returned
    /// once and never stored.
    pub(crate) fn pair(
        &self,
        name: &str,
        model: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(Device, String), String> {
        let token = new_token();
        let hash = token_hash(&token);
        let device = self.change(|record| {
            let device = Device {
                device_id: new_device_id(DeviceKind::Phone, record, &self.revoked_ids()),
                kind: DeviceKind::Phone,
                machine_id: None,
                name: name.to_string(),
                model: model.map(str::to_string),
                token_sha256: hash,
                paired_at: stamp(now),
                last_seen_at: Some(stamp(now)),
                push: None,
                approval_keys: None,
                extra: Map::new(),
            };
            record.devices.push(device.clone());
            Ok(device)
        })?;
        self.lock()
            .seen_written
            .insert(device.device_id.clone(), now);
        Ok((device, token))
    }

    /// M2: pair another Mac: a new `mc-` device with a fresh token, which
    /// is returned once and never stored. The same Mac pairing again (its
    /// `machineId`) replaces its old device, whose id is revoked (returned
    /// so its streams can close): one device per Mac.
    pub(crate) fn pair_machine(
        &self,
        name: &str,
        machine_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Device, String, Option<String>), String> {
        let token = new_token();
        let hash = token_hash(&token);
        let (device, replaced) = self.change(|record| {
            let replaced = record
                .machines
                .iter()
                .find(|d| d.machine_id.as_deref() == Some(machine_id))
                .map(|d| d.device_id.clone());
            if let Some(old) = &replaced {
                // Before the write, as in `revoke`: the old token is dead
                // from now on, and its id never comes back.
                self.revoked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(old.clone());
                record.machines.retain(|d| &d.device_id != old);
            }
            let device = Device {
                device_id: new_device_id(DeviceKind::Machine, record, &self.revoked_ids()),
                kind: DeviceKind::Machine,
                machine_id: Some(machine_id.to_string()),
                name: name.to_string(),
                model: None,
                token_sha256: hash,
                paired_at: stamp(now),
                last_seen_at: Some(stamp(now)),
                push: None,
                approval_keys: None,
                extra: Map::new(),
            };
            record.machines.push(device.clone());
            Ok((device, replaced))
        })?;
        crate::runs_on::Seat::forget(); // M4: a paired Mac shares the vault
        self.lock()
            .seen_written
            .insert(device.device_id.clone(), now);
        Ok((device, token, replaced))
    }

    /// Remove a paired phone (or Mac). `Ok(false)` when there was none by
    /// that id.
    pub(crate) fn revoke(&self, device_id: &str) -> Result<bool, String> {
        crate::runs_on::Seat::forget(); // M4: a Mac may be unpaired
        self.change(|record| {
            let before = record.devices.len() + record.machines.len();
            record.devices.retain(|d| d.device_id != device_id);
            record.machines.retain(|d| d.device_id != device_id);
            let removed = record.devices.len() + record.machines.len() != before;
            if removed {
                // Before the write lands: a request that checks from now on
                // is refused, and the id is never handed out again. Fail
                // closed: if the write then fails, the phone stays in the
                // file (and `revoke` returns the error) but is refused until
                // a restart, never live again by accident. Retrying revoke
                // finishes the job.
                self.revoked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(device_id.to_string());
            }
            Ok(removed)
        })
    }

    /// v1.1: set or clear `device_id`'s push registration. `Ok(false)` when
    /// no phone has that id (it was revoked meanwhile). A token another
    /// phone held moves to this one: Apple names one installation by it.
    pub(crate) fn set_push(
        &self,
        device_id: &str,
        push: Option<PushTarget>,
    ) -> Result<bool, String> {
        self.change(|record| {
            if !record.devices.iter().any(|d| d.device_id == device_id) {
                return Ok(false);
            }
            if let Some(target) = &push {
                for other in record.devices.iter_mut() {
                    if other.device_id != device_id
                        && other.push.as_ref().is_some_and(|p| p.token == target.token)
                    {
                        other.push = None;
                    }
                }
            }
            if let Some(device) = record.devices.iter_mut().find(|d| d.device_id == device_id) {
                device.push = push;
            }
            Ok(true)
        })
    }

    /// B4: the wait setting, in seconds (clamped to the settings' range).
    pub(crate) fn set_approval_wait(&self, secs: u64) -> Result<(), String> {
        let range = crate::approvals::PHONE_WAIT_RANGE;
        let secs = secs.clamp(*range.start(), *range.end());
        self.change(|record| {
            record.approval_wait_secs = Some(secs);
            Ok(())
        })
    }

    /// B4: set or clear `device_id`'s approval keys. `Ok(false)` when no phone
    /// has that id (revoked meanwhile).
    pub(crate) fn set_approval_keys(
        &self,
        device_id: &str,
        keys: Option<ApprovalKeys>,
    ) -> Result<bool, String> {
        self.change(|record| {
            let Some(device) = record.devices.iter_mut().find(|d| d.device_id == device_id) else {
                return Ok(false);
            };
            device.approval_keys = keys;
            Ok(true)
        })
    }

    /// v1.1: forget an APNs token Apple said is no longer valid, whichever
    /// phone holds it. `Ok(false)` when none did.
    pub(crate) fn drop_push_token(&self, token: &str) -> Result<bool, String> {
        if !self
            .read()?
            .devices
            .iter()
            .any(|d| d.push.as_ref().is_some_and(|p| p.token == token))
        {
            return Ok(false);
        }
        self.change(|record| {
            let mut dropped = false;
            for device in record.devices.iter_mut() {
                if device.push.as_ref().is_some_and(|p| p.token == token) {
                    device.push = None;
                    dropped = true;
                }
            }
            Ok(dropped)
        })
    }

    pub(crate) fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.change(|record| {
            record.enabled = enabled;
            Ok(())
        })
    }

    /// Change the record for one of the window's mode reports. The read, the
    /// change and the write all hold the store's lock, so two reports in
    /// flight never lose one another (second review, finding 3, 2026-11-04:
    /// the list was built from a `read()` snapshot and written in a separate
    /// `change()`). A damaged file is left where it is and is an error (the
    /// window reports without the person looking at phones, so only Settings
    /// moves one aside), and nothing is written when `f` changed nothing or
    /// while the switch is off and no phone is paired, when nothing uses the
    /// modes (finding 4: every new chat's first send and every mode change
    /// wrote the file).
    fn report(&self, f: impl FnOnce(&mut Record)) -> Result<Report, String> {
        let mut cached = self.lock();
        let mut record = crate::state_file::read_json_or::<Record>(
            &self.path,
            WHAT,
            crate::state_file::OnDamage::Leave,
        )?
        .unwrap_or_default();
        // A paired Mac's send runs in the mode the window shows too (M2).
        if !record.enabled && record.devices.is_empty() && record.machines.is_empty() {
            return Ok(Report::Unused);
        }
        let before = record.clone();
        f(&mut record);
        if record == before {
            return Ok(Report::Same);
        }
        write(&self.path, &record)?;
        cached.record = Some(record);
        Ok(Report::Kept)
    }

    /// `hollow serve set-default-mode` (M1 PR 3): the mode a phone's new
    /// chat starts in where no window reports one, and the agent it is for.
    /// The person's own command, so it is written whatever the switch says
    /// (a damaged file is moved aside, as Settings would).
    pub(crate) fn set_default_mode(&self, mode: &str, agent: Option<&str>) -> Result<(), String> {
        self.change(|record| {
            record.new_chat_mode = Some(mode.to_string());
            record.new_chat_agent = agent.map(str::to_string);
            Ok(())
        })
    }

    /// Keep the mode for new chats the window reported, and the agent it is
    /// for ([`Self::report`]).
    pub(crate) fn set_new_chat_mode(
        &self,
        mode: &str,
        agent: Option<&str>,
    ) -> Result<Report, String> {
        self.report(|record| {
            record.new_chat_mode = Some(mode.to_string());
            record.new_chat_agent = agent.map(str::to_string);
        })
    }

    /// Keep the window's mode choices for chats ([`Self::report`]):
    /// `replace` takes `choices` as the whole record (the window's full
    /// report), else each one is the newest choice for its chat (a choice
    /// just made). `plan` is the window's fallback for chats it has no
    /// choice for.
    pub(crate) fn set_chat_modes(
        &self,
        choices: &[(String, String)],
        replace: bool,
        plan: bool,
    ) -> Result<Report, String> {
        self.report(|record| {
            let modes = &mut record.chat_modes;
            if replace {
                modes.clear();
            }
            for (key, mode) in choices {
                modes.retain(|(k, _)| k != key);
                modes.push((key.clone(), mode.clone()));
            }
            let over = modes.len().saturating_sub(CHAT_MODES_MAX);
            modes.drain(..over);
            record.chat_modes_plan = plan;
        })
    }

    /// The mode the window shows for chat `key` when the person chose one:
    /// that choice, else Plan when the window keeps no complete record, else
    /// `None` (the window shows the chat's recorded mode).
    pub(crate) fn chat_mode(&self, key: &str) -> Result<Option<String>, String> {
        let record = self.read()?;
        let chosen = record
            .chat_modes
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, mode)| mode.clone());
        Ok(chosen.or_else(|| record.chat_modes_plan.then(|| "plan".to_string())))
    }
}

fn write(path: &Path, record: &Record) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{WHAT}: {e}"))?;
    }
    let text = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(path, text.as_bytes())
}

// ── Pairing window (memory only) ─────────────────────────────────────────

struct Window {
    code: String,
    expires: DateTime<Utc>,
    failures: u32,
}

/// The open window as Settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenWindow {
    /// `XXXX-XXXX`.
    pub display: String,
    /// The code without its dash (the pairing link's `code`).
    pub code: String,
    pub expires: DateTime<Utc>,
}

/// What a code typed on a phone found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// The code matched; the window is now closed (one use).
    Paired,
    /// No window open: none, expired, used, or closed by wrong codes.
    Closed,
    /// Wrong; this many tries are left (0 closed the window).
    Wrong { attempts_left: u32 },
}

/// The pairing window: at most one at a time.
#[derive(Default)]
pub(crate) struct Pairing {
    window: Mutex<Option<Window>>,
}

/// A code as typed: case, spaces and dashes ignored.
pub(crate) fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// A fresh pairing-alphabet code ([`CODE_LEN`] characters).
pub(crate) fn random_code() -> String {
    let mut code = String::with_capacity(CODE_LEN);
    let mut bytes = [0u8; CODE_LEN];
    for slot in bytes.iter_mut() {
        // Rejection sampling keeps every character equally likely.
        *slot = loop {
            let mut one = [0u8; 1];
            rand::rngs::OsRng.fill_bytes(&mut one);
            if (one[0] as usize) < 248 {
                break CODE_ALPHABET[(one[0] as usize) % CODE_ALPHABET.len()];
            }
        };
    }
    code.extend(bytes.iter().map(|b| *b as char));
    code
}

pub(crate) fn display(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

impl Pairing {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Window>> {
        self.window.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a fresh window, closing any open one.
    pub(crate) fn open(&self, now: DateTime<Utc>) -> OpenWindow {
        self.open_with(&random_code(), now)
    }

    /// Open a window with a known code (tests, and [`Self::open`]).
    pub(crate) fn open_with(&self, code: &str, now: DateTime<Utc>) -> OpenWindow {
        let expires = now + Duration::minutes(PAIRING_MINUTES);
        *self.lock() = Some(Window {
            code: code.to_string(),
            expires,
            failures: 0,
        });
        OpenWindow {
            display: display(code),
            code: code.to_string(),
            expires,
        }
    }

    pub(crate) fn close(&self) {
        *self.lock() = None;
    }

    /// The open window, unless it has expired (an expired one is dropped).
    pub(crate) fn current(&self, now: DateTime<Utc>) -> Option<OpenWindow> {
        let mut window = self.lock();
        if window.as_ref().is_some_and(|w| w.expires <= now) {
            *window = None;
        }
        window.as_ref().map(|w| OpenWindow {
            display: display(&w.code),
            code: w.code.clone(),
            expires: w.expires,
        })
    }

    /// Try a code typed on a phone.
    pub(crate) fn attempt(&self, typed: &str, now: DateTime<Utc>) -> Attempt {
        let mut guard = self.lock();
        let Some(window) = guard.as_mut() else {
            return Attempt::Closed;
        };
        if window.expires <= now {
            *guard = None;
            return Attempt::Closed;
        }
        let typed = normalize_code(typed);
        if same_bytes(typed.as_bytes(), window.code.as_bytes()) {
            *guard = None;
            return Attempt::Paired;
        }
        window.failures += 1;
        let attempts_left = MAX_WRONG_CODES.saturating_sub(window.failures);
        if attempts_left == 0 {
            *guard = None;
        }
        Attempt::Wrong { attempts_left }
    }
}
