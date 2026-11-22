//! The lease lock (Paperclip PR 18): kernel evidence that a full-vault lease
//! holder is still alive.
//!
//! The lease file (`.hollow-history/active-run.nosync`, `active-run` before night ST6) lives in the vault, where an
//! agent can rewrite it, so the pid it records is never trusted alone. While a
//! Hollow process holds the lease it also holds an exclusive `flock` on
//! `{data_dir}/Hollow/Leases/{vault-stem}.lock` (outside the vault, keyed by
//! the canonical vault path like the session database). The lock is released
//! when its [`HeldLock`] drops (an explicit unlock, so a child mid-spawn
//! holding a copy of the descriptor cannot keep it) or the process dies, so
//! "the lock is free" is proof that no live process holds it, which the
//! reclaim rule in `automated_history` requires before treating a lease as
//! left behind.
//!
//! `flock` locks belong to an open file description, so a second open in the
//! same process conflicts with the first: a probe from the holder's own
//! process reads `Held`, and a test can play "another process" by opening the
//! path itself.
//!
//! Failures are never fatal: [`hold`] returns `None` (the lease still works;
//! it just cannot be told apart from a dead one by the lock), and [`probe`]
//! returns [`LockProbe::Unknown`], which never allows a reclaim.

use crate::flock::HeldLock;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// What a try-lock of a vault's lease lock found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockProbe {
    /// Nobody holds it (the probe took it and let it go at once).
    Free,
    /// Some process (possibly this one, through another descriptor) holds it.
    Held,
    /// It could not be opened or locked: treat as "maybe held".
    Unknown,
}

/// Where the lease locks live: under the app data dir, outside the vault.
/// Test builds get a per-process temp dir (the `run_permit::permits_dir`
/// pattern), so a test can never touch a lock an installed Hollow holds.
#[cfg(not(test))]
pub(crate) fn leases_dir() -> PathBuf {
    crate::instance::support_dir()
        .map(|dir| dir.join("Leases"))
        .unwrap_or_else(|| std::env::temp_dir().join("hollow-leases"))
}

#[cfg(test)]
pub(crate) fn leases_dir() -> PathBuf {
    crate::test_temp::process_dir("hollow-test-leases-")
}

/// The lock file for `vault`: `{vault folder}-{sha256(canonical)[..16]}.lock`.
pub(crate) fn lock_path(vault: &Path) -> PathBuf {
    leases_dir().join(format!(
        "{}.lock",
        crate::session_store::vault_file_stem(vault)
    ))
}

/// History's object store lock for `vault` (Q-R9, 2026-11-13), beside its
/// lease lock: `{vault folder}-{sha256(canonical)[..16]}.objects.lock`. Object
/// cleanup holds it exclusively and every write that stores objects before
/// its record names them holds it shared
/// (`automated_history::objects_in_flight`), across processes.
pub(crate) fn objects_lock_path(vault: &Path) -> PathBuf {
    leases_dir().join(format!(
        "{}.objects.lock",
        crate::session_store::vault_file_stem(vault)
    ))
}

/// Open (creating) [`objects_lock_path`], holding nothing yet.
pub(crate) fn open_objects_lock(vault: &Path) -> std::io::Result<File> {
    open_lock_at(&objects_lock_path(vault))
}

fn open_lock(vault: &Path) -> std::io::Result<File> {
    open_lock_at(&lock_path(vault))
}

fn open_lock_at(path: &Path) -> std::io::Result<File> {
    let dir = leases_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: the locks hold no data, but nobody else needs them.
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Take `vault`'s lease lock for as long as the returned lock lives. `None`
/// on any failure (logged), including when another descriptor holds it.
pub(crate) fn hold(vault: &Path) -> Option<HeldLock> {
    hold_inner(vault, true)
}

/// [`hold`] without the "already held" log, for a reclaim, where a held lock
/// just means another process got there first (Paperclip PR 18).
pub(crate) fn hold_quiet(vault: &Path) -> Option<HeldLock> {
    hold_inner(vault, false)
}

fn hold_inner(vault: &Path, log_held: bool) -> Option<HeldLock> {
    // Another process's [`probe`] holds the lock for an instant; a couple of
    // short retries keep that from costing this lease its lock.
    for attempt in 0..HOLD_ATTEMPTS {
        let file = match open_lock(vault) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[history] could not open the lease lock: {error}");
                return None;
            }
        };
        match HeldLock::try_take(file) {
            Ok(lock) => return Some(lock),
            Err(TryLockError::WouldBlock) if attempt + 1 < HOLD_ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(TryLockError::WouldBlock) => {
                if log_held {
                    eprintln!("[history] the lease lock is already held; this lease can only be told apart from a dead one by its pid");
                }
                return None;
            }
            Err(TryLockError::Error(error)) => {
                eprintln!("[history] could not take the lease lock: {error}");
                return None;
            }
        }
    }
    None
}

/// Tries [`hold`] makes before giving up on a held lock.
const HOLD_ATTEMPTS: usize = 3;

/// Try `vault`'s lease lock without keeping it.
pub(crate) fn probe(vault: &Path) -> LockProbe {
    let Ok(file) = open_lock(vault) else {
        return LockProbe::Unknown;
    };
    match HeldLock::try_take(file) {
        // Dropping the lock unlocks it at once, so a probe never keeps it.
        Ok(_lock) => LockProbe::Free,
        Err(TryLockError::WouldBlock) => LockProbe::Held,
        Err(TryLockError::Error(_)) => LockProbe::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_reads_held_until_its_file_is_dropped() {
        let vault = tempfile::Builder::new()
            .prefix("vault-lease-")
            .tempdir()
            .unwrap();
        assert_eq!(probe(vault.path()), LockProbe::Free);
        assert_eq!(
            probe(vault.path()),
            LockProbe::Free,
            "a probe never keeps the lock"
        );
        let held = hold(vault.path()).expect("the lock is free");
        assert_eq!(
            probe(vault.path()),
            LockProbe::Held,
            "another descriptor conflicts"
        );
        assert!(hold(vault.path()).is_none(), "a second holder is refused");
        // A sibling test that spawns a process while this lock is held gives
        // the child a copy of the descriptor until its exec closes it; this
        // copy plays that child. The release must not wait for it, so there
        // is no polling here (merged plan wave 2 lock flake).
        let child_copy = held.descriptor_copy();
        drop(held);
        assert_eq!(probe(vault.path()), LockProbe::Free);
        drop(child_copy);

        let other = tempfile::Builder::new()
            .prefix("vault-lease-")
            .tempdir()
            .unwrap();
        let _held = hold(vault.path()).unwrap();
        assert_eq!(probe(other.path()), LockProbe::Free, "one lock per vault");
    }

    #[test]
    fn lease_locks_stay_out_of_the_real_app_data_dir_and_the_vault() {
        let vault = tempfile::Builder::new()
            .prefix("vault-lease-")
            .tempdir()
            .unwrap();
        let path = lock_path(vault.path());
        assert!(path.starts_with(std::env::temp_dir()), "{path:?}");
        assert!(!path.starts_with(vault.path()), "{path:?}");
        if let Some(real) = dirs::data_dir() {
            assert!(!path.starts_with(real.join("Hollow")), "{path:?}");
        }
    }
}
