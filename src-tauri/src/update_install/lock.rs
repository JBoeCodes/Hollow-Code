//! One installer at a time: `installer.lock` in the state folder, the same
//! file and protocol `scripts/install-macos-app.mjs` uses, so this installer
//! and the build workspace's `.mjs` installer refuse each other too.
//!
//! - The lock is created whole: its text goes to a private temp file that is
//!   hard-linked into place (a link never replaces a file), so a contender
//!   never reads an empty lock.
//! - It names its holder: `{"pid", "pidStartedAt", "installId", "what",
//!   "startedAt", "by"}`. The `.mjs` reads `pid` (and ignores the rest).
//! - A holder is live when its pid is alive and, if the lock recorded the
//!   holder's start stamp, that pid still has the same start (a reused pid
//!   is a dead holder). An unknown answer counts as live: refuse rather
//!   than take a lock that may be held.
//! - A dead holder's lock is taken over atomically: the contender claims
//!   that very file by hard-linking it to `installer.lock.takeover-<inode>`,
//!   a name only one can create, and removes the lock only if the claim is
//!   the same file (inode and text) it judged dead.
//! - Release removes the lock only while it still holds this holder's text.

use super::effects::Procs;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(crate) const LOCK_FILE: &str = "installer.lock";
/// A takeover claim older than this was left by an installer that died
/// mid-takeover (wall clock, like the claim's ctime).
const ABANDONED_CLAIM: Duration = Duration::from_secs(10);

/// The held lock; released on drop.
#[derive(Debug)]
pub(crate) struct InstallerLock {
    path: PathBuf,
    text: String,
}

impl Drop for InstallerLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|t| t == self.text) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LockError {
    /// A live installer holds it: its pid and what it is doing.
    Busy(String),
    Io(String),
}

impl LockError {
    pub(crate) fn message(&self) -> String {
        match self {
            LockError::Busy(holder) => format!(
                "another Hollow installer is running ({holder}), so this one did not start; wait for it to finish, then try again"
            ),
            LockError::Io(why) => format!("could not take the installer lock: {why}"),
        }
    }
}

/// What the lock file says about its holder.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Holder {
    pid: Option<i64>,
    pid_started_at: Option<u64>,
    install_id: Option<String>,
    what: Option<String>,
}

impl Holder {
    fn describe(&self) -> String {
        let mut text = format!(
            "pid {}, {}",
            self.pid.map(|p| p.to_string()).unwrap_or("unknown".into()),
            self.what.as_deref().unwrap_or("install")
        );
        if let Some(id) = self.install_id.as_deref().filter(|id| !id.is_empty()) {
            text.push(' ');
            text.push_str(id);
        }
        text
    }

    /// Live unless its pid is gone or now belongs to another process.
    fn live(&self, procs: &dyn Procs) -> bool {
        let Some(pid) = self
            .pid
            .and_then(|p| i32::try_from(p).ok())
            .filter(|p| *p > 0)
        else {
            return false;
        };
        match procs.alive(pid) {
            Some(false) => false,
            _ => match (self.pid_started_at, procs.started_micros(pid)) {
                (Some(recorded), Some(now)) => recorded == now,
                _ => true,
            },
        }
    }
}

/// Take the lock in `dir` for `what` (`install`, `go back`) as `pid`.
pub(crate) fn acquire(
    dir: &Path,
    install_id: &str,
    what: &str,
    pid: i32,
    started_at: &str,
    procs: &dyn Procs,
) -> Result<InstallerLock, LockError> {
    create_private_dir(dir).map_err(|e| LockError::Io(e.to_string()))?;
    let path = dir.join(LOCK_FILE);
    let text = serde_json::json!({
        "pid": pid,
        "pidStartedAt": procs.started_micros(pid),
        "installId": install_id,
        "what": what,
        "startedAt": started_at,
        "by": "hollow install-update",
    })
    .to_string();
    for _ in 0..40 {
        match create_with_text(&path, &text, pid) {
            Ok(true) => {
                return Ok(InstallerLock {
                    path,
                    text: text.clone(),
                })
            }
            Ok(false) => {}
            Err(e) => return Err(LockError::Io(e.to_string())),
        }
        let Some((ino, seen)) = read_lock(&path).map_err(|e| LockError::Io(e.to_string()))? else {
            continue;
        };
        // Unreadable means damaged: the lock is always written whole.
        let holder: Holder = serde_json::from_str(&seen).unwrap_or_default();
        if holder.live(procs) {
            return Err(LockError::Busy(holder.describe()));
        }
        let claim = dir.join(format!("{LOCK_FILE}.takeover-{ino}"));
        match fs::hard_link(&path, &claim) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Another installer is taking it over; wait, unless it died
                // doing so.
                let abandoned = fs::symlink_metadata(&claim)
                    .ok()
                    .and_then(|m| {
                        let ctime =
                            SystemTime::UNIX_EPOCH + Duration::from_secs(m.ctime().max(0) as u64);
                        SystemTime::now().duration_since(ctime).ok()
                    })
                    .is_some_and(|age| age >= ABANDONED_CLAIM);
                if abandoned {
                    let _ = fs::remove_file(&claim);
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(e) => return Err(LockError::Io(e.to_string())),
        }
        let claimed = read_lock(&claim);
        if let Ok(Some((claimed_ino, claimed_text))) = claimed {
            if claimed_ino == ino && claimed_text == seen {
                let _ = fs::remove_file(&path);
            }
        }
        let _ = fs::remove_file(&claim);
    }
    Err(LockError::Io(format!(
        "{}: other installers kept taking it over",
        path.display()
    )))
}

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Create `path` holding `text`, or `false` when it exists.
fn create_with_text(path: &Path, text: &str, pid: i32) -> io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_file_name(format!(
        "{LOCK_FILE}.{pid}.{:08x}.tmp",
        rand::random::<u32>()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        match fs::hard_link(&tmp, path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        }
    })();
    let _ = fs::remove_file(&tmp);
    result
}

/// The lock's inode and text through one descriptor; `None` when gone.
fn read_lock(path: &Path) -> io::Result<Option<(u64, String)>> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let ino = file.metadata()?.ino();
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(Some((ino, text)))
}

/// Who holds the lock now, for `--status` (no takeover, no write).
pub(crate) fn holder(dir: &Path, procs: &dyn Procs) -> Option<(String, bool)> {
    let (_, text) = read_lock(&dir.join(LOCK_FILE)).ok()??;
    let holder: Holder = serde_json::from_str(&text).unwrap_or_default();
    Some((holder.describe(), holder.live(procs)))
}
