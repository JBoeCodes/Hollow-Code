//! The login shell's PATH, for the agent CLIs Hollow starts (B03).
//!
//! A Finder- or Dock-launched Hollow inherits launchd's bare PATH, so a tool
//! whose folder only `~/.zshrc` adds (`~/google-cloud-sdk/bin`, `~/.bun/bin`,
//! a Grok install) is invisible to every agent. [`start`] asks the user's
//! login shell for its PATH once, on its own thread at launch, and
//! [`crate::agent::cli_path`] folds the answer in once it has arrived. Until
//! then, and whenever the shell can't answer (it hangs, fails, or prints no
//! marker), agents get exactly the PATH they got before: Hollow never waits on
//! the shell.
//!
//! [`merge`] builds that PATH folder by folder, each tagged with where it came
//! from; [`crate::agent::cli_path`] joins it, and Settings, Advanced's "Folders
//! agents can run tools from" ([`agent_tool_path`], A12) lists the same
//! entries, so the two can't drift.

use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Printed before the PATH, on its own line, so rc-file noise can't be
/// mistaken for it.
const MARKER: &str = "__HOLLOW_PATH__=";

/// Longest the login shell may take to answer.
const READ_LIMIT: Duration = Duration::from_secs(5);

/// The login shell's PATH folders, or why they couldn't be read. Unset until
/// the read finishes.
static SHELL_PATH: OnceLock<Result<Vec<String>, String>> = OnceLock::new();

/// Read the login shell's PATH on a background thread (never the main one).
/// Once per process; later calls do nothing.
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("shell-path".into())
        .spawn(|| {
            let shell = login_shell();
            let home = dirs::home_dir().unwrap_or_default();
            let started = Instant::now();
            let result = read_shell_path(&shell, &home, READ_LIMIT);
            // `elog`, never `eprintln!`: with stderr a pipe nobody reads any
            // more, `eprintln!` panics and a release build aborts (the
            // 2026-11-11 crashes, `launch.rs`).
            match &result {
                Ok(dirs) => crate::launch::elog(&format!(
                    "[shell-path] read {} folders from {} in {} ms",
                    dirs.len(),
                    shell.display(),
                    started.elapsed().as_millis()
                )),
                Err(why) => crate::launch::elog(&format!(
                    "[shell-path] couldn't read PATH from {}: {why}; agents keep Hollow's own PATH",
                    shell.display()
                )),
            }
            let _ = SHELL_PATH.set(result);
        });
    if let Err(error) = spawned {
        crate::launch::elog(&format!(
            "[shell-path] couldn't start the reader thread: {error}"
        ));
    }
}

/// The login shell's PATH folders, once read. `None` while the read is still
/// running or when it failed.
pub(crate) fn folders() -> Option<&'static [String]> {
    shell_folders(SHELL_PATH.get())
}

/// The shell's folders from the cell's value: `None` while the read is still
/// running (the cell is unset) or when it failed.
fn shell_folders(read: Option<&Result<Vec<String>, String>>) -> Option<&[String]> {
    match read {
        Some(Ok(dirs)) => Some(dirs.as_slice()),
        _ => None,
    }
}

/// Where a folder in the agents' PATH came from (A12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ToolPathFrom {
    /// Hollow's own tool folders (`~/.cargo/bin`, `~/.local/bin`, Homebrew).
    Hollow,
    /// The login shell's PATH (what `~/.zshrc` adds).
    Shell,
    /// The PATH Hollow itself was started with.
    Inherited,
    /// `/usr/bin:/bin:/usr/sbin:/sbin`.
    System,
}

/// One folder of the agents' PATH and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathDir {
    pub dir: String,
    pub from: ToolPathFrom,
}

/// Whether the login shell's PATH has been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ShellPathState {
    Read,
    /// Still running: agents get Hollow's own PATH until it answers.
    Pending,
    /// It hung, failed or printed no marker; agents keep Hollow's own PATH.
    Failed,
}

/// The login shell's read, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ShellPathRead {
    pub state: ShellPathState,
    /// Why it failed (`timed out after 5000 ms`, `exit code 1`, `no marker`);
    /// `None` unless `state` is `Failed`.
    pub detail: Option<String>,
}

/// One row of [`AgentToolPath`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentToolPathEntry {
    pub dir: String,
    pub from: ToolPathFrom,
    /// The folder is there now; a missing one is listed but holds no tools.
    /// `None` when the check didn't answer within [`EXISTS_DEADLINE`] (a
    /// stalled network mount; overnight Spare 1).
    pub exists: Option<bool>,
}

/// How long Settings waits for all the folders' checks together.
pub(crate) const EXISTS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// `agent_tool_path` (A12): the PATH every agent CLI gets, in order, and
/// whether the login shell's part of it has been read. Read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentToolPath {
    pub entries: Vec<AgentToolPathEntry>,
    pub shell: ShellPathRead,
}

/// [`AgentToolPath`] for this process: the shell's read as it stands now, the
/// inherited PATH and the home folder, checking each folder on disk, all at
/// once, for at most [`EXISTS_DEADLINE`] (a stalled network folder used to
/// hang the list; overnight Spare 1).
pub(crate) fn agent_tool_path() -> AgentToolPath {
    let home = dirs::home_dir().unwrap_or_default();
    let inherited = std::env::var("PATH").ok();
    let exists = |dirs: &[&str]| dirs_exist(dirs, EXISTS_DEADLINE, |dir| Path::new(dir).is_dir());
    tool_path_report(SHELL_PATH.get(), inherited.as_deref(), &home, exists)
}

/// The folders whose [`dirs_exist`] check is still running.
static CHECKING: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Run `check` on each of `dirs` on its own thread and wait at most
/// `deadline` for all of them: `None` for each that hasn't answered (a
/// thread blocked on a stalled mount is left to finish on its own).
pub(crate) fn dirs_exist(
    dirs: &[&str],
    deadline: std::time::Duration,
    check: impl Fn(&str) -> bool + Clone + Send + 'static,
) -> Vec<Option<bool>> {
    let checking = || CHECKING.lock().unwrap_or_else(|e| e.into_inner());
    let (tx, rx) = std::sync::mpsc::channel();
    for (at, dir) in dirs.iter().enumerate() {
        // A folder whose earlier check is still blocked reads as unknown at
        // once: at most one stuck thread per stalled folder (review).
        if !checking().insert(dir.to_string()) {
            continue;
        }
        let (tx, owned, check) = (tx.clone(), dir.to_string(), check.clone());
        let spawned = std::thread::Builder::new()
            .name("tool-path-check".into())
            .spawn(move || {
                let there = check(&owned);
                CHECKING
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&owned);
                let _ = tx.send((at, there));
            });
        if let Err(error) = spawned {
            checking().remove(*dir);
            crate::launch::elog(&format!(
                "[shell-path] a folder check could not start: {error}"
            ));
        }
    }
    drop(tx);
    let mut found = vec![None; dirs.len()];
    let end = std::time::Instant::now() + deadline;
    while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((at, there)) => found[at] = Some(there),
            // Every check answered, or the deadline passed.
            Err(_) => break,
        }
    }
    found
}

/// [`AgentToolPath`] from one look at the shell's read (`None` while it
/// runs), so the entries and the state can't disagree: the same merge
/// [`crate::agent::cli_path`] joins.
pub(crate) fn tool_path_report(
    read: Option<&Result<Vec<String>, String>>,
    inherited: Option<&str>,
    home: &Path,
    exists: impl FnOnce(&[&str]) -> Vec<Option<bool>>,
) -> AgentToolPath {
    let dirs =
        crate::agent::cli_path_dirs_with(shell_folders(read).unwrap_or_default(), inherited, home);
    let shell = match read {
        None => ShellPathRead {
            state: ShellPathState::Pending,
            detail: None,
        },
        Some(Ok(_)) => ShellPathRead {
            state: ShellPathState::Read,
            detail: None,
        },
        Some(Err(why)) => ShellPathRead {
            state: ShellPathState::Failed,
            detail: Some(why.clone()),
        },
    };
    let found = exists(&dirs.iter().map(|d| d.dir.as_str()).collect::<Vec<_>>());
    AgentToolPath {
        entries: dirs
            .into_iter()
            .enumerate()
            .map(|(at, d)| AgentToolPathEntry {
                exists: found.get(at).copied().flatten(),
                dir: d.dir,
                from: d.from,
            })
            .collect(),
        shell,
    }
}

/// `$SHELL` when `/etc/shells` lists it, else `/bin/zsh` (macOS's default).
fn login_shell() -> PathBuf {
    let listed = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    shell_choice(std::env::var("SHELL").ok().as_deref(), &listed)
}

fn shell_choice(shell: Option<&str>, etc_shells: &str) -> PathBuf {
    match shell {
        Some(s) if s.starts_with('/') && etc_shells.lines().any(|line| line.trim() == s) => {
            PathBuf::from(s)
        }
        _ => PathBuf::from("/bin/zsh"),
    }
}

/// Run `shell -ilc` once and read the PATH it ends up with. The shell runs in
/// its own session (no controlling terminal, so an interactive rc can't stop
/// on one) with stdin and stderr closed; past `limit` its whole process group
/// is killed and reaped.
pub(crate) fn read_shell_path(
    shell: &Path,
    home: &Path,
    limit: Duration,
) -> Result<Vec<String>, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(shell);
    cmd.arg("-ilc")
        .arg(format!("printf '\\n{MARKER}%s\\n' \"$PATH\""))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("TERM", "dumb")
        .current_dir(home);
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start it: {e}"))?;
    let pgid = child.id() as libc::pid_t;
    let kill_all = |child: &mut std::process::Child| {
        // SAFETY: plain syscall on the group this function created.
        unsafe { libc::killpg(pgid, libc::SIGKILL) };
        let _ = child.kill();
        let _ = child.wait();
    };
    // Read on a thread, so a chatty rc can't fill the pipe and stall the shell.
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                kill_all(&mut child);
                return Err(format!("timed out after {} ms", limit.as_millis()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                kill_all(&mut child);
                return Err(format!("couldn't wait for it: {e}"));
            }
        }
    };
    // The output normally closes with the shell. If something the rc left
    // running still holds the pipe, end the group (never when it closed by
    // itself, so a background job the rc started, e.g. a zcompile, is left
    // alone), and don't wait on it past the deadline.
    let out = match rx.recv_timeout(Duration::from_millis(300)) {
        Ok(out) => out,
        Err(_) => {
            // SAFETY: as above.
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(200));
            rx.recv_timeout(wait)
                .map_err(|_| "its output didn't close".to_string())?
        }
    };
    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("exit code {code}"),
            None => "ended by a signal".into(),
        });
    }
    parse_marked_path(&String::from_utf8_lossy(&out)).ok_or_else(|| "no marker".to_string())
}

/// The PATH after the last marker line: only absolute folders without
/// control characters. `None` when no line carries the marker.
pub(crate) fn parse_marked_path(stdout: &str) -> Option<Vec<String>> {
    let line = stdout
        .lines()
        .rev()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix(MARKER))?;
    Some(
        line.split(':')
            .filter(|dir| dir.starts_with('/') && !dir.chars().any(char::is_control))
            .map(str::to_string)
            .collect(),
    )
}

/// One PATH, folder by folder: Hollow's tool folders, then the shell's, then
/// the inherited PATH, then the system folders; the first copy of each folder
/// wins and keeps that source.
pub(crate) fn merge(
    hollow_dirs: &[String],
    shell: &[String],
    inherited: &[String],
    system: &[String],
) -> Vec<PathDir> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (from, dirs) in [
        (ToolPathFrom::Hollow, hollow_dirs),
        (ToolPathFrom::Shell, shell),
        (ToolPathFrom::Inherited, inherited),
        (ToolPathFrom::System, system),
    ] {
        for dir in dirs {
            if !dir.is_empty() && seen.insert(dir.as_str()) {
                out.push(PathDir {
                    dir: dir.clone(),
                    from,
                });
            }
        }
    }
    out
}

/// [`merge`]'s folders as a PATH string.
pub(crate) fn join(dirs: &[PathDir]) -> String {
    dirs.iter()
        .map(|d| d.dir.as_str())
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(dirs: &[&str]) -> Vec<String> {
        dirs.iter().map(|d| d.to_string()).collect()
    }

    fn sources(report: &AgentToolPath) -> Vec<ToolPathFrom> {
        report.entries.iter().map(|e| e.from).collect()
    }

    fn write_exec(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_last_marker_wins_and_rc_noise_is_ignored() {
        let out = "Welcome!\n__HOLLOW_PATH__=/early\nPATH=/noise\n\n__HOLLOW_PATH__=/a:/b/bin\n";
        assert_eq!(parse_marked_path(out), Some(strings(&["/a", "/b/bin"])));
    }

    #[test]
    fn no_marker_reads_as_nothing() {
        assert_eq!(parse_marked_path("PATH=/usr/bin\nhello\n"), None);
        assert_eq!(parse_marked_path(""), None);
    }

    #[test]
    fn relative_empty_and_control_entries_are_dropped() {
        let out = "\n__HOLLOW_PATH__=/ok::bin:./x:/bad\u{7}dir:~/y:/also/ok\r\n";
        assert_eq!(parse_marked_path(out), Some(strings(&["/ok", "/also/ok"])));
    }

    #[test]
    fn merge_keeps_order_and_the_first_copy() {
        let merged = join(&merge(
            &strings(&["/h/.cargo/bin", "/opt/homebrew/bin"]),
            &strings(&["/h/google-cloud-sdk/bin", "/opt/homebrew/bin", "/usr/bin"]),
            &strings(&["/inherited", "/h/.cargo/bin"]),
            &strings(&["/usr/bin", "/bin"]),
        ));
        assert_eq!(
            merged,
            "/h/.cargo/bin:/opt/homebrew/bin:/h/google-cloud-sdk/bin:/usr/bin:/inherited:/bin"
        );
    }

    /// A12: each folder is labelled with the first source that listed it.
    #[test]
    fn merge_labels_each_folder_with_its_first_source() {
        let merged = merge(
            &strings(&["/h/.cargo/bin", "/opt/homebrew/bin"]),
            &strings(&[
                "/h/google-cloud-sdk/bin",
                "/opt/homebrew/bin",
                "",
                "/usr/bin",
            ]),
            &strings(&["/inherited", "/h/.cargo/bin", "/usr/bin"]),
            &strings(&["/usr/bin", "/bin"]),
        );
        let labelled: Vec<(&str, ToolPathFrom)> =
            merged.iter().map(|d| (d.dir.as_str(), d.from)).collect();
        assert_eq!(
            labelled,
            [
                ("/h/.cargo/bin", ToolPathFrom::Hollow),
                ("/opt/homebrew/bin", ToolPathFrom::Hollow),
                ("/h/google-cloud-sdk/bin", ToolPathFrom::Shell),
                ("/usr/bin", ToolPathFrom::Shell),
                ("/inherited", ToolPathFrom::Inherited),
                ("/bin", ToolPathFrom::System),
            ]
        );
        let froms: Vec<ToolPathFrom> = merged.iter().map(|d| d.from).collect();
        assert_eq!(
            serde_json::to_value(froms).unwrap(),
            serde_json::json!(["hollow", "hollow", "shell", "shell", "inherited", "system"])
        );
    }

    /// A12: with the cell unset the report says pending and lists exactly
    /// the PATH agents get now, with no shell folders.
    #[test]
    fn the_report_is_pending_until_the_shell_answers() {
        let home = Path::new("/h");
        let inherited = Some("/inherited:/usr/bin:/bin");
        let report = tool_path_report(None, inherited, home, |dirs| {
            dirs.iter().map(|dir| Some(*dir != "/inherited")).collect()
        });
        assert_eq!(report.shell.state, ShellPathState::Pending);
        assert_eq!(report.shell.detail, None);
        assert!(!sources(&report).contains(&ToolPathFrom::Shell));
        assert_eq!(report.entries[0].dir, "/h/.cargo/bin");
        assert_eq!(report.entries[0].from, ToolPathFrom::Hollow);
        let missing: Vec<&str> = report
            .entries
            .iter()
            .filter(|e| e.exists == Some(false))
            .map(|e| e.dir.as_str())
            .collect();
        assert_eq!(missing, ["/inherited"]);
        let dirs: Vec<String> = report.entries.iter().map(|e| e.dir.clone()).collect();
        assert_eq!(
            dirs.join(":"),
            crate::self_install::tool_path(inherited, home),
            "pending lists the PATH agents get before the shell answers"
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["shell"]["state"], "pending");
        assert_eq!(json["entries"][0]["from"], "hollow");
        assert!(json["entries"][0]["exists"].is_boolean());
    }

    /// Overnight Spare 1: a folder whose check hangs (a stalled network
    /// mount) reads as unknown once the shared deadline passes, and the
    /// others still answer; the list never waits longer than the deadline.
    #[test]
    fn a_folder_check_that_hangs_reads_as_unknown_by_the_deadline() {
        // No wall-clock bound (check-timing-asserts): the stalled folder's
        // `None` shows the read returned before its 10 s check did.
        let found = dirs_exist(
            &["/fast", "/a15-stalled", "/gone"],
            std::time::Duration::from_secs(1),
            |dir| {
                if dir == "/a15-stalled" {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
                dir != "/gone"
            },
        );
        assert_eq!(found, [Some(true), None, Some(false)]);
        // A read while that check is still blocked starts no second thread
        // on the folder (one would answer `true` here): it reads unknown at
        // once (review).
        let again = dirs_exist(
            &["/a15-stalled", "/fast"],
            std::time::Duration::from_secs(5),
            |_| true,
        );
        assert_eq!(again, [None, Some(true)]);
        let none = dirs_exist(&[], std::time::Duration::from_secs(5), |_| true);
        assert!(none.is_empty());

        let home = Path::new("/h");
        let report = tool_path_report(None, Some("/usr/bin"), home, |dirs| {
            dirs.iter()
                .map(|dir| (*dir != "/usr/bin").then_some(true))
                .collect()
        });
        let usr = report.entries.iter().find(|e| e.dir == "/usr/bin").unwrap();
        assert_eq!(usr.exists, None);
        let json = serde_json::to_value(usr).unwrap();
        assert!(json["exists"].is_null());
    }

    /// A12: a read adds the shell's folders; a failure says why and keeps
    /// Hollow's own PATH.
    #[test]
    fn the_report_says_read_or_failed_with_the_reason() {
        let home = Path::new("/h");
        let inherited = Some("/usr/bin:/bin");
        let read: Result<Vec<String>, String> = Ok(strings(&["/h/google-cloud-sdk/bin"]));
        let all = |dirs: &[&str]| vec![Some(true); dirs.len()];
        let report = tool_path_report(Some(&read), inherited, home, all);
        assert_eq!(report.shell.state, ShellPathState::Read);
        assert!(report
            .entries
            .iter()
            .any(|e| e.dir == "/h/google-cloud-sdk/bin" && e.from == ToolPathFrom::Shell));

        let failed: Result<Vec<String>, String> = Err("timed out after 5000 ms".into());
        let report = tool_path_report(Some(&failed), inherited, home, all);
        assert_eq!(report.shell.state, ShellPathState::Failed);
        assert_eq!(
            report.shell.detail.as_deref(),
            Some("timed out after 5000 ms")
        );
        assert!(!sources(&report).contains(&ToolPathFrom::Shell));
    }

    #[test]
    fn a_listed_shell_is_used_and_anything_else_falls_back_to_zsh() {
        let shells = "# comment\n/bin/zsh\n/opt/homebrew/bin/fish\n";
        assert_eq!(
            shell_choice(Some("/opt/homebrew/bin/fish"), shells),
            PathBuf::from("/opt/homebrew/bin/fish")
        );
        assert_eq!(
            shell_choice(Some("/tmp/evil"), shells),
            PathBuf::from("/bin/zsh")
        );
        assert_eq!(shell_choice(None, shells), PathBuf::from("/bin/zsh"));
    }

    #[test]
    fn a_fake_shell_that_prints_the_marker_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        let shell = tmp.path().join("fake-shell");
        write_exec(
            &shell,
            "#!/bin/sh\necho 'rc noise'\nprintf '\\n__HOLLOW_PATH__=/x/google-cloud-sdk/bin:/usr/bin\\n'\n",
        );
        let dirs = read_shell_path(&shell, tmp.path(), Duration::from_secs(5)).unwrap();
        assert_eq!(dirs, strings(&["/x/google-cloud-sdk/bin", "/usr/bin"]));
    }

    #[test]
    fn a_shell_that_hangs_is_killed_at_the_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let shell = tmp.path().join("fake-shell");
        let pid_file = tmp.path().join("pid");
        write_exec(
            &shell,
            &format!("#!/bin/sh\necho $$ > '{}'\nsleep 30\n", pid_file.display()),
        );
        let started = Instant::now();
        let result = read_shell_path(&shell, tmp.path(), Duration::from_secs(1));
        // The 1 s limit, not the shell's 30 s sleep ("timed out" below).
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert!(result.unwrap_err().contains("timed out"));
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: signal 0 only checks whether the process exists.
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "the shell was left running"
        );
    }

    #[test]
    fn a_failing_or_markerless_shell_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let failing = tmp.path().join("failing");
        write_exec(
            &failing,
            "#!/bin/sh\nprintf '\\n__HOLLOW_PATH__=/a\\n'\nexit 1\n",
        );
        assert_eq!(
            read_shell_path(&failing, tmp.path(), Duration::from_secs(5)).unwrap_err(),
            "exit code 1"
        );
        let garbage = tmp.path().join("garbage");
        write_exec(&garbage, "#!/bin/sh\nprintf 'PATH=/a\\x01\\x02\\n'\n");
        assert_eq!(
            read_shell_path(&garbage, tmp.path(), Duration::from_secs(5)).unwrap_err(),
            "no marker"
        );
    }

    #[test]
    fn a_background_child_holding_the_pipe_does_not_stall_the_read() {
        let tmp = tempfile::tempdir().unwrap();
        let shell = tmp.path().join("fake-shell");
        write_exec(
            &shell,
            "#!/bin/sh\nsleep 30 &\nprintf '\\n__HOLLOW_PATH__=/a\\n'\n",
        );
        let started = Instant::now();
        let dirs = read_shell_path(&shell, tmp.path(), Duration::from_secs(5)).unwrap();
        assert_eq!(dirs, strings(&["/a"]));
        // Read before the 5 s limit (else an error above), and far short of
        // the background child's 30 s.
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }
}
