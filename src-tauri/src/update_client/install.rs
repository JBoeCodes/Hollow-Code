//! Handing a verified staged build to `hollow install-update`, and "Update
//! when chats finish": a wait with **no expiry** (design section 7). Local
//! builds use it too since row U10, which removed the old 30-minute waiter.
//! The wait lives in memory only: if Hollow quits, it is gone.

use crate::update_install::verify::Team;
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// The installer's log, in its journal folder.
pub(crate) const LOG: &str = "install-update.log";
/// How often "Update when chats finish" looks again.
pub(crate) const POLL: Duration = Duration::from_secs(5);

/// One running thing that a restart would end (`lifecycle`'s list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpdateWork {
    pub key: String,
    pub name: String,
}

/// What `update_install_now` did. Work that runs is an answer, not an
/// error, so the window can offer "Update when chats finish".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum UpdateInstallOutcome {
    /// The installer started; Hollow quits in a moment and reopens updated.
    Started { version: String },
    /// Something runs (and no confirmed list was given): nothing happened.
    WorkRunning { work: Vec<UpdateWork> },
    /// The confirmed list no longer matches what runs (the restart check's
    /// `RESTART_WORK_CHANGED`): confirm again.
    WorkChanged { work: Vec<UpdateWork> },
}

/// The installer's command line (after `install-update`).
#[derive(Debug, Clone)]
pub(crate) struct HelperArgs {
    pub(crate) staged: PathBuf,
    pub(crate) version: String,
    pub(crate) team: Team,
    pub(crate) app: PathBuf,
    pub(crate) state_dir: PathBuf,
    /// `None`: no Sessions folder yet, so `--no-snapshot`.
    pub(crate) sessions: Option<PathBuf>,
    /// For a channel switch, Go back (row U6) and every Local build (U10).
    pub(crate) allow_downgrade: bool,
    pub(crate) install_id: String,
}

pub(crate) fn helper_args(a: &HelperArgs) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    let mut flag = |name: &str, value: &Path| {
        out.push(name.into());
        out.push(value.as_os_str().to_owned());
    };
    flag("--staged", &a.staged);
    flag("--expected-version", Path::new(&a.version));
    flag("--expected-team", Path::new(&a.team.label()));
    flag("--app", &a.app);
    flag("--journal-dir", &a.state_dir);
    match &a.sessions {
        Some(dir) => flag("--snapshot-source", dir),
        None => out.push("--no-snapshot".into()),
    }
    out.push("--install-id".into());
    out.push(a.install_id.clone().into());
    out.push("--relaunch".into());
    if a.allow_downgrade {
        out.push("--allow-downgrade".into());
    }
    out
}

/// `/…/Hollow.app` when `exe` is `/…/Hollow.app/Contents/MacOS/<name>`.
pub(crate) fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// What a "when chats finish" wait does once nothing runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitAction {
    /// Install the ready update ("Update when chats finish").
    Update,
    /// Go back to the previous version (row U6), restoring the data from
    /// before the update when asked.
    GoBack { restore_data: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitKind {
    Update,
    GoBack,
}

impl WaitAction {
    pub(crate) fn kind(self) -> WaitKind {
        match self {
            WaitAction::Update => WaitKind::Update,
            WaitAction::GoBack { .. } => WaitKind::GoBack,
        }
    }
}

pub(crate) const WAITING: u8 = 0;
pub(crate) const CANCELLED: u8 = 1;
pub(crate) const INSTALLING: u8 = 2;

/// How a wait ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WaitEnd {
    Installed,
    Cancelled,
    Failed(String),
}

/// Look every `interval` until `attempt` installs (`Ok(true)`) or fails, or
/// the ticket is cancelled. No timeout: it waits as long as work runs.
pub(crate) fn wait_until_idle(
    ticket: &AtomicU8,
    interval: Duration,
    sleep: &dyn Fn(Duration),
    mut attempt: impl FnMut() -> Result<bool, String>,
) -> WaitEnd {
    loop {
        if ticket.load(Ordering::SeqCst) == CANCELLED {
            return WaitEnd::Cancelled;
        }
        match attempt() {
            Ok(true) => return WaitEnd::Installed,
            Ok(false) => {}
            Err(why) => return WaitEnd::Failed(why),
        }
        sleep(interval);
    }
}

/// Claim the ticket for an install (`false` when it was cancelled first).
pub(crate) fn claim(ticket: &AtomicU8) -> bool {
    ticket
        .compare_exchange(WAITING, INSTALLING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Give a claimed ticket back (the install found work after all).
pub(crate) fn release(ticket: &AtomicU8) {
    let _ = ticket.compare_exchange(INSTALLING, WAITING, Ordering::SeqCst, Ordering::SeqCst);
}

/// Cancel a waiting ticket; refused once its install started.
pub(crate) fn cancel(ticket: &AtomicU8) -> bool {
    ticket
        .compare_exchange(WAITING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}
