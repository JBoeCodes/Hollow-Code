//! A held `flock` that is let go the moment it drops, even while another
//! copy of its descriptor is still open somewhere.
//!
//! A `flock` belongs to the open file description, not to one descriptor, and
//! closing a descriptor only releases it when that was the LAST descriptor on
//! the description. Rust opens every file close-on-exec, so a child Hollow
//! spawns never keeps one past its `exec`, but while a child is being spawned
//! (`posix_spawn`, or `fork` then `exec`) it holds a copy of every descriptor
//! its parent has open. A plain `drop(file)` on one thread while another
//! thread is spawning can therefore leave the lock held until that child
//! execs, and a retake right after the drop reads `WouldBlock`. That is what
//! made `run_pump`'s queue-ownership test and `run_permit`'s
//! drop-frees-the-slot test fail now and then in a full parallel
//! `cargo test --lib` (merged plan, wave 2 lock flake), and it is the same
//! window a production release has while Hollow spawns an agent.
//!
//! `flock(LOCK_UN)` acts on the description itself, whoever else holds a copy
//! of it, so [`HeldLock`] unlocks explicitly before its file closes and the
//! release is immediate. Used by the run queue owner lock (`run_pump`), the
//! run permit slots (`run_permit`), the lease lock (`lease_lock`) and the
//! self-hosted jobs' file and drain locks (`self_hosted_jobs`); a new
//! exclusive `flock` should use it too
//! rather than hold a bare [`File`]. The run ledger's `FileLock`
//! (`process_group::orphans`) still drops a bare file: it already polls for
//! up to its wait, so a retake rides out the window.

use std::fs::{File, TryLockError};

/// A `flock` on a file (exclusive unless taken with [`HeldLock::try_take_shared`]),
/// released on drop by an explicit unlock.
#[derive(Debug)]
pub(crate) struct HeldLock(File);

impl HeldLock {
    /// Take the exclusive lock on `file` without waiting. On `Err` the file is
    /// closed holding nothing.
    pub(crate) fn try_take(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self(file))
    }

    /// Take the exclusive lock on `file`, waiting for it. On `Err` the file
    /// is closed holding nothing.
    pub(crate) fn take(file: File) -> std::io::Result<Self> {
        file.lock()?;
        Ok(Self(file))
    }

    /// Take a shared `flock` on `file` without waiting: any number of shared
    /// holders, none beside an exclusive one. History's object store lock
    /// (`automated_history::objects_in_flight`) is the one user. On `Err` the
    /// file is closed holding nothing.
    pub(crate) fn try_take_shared(file: File) -> Result<Self, TryLockError> {
        file.try_lock_shared()?;
        Ok(Self(file))
    }

    /// A second descriptor on the locked file description: what a child
    /// being spawned at the moment of release would hold (tests only).
    #[cfg(test)]
    pub(crate) fn descriptor_copy(&self) -> File {
        self.0.try_clone().expect("dup the lock descriptor")
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        // Best effort: if the unlock fails, closing the file below still
        // releases the lock once no other copy remains.
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::path::Path;

    fn open(path: &Path) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap()
    }

    #[test]
    fn a_dropped_lock_is_free_at_once_even_while_a_copy_of_its_descriptor_lives() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("held.lock");
        let held = HeldLock::try_take(open(&path)).expect("free");
        assert!(
            matches!(
                HeldLock::try_take(open(&path)),
                Err(TryLockError::WouldBlock)
            ),
            "a second description conflicts"
        );
        // A dup shares the open file description exactly as a child caught
        // mid-spawn does. Before the explicit unlock this copy kept the lock.
        let copy = held.descriptor_copy();
        drop(held);
        let again = HeldLock::try_take(open(&path)).expect("released on drop despite the copy");
        drop(again);
        drop(copy);
    }

    /// The real race, not the dup stand-in: spawner threads keep children
    /// in flight while the main thread drops and retakes locks, first as
    /// bare files (the old code, which can fail) and then as [`HeldLock`]s
    /// (which must never fail). Ignored because it spawns thousands of
    /// processes; run it on demand and read the two counts:
    /// `cargo test --lib flock::tests::stress -- --ignored --nocapture`.
    #[test]
    #[ignore = "stress loop: spawns thousands of processes"]
    fn stress_release_beside_concurrent_spawns() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        const ROUNDS: usize = 2000;
        const SPAWNERS: usize = 3;
        let stop = Arc::new(AtomicBool::new(false));
        let spawned = Arc::new(AtomicUsize::new(0));
        let spawners: Vec<_> = (0..SPAWNERS)
            .map(|_| {
                let stop = stop.clone();
                let spawned = spawned.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = std::process::Command::new("/usr/bin/true").status();
                        spawned.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        let dir = tempfile::TempDir::new().unwrap();
        let mut bare_failures = 0;
        for round in 0..ROUNDS {
            let path = dir.path().join(format!("bare-{round}.lock"));
            let first = open(&path);
            first.try_lock().expect("a fresh path is free");
            // Hold across a few spawns so one can copy the descriptor.
            std::thread::sleep(Duration::from_micros(300));
            drop(first);
            match open(&path).try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => bare_failures += 1,
                Err(TryLockError::Error(error)) => panic!("{error}"),
            }
        }
        let mut held_failures = 0;
        for round in 0..ROUNDS {
            let path = dir.path().join(format!("held-{round}.lock"));
            let first = HeldLock::try_take(open(&path)).expect("a fresh path is free");
            std::thread::sleep(Duration::from_micros(300));
            drop(first);
            match HeldLock::try_take(open(&path)) {
                Ok(_) => {}
                Err(TryLockError::WouldBlock) => held_failures += 1,
                Err(TryLockError::Error(error)) => panic!("{error}"),
            }
        }

        stop.store(true, Ordering::Relaxed);
        for spawner in spawners {
            spawner.join().unwrap();
        }
        eprintln!(
            "flock stress: {ROUNDS} rounds each, {} children spawned; \
             bare File drop failed {bare_failures}, HeldLock drop failed {held_failures}",
            spawned.load(Ordering::Relaxed)
        );
        assert_eq!(held_failures, 0, "a HeldLock release is immediate");
    }
}
