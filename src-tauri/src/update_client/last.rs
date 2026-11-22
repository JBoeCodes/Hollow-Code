//! What the last update did, for the notice after the restart: "Updated to
//! 0.6.<n>" with its notes, or "went back because …" from the installer's
//! `last-install-failure.json` (`update_install::run::write_failure`).
//!
//! Before the installer starts, the app writes `last-update.json` (the
//! install id, from and to versions, notes). At the next start the window
//! asks `update_last_result` until it acknowledges with
//! `update_ack_last_result`.

use super::state::Channel;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const FILE: &str = "last-update.json";

/// The record written before the installer starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Started {
    pub(crate) schema: u32,
    pub(crate) install_id: String,
    pub(crate) from_version: String,
    pub(crate) to_version: String,
    pub(crate) notes: String,
    pub(crate) channel: Channel,
    pub(crate) at: String,
    /// The app that wrote it (its own result is never shown to itself).
    pub(crate) pid: u32,
    #[serde(default)]
    pub(crate) build_id: Option<String>,
    #[serde(default)]
    pub(crate) acknowledged: bool,
    /// Row U6: an update (default) or the person's Go back, which writes
    /// `to_version` = the previous version.
    #[serde(default)]
    pub(crate) kind: StartedKind,
    /// Go back only: the Sessions databases were to be restored from the
    /// snapshot taken before the update.
    #[serde(default)]
    pub(crate) restore_data: bool,
    /// Row U6: the skip this result implies was applied at a launch
    /// (`Client::settle_go_back_skip`), whether or not the window has
    /// acknowledged the notice; it is applied once.
    #[serde(default)]
    pub(crate) skip_settled: bool,
    /// A Local build's receipt and branch (row U10), for the notice and the
    /// build receipt after the restart.
    #[serde(default)]
    pub(crate) local: Option<super::local::LocalRecord>,
    /// Row U7: the overnight installer started it, so the next launch posts
    /// one Hollow notification about it ([`overnight_notice`]).
    #[serde(default)]
    pub(crate) overnight: bool,
    /// Row U7: that notification was posted (never twice).
    #[serde(default)]
    pub(crate) overnight_notified: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StartedKind {
    #[default]
    Install,
    GoBack,
}

/// The notice after an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateLastResult {
    /// This app is the new version.
    Updated {
        version: String,
        notes: String,
        channel: Channel,
        at: String,
        /// "Updated to the local build of <branch>" (row U10).
        local: Option<super::local::UpdateLocalBuild>,
    },
    /// The new version failed to start twice and the installer put the
    /// previous one back, or (`manual`, row U6) the person chose Go back:
    /// `attempted` is then the version gone back from (now skipped) and
    /// `restored` this one.
    WentBack {
        attempted: String,
        restored: Option<String>,
        reason: String,
        at: String,
        /// The person's Go back, not the automatic one.
        manual: bool,
        /// The chats were put back as they were before the update; the
        /// newer ones are kept aside (`<file>.before-go-back`).
        #[serde(rename = "dataRestored")]
        data_restored: bool,
    },
    /// The person's Go back did not happen (row U6): this is still the
    /// version they went back from, which is no longer skipped.
    NotWentBack {
        target: String,
        reason: Option<String>,
        at: String,
    },
    /// The installer didn't swap (it refused, or Hollow was opened again
    /// meanwhile); this is still the old version.
    NotInstalled {
        attempted: String,
        reason: Option<String>,
        at: String,
    },
}

pub(crate) fn write(path: &Path, started: &Started) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(started).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(path, &bytes)
}

pub(crate) fn read_started(path: &Path) -> Option<Started> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The installer's failure file, when it is about `install_id`.
fn failure(state_dir: Option<&Path>, install_id: &str) -> Option<serde_json::Value> {
    let path = state_dir?.join(crate::update_install::run::FAILURE_FILE);
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (value.get("installId").and_then(|v| v.as_str()) == Some(install_id)).then_some(value)
}

/// The record's read-modify-writes (the window's acknowledgement and the
/// launch's skip) in one process, so neither loses the other's flag.
static REWRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rewrite(path: &Path, change: impl FnOnce(&mut Started) -> bool) -> Result<(), String> {
    let _held = REWRITE.lock().unwrap_or_else(|p| p.into_inner());
    match read_started(path) {
        Some(mut started) => match change(&mut started) {
            true => write(path, &started),
            false => Ok(()),
        },
        None => Ok(()),
    }
}

/// The notice for this app, if one is waiting. `build` is the running
/// app's identity: a Local build counts as updated only when it is that
/// build ([`super::local::is_running`]), since a branch build may carry the
/// running version (review M2); without it a Local install never reads as
/// updated.
pub(crate) fn read(
    path: &Path,
    state_dir: Option<&Path>,
    running: &str,
    build: Option<&crate::app_menu::BuildIdentity>,
    own_pid: u32,
) -> Option<UpdateLastResult> {
    let started = read_started(path)?;
    if started.acknowledged {
        return None;
    }
    result_of(started, state_dir, running, build, own_pid)
}

/// The result the launch's skip follows: as [`read`], acknowledged or
/// not, until [`mark_skip_settled`].
pub(crate) fn for_skip(
    path: &Path,
    state_dir: Option<&Path>,
    running: &str,
    build: Option<&crate::app_menu::BuildIdentity>,
    own_pid: u32,
) -> Option<UpdateLastResult> {
    let started = read_started(path)?;
    if started.skip_settled {
        return None;
    }
    result_of(started, state_dir, running, build, own_pid)
}

pub(crate) fn mark_skip_settled(path: &Path) -> Result<(), String> {
    rewrite(path, |started| {
        let changed = !started.skip_settled;
        started.skip_settled = true;
        changed
    })
}

fn result_of(
    started: Started,
    state_dir: Option<&Path>,
    running: &str,
    build: Option<&crate::app_menu::BuildIdentity>,
    own_pid: u32,
) -> Option<UpdateLastResult> {
    if started.pid == own_pid {
        return None;
    }
    if started.kind == StartedKind::GoBack {
        return Some(if running == started.to_version {
            UpdateLastResult::WentBack {
                attempted: started.from_version,
                restored: Some(started.to_version),
                reason: String::new(),
                at: started.at,
                manual: true,
                data_restored: started.restore_data,
            }
        } else {
            UpdateLastResult::NotWentBack {
                target: started.to_version,
                reason: journal_reason(state_dir, &started.install_id),
                at: started.at,
            }
        });
    }
    let is_this_build = match &started.local {
        Some(l) => build.is_some_and(|b| super::local::is_running(&l.receipt, b)),
        None => true,
    };
    if running == started.to_version && is_this_build {
        let local = started
            .local
            .as_ref()
            .map(|l| super::local::UpdateLocalBuild {
                workspace_id: l.receipt.workspace_id.clone(),
                branch: l.branch.clone(),
                build_id: l.receipt.build_id.clone(),
                dirty: l.receipt.dirty,
            });
        return Some(UpdateLastResult::Updated {
            version: started.to_version,
            notes: started.notes,
            channel: started.channel,
            at: started.at,
            local,
        });
    }
    let text =
        |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).map(str::to_string);
    match failure(state_dir, &started.install_id) {
        Some(f) if f.get("wentBack").and_then(|w| w.as_bool()) == Some(true) => {
            Some(UpdateLastResult::WentBack {
                attempted: text(&f, "attemptedVersion").unwrap_or(started.to_version),
                restored: text(&f, "restoredVersion"),
                reason: text(&f, "reason").unwrap_or_default(),
                at: text(&f, "at").unwrap_or(started.at),
                manual: false,
                data_restored: false,
            })
        }
        f => Some(UpdateLastResult::NotInstalled {
            attempted: started.to_version,
            reason: f.as_ref().and_then(|f| text(f, "reason")),
            at: started.at,
        }),
    }
}

/// Why a go back's journal settled where it did (its last step's reason).
fn journal_reason(state_dir: Option<&Path>, install_id: &str) -> Option<String> {
    let path = state_dir?
        .join(crate::update_install::journal::DIR)
        .join(format!(
            "{}.json",
            crate::update_install::journal::safe_id(install_id)
        ));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    value
        .get("steps")?
        .as_array()?
        .iter()
        .rev()
        .find_map(|s| s.get("reason").and_then(|r| r.as_str()).map(str::to_string))
}

/// Row U7: the one Hollow notification after an overnight install (`None`
/// for any other install, in the app that wrote the record, and once
/// posted). The record is marked notified before this returns, so a crash
/// while posting can't repeat it.
pub(crate) fn overnight_notice(
    path: &Path,
    state_dir: Option<&Path>,
    running: &str,
    build: Option<&crate::app_menu::BuildIdentity>,
    own_pid: u32,
) -> Option<super::overnight::Notice> {
    let started = read_started(path)?;
    if !started.overnight || started.overnight_notified || started.pid == own_pid {
        return None;
    }
    let result = result_of(started, state_dir, running, build, own_pid)?;
    let mut marked = false;
    rewrite(path, |started| {
        marked = !started.overnight_notified;
        started.overnight_notified = true;
        marked
    })
    .ok()?;
    marked.then(|| super::overnight::notice(&result, running))
}

/// The window showed the notice.
pub(crate) fn acknowledge(path: &Path) -> Result<(), String> {
    rewrite(path, |started| {
        let changed = !started.acknowledged;
        started.acknowledged = true;
        changed
    })
}
