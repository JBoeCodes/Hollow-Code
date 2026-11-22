//! `hollow install-update`: the installer inside the app (updates design
//! 2026-11-14, section 6 and section 9; lane U row U4).
//!
//! A port of `scripts/install-macos-app.mjs` into the binary, so an update
//! installs without a repository checkout or Node. It is meant to run from
//! the staged new bundle's own binary as a detached process ([`detach`]: own
//! session and process group, no `HOLLOW_*` variables) while the old app
//! quits. The `.mjs` path and `self_install.rs` are unchanged; this mode is
//! additive (U10 consolidates later).
//!
//! What one install does, in order ([`run::install`]):
//!
//! 1. Checks the install folder is writable (else exit 5, nothing changed).
//! 2. Takes the installer lock ([`lock`]), the same `installer.lock` the
//!    `.mjs` takes, so the two installers never run at once (exit 3).
//! 3. Waits for the app to quit (exit 7 if it does not), then finishes or
//!    undoes any half-finished install from its journal ([`swap::recover`]).
//! 4. Opens a journal ([`journal`], state `prepared`), checks the staged
//!    bundle ([`verify`]: no escaping symlinks, not inside the installed app,
//!    `codesign --verify --deep --strict`, the expected team, version and
//!    bundle id, no downgrade without `--allow-downgrade`; exit 4).
//! 5. Copies it beside the installed app and verifies the copy again
//!    (`staged-verified`), then snapshots the Sessions databases
//!    ([`snapshot`], `snapshot-written`; a failure stops the install, exit 6).
//! 6. Swaps it in ([`swap`], `promoted`): an atomic `renamex_np(RENAME_SWAP)`,
//!    so there is never a moment with no app at the install path, and the old
//!    app becomes `.Hollow.app.previous` (the older previous waits as
//!    `.Hollow.app.update-discard` until the new app is healthy).
//! 7. Reopens the app and watches it start ([`health`], `started`, then
//!    `healthy`). Two failed starts undo the swap (`rolled-back`), reopen
//!    the previous app and write `last-install-failure.json` (exit 8).
//!
//! `--go-back` swaps `.Hollow.app.previous` back, journaled; with
//! `--restore-snapshot` it first puts the Sessions databases back from an
//! install's snapshot ([`restore`], row U6; exit 12 when it can't, with
//! nothing changed).
//!
//! Every process, file-system, codesign, launch and clock effect goes through
//! the small traits in [`effects`], so tests run on fakes and temp folders.
//! Flags, exit codes and the journal and failure-file schemas:
//! `docs/reference/build-and-ship.md`, "In-app installer".

pub(crate) mod cli;
pub(crate) mod detach;
pub(crate) mod effects;
pub(crate) mod health;
pub(crate) mod journal;
pub(crate) mod lock;
pub(crate) mod restore;
pub(crate) mod run;
pub(crate) mod snapshot;
pub(crate) mod swap;
pub(crate) mod verify;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod restore_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// The team the update's Developer ID belongs to (design section 3). A
/// config value: `--expected-team` or `HOLLOW_UPDATE_TEAM` override it, and
/// `-` (or `adhoc`) expects an ad hoc signature with no team.
pub(crate) const DEFAULT_TEAM: &str = "TEAM000000";
/// The only bundle this installer installs.
pub(crate) const BUNDLE_ID: &str = "com.hollow.app";
pub(crate) const DEFAULT_APP: &str = "/Applications/Hollow.app";

/// The exit statuses of `hollow install-update` (documented in
/// build-and-ship.md; U5 maps them to words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    /// Installed and started healthy, went back, or `--status`.
    Ok = 0,
    /// Failed before anything moved, or every move was undone.
    Failed = 1,
    /// Bad command line.
    Usage = 2,
    /// Another installer holds the lock.
    Busy = 3,
    /// The staged bundle was refused (signature, team, version, bundle id,
    /// symlinks, location, downgrade).
    Refused = 4,
    /// The install folder is not writable by this account.
    NotWritable = 5,
    /// The Sessions snapshot failed, so nothing was swapped.
    SnapshotFailed = 6,
    /// Hollow did not quit in time, so nothing was swapped.
    AppRunning = 7,
    /// The new version failed to start twice; the previous one is back.
    WentBack = 8,
    /// Putting things back failed, or leftovers need a person: the journal
    /// says what is where.
    NeedsHand = 9,
    /// `--go-back` with no previous app to go back to.
    NothingToGoBack = 10,
    /// `--go-back` swapped, but the app did not start (nothing reversed).
    StartFailed = 11,
    /// `--go-back --restore-snapshot`: the snapshot could not be restored,
    /// so the data is untouched and the apps were not swapped.
    RestoreFailed = 12,
}

impl Exit {
    pub(crate) fn code(self) -> i32 {
        self as i32
    }
    /// The one-word name in the result line.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Exit::Ok => "ok",
            Exit::Failed => "failed",
            Exit::Usage => "usage",
            Exit::Busy => "busy",
            Exit::Refused => "refused",
            Exit::NotWritable => "not-writable",
            Exit::SnapshotFailed => "snapshot-failed",
            Exit::AppRunning => "app-running",
            Exit::WentBack => "went-back",
            Exit::NeedsHand => "needs-hand",
            Exit::NothingToGoBack => "nothing-to-go-back",
            Exit::StartFailed => "start-failed",
            Exit::RestoreFailed => "restore-failed",
        }
    }
}

/// How a run ended: the status and a sentence for a person.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub(crate) exit: Exit,
    pub(crate) message: String,
    pub(crate) journal: Option<PathBuf>,
}

impl Outcome {
    pub(crate) fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
            journal: None,
        }
    }
}

/// The paths beside the installed app. `previous` is shared with the `.mjs`
/// installer (Go back works on either's install); the two in-flight names
/// are this installer's own, so the `.mjs` folder-based recovery never
/// touches them and this one never touches the `.mjs`'s `.installing`.
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub(crate) installed: PathBuf,
    pub(crate) previous: PathBuf,
    /// The verified copy of the staged bundle, before the swap; after the
    /// swap, briefly the old app on its way to `previous`.
    pub(crate) staging: PathBuf,
    /// The older previous, kept until the new app is healthy.
    pub(crate) discard: PathBuf,
    /// The `.mjs` installer's in-flight names: present means its install
    /// did not finish, and this installer refuses to guess.
    pub(crate) foreign: Vec<PathBuf>,
}

impl Layout {
    pub(crate) fn for_app(app: &Path) -> Self {
        let dir = install_dir(app);
        let name = app
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Hollow.app".into());
        let at = |suffix: &str| dir.join(format!(".{name}.{suffix}"));
        Self {
            installed: app.to_path_buf(),
            previous: at("previous"),
            staging: at("update-staging"),
            discard: at("update-discard"),
            foreign: vec![at("installing"), at("discard"), at("rollback")],
        }
    }

    pub(crate) fn exe(&self, bundle: &Path, executable: &str) -> PathBuf {
        bundle.join("Contents").join("MacOS").join(executable)
    }
}

pub(crate) fn install_dir(app: &Path) -> PathBuf {
    app.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/Library/Application Support/com.hollow.app/installer`: the folder
/// the `.mjs` keeps its lock and journals in. This installer shares the
/// lock and keeps its own journals in `update-journal/` there.
pub(crate) fn default_state_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        Some(
            dirs::data_dir()?
                .join(crate::instance::current().app_data_name())
                .join("installer"),
        )
    }
}

/// `~/Library/Application Support/Hollow/Sessions`: where the Sessions
/// databases live (`instance::support_dir`, everyday instance).
pub(crate) fn default_sessions_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        Some(crate::instance::support_dir()?.join("Sessions"))
    }
}

/// Milliseconds since the epoch as an ISO-8601 UTC string.
pub(crate) fn iso(ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
