//! Unpacking the downloaded `Hollow.app.tar.gz` (signature already checked)
//! into an empty staging folder, one vetted member at a time.
//!
//! Refused, and the whole unpack fails: an absolute path, a `..`
//! component, a member outside the one top-level `Hollow.app`, a symlink
//! that is absolute or resolves outside `Hollow.app`, a member that would
//! be written through a symlink (an earlier member's), a hard link, a
//! device, a FIFO, and archives past [`Limits`]. Set-id bits are dropped (the
//! `tar` crate keeps only `0o777` without `preserve_permissions`). The
//! release builder makes the archive with `/usr/bin/tar --no-mac-metadata
//! -czf … -C <dir> Hollow.app` (`scripts/release/nightly.mjs`).

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The one top-level member.
pub(crate) const APP: &str = "Hollow.app";

/// Caps that keep a hostile archive from filling the disk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Total bytes of every member (the installed app is about 163 MB).
    pub(crate) max_bytes: u64,
    pub(crate) max_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 2 << 30,
            max_entries: 200_000,
        }
    }
}

/// The member's path inside the archive, as clean components: no root, no
/// `..`, a leading `./` dropped, starting with [`APP`].
fn clean(path: &Path) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("{} climbs out with ..", path.display())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{} is an absolute path", path.display()))
            }
        }
    }
    if out.components().next() != Some(Component::Normal(APP.as_ref())) {
        return Err(format!("{} is outside {APP}", path.display()));
    }
    Ok(out)
}

/// Fold `..` and `.` lexically; `None` when it climbs past the start.
fn fold(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Whether a symlink at `member` (archive path) pointing at `target` stays
/// inside `Hollow.app`.
pub(crate) fn link_stays_inside(member: &Path, target: &Path) -> bool {
    if target.has_root() || target.as_os_str().is_empty() {
        return false;
    }
    let base = member.parent().unwrap_or(Path::new(""));
    fold(&base.join(target)).is_some_and(|p| p.starts_with(APP))
}

/// No existing part of `rel` under `dest`, the last part included, is a
/// symlink: nothing is written through a link.
fn no_link_on_the_way(dest: &Path, rel: &Path) -> Result<(), String> {
    let mut at = dest.to_path_buf();
    for part in rel.components() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "{} would be written through a symlink",
                    rel.display()
                ))
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

/// Unpack the gzip'd tar `bytes` into `dest` (created, must be empty) and
/// return `dest/Hollow.app`.
pub(crate) fn unpack_tar_gz(bytes: &[u8], dest: &Path, limits: Limits) -> Result<PathBuf, String> {
    unpack_tar(flate2::read::GzDecoder::new(bytes), dest, limits)
}

pub(crate) fn unpack_tar(
    reader: impl Read,
    dest: &Path,
    limits: Limits,
) -> Result<PathBuf, String> {
    let refuse = |why: String| format!("The downloaded update can't be unpacked safely: {why}.");
    fs::create_dir_all(dest)
        .map_err(|e| refuse(format!("could not make the staging folder: {e}")))?;
    if fs::read_dir(dest)
        .map_err(|e| refuse(e.to_string()))?
        .next()
        .is_some()
    {
        return Err(refuse("the staging folder isn't empty".into()));
    }
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(false);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(false);
    let (mut bytes, mut count) = (0u64, 0usize);
    let entries = archive
        .entries()
        .map_err(|e| refuse(format!("it isn't a tar archive ({e})")))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| refuse(format!("a damaged member ({e})")))?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        count += 1;
        if count > limits.max_entries {
            return Err(refuse(format!("more than {} members", limits.max_entries)));
        }
        // `size()` is the PAX size when the archive has one (the header field
        // can say 0 while the member carries far more).
        bytes = bytes.saturating_add(entry.size());
        if bytes > limits.max_bytes {
            return Err(refuse(format!("more than {} bytes", limits.max_bytes)));
        }
        let raw = entry
            .path()
            .map_err(|e| refuse(format!("a member with an unreadable name ({e})")))?
            .into_owned();
        let rel = clean(&raw).map_err(refuse)?;
        no_link_on_the_way(dest, &rel).map_err(refuse)?;
        let target = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| refuse(format!("{}: {e}", rel.display())))?;
            continue;
        }
        if rel == Path::new(APP) {
            return Err(refuse(format!("{APP} is not a folder")));
        }
        if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|e| refuse(e.to_string()))?
                .ok_or_else(|| refuse(format!("{} is a symlink to nothing", rel.display())))?
                .into_owned();
            if !link_stays_inside(&rel, &link) {
                return Err(refuse(format!(
                    "{} is a symlink that points outside {APP}",
                    rel.display()
                )));
            }
        } else if !(kind.is_file() || kind.is_contiguous()) {
            return Err(refuse(format!(
                "{} is not a file, folder or symlink",
                rel.display()
            )));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| refuse(format!("{}: {e}", rel.display())))?;
        }
        entry
            .unpack(&target)
            .map_err(|e| refuse(format!("{}: {e}", rel.display())))?;
    }
    let app = dest.join(APP);
    match fs::symlink_metadata(&app) {
        Ok(meta) if meta.is_dir() => Ok(app),
        _ => Err(refuse(format!("it has no {APP} folder"))),
    }
}
