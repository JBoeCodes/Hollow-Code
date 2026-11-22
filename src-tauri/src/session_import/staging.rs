//! The staging area (H5a), outside the vault: `<app support>/Imports/<id>/`.
//!
//! An inspect pass writes what it found here (a JSON manifest) and nothing
//! enters the session database until the person commits. A staged import is
//! deleted after its commit, when the person cancels, or after
//! [`KEEP_DAYS`] ([`sweep`]). Shared with the context scanners (CX19): any
//! consumer stages its own JSON documents under its own id with
//! [`Staging::write_json`].
//!
//! Ids are minted here ([`mint_id`]) and checked ([`is_safe_id`]) before they
//! become a folder name: `imp-` then lowercase letters, digits and `-`.

use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// A staged import older than this is deleted by [`sweep`].
pub(crate) const KEEP_DAYS: u64 = 7;

/// The longest staging id.
const ID_MAX: usize = 64;

/// The staging root for this app: `<app support>/Imports`. Test builds get a
/// per-process temp dir, so a test never writes into the real app data.
#[cfg(not(test))]
pub(crate) fn default_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("Imports"))
}

#[cfg(test)]
pub(crate) fn default_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir("hollow-test-imports-"))
}

/// A new staging id: `imp-<yyyymmdd>-<8 hex>`.
pub(crate) fn mint_id() -> String {
    use rand::Rng;
    let day = chrono::Utc::now().format("%Y%m%d");
    let suffix: u32 = rand::thread_rng().gen();
    format!("imp-{day}-{suffix:08x}")
}

/// Whether `id` may name a staging folder.
pub(crate) fn is_safe_id(id: &str) -> bool {
    id.len() <= ID_MAX
        && id.starts_with("imp-")
        && id.len() > 4
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// One staging root (the app's, or a test's temp dir).
#[derive(Debug, Clone)]
pub(crate) struct Staging {
    root: PathBuf,
}

impl Staging {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Staging { root: root.into() }
    }

    /// The folder of staged import `id`.
    pub fn dir(&self, id: &str) -> Result<PathBuf, String> {
        if !is_safe_id(id) {
            return Err("That isn't an import's id.".into());
        }
        Ok(self.root.join(id))
    }

    /// Write `value` as `<id>/<name>`, through a temp file and a rename so a
    /// reader never sees half a document.
    pub fn write_json<T: Serialize>(&self, id: &str, name: &str, value: &T) -> Result<(), String> {
        let dir = self.dir(id)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("staging folder: {e}"))?;
        let bytes = serde_json::to_vec(value).map_err(|e| format!("staging encode: {e}"))?;
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, bytes).map_err(|e| format!("staging write: {e}"))?;
        std::fs::rename(&tmp, dir.join(name)).map_err(|e| format!("staging write: {e}"))
    }

    /// Read `<id>/<name>`; `None` when it isn't there.
    pub fn read_json<T: DeserializeOwned>(
        &self,
        id: &str,
        name: &str,
    ) -> Result<Option<T>, String> {
        let path = self.dir(id)?.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("staging read: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("staging read: {e}")),
        }
    }

    /// Delete staged import `id` (nothing when it isn't there).
    pub fn discard(&self, id: &str) -> Result<(), String> {
        let dir = self.dir(id)?;
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("staging delete: {e}")),
        }
    }

    /// The staged ids, newest folder first.
    pub fn ids(&self) -> Vec<String> {
        let Ok(read) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out: Vec<(SystemTime, String)> = read
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                is_safe_id(&name).then_some(name)
            })
            .map(|name| {
                let at = std::fs::metadata(self.root.join(&name))
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (at, name)
            })
            .collect();
        out.sort_by(|a, b| b.cmp(a));
        out.into_iter().map(|(_, id)| id).collect()
    }
}

/// Delete staged imports last written more than [`KEEP_DAYS`] before `now`;
/// the ids deleted. A folder whose time can't be read is left.
pub(crate) fn sweep(staging: &Staging, now: SystemTime) -> Vec<String> {
    let keep = Duration::from_secs(KEEP_DAYS * 24 * 60 * 60);
    let mut gone = Vec::new();
    for id in staging.ids() {
        let Ok(dir) = staging.dir(&id) else { continue };
        let Some(at) = std::fs::metadata(&dir).and_then(|m| m.modified()).ok() else {
            continue;
        };
        if now.duration_since(at).is_ok_and(|age| age > keep) && staging.discard(&id).is_ok() {
            gone.push(id);
        }
    }
    gone
}
