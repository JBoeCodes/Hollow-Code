//! Per-process test folders left in `$TMPDIR` (overnight 10-12/13, S6). About
//! twenty modules give their tests a folder named after the test process
//! (`hollow-test-sessions-<pid>`, `hollow-agent-settings-tests-<pid>`, …):
//! shared by that process's tests while they run, so none can remove it, and
//! nothing did afterwards. They piled up at ~180 MB a run (24 GB in one lane's
//! TMPDIR on 10-11). [`sweep_in`] removes those of test processes that are
//! gone; one test runs it, so each `cargo test --lib` clears earlier runs'
//! leftovers and at most the last run's stay.
//!
//! Overnight 10-20 (A1): those modules now take their folder from
//! [`process_dir`], which sweeps on its first call in a process (so a
//! filtered `cargo test --lib <module>` clears leftovers too) and removes
//! every folder it handed out when the process exits. A run that ends
//! normally leaves nothing; one that is killed is swept by the next.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

/// The folders [`process_dir`] handed out in this process.
static HANDED_OUT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static FIRST_CALL: Once = Once::new();

/// `$TMPDIR/<prefix><pid>`: the folder this test process's tests share under
/// `prefix` (which must start with one of [`PREFIXES`]). The first call in a
/// process sweeps dead processes' folders ([`sweep_in`]) and registers an
/// exit hook that removes every folder handed out.
pub(crate) fn process_dir(prefix: &str) -> PathBuf {
    debug_assert!(
        PREFIXES.iter().any(|p| prefix.starts_with(p)),
        "{prefix} must start with one of test_temp::PREFIXES"
    );
    FIRST_CALL.call_once(|| {
        sweep_in(&std::env::temp_dir(), std::process::id(), &|pid| {
            crate::proc_info::pid_alive(pid as i32) != Some(false)
        });
        // SAFETY: registers a plain `extern "C"` function that captures nothing.
        unsafe { libc::atexit(remove_handed_out) };
    });
    let dir = std::env::temp_dir().join(format!("{prefix}{}", std::process::id()));
    let mut handed_out = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
    if !handed_out.contains(&dir) {
        handed_out.push(dir.clone());
    }
    dir
}

/// The exit hook: other test threads may still hold files, so every error is
/// ignored, and a held lock means skip (the next run's sweep removes it).
extern "C" fn remove_handed_out() {
    let Ok(handed_out) = HANDED_OUT.try_lock() else {
        return;
    };
    for dir in handed_out.iter() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The per-process test folder prefixes; the first all-digit `-` part after
/// the prefix is the owning test process. Every folder with one of these is
/// made only by tests. (The last six are removed by their test, so they are
/// left only when it panics.) Never add a prefix the app uses at run time,
/// e.g. `hollow-skill-test-`, `hollow-capture-`, `hollow-activation-`.
const PREFIXES: &[&str] = &[
    "hollow-test-",
    "hollow-sessionstore-vault-",
    "hollow-agent-settings-tests-",
    "hollow-doc-markers-",
    "hollow-build-slots-tests-",
    "hollow-run-mcp-test-",
    "hollow-img-test-",
    "hollow-preview-test-",
    "hollow-results-review-",
    "hollow-apps-test-",
    "hollow-bm-test-",
    "hollow-browser-dl-",
];

/// The test process that owns `name`, if it is one of ours.
fn owner(name: &str) -> Option<u32> {
    let rest = PREFIXES.iter().find_map(|p| name.strip_prefix(p))?;
    rest.split('-')
        .find(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))?
        .parse()
        .ok()
}

/// Remove the per-process test folders in `tmp` whose process isn't `own`
/// and isn't alive (`alive` false; unknown counts as alive). Never follows a
/// link. Returns what it removed.
pub(crate) fn sweep_in(tmp: &Path, own: u32, alive: &dyn Fn(u32) -> bool) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(tmp) else {
        return removed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(owner) else {
            continue;
        };
        if pid == own || alive(pid) {
            continue;
        }
        let path = entry.path();
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

#[test]
fn earlier_test_runs_leave_no_folders_behind() {
    // The real sweep, for this TMPDIR: earlier runs' folders go.
    sweep_in(&std::env::temp_dir(), std::process::id(), &|pid| {
        crate::proc_info::pid_alive(pid as i32) != Some(false)
    });

    let tmp = tempfile::tempdir().unwrap();
    let t = tmp.path();
    let dirs = [
        "hollow-test-sessions-111",
        "hollow-sessionstore-vault-111-legacy-ab12cd",
        "hollow-agent-settings-tests-111",
        "hollow-run-mcp-test-111",
        "hollow-browser-dl-111",
        "hollow-doc-markers-111",
        "hollow-test-sessions-222",
        "hollow-test-sessions-333",
        "hollow-test-leases",
        "hollow-operator-111",
        "other-111",
    ];
    for d in dirs {
        std::fs::create_dir_all(t.join(d).join("inside")).unwrap();
    }
    std::fs::write(t.join("hollow-test-wakes-111"), "a file, not a folder").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("keep"), "x").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), t.join("hollow-test-budgets-111")).unwrap();
    // 111 is gone, 222 is alive, 333 is this process.
    let mut removed: Vec<String> = sweep_in(t, 333, &|pid| pid == 222)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    removed.sort();
    assert_eq!(
        removed,
        [
            "hollow-agent-settings-tests-111",
            "hollow-browser-dl-111",
            "hollow-doc-markers-111",
            "hollow-run-mcp-test-111",
            "hollow-sessionstore-vault-111-legacy-ab12cd",
            "hollow-test-sessions-111",
        ]
    );
    for kept in [
        "hollow-test-sessions-222",
        "hollow-test-sessions-333",
        "hollow-test-leases",
        "hollow-operator-111",
        "other-111",
        "hollow-test-wakes-111",
    ] {
        assert!(t.join(kept).exists(), "{kept}");
    }
    assert!(
        elsewhere.path().join("keep").exists(),
        "a link is never followed"
    );
    assert_eq!(owner("hollow-test-sessions-93557"), Some(93557));
    assert_eq!(
        owner("hollow-sessionstore-vault-93557-with-write"),
        Some(93557)
    );
    assert_eq!(owner("hollow-test-leases"), None);
}

/// The child half of [`a_test_process_leaves_no_folders_at_exit`], run alone
/// (`--ignored`) with the parent's `TMPDIR`; a no-op otherwise.
#[test]
#[ignore = "a child-process helper, not a test; a_test_process_leaves_no_folders_at_exit runs it with HOLLOW_TEST_PROCESS_DIR_CHILD"]
fn process_dir_child() {
    if std::env::var_os("HOLLOW_TEST_PROCESS_DIR_CHILD").is_none() {
        return;
    }
    for prefix in ["hollow-test-child-", "hollow-doc-markers-"] {
        let dir = process_dir(prefix);
        std::fs::create_dir_all(dir.join("inside")).unwrap();
        std::fs::write(dir.join("inside/file"), "x").unwrap();
    }
    println!("MADE");
}

#[test]
fn a_test_process_leaves_no_folders_at_exit() {
    let tmp = tempfile::tempdir().unwrap();
    // A dead test process's folder: the first process_dir call sweeps it.
    // The pid is above macOS's pid limit (99,999), so it is never alive.
    let dead = 4_000_000;
    std::fs::create_dir_all(
        tmp.path()
            .join(format!("hollow-test-sessions-{dead}"))
            .join("inside"),
    )
    .unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "test_temp::process_dir_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TMPDIR", tmp.path())
        .env("HOLLOW_TEST_PROCESS_DIR_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("MADE"), "the child ran: {stdout}");
    let left: Vec<String> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("hollow-"))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}
