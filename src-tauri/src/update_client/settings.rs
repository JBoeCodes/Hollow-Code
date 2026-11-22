//! The client's settings, `update-settings.json` in the app-data folder
//! (`agent_settings::app_data_dir`). Every field has a default, so an older
//! or missing file reads as the defaults. The token is never here: it is in
//! the Keychain (`token.rs`), so a settings export can't carry it.

use super::state::Channel;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) const FILE: &str = "update-settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateSettings {
    /// `main` (default) or `nightly`.
    pub channel: Channel,
    /// "Install updates overnight" (off by default): the overnight installer
    /// (row U7, `overnight.rs`) reads it at every look.
    pub install_overnight: bool,
    /// The version the person went back from (row U6 sets it): never
    /// offered until a newer one is published.
    pub skipped_version: Option<String>,
    /// The version hidden with "Not now": its button stays hidden until a
    /// newer version is ready.
    pub dismissed_version: Option<String>,
    /// The last finished check (RFC 3339).
    pub last_checked_at: Option<String>,
    /// Set by a channel switch: one lower version may be offered and
    /// installed (with `--allow-downgrade`). Cleared when that install
    /// starts, when the channel offers nothing lower or a higher build, and
    /// 24 hours after the switch.
    pub allow_downgrade_once: bool,
    /// When the switch set it (milliseconds since the epoch).
    pub allow_downgrade_since_ms: Option<u64>,
    /// Row U9: the person this Mac's token belongs to, as the update host's
    /// `/me` last said (`people.rs`). Cleared by a new token or a disconnect.
    pub person_name: Option<String>,
    /// Row U9: the channels that token may use, as `/me` last said (`[]`:
    /// none); `None` until it answers (then both are offered).
    pub allowed_channels: Option<Vec<Channel>>,
    /// "This Mac publishes to Main" (row U8, off by default): Settings,
    /// Updates shows Publish to Main and People (row U9), and the nudge
    /// runs only when on.
    pub publisher: bool,
    /// The Local build (its receipt id) hidden with "Not now" (row U10):
    /// not offered again at launch; a new build, or Show in Update button in
    /// Changes, offers one again.
    pub dismissed_local_build: Option<String>,
}

/// How long a channel switch's downgrade permission lasts.
pub(crate) const DOWNGRADE_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

impl UpdateSettings {
    /// The downgrade permission, if it hasn't expired at `now_ms`.
    pub(crate) fn downgrade_allowed(&self, now_ms: u64) -> bool {
        self.allow_downgrade_once
            && self
                .allow_downgrade_since_ms
                // A time in the future (a clock set back) counts as expired.
                .is_some_and(|since| since <= now_ms && now_ms - since < DOWNGRADE_WINDOW_MS)
    }
}

/// The settings file, read and written whole under one lock.
pub(crate) struct Store {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Store {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The saved settings; an unreadable or invalid file reads as defaults.
    pub(crate) fn read(&self) -> UpdateSettings {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        read_at(&self.path)
    }

    /// Change the settings with `f` and save them (atomic replace).
    pub(crate) fn update(
        &self,
        f: impl FnOnce(&mut UpdateSettings),
    ) -> Result<UpdateSettings, String> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut settings = read_at(&self.path);
        f(&mut settings);
        let bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|e| format!("Could not encode the update settings: {e}"))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Could not save the update settings: {e}"))?;
        }
        crate::vault::atomic_write(&self.path, &bytes)
            .map_err(|e| format!("Could not save the update settings: {e}"))?;
        Ok(settings)
    }
}

/// The file, or the defaults. A file that exists but doesn't parse is
/// moved aside to `update-settings.json.bad` first (it may hold a skipped
/// version the person chose), so the next save doesn't silently replace it.
fn read_at(path: &Path) -> UpdateSettings {
    let Ok(bytes) = std::fs::read(path) else {
        return UpdateSettings::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(settings) => settings,
        Err(_) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, PathBuf::from(bad));
            UpdateSettings::default()
        }
    }
}
