//! Move Hollow home: folders that sit in iCloud (a File Provider domain).
//!
//! Measured on a Mac with Desktop & Documents in iCloud (2026-11-12, scratch
//! folders only): `rename(2)` of a folder out of the domain is not a cheap
//! same-volume rename. The File Provider daemon processes the whole subtree,
//! so 2,000 and 10,000 files took 4 s, 30,000 files took 6 min 25 s, and the
//! real vault (29,055 files) ran 8 min 51 s and then failed with ESTALE (os
//! error 70). An APFS clone of the same 30,000-file tree took 4 to 18 s and
//! removing the original 1.4 s. Renaming or cloning an iCloud-evicted
//! (dataless) file out of the domain blocks while it downloads, one file at a
//! time (3 MB took 41 to 53 s); about 12 small files a second download with
//! 128 reader threads, 14 to 26 MB/s per large file.
//!
//! So a folder in iCloud moves by **clone, verify, rename into place, remove
//! the original last**, and its evicted files are downloaded first, in
//! parallel, by `Hollow move-home --download` (or by the move's own
//! preflight refusing until none are left). This module holds the file-system
//! pieces; `move_home_run` journals and undoes them.

use crate::move_home::{Probes, SF_DATALESS};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `clonefile(2)` flag: do not follow a symlink at the source.
#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: u32 = 0x0001;

/// Small evicted files that download per second with [`READ_THREADS`]
/// readers (measured 11.5 to 12.5 with 128 and 256 threads).
pub const FILES_PER_SEC: f64 = 12.0;
/// Download speed of one large evicted file (measured 14 to 26 MB/s; the
/// low end, so the estimate errs long).
pub const BYTES_PER_SEC: f64 = 14.0 * 1024.0 * 1024.0;
/// Reader threads for the download.
pub const READ_THREADS: usize = 128;
/// Clones per second used to say how long the copy takes (measured on real
/// iCloud scratch folders: 30,000 files in 3.6 s, 100,000 in 12.4 s, 30,000
/// in 18 s once with a big upload backlog; walks and removal are faster).
pub const CLONE_FILES_PER_SEC: f64 = 4000.0;

/// Roughly how long downloading `files` evicted files of `bytes` takes.
pub fn download_secs(files: u64, bytes: u64) -> u64 {
    (files as f64 / FILES_PER_SEC + bytes as f64 / BYTES_PER_SEC).ceil() as u64
}

/// Roughly how long cloning, verifying and removing a folder of `files`
/// takes (a clone, two stat walks and the removal).
pub fn copy_secs(files: u64) -> u64 {
    (files as f64 / CLONE_FILES_PER_SEC * 1.5).ceil() as u64 + 10
}

/// "3 h 5 min", "12 min", "40 s".
pub fn duration_text(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else if secs < 90 * 60 {
        format!("{} min", secs.div_ceil(60))
    } else {
        let m = secs.div_ceil(60);
        format!("{} h {} min", m / 60, m % 60)
    }
}

/// Special files (sockets, pipes, devices) known to be throwaway runtime
/// files, by name: whatever made them makes them again. They are left out
/// of a copy and of its comparison, and never touched in the source. Run 1
/// of 11-13 stopped on git's fsmonitor socket, which `cp` can't copy.
/// - `fsmonitor--daemon.ipc`: git's file-system monitor daemon socket in a
///   repository's `.git` folder (`git fsmonitor--daemon` makes it again).
pub(crate) const THROWAWAY_SPECIAL_FILES: &[&str] = &["fsmonitor--daemon.ipc"];

/// The special kind of a file type: socket, pipe or device; `None` for a
/// file, folder or link.
pub(crate) fn special_kind(ft: &std::fs::FileType) -> Option<&'static str> {
    use std::os::unix::fs::FileTypeExt;
    if ft.is_socket() {
        Some("socket")
    } else if ft.is_fifo() {
        Some("pipe")
    } else if ft.is_block_device() {
        Some("block device")
    } else if ft.is_char_device() {
        Some("character device")
    } else {
        None
    }
}

/// Whether a special file with this name is a known throwaway one.
pub(crate) fn is_throwaway_special(name: &std::ffi::OsStr) -> bool {
    THROWAWAY_SPECIAL_FILES
        .iter()
        .any(|n| name == std::ffi::OsStr::new(n))
}

/// The special files under a folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SpecialFiles {
    /// Known throwaway ones (left out of the copy and the comparison).
    pub throwaway: Vec<PathBuf>,
    /// Any other: (path, kind). A copy stops on these.
    pub unexpected: Vec<(PathBuf, &'static str)>,
}

impl SpecialFiles {
    /// The stop message for the first unexpected one, if any.
    pub(crate) fn refusal(&self) -> Option<String> {
        let (p, kind) = self.unexpected.first()?;
        let more = match self.unexpected.len() {
            1 => String::new(),
            n => format!(" ({} more special files after it)", n - 1),
        };
        Some(format!(
            "{} is a {kind}, a special file a copy can't carry{more}. Move or remove it (after closing what made it), then check the move again",
            p.display()
        ))
    }
}

/// Every socket, pipe and device file under `root` (links never followed).
/// A folder that can't be listed is an error: what it holds is unknown.
pub(crate) fn special_files(root: &Path) -> Result<SpecialFiles, String> {
    let mut out = SpecialFiles::default();
    let meta =
        std::fs::symlink_metadata(root).map_err(|e| format!("stat {}: {e}", root.display()))?;
    if let Some(kind) = special_kind(&meta.file_type()) {
        let name = root.file_name().unwrap_or_default();
        if is_throwaway_special(name) {
            out.throwaway.push(root.to_path_buf());
        } else {
            out.unexpected.push((root.to_path_buf(), kind));
        }
        return Ok(out);
    }
    if !meta.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = std::fs::read_dir(&d).map_err(|e| format!("list {}: {e}", d.display()))?;
        for e in rd {
            let e = e.map_err(|e| format!("list {}: {e}", d.display()))?;
            let ft = e
                .file_type()
                .map_err(|x| format!("stat {}: {x}", e.path().display()))?;
            if ft.is_dir() {
                stack.push(e.path());
            } else if let Some(kind) = special_kind(&ft) {
                if is_throwaway_special(&e.file_name()) {
                    out.throwaway.push(e.path());
                } else {
                    out.unexpected.push((e.path(), kind));
                }
            }
        }
    }
    out.throwaway.sort();
    out.unexpected.sort();
    Ok(out)
}

/// Remove the known throwaway special files from a copy (never the
/// source). Returns how many.
pub(crate) fn drop_throwaway_specials(copy: &Path) -> Result<u64, String> {
    let found = special_files(copy)?;
    for p in &found.throwaway {
        std::fs::remove_file(p).map_err(|e| format!("remove {}: {e}", p.display()))?;
    }
    Ok(found.throwaway.len() as u64)
}

fn path_c(p: &Path) -> Option<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).ok()
}

/// Clone the folder `from` to the new path `to` in one `clonefile(2)` call
/// (modes, times, extended attributes and symlinks come along; hard links
/// become separate files). When that is refused, `cp -cRp`. `to` must not
/// exist; on failure it is removed. Returns which one did it.
pub(crate) fn clone_tree(from: &Path, to: &Path) -> Result<&'static str, String> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(format!("{} already exists", to.display()));
    }
    let mut why = String::new();
    #[cfg(target_os = "macos")]
    {
        if let (Some(a), Some(b)) = (path_c(from), path_c(to)) {
            // SAFETY: two valid NUL-terminated paths.
            if unsafe { libc::clonefile(a.as_ptr(), b.as_ptr(), CLONE_NOFOLLOW) } == 0 {
                return Ok("clone");
            }
            why = format!("clonefile: {}", std::io::Error::last_os_error());
        }
    }
    let _ = remove_tree(to);
    let out = std::process::Command::new("/bin/cp")
        .args(["-cRp"])
        .arg(from)
        .arg(to)
        .output()
        .map_err(|e| format!("cp: {e}"))?;
    if out.status.success() {
        return Ok("copy");
    }
    let _ = remove_tree(to);
    Err(format!(
        "copy {} to {}: {why}; cp: {}",
        from.display(),
        to.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// Remove a folder tree (never following links), making read-only folders
/// writable when that is what stops it.
pub(crate) fn remove_tree(p: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(p) {
        Err(_) => return Ok(()),
        Ok(m) if !m.is_dir() => {
            return std::fs::remove_file(p).map_err(|e| format!("remove {}: {e}", p.display()))
        }
        Ok(_) => {}
    }
    if std::fs::remove_dir_all(p).is_ok() {
        return Ok(());
    }
    open_up(p);
    std::fs::remove_dir_all(p).map_err(|e| format!("remove {}: {e}", p.display()))
}

fn open_up(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(e.path());
                }
            }
        }
    }
}

/// What a verified tree holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TreeStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Dir(u32),
    File(u64, u32, u128),
    Link(PathBuf),
    Other,
}

fn entries(dir: &Path) -> Result<BTreeMap<std::ffi::OsString, Kind>, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mut out = BTreeMap::new();
    let rd = std::fs::read_dir(dir).map_err(|e| format!("list {}: {e}", dir.display()))?;
    for e in rd {
        let e = e.map_err(|e| format!("list {}: {e}", dir.display()))?;
        let m = std::fs::symlink_metadata(e.path())
            .map_err(|e2| format!("stat {}: {e2}", e.path().display()))?;
        let ft = m.file_type();
        // A known throwaway special file (git's fsmonitor socket) is left
        // out of every comparison: the copy drops it, the source keeps it.
        if special_kind(&ft).is_some() && is_throwaway_special(&e.file_name()) {
            continue;
        }
        let kind = if ft.is_symlink() {
            Kind::Link(std::fs::read_link(e.path()).unwrap_or_default())
        } else if ft.is_dir() {
            Kind::Dir(m.permissions().mode() & 0o7777)
        } else if ft.is_file() {
            Kind::File(
                m.len(),
                m.permissions().mode() & 0o7777,
                m.mtime() as u128 * 1_000_000_000 + m.mtime_nsec() as u128,
            )
        } else {
            Kind::Other
        };
        out.insert(e.file_name(), kind);
    }
    Ok(out)
}

/// Compare two trees entry by entry: names, kinds, file sizes, modes and
/// modification times, symlink targets. With `subset`, every entry of `a`
/// must be in `b` the same (`a` is what a partly removed original still
/// holds); without it the two must be identical. Folder times are not
/// compared (a clone sets them when it makes the folder). Returns what `a`
/// holds, or the first difference.
pub(crate) fn compare_trees(a: &Path, b: &Path, subset: bool) -> Result<TreeStats, String> {
    let mut st = TreeStats::default();
    let mut stack = vec![(a.to_path_buf(), b.to_path_buf())];
    while let Some((da, db)) = stack.pop() {
        let ea = entries(&da)?;
        let eb = entries(&db)?;
        for (name, ka) in &ea {
            let Some(kb) = eb.get(name) else {
                return Err(format!(
                    "{} is missing from {}",
                    name.to_string_lossy(),
                    db.display()
                ));
            };
            match (ka, kb) {
                (Kind::Dir(ma), Kind::Dir(mb)) if ma == mb => {
                    st.dirs += 1;
                    stack.push((da.join(name), db.join(name)));
                }
                (Kind::File(sa, ma, ta), Kind::File(sb, mb, tb))
                    if sa == sb && ma == mb && ta == tb =>
                {
                    st.files += 1;
                    st.bytes += sa;
                }
                (Kind::Link(x), Kind::Link(y)) if x == y => st.links += 1,
                (Kind::Other, Kind::Other) => {}
                _ => {
                    return Err(format!(
                        "{} differs from {}",
                        da.join(name).display(),
                        db.join(name).display()
                    ))
                }
            }
        }
        if !subset {
            if let Some(extra) = eb.keys().find(|n| !ea.contains_key(*n)) {
                return Err(format!(
                    "{} has an extra entry {}",
                    db.display(),
                    extra.to_string_lossy()
                ));
            }
        }
    }
    Ok(st)
}

/// What a tree holds right now (files, folders, links, bytes), or why it
/// can't be read.
pub(crate) fn tree_stats(p: &Path) -> Result<TreeStats, String> {
    compare_trees(p, p, true)
}

/// Whether every entry of `a` (a partly removed original) is in `b` with
/// the same name and kind (file, folder, link). Sizes and times are not
/// compared: by the time a rollback asks, files the run rewrote differ in
/// the copy. This is the guard before the rollback deletes `a`: it must be
/// what a retire left, not something else that appeared at that path.
pub(crate) fn names_subset(a: &Path, b: &Path) -> Result<(), String> {
    let class = |k: &Kind| match k {
        Kind::Dir(_) => 0,
        Kind::File(..) => 1,
        Kind::Link(_) => 2,
        Kind::Other => 3,
    };
    let mut stack = vec![(a.to_path_buf(), b.to_path_buf())];
    while let Some((da, db)) = stack.pop() {
        let ea = entries(&da)?;
        let eb = entries(&db)?;
        for (name, ka) in &ea {
            match eb.get(name) {
                Some(kb) if class(ka) == class(kb) => {
                    if matches!(ka, Kind::Dir(_)) {
                        stack.push((da.join(name), db.join(name)));
                    }
                }
                _ => {
                    return Err(format!(
                        "{} is not in {}",
                        da.join(name).display(),
                        db.display()
                    ))
                }
            }
        }
    }
    Ok(())
}

/// The first file under `root` modified after `since` (nanoseconds since the
/// epoch): something wrote there after the copy was made.
///
/// A folder that can't be listed counts as written: its contents were never
/// looked at, and a retire must not delete what it didn't check.
pub(crate) fn modified_since(root: &Path, since: u128) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            return Some(d);
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if m.is_dir() {
                stack.push(p);
            } else if m.is_file()
                && m.mtime() as u128 * 1_000_000_000 + m.mtime_nsec() as u128 > since
            {
                return Some(p);
            }
        }
    }
    None
}

/// What is evicted (dataless) under a folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Evicted {
    pub files: u64,
    pub bytes: u64,
    /// Evicted folders (listing one downloads its entries' names).
    pub dirs: u64,
}

/// Count the evicted files and folders under `root` with `lstat` only (no
/// listing of an evicted folder, no read of an evicted file).
pub fn count_evicted(root: &Path, probes: &dyn Probes) -> Evicted {
    let mut ev = Evicted::default();
    walk_evicted(root, probes, &mut |_, size, is_dir| {
        if is_dir {
            ev.dirs += 1;
        } else {
            ev.files += 1;
            ev.bytes += size;
        }
    });
    ev
}

/// Visit every evicted entry under `root` with `lstat` only. An evicted
/// folder is reported and never listed (listing it downloads it).
fn walk_evicted(root: &Path, probes: &dyn Probes, f: &mut dyn FnMut(&Path, u64, bool)) {
    let Ok(m) = std::fs::symlink_metadata(root) else {
        return;
    };
    if !m.is_dir() {
        if m.file_type().is_file() && probes.flags(root, &m) & SF_DATALESS != 0 {
            f(root, m.len(), false);
        }
        return;
    }
    if probes.flags(root, &m) & SF_DATALESS != 0 {
        f(root, 0, true);
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            let ft = m.file_type();
            if ft.is_symlink() {
                continue;
            }
            let dataless = probes.flags(&p, &m) & SF_DATALESS != 0;
            if ft.is_dir() {
                if dataless {
                    f(&p, 0, true);
                } else {
                    stack.push(p);
                }
            } else if ft.is_file() && dataless {
                f(&p, m.len(), false);
            }
        }
    }
}

/// A progress line while downloading.
#[derive(Debug, Clone)]
pub struct Progress {
    pub files_done: u64,
    pub bytes_done: u64,
    pub failed: u64,
    pub elapsed: Duration,
}

/// What a download pass did.
#[derive(Debug, Default, Clone)]
pub struct Downloaded {
    pub files: u64,
    pub bytes: u64,
    pub failed: Vec<String>,
    /// Evicted folders listed (which downloads their names).
    pub dirs_opened: u64,
    pub stopped: bool,
}

/// Download every evicted file under `roots` by reading it end to end with
/// [`READ_THREADS`] threads at once. An evicted folder is listed so its
/// entries are found; files that are already local are skipped, so a rerun
/// resumes. `on_progress` is called about every 5 s from the walking thread.
/// Setting `stop` ends the pass at the next file.
pub fn download_evicted(
    roots: &[PathBuf],
    probes: &dyn Probes,
    threads: usize,
    stop: &AtomicBool,
    on_progress: &(dyn Fn(&Progress) + Sync),
) -> Downloaded {
    let started = Instant::now();
    let done = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(Mutex::new(Vec::<String>::new()));
    let (tx, rx) = std::sync::mpsc::sync_channel::<PathBuf>(threads.max(1) * 4);
    let rx = Arc::new(Mutex::new(rx));
    let mut out = Downloaded::default();
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            let rx = Arc::clone(&rx);
            let (done, bytes, failed) =
                (Arc::clone(&done), Arc::clone(&bytes), Arc::clone(&failed));
            scope.spawn(move || {
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let next = rx.lock().ok().and_then(|r| r.recv().ok());
                    let Some(path) = next else { break };
                    match std::fs::File::open(&path) {
                        Ok(mut f) => {
                            let mut n = 0u64;
                            let r = loop {
                                match f.read(&mut buf) {
                                    Ok(0) => break Ok(()),
                                    Ok(k) => n += k as u64,
                                    Err(e) => break Err(e),
                                }
                            };
                            match r {
                                Ok(()) => {
                                    done.fetch_add(1, Ordering::Relaxed);
                                    bytes.fetch_add(n, Ordering::Relaxed);
                                }
                                Err(e) => {
                                    if let Ok(mut v) = failed.lock() {
                                        v.push(format!("{}: {e}", path.display()));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if let Ok(mut v) = failed.lock() {
                                v.push(format!("{}: {e}", path.display()));
                            }
                        }
                    }
                }
            });
        }
        // Progress comes from its own thread, so it keeps printing while the
        // walker waits in a listing or the readers wait on iCloud.
        let finished = Arc::new(AtomicBool::new(false));
        {
            let (done, bytes, failed, finished) = (
                Arc::clone(&done),
                Arc::clone(&bytes),
                Arc::clone(&failed),
                Arc::clone(&finished),
            );
            scope.spawn(move || {
                let mut last = Instant::now();
                while !finished.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(200));
                    if last.elapsed() >= Duration::from_secs(5) {
                        last = Instant::now();
                        on_progress(&Progress {
                            files_done: done.load(Ordering::Relaxed),
                            bytes_done: bytes.load(Ordering::Relaxed),
                            failed: failed.lock().map(|v| v.len() as u64).unwrap_or(0),
                            elapsed: started.elapsed(),
                        });
                    }
                }
            });
        }
        let mut stack: Vec<PathBuf> = roots.to_vec();
        'walk: while let Some(p) = stack.pop() {
            if stop.load(Ordering::Relaxed) {
                out.stopped = true;
                break;
            }
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            let ft = m.file_type();
            if ft.is_symlink() {
                continue;
            }
            if ft.is_file() {
                if probes.flags(&p, &m) & SF_DATALESS != 0 && tx.send(p).is_err() {
                    break 'walk;
                }
            } else if ft.is_dir() {
                if probes.flags(&p, &m) & SF_DATALESS != 0 {
                    out.dirs_opened += 1;
                }
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            }
        }
        drop(tx);
        // The readers drain what is queued; the reporter stops once they end.
        while Arc::strong_count(&rx) > 1 {
            std::thread::sleep(Duration::from_millis(100));
        }
        finished.store(true, Ordering::Relaxed);
    });
    out.files = done.load(Ordering::Relaxed);
    out.bytes = bytes.load(Ordering::Relaxed);
    out.failed = failed.lock().map(|v| v.clone()).unwrap_or_default();
    on_progress(&Progress {
        files_done: out.files,
        bytes_done: out.bytes,
        failed: out.failed.len() as u64,
        elapsed: started.elapsed(),
    });
    out
}

/// The probe's stand-in for a File Provider rename test: a small tree with a
/// nested folder, a symlink and an extended attribute, cloned out of `src`
/// into `dest_dir`, compared, removed there, and removed from `src`. This is
/// the same operation the move does to a whole folder.
pub(crate) fn clone_probe(src: &Path, dest_dir: &Path, tag: &str) -> Result<(), String> {
    let name = format!(".hollow-move-probe-{tag}");
    let a = src.join(&name);
    let b = dest_dir.join(&name);
    let _ = remove_tree(&a);
    let _ = remove_tree(&b);
    let result = (|| -> Result<(), String> {
        std::fs::create_dir(&a)
            .map_err(|e| format!("Couldn't write a probe folder in {}: {e}", src.display()))?;
        std::fs::create_dir(a.join("nested")).map_err(|e| e.to_string())?;
        std::fs::write(a.join("nested").join("file.txt"), b"probe").map_err(|e| e.to_string())?;
        std::fs::write(a.join("top.txt"), vec![7u8; 4096]).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink("top.txt", a.join("link")).map_err(|e| e.to_string())?;
        clone_tree(&a, &b)?;
        compare_trees(&a, &b, false)?;
        remove_tree(&b)?;
        Ok(())
    })();
    let _ = remove_tree(&a);
    let _ = remove_tree(&b);
    result
}
