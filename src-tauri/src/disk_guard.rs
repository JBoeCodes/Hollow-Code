//! Disk guard (overnight D1): free disk fell from 259 GB to ~94 GB in a day
//! of builds. Hollow says so before a build fails for it. Read-only: it
//! never deletes; the cleanups that do (A5's `build.prune_install_targets`,
//! A9's `build.clean_worktrees`, and the Disk space panel's buttons for
//! them, overnight H20) each ask first.
//!
//! **What it checks.** Free space on the home volume (`statvfs`, what a
//! non-root process can use), and on the vault's when it is another volume
//! (an external drive), against the threshold: Settings, Advanced, Disk
//! space's "Warn under" ([`threshold_gb`], app data `disk-guard.json`;
//! [`DEFAULT_THRESHOLD_GB`] until set; morning N6). When either is
//! under, it sizes Hollow's own folders with `du` (within [`SIZE_BUDGET`];
//! one it couldn't size in time says so):
//! - `~/Library/Caches/hollow*`: cargo targets (installs, overnight lanes);
//! - each build worktree, `<worktrees root>/<repo>/<worktree>` (the root is
//!   `build_workspaces::worktrees_root` of the configured `hollowHome`);
//! - `~/.cache/<hollow>`: debug logs.
//!
//! **The bell.** The minute timer ([`check_daily`]) reads free space each
//! tick (cheap) and, when it is low, warns once a local day, naming the
//! largest folders and the two cleanups, with a link to Settings, Advanced's
//! Disk space. The read action `system.disk_guard` and that section give the
//! same report any time.
//!
//! **Why targets grow.** On macOS a dev build keeps its debug info in
//! `debug/deps/*.rcgu.o` ("unpacked") and old ones are never collected: an
//! overnight lane's target reached 142 GB, 136 GB of it such objects.
//! Deleting a target only costs a cold next build.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_THRESHOLD_GB: u64 = 50;
/// The threshold Settings accepts, in GB.
pub const THRESHOLD_RANGE: std::ops::RangeInclusive<u64> = 1..=10_000;
const SIZE_BUDGET: Duration = Duration::from_secs(20);
const LISTED: usize = 10;
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One Hollow-owned folder and its size.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub path: String,
    /// `cargo-target` | `worktree` | `cache`.
    pub kind: String,
    /// `None`: not sized (out of time, or `du` failed).
    pub bytes: Option<u64>,
}

/// The vault's volume, when it isn't the home volume.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub path: String,
    pub free_bytes: Option<u64>,
    pub low: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The home volume's free space.
    pub free_bytes: Option<u64>,
    pub threshold_gb: u64,
    /// Home or the vault's volume is under the threshold.
    pub low: bool,
    /// The vault's volume, only when it differs from home's.
    pub vault: Option<Volume>,
    /// Largest first; sized only when asked (`sized`), else unsized.
    pub folders: Vec<Folder>,
    /// What frees space, each asking first.
    pub cleanup: Vec<String>,
}

/// Bytes a non-root process can still write on `path`'s volume.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path; `s` is a zeroed out-param.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    (rc == 0).then(|| (s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
}

/// What identifies the free space `path` draws on: its volume's total block
/// count and block size, which don't move while a clean runs (the available
/// count does, so it is never part of the key). Volumes of one APFS container report
/// the container's total, so folders with the same key are one pool.
pub(crate) fn volume_key(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path; `s` is a zeroed out-param.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    (rc == 0).then_some((s.f_blocks as u64, s.f_frsize as u64))
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    threshold_gb: Option<u64>,
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("disk-guard.json"))
}

/// The threshold Hollow warns under, in GB: Settings' "Warn under", else
/// [`DEFAULT_THRESHOLD_GB`] (also when the file is unreadable).
pub fn threshold_gb() -> u64 {
    settings_path()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .and_then(|s| s.threshold_gb)
        .filter(|g| THRESHOLD_RANGE.contains(g))
        .unwrap_or(DEFAULT_THRESHOLD_GB)
}

/// Save the threshold (1–10,000 GB). Returns it.
pub fn set_threshold_gb(gb: u64) -> Result<u64, String> {
    if !THRESHOLD_RANGE.contains(&gb) {
        return Err(format!(
            "Warn under 1 to 10,000 GB, not {gb}. Nothing changed."
        ));
    }
    let path = settings_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("settings folder: {e}"))?;
    }
    let text = serde_json::to_vec_pretty(&Settings {
        threshold_gb: Some(gb),
    })
    .map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, &text)?;
    Ok(gb)
}

/// `path`'s volume (its device), for telling the vault's from home's.
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

/// The vault's volume when it isn't `home`'s, against `threshold_gb`.
pub fn vault_volume(home: &Path, vault: Option<&Path>, threshold_gb: u64) -> Option<Volume> {
    let vault = vault?;
    let (h, v) = (device(home)?, device(vault)?);
    (h != v).then(|| {
        let free = free_bytes(vault);
        Volume {
            path: vault.to_string_lossy().to_string(),
            free_bytes: free,
            low: under(free, threshold_gb),
        }
    })
}

fn under(free: Option<u64>, threshold_gb: u64) -> bool {
    free.is_some_and(|b| (b as f64) < threshold_gb as f64 * GB)
}

/// The build worktrees' root for the config at `config_path` (its
/// `hollowHome`, else the instance's home folder).
pub fn worktrees_root(config_path: &Path) -> Option<PathBuf> {
    let home = crate::setup::load_config_map(config_path)
        .ok()
        .and_then(|m| {
            m.get("hollowHome")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    crate::build_workspaces::worktrees_root(home.as_deref()).ok()
}

/// Hollow's own folders: caches under `home`, and each worktree under
/// `worktrees` (`<repo>/<worktree>`, the unit A9's cleanup removes); unsized.
pub fn candidates(home: &Path, worktrees: Option<&Path>) -> Vec<Folder> {
    let mut out = Vec::new();
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    };
    for p in dirs(&home.join("Library/Caches")) {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.starts_with("hollow") {
            let kind = if p.join("CACHEDIR.TAG").is_file() || p.join("debug").is_dir() {
                "cargo-target"
            } else {
                "cache"
            };
            out.push(Folder {
                path: p.to_string_lossy().to_string(),
                kind: kind.into(),
                bytes: None,
            });
        }
    }
    for repo in worktrees.map(dirs).unwrap_or_default() {
        for p in dirs(&repo) {
            out.push(Folder {
                path: p.to_string_lossy().to_string(),
                kind: "worktree".into(),
                bytes: None,
            });
        }
    }
    let dot = crate::instance::dot_cache_dir(home);
    if dot.is_dir() {
        out.push(Folder {
            path: dot.to_string_lossy().to_string(),
            kind: "cache".into(),
            bytes: None,
        });
    }
    out
}

/// `du -sk`, within `limit`.
fn du_bytes(path: &Path, limit: Duration) -> Option<u64> {
    if limit.is_zero() {
        return None;
    }
    let mut child = std::process::Command::new("/usr/bin/du")
        .arg("-sk")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = reader.join().ok()?;
    let kb: u64 = text.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

/// Size `folders` in order within one shared budget, then largest first.
pub fn sized(mut folders: Vec<Folder>, budget: Duration) -> Vec<Folder> {
    let started = Instant::now();
    for f in &mut folders {
        let left = budget.saturating_sub(started.elapsed());
        f.bytes = du_bytes(Path::new(&f.path), left);
    }
    folders.sort_by_key(|f| std::cmp::Reverse(f.bytes.unwrap_or(0)));
    folders
}

const CLEANUP: [&str; 2] = [
    "Install build folders: ask an agent for build.install_targets, then build.prune_install_targets (asks first; keeps the installed and rollback builds).",
    "Worktrees: ask an agent for build.worktree_hygiene, then build.clean_worktrees (asks first; only merged, clean, pushed ones).",
];

/// The report for `home` (and the vault's volume when it is another one);
/// folders sized when `size` (slow: `du`).
pub fn report(
    home: &Path,
    vault: Option<&Path>,
    worktrees: Option<&Path>,
    threshold_gb: u64,
    size: bool,
) -> Report {
    let free = free_bytes(home);
    let vault = vault_volume(home, vault, threshold_gb);
    let low = under(free, threshold_gb) || vault.as_ref().is_some_and(|v| v.low);
    let mut folders = candidates(home, worktrees);
    if size {
        folders = sized(folders, SIZE_BUDGET);
    }
    folders.truncate(LISTED);
    Report {
        free_bytes: free,
        threshold_gb,
        low,
        vault,
        folders,
        cleanup: CLEANUP.iter().map(|s| s.to_string()).collect(),
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.0} GB", bytes as f64 / GB)
}

/// The bell's title and body.
pub fn bell_text(r: &Report) -> (String, String) {
    let free = r.free_bytes.map(gb).unwrap_or_else(|| "?".into());
    let title = match &r.vault {
        Some(v) if v.low && !under(r.free_bytes, r.threshold_gb) => format!(
            "Disk space is low on the vault's volume: {} free",
            v.free_bytes.map(gb).unwrap_or_else(|| "?".into())
        ),
        Some(v) if v.low => format!(
            "Disk space is low: {free} free, and {} on the vault's volume",
            v.free_bytes.map(gb).unwrap_or_else(|| "?".into())
        ),
        _ => format!("Disk space is low: {free} free"),
    };
    let top: Vec<String> = r
        .folders
        .iter()
        .filter_map(|f| f.bytes.map(|b| (f, b)))
        .take(3)
        .map(|(f, b)| format!("{} ({})", f.path, gb(b)))
        .collect();
    let mut body = format!("Under the {} GB Hollow warns at. ", r.threshold_gb);
    if !top.is_empty() {
        body.push_str(&format!("Largest Hollow folders: {}. ", top.join(", ")));
    }
    body.push_str("Nothing was deleted. Settings → Advanced → Disk space lists them and the two cleanups (each asks first).");
    (title, body)
}

/// The day this process last warned: a disk too full to write `jobs.json`
/// must not make every tick run `du` again.
static WARNED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The minute timer's check: warn once a local day while space is low.
pub async fn check_daily(app: &impl crate::core_host::CoreHost, vault: &Path, config_path: &Path) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let today = chrono::Local::now().date_naive().to_string();
    // Already warned today: nothing to read.
    if WARNED.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(today.as_str()) {
        return;
    }
    let threshold = threshold_gb();
    // Cheap each tick: the small settings file, `statvfs` of home, and two
    // `stat`s plus a `statvfs` for the vault only when it is another volume.
    let low = under(free_bytes(&home), threshold)
        || vault_volume(&home, Some(vault), threshold).is_some_and(|v| v.low);
    if !low {
        return;
    }
    {
        let mut warned = WARNED.lock().unwrap_or_else(|e| e.into_inner());
        if warned.as_deref() == Some(today.as_str())
            || crate::schedule_windows::load_jobs()
                .last
                .get("disk-guard")
                .map(String::as_str)
                == Some(today.as_str())
        {
            return;
        }
        // Marked first, so a slow `du` can't let the next tick warn again.
        *warned = Some(today.clone());
    }
    crate::schedule_windows::stamp_job("disk-guard", &today, chrono::Utc::now());
    let worktrees = worktrees_root(config_path);
    let vault_path = vault.to_path_buf();
    let Ok(r) = tauri::async_runtime::spawn_blocking(move || {
        report(
            &home,
            Some(&vault_path),
            worktrees.as_deref(),
            threshold,
            true,
        )
    })
    .await
    else {
        return;
    };
    let (title, body) = bell_text(&r);
    let _ = crate::notifications::create(
        app,
        vault,
        crate::notifications::NotificationInput {
            title,
            body,
            kind: "warning".into(),
            source: Some("disk-guard".into()),
            action_label: Some("Open".into()),
            action_url: Some("settings:system".into()),
            notify_native: true,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_is_read_and_compared_to_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_bytes(dir.path()).expect("statvfs reads a real volume");
        assert!(free > 0);
        assert!(
            !report(dir.path(), None, None, 0, false).low,
            "nothing is under 0 GB"
        );
        assert!(
            report(dir.path(), None, None, 1_000_000_000, false).low,
            "everything is under an exabyte"
        );
        assert_eq!(free_bytes(Path::new("/no/such/volume/at/all")), None);
    }

    #[test]
    fn only_hollow_folders_are_listed_and_sized_largest_first() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let target = h.join("Library/Caches/hollow-overnight-b");
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.join("CACHEDIR.TAG"), "x").unwrap();
        std::fs::write(target.join("debug/deps/a.rcgu.o"), vec![0u8; 300_000]).unwrap();
        std::fs::create_dir_all(h.join("Library/Caches/com.apple.Safari")).unwrap();
        std::fs::create_dir_all(h.join("Library/Caches/HollowTemp")).unwrap();
        let wt = h.join("Hollow/Worktrees/hollow-repo/release-1");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("f"), vec![0u8; 20_000]).unwrap();
        let listed = candidates(h, Some(&h.join("Hollow/Worktrees")));
        let kinds: Vec<(&str, &str)> = listed
            .iter()
            .map(|f| (f.path.rsplit('/').next().unwrap(), f.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("HollowTemp", "cache"),
                ("hollow-overnight-b", "cargo-target"),
                ("release-1", "worktree")
            ],
            "Safari's cache is not Hollow's"
        );
        let s = sized(listed, Duration::from_secs(10));
        assert!(s[0].path.ends_with("hollow-overnight-b"), "{s:?}");
        assert!(s[0].bytes.unwrap() >= 300_000);
        // Out of time: unsized, not a guess.
        let none = sized(candidates(h, None), Duration::ZERO);
        assert!(none.iter().all(|f| f.bytes.is_none()));
    }

    #[test]
    fn the_bell_names_free_space_the_largest_folders_and_that_nothing_was_deleted() {
        let r = Report {
            free_bytes: Some(40 * 1024 * 1024 * 1024),
            threshold_gb: 50,
            low: true,
            vault: None,
            folders: vec![
                Folder {
                    path: "/u/Library/Caches/hollow-overnight-b".into(),
                    kind: "cargo-target".into(),
                    bytes: Some(142 * 1024 * 1024 * 1024),
                },
                Folder {
                    path: "/u/Hollow/Worktrees/x".into(),
                    kind: "worktree".into(),
                    bytes: None,
                },
            ],
            cleanup: Vec::new(),
        };
        let (title, body) = bell_text(&r);
        assert_eq!(title, "Disk space is low: 40 GB free");
        assert!(body.contains("hollow-overnight-b (142 GB)"), "{body}");
        assert!(
            !body.contains("Worktrees/x"),
            "an unsized folder isn't named with a guess"
        );
        assert!(body.contains("Nothing was deleted"));
        assert!(body.contains("each asks first"));
        // The vault on another, fuller volume is named.
        let gb = 1024 * 1024 * 1024;
        let on_vault = Report {
            free_bytes: Some(400 * gb),
            low: true,
            vault: Some(Volume {
                path: "/Volumes/Ext/vault".into(),
                free_bytes: Some(9 * gb),
                low: true,
            }),
            ..r.clone()
        };
        assert_eq!(
            bell_text(&on_vault).0,
            "Disk space is low on the vault's volume: 9 GB free"
        );
        let both = Report {
            free_bytes: Some(40 * gb),
            ..on_vault.clone()
        };
        assert_eq!(
            bell_text(&both).0,
            "Disk space is low: 40 GB free, and 9 GB on the vault's volume"
        );
    }

    #[test]
    fn the_threshold_is_saved_in_range_and_the_vault_counts_only_on_another_volume() {
        assert!(set_threshold_gb(0).unwrap_err().contains("1 to 10,000 GB"));
        assert!(set_threshold_gb(10_001).is_err());
        assert_eq!(set_threshold_gb(80).unwrap(), 80);
        assert_eq!(threshold_gb(), 80);
        assert_eq!(
            set_threshold_gb(DEFAULT_THRESHOLD_GB).unwrap(),
            DEFAULT_THRESHOLD_GB
        );
        // A vault on the home volume isn't reported twice.
        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("vault");
        std::fs::create_dir_all(&inside).unwrap();
        assert_eq!(vault_volume(dir.path(), Some(&inside), 50), None);
        assert_eq!(vault_volume(dir.path(), None, 50), None);
        assert!(report(dir.path(), Some(&inside), None, 50, false)
            .vault
            .is_none());
    }
}
