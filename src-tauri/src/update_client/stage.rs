//! The staging folder: `<app data>/updates/staging/<version>/Hollow.app`,
//! never under `/Applications`. A version folder counts as ready only with
//! its `ready.json`, written after the bundle passed the app's own checks;
//! a ready one is reused (no second download), and folders of other
//! versions are removed.

use super::state::valid_version;
use super::unpack::{self, Limits};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const READY: &str = "ready.json";
const UNPACK: &str = "unpack";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadyMark {
    schema: u32,
    version: String,
    verified_at: String,
}

pub(crate) struct Staging {
    root: PathBuf,
}

impl Staging {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, version: &str) -> Result<PathBuf, String> {
        if !valid_version(version) {
            return Err(format!("{version} is not a version Hollow stages"));
        }
        Ok(self.root.join(version))
    }

    /// The staged bundle of `version`, when it was verified before.
    pub(crate) fn ready(&self, version: &str) -> Option<PathBuf> {
        let dir = self.dir(version).ok()?;
        let mark: ReadyMark = serde_json::from_slice(&fs::read(dir.join(READY)).ok()?).ok()?;
        let bundle = dir.join(unpack::APP);
        let is_dir = fs::symlink_metadata(&bundle)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        (mark.version == version && is_dir).then_some(bundle)
    }

    /// Unpack `archive` (its signature already checked) as `version`, run
    /// `verify` on the bundle, and mark it ready. Anything that fails
    /// leaves no folder for that version.
    pub(crate) fn stage<T>(
        &self,
        version: &str,
        archive: &[u8],
        limits: Limits,
        now: &str,
        verify: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<(PathBuf, T), String> {
        let dir = self.dir(version)?;
        remove(&dir).map_err(|e| format!("Could not clear the old staging folder: {e}"))?;
        private_dir(&self.root)
            .and_then(|()| private_dir(&dir))
            .map_err(|e| format!("Could not make the staging folder: {e}"))?;
        let result = (|| {
            let unpacked = dir.join(UNPACK);
            let app = unpack::unpack_tar_gz(archive, &unpacked, limits)?;
            let bundle = dir.join(unpack::APP);
            fs::rename(&app, &bundle)
                .map_err(|e| format!("Could not move the unpacked update into place: {e}"))?;
            remove(&unpacked).map_err(|e| e.to_string())?;
            let checked = verify(&bundle)?;
            let mark = ReadyMark {
                schema: 1,
                version: version.to_string(),
                verified_at: now.to_string(),
            };
            let bytes = serde_json::to_vec_pretty(&mark).map_err(|e| e.to_string())?;
            crate::vault::atomic_write(&dir.join(READY), &bytes)
                .map_err(|e| format!("Could not mark the update ready: {e}"))?;
            Ok((bundle, checked))
        })();
        if result.is_err() {
            let _ = remove(&dir);
        }
        result
    }

    /// Remove every version folder (and stray file) but `keep`'s.
    pub(crate) fn prune(&self, keep: Option<&str>) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            if keep.is_some_and(|k| entry.file_name() == k) {
                continue;
            }
            let _ = remove(&entry.path());
        }
    }

    /// Forget a staged version (it failed its checks at install time).
    pub(crate) fn discard(&self, version: &str) {
        if let Ok(dir) = self.dir(version) {
            let _ = remove(&dir);
        }
    }
}

/// Remove a file, symlink (never followed) or folder; absent is fine.
fn remove(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

/// Create `path` (and its parents) and make it 0700: only this account
/// reads what is staged.
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}
