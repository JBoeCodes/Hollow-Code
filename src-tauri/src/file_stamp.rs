//! Parsed files reused while their stamp holds (overnight 10-11/12 P5). The
//! same rule as the workspace brief cache (overnight Y7): a file's length,
//! modification and change times (ns) and inode. The change time can't be
//! set back, so a write that restores the mtime is still seen; a file
//! replaced by a rename has a new inode. An entry is trusted for at most
//! `max_age` even with its stamp unchanged (coarse times and reused inodes on
//! network or FAT volumes), and past `max` entries the cache starts over
//! instead of tracking age.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Length, mtime (ns), ctime (ns), inode.
pub(crate) type Stamp = (u64, i128, i128, u64);

pub(crate) fn stamp(meta: &std::fs::Metadata) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    (
        meta.len(),
        meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
        meta.ctime() as i128 * 1_000_000_000 + meta.ctime_nsec() as i128,
        meta.ino(),
    )
}

type Entry<T> = (Instant, Stamp, T);

/// What was parsed from each file, by absolute path.
pub(crate) struct StampCache<T> {
    entries: Mutex<Option<HashMap<PathBuf, Entry<T>>>>,
    max: usize,
    max_age: Duration,
}

impl<T: Clone> StampCache<T> {
    pub(crate) const fn new(max: usize, max_age: Duration) -> Self {
        Self {
            entries: Mutex::new(None),
            max,
            max_age,
        }
    }

    /// The value parsed from `path` when it had `stamp`, if still trusted.
    pub(crate) fn get(&self, path: &Path, stamp: Stamp) -> Option<T> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let (at, seen, value) = entries.as_ref()?.get(path)?;
        (*seen == stamp && at.elapsed() < self.max_age).then(|| value.clone())
    }

    /// Remember `value` as what `path` held at `stamp`. Not when both times
    /// are whole seconds: a FAT, exFAT or SMB volume, where a same-length
    /// rewrite within the tick would look unchanged.
    pub(crate) fn put(&self, path: &Path, stamp: Stamp, value: T) {
        if stamp.1 % 1_000_000_000 == 0 && stamp.2 % 1_000_000_000 == 0 {
            return;
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let map = entries.get_or_insert_with(HashMap::new);
        if map.len() >= self.max && !map.contains_key(path) {
            map.clear();
        }
        map.insert(path.to_path_buf(), (Instant::now(), stamp, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_or_its_age_decides_and_a_full_cache_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "one").unwrap();
        let s = stamp(&std::fs::metadata(&a).unwrap());
        let cache: StampCache<&str> = StampCache::new(2, Duration::from_secs(60));
        assert_eq!(cache.get(&a, s), None);
        cache.put(&a, s, "one");
        assert_eq!(cache.get(&a, s), Some("one"));
        // Any part of the stamp moving is a miss.
        assert_eq!(cache.get(&a, (s.0 + 1, s.1, s.2, s.3)), None);
        assert_eq!(cache.get(&a, (s.0, s.1, s.2 + 1, s.3)), None);
        assert_eq!(cache.get(&a, (s.0, s.1, s.2, s.3 + 1)), None);
        // Past its age, an unchanged stamp is read again.
        let stale: StampCache<&str> = StampCache::new(2, Duration::ZERO);
        stale.put(&a, s, "one");
        assert_eq!(stale.get(&a, s), None);
        // Whole-second times (a coarse volume) are never trusted.
        let coarse = (s.0, 1_000_000_000, 2_000_000_000, s.3);
        cache.put(&a, coarse, "coarse");
        assert_eq!(cache.get(&a, coarse), None);
        // Full: a new path starts the cache over; an existing one updates in place.
        cache.put(Path::new("/b"), s, "b");
        cache.put(&a, s, "one again");
        assert_eq!(cache.get(Path::new("/b"), s), Some("b"));
        cache.put(Path::new("/c"), s, "c");
        assert_eq!(cache.get(&a, s), None);
        assert_eq!(cache.get(Path::new("/c"), s), Some("c"));
    }
}
