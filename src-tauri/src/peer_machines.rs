//! The Macs this Hollow is paired with (M2 PR 2, the MacBook side of the
//! peer link, `docs/phone-api/machines.md`).
//!
//! **The record.** `<support>/Peers/machines.json`, mode 0600 in a 0700
//! folder, written whole through a temp file and a rename:
//! `{"version":1,"machines":[MachineRecord...]}`. One record per Mac, keyed by
//! the `machineId` that Mac reported when it paired; its SSH aliases in the
//! order they are tried (the personal tailnet's alias first, then the home
//! LAN's), the port its Hollow answers on there, and what it said about
//! itself. At most [`MACHINES_MAX`]. Not `settings.json` (Settings export
//! carries that file, and a peer list must not travel), never the vault (it
//! syncs through GitHub).
//!
//! **The token is never in this file**, nor in argv or a log: it goes to
//! `secret_store` under `peer:<machineId>` ([`token_key`]), the one Keychain
//! bundle the app keeps. Forgetting a Mac removes both.
//!
//! A file that isn't the JSON Hollow writes is left where it is and every
//! read and write says so (the person moves it aside; nothing here deletes
//! it). Fields a later Hollow adds are kept on every write.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Paired Macs, at most.
pub(crate) const MACHINES_MAX: usize = 4;
/// SSH aliases per Mac, at most.
pub(crate) const HOSTS_MAX: usize = 8;
/// The port a Hollow's phone and peer API answers on (its own loopback).
pub(crate) const DEFAULT_REMOTE_PORT: u16 = 47680;
/// `coo_seen_at` is renewed at most this often per Mac.
const COO_SEEN_EVERY_SECS: i64 = 3600;
/// A peer's old slug keeps resolving to it this long after a rename.
pub(crate) const SLUG_ALIAS_SECS: i64 = 7 * 24 * 3600;
/// Old slugs kept per Mac, at most.
const SLUG_ALIASES_MAX: usize = 4;
/// `last_seen_at` is written at most this often per Mac.
const TOUCH_EVERY_SECS: i64 = 60;
const VERSION: u32 = 1;
const FILE_NAME: &str = "machines.json";
const WHAT: &str = "The list of paired Macs";

/// A slug a paired Mac used to have, still resolving to it until `until`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SlugAlias {
    pub slug: String,
    /// RFC 3339.
    pub until: String,
}

/// One paired Mac, as this Hollow keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineRecord {
    /// `m-` and 16 hex digits, the other Mac's own id (`GET /v1/machine`).
    pub machine_id: String,
    /// That Mac's slug (`home/<slug>.md`), when it has one.
    #[serde(default)]
    pub slug: Option<String>,
    /// That Mac's name as its owner sees it.
    pub name: String,
    /// SSH aliases (or `user@host`), tried in this order.
    pub ssh_hosts: Vec<String>,
    /// The port its Hollow answers on, on its own loopback.
    #[serde(default = "default_port")]
    pub remote_port: u16,
    /// This Mac's device id there (`mc-` and 8 hex digits).
    pub device_id: String,
    pub paired_at: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    /// Its Hollow's version line, `0.5.17 (abc1234)`.
    #[serde(default)]
    pub last_version: Option<String>,
    #[serde(default = "one")]
    pub peer_api: u32,
    /// It answered 401: the token is dead there (revoked, or the record was
    /// reset). Pairing again clears it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub token_revoked: bool,
    /// M3a: it runs a runner COO, at this chat key, as its last `GET
    /// /v1/machine` said (the `coo` capability); none when it doesn't. This
    /// Mac's COO is then the dispatcher (`dispatcher_coo::active`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coo: Option<String>,
    /// When it last said whether it runs one (RFC 3339): a `coo` said over a
    /// day ago no longer counts (`dispatcher_coo::COO_FRESH_HOURS`); the app
    /// asks again hourly while it has one (`dispatcher_coo::start_refresh`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coo_seen_at: Option<String>,
    /// M3a PR2: since when it runs a runner COO (RFC 3339), as this Mac saw
    /// it: stamped when `coo` goes from none to a key (pairing included),
    /// kept while it has one (the key changes with its daily thread),
    /// cleared when it has none. None for a record from before the field
    /// (the COO's status then names no time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coo_since: Option<String>,
    /// M5 UI: the capabilities its last `GET /v1/machine` listed (`moves`
    /// says it takes a chat moved to it); empty until it answers once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Slugs it had before it was renamed (its own announcement said so),
    /// each resolving to it for [`SLUG_ALIAS_SECS`]: an owner stamp or route
    /// written with the old name keeps working meanwhile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub old_slugs: Vec<SlugAlias>,
    /// Fields a later Hollow wrote, kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MachineRecord {
    /// The old slugs that still resolve to this Mac at `now`.
    pub(crate) fn live_old_slugs(&self, now: DateTime<Utc>) -> Vec<String> {
        self.old_slugs
            .iter()
            .filter(|a| {
                DateTime::parse_from_rfc3339(&a.until).is_ok_and(|u| u.with_timezone(&Utc) > now)
            })
            .map(|a| a.slug.clone())
            .collect()
    }
}

fn default_port() -> u16 {
    DEFAULT_REMOTE_PORT
}

fn one() -> u32 {
    1
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    machines: Vec<MachineRecord>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The `secret_store` key a Mac's token is saved under.
pub(crate) fn token_key(machine_id: &str) -> String {
    format!("peer:{machine_id}")
}

/// Save a Mac's token (never logged; the error never carries it).
pub(crate) fn save_token(machine_id: &str, token: &str) -> Result<(), String> {
    #[cfg(test)]
    if tests::FAIL_SAVE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_deref()
        == Some(machine_id)
    {
        return Err("Hollow couldn't save the key for that Mac in the Keychain (test).".into());
    }
    crate::secret_store::set(&token_key(machine_id), token)
        .map_err(|e| format!("Hollow couldn't save the key for that Mac in the Keychain ({e})."))
}

/// A Mac's saved token, if any.
pub(crate) fn read_token(machine_id: &str) -> Result<Option<String>, String> {
    crate::secret_store::get(&token_key(machine_id))
        .map_err(|e| format!("Hollow couldn't read the key for that Mac from the Keychain ({e})."))
}

/// Paired Macs' record ([`MachineRecord`]s in `machines.json`).
pub(crate) struct Machines {
    path: PathBuf,
    /// One read-change-write at a time: shared by every [`Machines::for_app`]
    /// in this process, its own for [`Machines::at`].
    lock: Arc<Mutex<()>>,
}

impl Machines {
    /// The app's record, `<support>/Peers/machines.json`.
    pub(crate) fn for_app() -> Option<Self> {
        static APP_LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
        let dir = crate::instance::support_dir()?;
        Some(Self {
            path: dir.join("Peers").join(FILE_NAME),
            lock: APP_LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone(),
        })
    }

    /// The record in `dir` (tests).
    #[cfg(test)]
    pub(crate) fn at(dir: &std::path::Path) -> Self {
        Self {
            path: dir.join(FILE_NAME),
            lock: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn read(&self) -> Result<File, String> {
        let file: Option<File> =
            crate::state_file::read_json_or(&self.path, WHAT, crate::state_file::OnDamage::Leave)?;
        let file = file.unwrap_or_default();
        if file.version > VERSION {
            return Err(format!(
                "{WHAT} was written by a newer Hollow (version {}); update Hollow on this Mac.",
                file.version
            ));
        }
        Ok(file)
    }

    fn write(&self, mut file: File) -> Result<(), String> {
        file.version = VERSION;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("{WHAT} couldn't be saved ({}): {e}.", dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        crate::vault::atomic_write_private(&self.path, format!("{text}\n").as_bytes())
    }

    fn change<T>(&self, edit: impl FnOnce(&mut File) -> Result<T, String>) -> Result<T, String> {
        self.change_if(|file| edit(file).map(|out| (out, true)))
    }

    /// [`Machines::change`], writing only when `edit` says it changed.
    fn change_if<T>(
        &self,
        edit: impl FnOnce(&mut File) -> Result<(T, bool), String>,
    ) -> Result<T, String> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = self.read()?;
        let (out, changed) = edit(&mut file)?;
        if changed {
            self.write(file)?;
        }
        Ok(out)
    }

    /// Every paired Mac, in the order they paired.
    pub(crate) fn list(&self) -> Result<Vec<MachineRecord>, String> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.read()?.machines)
    }

    #[cfg(test)]
    pub(crate) fn get(&self, machine_id: &str) -> Result<Option<MachineRecord>, String> {
        Ok(self
            .list()?
            .into_iter()
            .find(|m| m.machine_id == machine_id))
    }

    /// Add a Mac, or replace its record (keyed by `machine_id`, keeping
    /// fields a later Hollow wrote into the old one).
    pub(crate) fn upsert(&self, record: MachineRecord) -> Result<(), String> {
        check(&record)?;
        self.change(|file| {
            let count = file.machines.len();
            match file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == record.machine_id)
            {
                Some(old) => {
                    let mut record = record;
                    for (k, v) in std::mem::take(&mut old.extra) {
                        record.extra.entry(k).or_insert(v);
                    }
                    *old = record;
                }
                None if count >= MACHINES_MAX => {
                    return Err(format!(
                        "Hollow pairs with at most {MACHINES_MAX} other Macs. Forget one first."
                    ))
                }
                None => file.machines.push(record),
            }
            Ok(())
        })?;
        // M4: the paired Macs decide whether the vault is shared.
        crate::runs_on::Seat::forget();
        Ok(())
    }

    /// Remove a Mac and its saved token. `Ok(false)` when it wasn't paired.
    /// The token goes even when the record can't be read.
    pub(crate) fn forget(&self, machine_id: &str) -> Result<bool, String> {
        let token = crate::secret_store::delete(&token_key(machine_id))
            .map_err(|e| format!("Hollow couldn't remove that Mac's key from the Keychain ({e})."));
        let removed = self.change(|file| {
            let before = file.machines.len();
            file.machines.retain(|m| m.machine_id != machine_id);
            Ok(file.machines.len() != before)
        })?;
        crate::runs_on::Seat::forget(); // M4
        token?;
        Ok(removed)
    }

    /// The Mac answered: note when, and its version. Written at most once a
    /// minute per Mac, unless the version changed or the token was marked
    /// dead. Failures are logged, never returned.
    pub(crate) fn touch(&self, machine_id: &str, version: Option<&str>, now: DateTime<Utc>) {
        let result = self.change_if(|file| {
            let Some(m) = file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == machine_id)
            else {
                return Ok(((), false));
            };
            let recent = m
                .last_seen_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| (now - at.with_timezone(&Utc)).num_seconds() < TOUCH_EVERY_SECS);
            let same_version = version.is_none() || version == m.last_version.as_deref();
            if recent && same_version && !m.token_revoked {
                return Ok(((), false));
            }
            m.last_seen_at = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            if let Some(v) = version {
                m.last_version = Some(v.to_string());
            }
            m.token_revoked = false;
            Ok(((), true))
        });
        if let Err(e) = result {
            crate::launch::elog(&format!("[peer] {e}"));
        }
    }

    /// The Mac announced `slug` (its `GET /v1/machine`): when it differs from
    /// the one kept, the kept one becomes an alias for [`SLUG_ALIAS_SECS`]
    /// and the record names the new one. Returns the old slug when a Mac that
    /// had one was renamed, so the caller can re-stamp owners that name it.
    ///
    /// A peer's word is trusted only about its own name. Ignored (nothing
    /// changes): no slug announced (that is no information, not a clear), one
    /// Hollow wouldn't accept, and one already held by this Mac (`own_slug`),
    /// another paired Mac or a live alias of another paired Mac, so a peer
    /// can't take over a name and then rename away to drag its owners along.
    /// The old slug is returned (and aliased) only if no other known Mac
    /// holds it either.
    pub(crate) fn refresh_slug(
        &self,
        machine_id: &str,
        slug: Option<&str>,
        own_slug: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let slug = slug.map(str::trim).filter(|s| !s.is_empty())?;
        if crate::context_roots::valid_slug(slug).is_err() || own_slug == Some(slug) {
            return None;
        }
        let result = self.change_if(|file| {
            let held_by_other = |name: &str, file: &File| {
                own_slug == Some(name)
                    || file.machines.iter().any(|o| {
                        o.machine_id != machine_id
                            && (o.slug.as_deref() == Some(name)
                                || o.live_old_slugs(now).iter().any(|a| a == name))
                    })
            };
            if held_by_other(slug, file) {
                return Ok((None, false));
            }
            let Some(idx) = file
                .machines
                .iter()
                .position(|m| m.machine_id == machine_id)
            else {
                return Ok((None, false));
            };
            let prev_taken = file.machines[idx]
                .slug
                .as_deref()
                .is_some_and(|p| held_by_other(p, file));
            let m = &mut file.machines[idx];
            let live = m.live_old_slugs(now);
            let before = m.old_slugs.len();
            m.old_slugs.retain(|a| live.contains(&a.slug));
            let mut changed = m.old_slugs.len() != before;
            let mut old = None;
            if m.slug.as_deref() != Some(slug) {
                if let Some(prev) = m.slug.take() {
                    m.old_slugs.retain(|a| a.slug != prev);
                    if !prev_taken {
                        let until = now + chrono::Duration::seconds(SLUG_ALIAS_SECS);
                        m.old_slugs.push(SlugAlias {
                            slug: prev.clone(),
                            until: until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        });
                        old = Some(prev);
                    }
                }
                // Back to a slug it once had: that alias is the name again.
                m.old_slugs.retain(|a| a.slug != slug);
                m.slug = Some(slug.to_string());
                changed = true;
            }
            while m.old_slugs.len() > SLUG_ALIASES_MAX {
                m.old_slugs.remove(0);
            }
            Ok((old, changed))
        });
        match result {
            Ok(old) => old,
            Err(e) => {
                crate::launch::elog(&format!("[peer] {e}"));
                None
            }
        }
    }

    /// The Mac listed `capabilities` in its `GET /v1/machine` (M5 UI: the
    /// window offers Move to runner only to a Mac listing `moves`). Written
    /// only when the list changed; failures are logged, never returned.
    pub(crate) fn set_capabilities(&self, machine_id: &str, capabilities: &[String]) {
        let result = self.change_if(|file| {
            match file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == machine_id)
            {
                Some(m) if m.capabilities != capabilities => {
                    m.capabilities = capabilities.to_vec();
                    Ok(((), true))
                }
                _ => Ok(((), false)),
            }
        });
        if let Err(e) = result {
            crate::launch::elog(&format!("[peer] {e}"));
        }
    }

    /// The Mac said at `now` whether it runs a runner COO, and at which
    /// chat key ([`MachineRecord::coo`], [`MachineRecord::coo_seen_at`]);
    /// written when that changed, or to renew `cooSeenAt` at most every
    /// [`COO_SEEN_EVERY_SECS`]. Failures are logged, never returned.
    pub(crate) fn set_coo(&self, machine_id: &str, coo: Option<&str>, now: DateTime<Utc>) {
        let result = self.change_if(|file| {
            let Some(m) = file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == machine_id)
            else {
                return Ok(((), false));
            };
            let stale = m
                .coo_seen_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_none_or(|at| {
                    (now - at.with_timezone(&Utc)).num_seconds() >= COO_SEEN_EVERY_SECS
                });
            let seen = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            let changed = m.coo.as_deref() != coo || stale;
            if changed {
                match coo {
                    None => m.coo_since = None,
                    Some(_) if m.coo.is_none() => m.coo_since = seen.clone(),
                    Some(_) => {}
                }
                m.coo = coo.map(str::to_string);
                m.coo_seen_at = seen;
            }
            Ok(((), changed))
        });
        if let Err(e) = result {
            crate::launch::elog(&format!("[peer] {e}"));
        }
    }

    /// The Mac answered 401: its token is dead there. `Ok(true)` when the
    /// record changed.
    pub(crate) fn mark_token_revoked(&self, machine_id: &str) -> Result<bool, String> {
        self.change_if(|file| {
            let changed = match file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == machine_id)
            {
                Some(m) if !m.token_revoked => {
                    m.token_revoked = true;
                    true
                }
                _ => false,
            };
            Ok((changed, changed))
        })
    }

    /// Replace a Mac's SSH aliases (reorder, add, remove).
    pub(crate) fn set_hosts(&self, machine_id: &str, hosts: Vec<String>) -> Result<(), String> {
        check_hosts(&hosts)?;
        self.change(|file| {
            let m = file
                .machines
                .iter_mut()
                .find(|m| m.machine_id == machine_id)
                .ok_or("That Mac isn't paired with this Hollow.")?;
            m.ssh_hosts = hosts;
            Ok(())
        })
    }
}

/// Why `host` can't be an SSH name here (it failed `remote_session::valid_host`).
pub(crate) fn host_refusal(host: &str) -> String {
    format!(
        "`{}` isn't an SSH name Hollow can use: an alias from ~/.ssh/config or user@host, made of letters, digits, dots, dashes, underscores and @, not starting with a dash.",
        crate::peer_client::remote_text(host)
    )
}

/// What a person typed in Pair a Mac: the SSH name to reach the Mac by and,
/// when given, the port its Hollow answers on there (`host:port`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PairAddress {
    pub host: String,
    pub port: Option<u16>,
}

/// Read a typed address (an ssh alias, `user@host`, a dotted or tailnet name,
/// an IPv4 address, each with an optional `:port`). Pairing only ever goes
/// over the person's own ssh login to that name, so this refuses what ssh
/// would read oddly: URLs and paths, IPv6 literals, this Mac's own loopback,
/// the unspecified, broadcast and multicast addresses, half-written IPv4
/// numbers and a port that isn't 1 to 65535.
pub(crate) fn parse_pair_address(input: &str) -> Result<PairAddress, String> {
    let typed = input.trim();
    let refuse = |why: &str| {
        Err(format!(
            "`{}` isn't an address Hollow can pair with: {why}",
            crate::peer_client::remote_text(typed)
        ))
    };
    if typed.is_empty() {
        return Err("Type the other Mac's name or address.".into());
    }
    if typed.contains("://") || typed.contains('/') || typed.contains('?') || typed.contains('#') {
        return refuse("type its name, not a web address or a path.");
    }
    if typed.chars().any(char::is_whitespace) {
        return refuse("it has a space in it.");
    }
    let (user, rest) = match typed.rsplit_once('@') {
        Some((user, rest)) => (Some(user), rest),
        None => (None, typed),
    };
    if user.is_some_and(|u| u.is_empty() || u.contains('@')) || rest.is_empty() {
        return refuse("expected name, user@name or name:port.");
    }
    let (name, port) = match rest.split_once(':') {
        None => (rest, None),
        Some((_, tail)) if tail.contains(':') => {
            return refuse("IPv6 addresses aren't supported; use its name.")
        }
        Some((name, tail)) => match tail.parse::<u16>() {
            Ok(p) if p >= 1 && tail.chars().all(|c| c.is_ascii_digit()) => (name, Some(p)),
            _ => return refuse("the port must be a number from 1 to 65535."),
        },
    };
    let bare = name.trim_end_matches('.');
    if bare.is_empty() || bare.split('.').any(str::is_empty) {
        return refuse("a name can't have empty parts.");
    }
    let lower = bare.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return refuse("that is this Mac, not another one.");
    }
    if bare.chars().all(|c| c.is_ascii_digit() || c == '.') {
        match bare.parse::<std::net::Ipv4Addr>() {
            Ok(ip) if ip.is_loopback() || ip.is_unspecified() => {
                return refuse("that is this Mac, not another one.")
            }
            Ok(ip) if ip.is_broadcast() || ip.is_multicast() => {
                return refuse("that isn't a Mac's address.")
            }
            Ok(_) => {}
            Err(_) => return refuse("that isn't a whole IPv4 address."),
        }
    }
    let host = match user {
        Some(u) => format!("{u}@{name}"),
        None => name.to_string(),
    };
    if !crate::remote_session::valid_host(&host) {
        return Err(host_refusal(&host));
    }
    Ok(PairAddress { host, port })
}

/// `mc-` and 8 lower-case hex digits, as the other Mac gives out.
pub(crate) fn valid_device_id(id: &str) -> bool {
    id.strip_prefix("mc-").is_some_and(|hex| {
        hex.len() == 8 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    })
}

fn check(record: &MachineRecord) -> Result<(), String> {
    if !crate::phone_access::valid_machine_id(&record.machine_id) {
        return Err("That Mac's id isn't one Hollow gives out.".into());
    }
    let name = record.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("That Mac's name must be 1 to 80 characters.".into());
    }
    if let Some(slug) = &record.slug {
        if crate::context_roots::valid_slug(slug).is_err() {
            return Err("That Mac's slug isn't one Hollow uses (lower-case letters, digits and single dashes).".into());
        }
    }
    if !valid_device_id(&record.device_id) {
        return Err("That Mac's device id for this one isn't one Hollow gives out.".into());
    }
    if record.remote_port == 0 {
        return Err("That Mac's port must be a number from 1 to 65535.".into());
    }
    check_hosts(&record.ssh_hosts)
}

fn check_hosts(hosts: &[String]) -> Result<(), String> {
    if hosts.is_empty() || hosts.len() > HOSTS_MAX {
        return Err(format!(
            "A Mac needs 1 to {HOSTS_MAX} SSH names (aliases from ~/.ssh/config)."
        ));
    }
    if let Some(bad) = hosts.iter().find(|h| !crate::remote_session::valid_host(h)) {
        return Err(host_refusal(bad));
    }
    let mut seen = std::collections::HashSet::new();
    if hosts.iter().any(|h| !seen.insert(h)) {
        return Err("Each SSH name may appear once.".into());
    }
    Ok(())
}

/// The aliases a pairing through `runner` keeps: that alias, then the same
/// alias with `-lan` when `resolve` (`ssh -G`, which prints the resolved
/// config without connecting) gives it a host of its own.
pub(crate) fn hosts_for(runner: &str, resolve: impl Fn(&str) -> Option<String>) -> Vec<String> {
    if !crate::remote_session::valid_host(runner) {
        return Vec::new();
    }
    let mut hosts = vec![runner.to_string()];
    if runner.contains('@') {
        return hosts;
    }
    let lan = format!("{runner}-lan");
    if resolve(&lan).is_some_and(|host| !host.eq_ignore_ascii_case(&lan)) {
        hosts.push(lan);
    }
    hosts
}

/// `ssh -G <alias>`'s `hostname` line: the alias itself when no Host block
/// names it.
pub(crate) fn resolved_host(ssh: &str, alias: &str) -> Option<String> {
    if !crate::remote_session::valid_host(alias) {
        return None;
    }
    let out = std::process::Command::new(ssh)
        .args(["-G", "--", alias])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("hostname ").map(|h| h.trim().to_string()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    /// `save_token` fails for this machine id (a Keychain write refused).
    pub(crate) static FAIL_SAVE: Mutex<Option<String>> = Mutex::new(None);

    fn record(id: &str) -> MachineRecord {
        MachineRecord {
            machine_id: id.into(),
            slug: Some("mac-mini".into()),
            name: "Mac mini".into(),
            ssh_hosts: vec!["runner".into(), "runner-lan".into()],
            remote_port: DEFAULT_REMOTE_PORT,
            device_id: "mc-1a2b3c4d".into(),
            paired_at: "2026-11-15T03:12:44Z".into(),
            last_seen_at: None,
            last_version: None,
            peer_api: 1,
            token_revoked: false,
            coo: None,
            coo_seen_at: None,
            coo_since: None,
            old_slugs: Vec::new(),
            capabilities: Vec::new(),
            extra: Map::new(),
        }
    }

    #[test]
    fn a_typed_address_keeps_a_name_and_port_and_refuses_what_ssh_would_misread() {
        let ok = |s: &str| parse_pair_address(s).unwrap();
        let at = |host: &str, port: Option<u16>| PairAddress {
            host: host.into(),
            port,
        };
        assert_eq!(ok("10.0.0.1"), at("10.0.0.1", None));
        assert_eq!(ok("runner-host"), at("runner-host", None));
        assert_eq!(
            ok("  runner.tail0000.ts.net "),
            at("runner.tail0000.ts.net", None)
        );
        assert_eq!(
            ok("builder@192.168.0.32:48000"),
            at("builder@192.168.0.32", Some(48000))
        );
        assert_eq!(
            ok("build-mac.local.:47680"),
            at("build-mac.local.", Some(47680))
        );
        for junk in [
            "",
            "   ",
            "https://example.com",
            "ssh://host",
            "host/path",
            "host name",
            "localhost",
            "app.localhost:1234",
            "127.0.0.1",
            "127.1.2.3:47680",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "999.1.1.1",
            "01.2.3.4",
            "10.0.0.01",
            "010.0.0.1",
            "1.2.3",
            "2130706433",
            "::1",
            "fe80::1:47680",
            "host:0",
            "host:65536",
            "host:+80",
            "host:",
            "host:abc",
            "@host",
            "a@b@host",
            "user@",
            "-oProxyCommand=x",
            "a..b",
            ".",
            "host;rm",
        ] {
            assert!(
                parse_pair_address(junk).is_err(),
                "{junk:?} should be refused"
            );
        }
    }

    #[test]
    fn the_record_is_private_keyed_by_machine_id_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let machines = Machines::at(&dir.path().join("Peers"));
        assert!(machines.list().unwrap().is_empty());
        machines.upsert(record("m-0000000000000001")).unwrap();
        let mut again = record("m-0000000000000001");
        again.name = "Studio".into();
        machines.upsert(again).unwrap();
        let list = machines.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Studio");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(machines.path()), 0o600);
            assert_eq!(mode(machines.path().parent().unwrap()), 0o700);
        }
        for n in 2..=4 {
            machines
                .upsert(record(&format!("m-000000000000000{n}")))
                .unwrap();
        }
        let full = machines.upsert(record("m-0000000000000005")).unwrap_err();
        assert!(full.contains("at most 4"), "{full}");
        // Bad input is refused before anything is written.
        let mut bad = record("m-0000000000000001");
        bad.ssh_hosts = vec!["-oProxyCommand=x".into()];
        assert!(machines.upsert(bad).is_err());
        let mut bad = record("m-0000000000000001");
        bad.ssh_hosts = vec![];
        assert!(machines.upsert(bad).is_err());
        assert!(machines.upsert(record("nope")).is_err());
        let mut bad = record("m-0000000000000001");
        bad.slug = Some("Not A Slug".into());
        assert!(machines.upsert(bad).is_err());
        let mut bad = record("m-0000000000000001");
        bad.device_id = "ph-1a2b3c4d".into();
        assert!(machines.upsert(bad).is_err());
        assert!(valid_device_id("mc-1a2b3c4d") && !valid_device_id("mc-1A2B3C4D"));
        assert_eq!(machines.list().unwrap().len(), 4);
    }

    #[test]
    fn a_later_hollows_fields_are_kept_and_a_damaged_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let machines = Machines::at(dir.path());
        let mut text = serde_json::to_value(File {
            version: 1,
            machines: vec![record("m-0000000000000001")],
            extra: Map::new(),
        })
        .unwrap();
        text["later"] = Value::from(true);
        text["machines"][0]["movedTo"] = Value::from("m-0000000000000002");
        std::fs::write(machines.path(), text.to_string()).unwrap();
        machines.upsert(record("m-0000000000000001")).unwrap();
        machines.touch("m-0000000000000001", Some("0.5.17 (abc1234)"), Utc::now());
        let back: Value =
            serde_json::from_str(&std::fs::read_to_string(machines.path()).unwrap()).unwrap();
        assert_eq!(back["later"], true);
        assert_eq!(back["machines"][0]["movedTo"], "m-0000000000000002");
        assert_eq!(back["machines"][0]["lastVersion"], "0.5.17 (abc1234)");

        std::fs::write(machines.path(), "{not json").unwrap();
        assert!(machines.list().is_err());
        assert!(machines.upsert(record("m-0000000000000003")).is_err());
        assert_eq!(
            std::fs::read_to_string(machines.path()).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn touch_is_at_most_once_a_minute_and_a_dead_token_is_marked_until_seen_again() {
        let dir = tempfile::tempdir().unwrap();
        let machines = Machines::at(dir.path());
        let id = "m-00000000000000aa";
        machines.upsert(record(id)).unwrap();
        let t0 = Utc::now();
        machines.touch(id, None, t0);
        let first = machines.get(id).unwrap().unwrap().last_seen_at;
        assert!(first.is_some());
        machines.touch(id, None, t0 + chrono::Duration::seconds(30));
        assert_eq!(machines.get(id).unwrap().unwrap().last_seen_at, first);
        machines.touch(id, None, t0 + chrono::Duration::seconds(61));
        assert_ne!(machines.get(id).unwrap().unwrap().last_seen_at, first);

        assert!(machines.mark_token_revoked(id).unwrap());
        assert!(!machines.mark_token_revoked(id).unwrap());
        assert!(machines.get(id).unwrap().unwrap().token_revoked);
        machines.touch(id, None, t0 + chrono::Duration::seconds(62));
        assert!(!machines.get(id).unwrap().unwrap().token_revoked);
    }

    #[test]
    fn the_token_lives_in_the_secret_store_never_the_file_and_forget_drops_both() {
        let dir = tempfile::tempdir().unwrap();
        let machines = Machines::at(dir.path());
        let id = "m-00000000000000bb";
        let token = "q3Zt8wQ0b1V4mN7xK2sY9cR5uE6aH0jL3pD8fG1iT4o";
        machines.upsert(record(id)).unwrap();
        save_token(id, token).unwrap();
        assert_eq!(read_token(id).unwrap().as_deref(), Some(token));
        let text = std::fs::read_to_string(machines.path()).unwrap();
        assert!(!text.contains(token));
        assert!(machines.forget(id).unwrap());
        assert!(!machines.forget(id).unwrap());
        assert_eq!(read_token(id).unwrap(), None);
        assert!(machines.list().unwrap().is_empty());
    }

    #[test]
    fn hosts_can_be_reordered_and_suggestions_add_the_lan_alias_only_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let machines = Machines::at(dir.path());
        let id = "m-00000000000000cc";
        machines.upsert(record(id)).unwrap();
        machines
            .set_hosts(id, vec!["runner-lan".into(), "runner".into()])
            .unwrap();
        assert_eq!(
            machines.get(id).unwrap().unwrap().ssh_hosts,
            ["runner-lan", "runner"]
        );
        assert!(machines
            .set_hosts(id, vec!["a".into(), "a".into()])
            .is_err());
        assert!(machines
            .set_hosts("m-00000000000000dd", vec!["a".into()])
            .is_err());

        let configured = |alias: &str| Some(format!("{alias}.example").replace("-lan.", ".lan."));
        assert_eq!(hosts_for("runner", configured), ["runner", "runner-lan"]);
        let unconfigured = |alias: &str| Some(alias.to_string());
        assert_eq!(hosts_for("runner", unconfigured), ["runner"]);
        assert_eq!(hosts_for("me@host", configured), ["me@host"]);
        assert!(hosts_for("-x", configured).is_empty());
    }
}
