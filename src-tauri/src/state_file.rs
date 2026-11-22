//! Fail-closed reads of Hollow's own JSON state and sidecar files (merged
//! plan slice 4, review B·F08).
//!
//! A store that is read, changed and written back must never mistake a file
//! it could not read for an empty one: the next save would replace the
//! user's data with that empty store. So only a missing file means empty.
//! Any other read error (permission denied, a directory in the way, an I/O
//! error) is returned and the file is left exactly where it is. A file that
//! reads but does not parse is moved aside to `<name>.corrupt-<timestamp>`
//! beside it, byte for byte, and the error says where it went; the next
//! save then starts a fresh store without destroying the damaged one.

use serde::de::DeserializeOwned;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// What a read does with a store that exists but does not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnDamage {
    /// Move it aside and say where. For writes, and for the screens the user
    /// opens, where the message naming the copy is seen.
    SetAside,
    /// Leave it where it is and say so. For agent actions and other
    /// unattended reads: the message naming a moved copy would reach only
    /// the agent, and the user's next screen would show a fresh, empty store
    /// with no explanation.
    Leave,
}

/// Read `path` as JSON. `Ok(None)` only when the file does not exist.
/// `what` names the store in errors, in plain words ("Results review
/// state"). A damaged file is moved aside ([`OnDamage::SetAside`]).
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path, what: &str) -> Result<Option<T>, String> {
    read_json_or(path, what, OnDamage::SetAside)
}

/// [`read_json`], choosing what happens to a damaged file.
pub(crate) fn read_json_or<T: DeserializeOwned>(
    path: &Path,
    what: &str,
    on_damage: OnDamage,
) -> Result<Option<T>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unreadable(path, what, &e)),
    };
    match on_damage {
        OnDamage::SetAside => parse_or_set_aside(path, &bytes, what).map(Some),
        OnDamage::Leave => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| left_damaged(path, what, &e.to_string())),
    }
}

/// Like [`read_json`] but never moves anything, for unattended read-only
/// checks (a background timer) where nobody would see the error explaining a
/// moved file. Any error means "don't act"; the next read through
/// [`read_json`] (a screen the user opens) moves the damage aside and says so.
pub(crate) fn peek_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The error for a store that exists but could not be read. Nothing moves.
pub(crate) fn unreadable(path: &Path, what: &str, e: &std::io::Error) -> String {
    let at = path.display();
    format!("{what} could not be read ({at}): {e}. It was left as is; nothing was overwritten.")
}

/// Parse `bytes` (just read from `path`); on a parse error move `path`
/// aside and return an error naming the copy.
pub(crate) fn parse_or_set_aside<T: DeserializeOwned>(
    path: &Path,
    bytes: &[u8],
    what: &str,
) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| damaged(path, what, &e.to_string()))
}

/// The error for a damaged store at `path`, after moving it aside. Also for
/// callers whose own validation (not JSON syntax) found the file unusable.
pub(crate) fn damaged(path: &Path, what: &str, why: &str) -> String {
    let at = path.display();
    match set_aside(path) {
        Ok(kept) => {
            let kept = kept.display();
            format!("{what} was damaged ({why}). It was moved to {kept}; a fresh one starts now.")
        }
        Err(e) => format!(
            "{what} was damaged ({why}) and could not be moved aside ({e}). \
             It was left as is at {at}; nothing was overwritten."
        ),
    }
}

/// The error for a damaged store left in place ([`OnDamage::Leave`]).
pub(crate) fn left_damaged(path: &Path, what: &str, why: &str) -> String {
    let at = path.display();
    format!(
        "{what} is damaged ({why}). It was left as is at {at}; nothing was moved or overwritten. \
         Opening it in Hollow moves it aside and says where."
    )
}

/// Rename `path` to a fresh `<name>.corrupt-<UTC timestamp>[-n]` beside it.
/// Never replaces an existing file.
pub(crate) fn set_aside(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?
        .to_string_lossy()
        .into_owned();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    for n in 0..100u32 {
        let candidate = if n == 0 {
            path.with_file_name(format!("{name}.corrupt-{stamp}"))
        } else {
            path.with_file_name(format!("{name}.corrupt-{stamp}-{n}"))
        };
        if std::fs::symlink_metadata(&candidate).is_ok() {
            continue;
        }
        std::fs::rename(path, &candidate).map_err(|e| e.to_string())?;
        return Ok(candidate);
    }
    Err("too many damaged copies with the same timestamp".into())
}

/// The `<name>.corrupt-*` copies beside `path`, sorted (tests and repair).
#[cfg(test)]
pub(crate) fn corrupt_copies(path: &Path) -> Vec<PathBuf> {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let prefix = format!("{name}.corrupt-");
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    type Map = BTreeMap<String, u32>;

    #[test]
    fn a_missing_file_is_the_only_empty_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        assert_eq!(read_json::<Map>(&path, "Store").unwrap(), None);
        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        assert_eq!(
            read_json::<Map>(&path, "Store").unwrap(),
            Some(Map::from([("a".to_string(), 1)]))
        );
    }

    #[test]
    fn a_damaged_file_is_moved_aside_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        // Invalid JSON, and invalid UTF-8 too.
        let bytes = b"{\"a\": \xff\xfe".to_vec();
        std::fs::write(&path, &bytes).unwrap();
        let err = read_json::<Map>(&path, "Store").unwrap_err();
        assert!(err.starts_with("Store was damaged"), "{err}");
        assert!(!path.exists(), "the damaged file is no longer in the way");
        let kept = corrupt_copies(&path);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(err.contains(&kept[0].display().to_string()), "{err}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), bytes);
        // A second damaged file never replaces the first copy.
        std::fs::write(&path, b"nope").unwrap();
        assert!(read_json::<Map>(&path, "Store").is_err());
        let kept = corrupt_copies(&path);
        assert_eq!(kept.len(), 2, "{kept:?}");
        // With the damage set aside, the store starts fresh.
        assert_eq!(read_json::<Map>(&path, "Store").unwrap(), None);
    }

    #[test]
    fn a_peek_never_moves_a_damaged_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        assert_eq!(peek_json::<Map>(&path).unwrap(), None);
        std::fs::write(&path, b"nope").unwrap();
        assert!(peek_json::<Map>(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"nope");
        assert!(corrupt_copies(&path).is_empty());
    }

    #[test]
    fn a_leave_read_reports_a_damaged_file_and_never_moves_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        assert_eq!(
            read_json_or::<Map>(&path, "Store", OnDamage::Leave).unwrap(),
            None
        );
        std::fs::write(&path, b"nope").unwrap();
        let err = read_json_or::<Map>(&path, "Store", OnDamage::Leave).unwrap_err();
        assert!(err.starts_with("Store is damaged"), "{err}");
        assert!(err.contains("left as is"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"nope");
        assert!(corrupt_copies(&path).is_empty());
        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        assert_eq!(
            read_json_or::<Map>(&path, "Store", OnDamage::Leave).unwrap(),
            Some(Map::from([("a".to_string(), 1)]))
        );
    }

    #[test]
    fn a_read_error_is_surfaced_and_the_file_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: a read error that is not
        // NotFound, whoever runs the test.
        let path = dir.path().join("store.json");
        std::fs::create_dir(&path).unwrap();
        let err = read_json::<Map>(&path, "Store").unwrap_err();
        assert!(err.starts_with("Store could not be read"), "{err}");
        assert!(path.is_dir());
        assert!(corrupt_copies(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_permission_error_is_surfaced_and_the_file_left_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            // Running as root: permissions don't stop the read.
            return;
        }
        let err = read_json::<Map>(&path, "Store").unwrap_err();
        assert!(err.starts_with("Store could not be read"), "{err}");
        assert!(corrupt_copies(&path).is_empty());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"a":1}"#);
    }
}
