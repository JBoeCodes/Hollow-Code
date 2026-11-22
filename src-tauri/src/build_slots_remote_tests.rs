//! `hollow-slot --remote` / `--on <host>` (2026-11-01, the remote runner):
//! the checkout's exact state goes to the host, the command runs there in the
//! host's own slots, output and the exit code come back, and anything that
//! keeps it from running there runs it here instead. A stand-in `ssh`
//! (`HOLLOW_SLOT_SSH`) plays the host: it runs the remote command with
//! `zsh`, the login shell there, in a fake home, with only the variables a
//! login there would have, so these tests need no second machine.

use super::SLOT_TOOL;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// Skips ssh's options, refuses host `down` the way ssh does (exit 255), and
/// runs the rest in the fake home with a clean environment.
const FAKE_SSH: &str = r#"#!/bin/sh
# Every command line a call gives ssh (FAKE_ARGV_LOG).
[ -z "$FAKE_ARGV_LOG" ] || printf '%s\n' "$*" >> "$FAKE_ARGV_LOG"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o | -p | -i | -l) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
host="$1"
shift
if [ "$host" = down ]; then
  echo "ssh: connect to host down port 22: Connection refused" >&2
  exit 255
fi
# A host that hands back a pack of its own making (FAKE_COLLECT_TAR).
case "$*" in
  *--remote-collect*) [ -z "$FAKE_COLLECT_TAR" ] || exec cat "$FAKE_COLLECT_TAR" ;;
  # A slow login for the run itself (FAKE_LOGIN_SLEEP seconds).
  *--remote-run*) [ -z "$FAKE_LOGIN_SLEEP" ] || sleep "$FAKE_LOGIN_SLEEP" ;;
  # A job's upload that starts late (FAKE_RECEIVE_SLEEP seconds).
  *--job-receive*) [ -z "$FAKE_RECEIVE_SLEEP" ] || sleep "$FAKE_RECEIVE_SLEEP" ;;
esac
cd "$FAKE_HOME" || exit 255
exec env -i HOME="$FAKE_HOME" PATH="$FAKE_PATH" HOLLOW_BUILD_SLOT_DIR="$FAKE_SLOTS" HOLLOW_REMOTE_TMP="$FAKE_TMP" HOLLOW_REMOTE_KEEP="${FAKE_KEEP:-8}" HOLLOW_REMOTE_MIN_FREE_GB="${FAKE_MIN_FREE_GB:-0}" ${FAKE_DF:+HOLLOW_REMOTE_DF="$FAKE_DF"} HOLLOW_REMOTE_PG_DIRS="${FAKE_PG_DIRS:-}" HOLLOW_SLOT_POLL_SECS=1 HOLLOW_SLOT_MIN_FREE_PCT="${FAKE_MIN_FREE_PCT:-0}" ${FAKE_SYSCTL:+HOLLOW_SLOT_SYSCTL="$FAKE_SYSCTL"} ${FAKE_NO_LOCKF:+HOLLOW_SLOT_NO_LOCKF=1} ${FAKE_FLOCK:+HOLLOW_SLOT_FLOCK="$FAKE_FLOCK"} ${FAKE_PERL5LIB:+PERL5LIB="$FAKE_PERL5LIB"} ${FAKE_PERL5OPT:+PERL5OPT="$FAKE_PERL5OPT"} /bin/zsh -f -c "$*"
"#;

/// A host's `hollow-run-preflight` that says no.
const REFUSING_PREFLIGHT: &str =
    "#!/bin/sh\necho 'hollow-run-preflight: not taking work: only 3 GB free' >&2\nexit 1\n";

struct Remote {
    dir: tempfile::TempDir,
}

impl Remote {
    fn new() -> Remote {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("remote-")
            .tempdir()
            .unwrap();
        let root = dir.path();
        std::fs::write(root.join("hollow-slot"), SLOT_TOOL).unwrap();
        for sub in [
            "bin",
            "host",
            "host-bin",
            "home",
            "slots",
            "host-slots",
            "host-tmp",
            "tmp",
        ] {
            std::fs::create_dir(root.join(sub)).unwrap();
        }
        let ssh = root.join("bin").join("fake-ssh");
        std::fs::write(&ssh, FAKE_SSH).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The host's Rust and Node, as its preparation reports them (smart
        // routing 2): stand-ins, so the rig needs neither.
        for (tool, says) in [("rustc", "rustc 1.96.0 (stand-in)"), ("node", "v26.0.0")] {
            let path = root.join("host-bin").join(tool);
            std::fs::write(&path, format!("#!/bin/sh\necho '{says}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // pmset -g batt on mains power, unless a test writes FAKE_BATTERY.
        let pmset = root.join("bin").join("fake-pmset");
        std::fs::write(
            &pmset,
            format!(
                "#!/bin/sh\nif [ -e '{}' ]; then echo \"Now drawing from 'Battery Power'\"; else echo \"Now drawing from 'AC Power'\"; fi\n",
                root.join("bin").join("FAKE_BATTERY").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pmset, std::fs::Permissions::from_mode(0o755)).unwrap();
        let remote = Remote { dir };
        RIG.with(|r| *r.borrow_mut() = Some(remote.path().to_path_buf()));
        let repo = remote.repo();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("kept.txt"), "committed\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "deleted here\n").unwrap();
        std::fs::write(repo.join("sub").join("in-sub.txt"), "sub\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
        remote.git(&repo, &["init", "-q", "-b", "main"]);
        remote.commit(&repo, "first");
        remote
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn repo(&self) -> PathBuf {
        self.path().join("repo")
    }
    /// The host's `~/hollow-remote`.
    fn host_root(&self) -> PathBuf {
        self.path().join("host").join("hollow-remote")
    }
    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output_within()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
    fn commit(&self, cwd: &Path, message: &str) {
        self.git(cwd, &["add", "-A"]);
        self.git(
            cwd,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    }
    /// `hollow-slot <args>` from `cwd`, with the stand-in ssh, this rig's
    /// slots, the host's PATH (`host-bin` first, for a preflight), and no
    /// runner set up unless `HOME`'s config or `HOLLOW_SLOT_REMOTE` says so.
    fn cmd(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg(self.path().join("hollow-slot"))
            .args(args)
            .current_dir(cwd)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.path().join("home"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOLLOW_SLOT_SSH", self.path().join("bin").join("fake-ssh"))
            .env("HOLLOW_BUILD_SLOT_DIR", self.path().join("slots"))
            .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
            .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
            .env("HOLLOW_SLOT_POLL_SECS", "1")
            .env("FAKE_HOME", self.path().join("host"))
            .env(
                "FAKE_PATH",
                format!(
                    "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                    self.path().join("host-bin").display()
                ),
            )
            .env("FAKE_SLOTS", self.path().join("host-slots"))
            .env("FAKE_TMP", self.path().join("host-tmp"))
            // The call's own scratch folder goes with the rig, even when a
            // test kills the call before it can clean up.
            .env("TMPDIR", self.path().join("tmp"))
            .env("RIG_LOCAL_ONLY", "this Mac only")
            .env_remove("HOLLOW_SLOT_REMOTE")
            // Routing (2026-11-03): no setting, no database or gcloud
            // variables from the test's own environment, and mains power,
            // so a laptop on battery doesn't move the rig's runs.
            .env_remove("HOLLOW_SLOT_ROUTE")
            .env_remove("HOLLOW_SLOT_PG")
            .env_remove("HOLLOW_SLOT_PG_TMP")
            .env_remove("HOLLOW_SLOT_LONG_SECS")
            // A stand-in command takes no time: without this, its history
            // would keep every heavy one here (a test sets it to see that).
            .env("HOLLOW_SLOT_QUICK_SECS", "0")
            .env_remove("HOLLOW_SLOT_DOWN_SECS")
            .env_remove("DATABASE_URL")
            .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
            .env(
                "HOLLOW_SLOT_PMSET",
                self.path().join("bin").join("fake-pmset"),
            )
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HOLLOW_SLOT_HOLDER_PID")
            .env_remove("HOLLOW_SLOT_CARGO_PID")
            .env_remove("HOLLOW_SLOT_CARGO")
            .env_remove("HOLLOW_SLOT_LABEL")
            .env_remove("HOLLOW_SLOT_SAY_ON")
            .env_remove("HOLLOW_SLOT_NO_LOCKF")
            .env_remove("HOLLOW_SLOT_FLOCK")
            .env_remove("HOLLOW_SLOT_MEMINFO")
            .env_remove("HOLLOW_SLOT_SETUP")
            .env_remove("HOLLOW_SLOT_SETUP_SAY")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            // A gate on the runner carries this Mac's identity: the rig's
            // checkout has none unless a test gives it one.
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL");
        cmd
    }
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.cmd(cwd, args).output_within().unwrap()
    }
    /// The host's worktree folders.
    fn host_work(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.host_root().join("work")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .collect();
        names.sort();
        names
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Wait (60 s at most, so a hang fails instead of blocking) until `done`.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no hang: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn alive(pid: &str) -> bool {
    Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// How long one call in these tests may take, its output's end included. A
/// passing call takes seconds even on a loaded runner. Past this it counts
/// as hung (2026-11-04: a job's `--job-receive` sat 17 minutes at 0% CPU in
/// a gate, and the gate waited with it): the test fails naming what was
/// still running and which pipes each process held, and stops it.
const CALL_LIMIT: Duration = Duration::from_secs(120);

thread_local! {
    /// The rig the test on this thread made last ([`Remote::new`]): what a
    /// hung call left is named by its folder, even after the call exited
    /// and its children moved to launchd.
    static RIG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// `Command::output` within CALL_LIMIT, with stdin from `/dev/null` (a call
/// that needs its own stdin spawns and uses [`WaitWithin`]). The call stays
/// in the test's process group, so a gate's Stop still reaches it.
trait OutputWithin {
    fn output_within(&mut self) -> std::io::Result<Output>;
}

impl OutputWithin for Command {
    fn output_within(&mut self) -> std::io::Result<Output> {
        let what = std::iter::once(self.get_program())
            .chain(self.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut child = self
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let out = Drain::new(child.stdout.take().unwrap());
        let err = Drain::new(child.stderr.take().unwrap());
        let status = bounded(&mut child, &what, &[&out, &err]);
        Ok(Output {
            status,
            stdout: out.take(),
            stderr: err.take(),
        })
    }
}

/// `Child::wait` and `wait_with_output` within CALL_LIMIT. Like std's, they
/// close a piped stdin first.
trait WaitWithin {
    fn wait_within(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn wait_with_output_within(self) -> std::io::Result<Output>;
}

impl WaitWithin for std::process::Child {
    fn wait_within(&mut self) -> std::io::Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        let what = format!("process {}", self.id());
        Ok(bounded(self, &what, &[]))
    }
    fn wait_with_output_within(mut self) -> std::io::Result<Output> {
        drop(self.stdin.take());
        let what = format!("process {}", self.id());
        let out = self.stdout.take().map(Drain::new);
        let err = self.stderr.take().map(Drain::new);
        let pipes: Vec<&Drain> = out.iter().chain(err.iter()).collect();
        let status = bounded(&mut self, &what, &pipes);
        Ok(Output {
            status,
            stdout: out.map(Drain::take).unwrap_or_default(),
            stderr: err.map(Drain::take).unwrap_or_default(),
        })
    }
}

/// A pipe read to its end on a thread of its own; what came so far stays
/// readable for a hang's report.
struct Drain {
    got: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    done: std::sync::mpsc::Receiver<()>,
}

impl Drain {
    fn new(mut pipe: impl std::io::Read + Send + 'static) -> Drain {
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, done) = std::sync::mpsc::channel();
        let into = std::sync::Arc::clone(&got);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => into.lock().unwrap().extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(());
        });
        Drain { got, done }
    }
    /// Whether the pipe ended by `deadline` (nothing holds it open).
    fn ended_by(&self, deadline: Instant) -> bool {
        let left = deadline.saturating_duration_since(Instant::now());
        !matches!(
            self.done.recv_timeout(left),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        )
    }
    fn so_far(&self) -> String {
        String::from_utf8_lossy(&self.got.lock().unwrap()).into_owned()
    }
    fn take(self) -> Vec<u8> {
        std::mem::take(&mut *self.got.lock().unwrap())
    }
}

/// Waits for `child` to exit and its `pipes` to end, within CALL_LIMIT;
/// past it, reports and stops what is still running and fails the test.
fn bounded(
    child: &mut std::process::Child,
    what: &str,
    pipes: &[&Drain],
) -> std::process::ExitStatus {
    let deadline = Instant::now() + CALL_LIMIT;
    let exited = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if let Some(status) = exited {
        if pipes.iter().all(|p| p.ended_by(deadline)) {
            return status;
        }
    }
    // A reaped child's pid may belong to another process by now: it seeds
    // nothing then, and only the rig's own processes are named.
    let rig = RIG.with(|r| r.borrow().clone());
    let left = still_running(exited.is_none().then(|| child.id() as i32), rig.as_deref());
    stop_all(&left);
    if exited.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let said: Vec<String> = pipes.iter().map(|p| p.so_far()).collect();
    let how = match exited {
        Some(status) => {
            format!("exited ({status}), but something it started still held its output")
        }
        None => "was still running".to_string(),
    };
    panic!(
        "a hang, not a slow host: `{what}` {how} after {} s.\nWhat was left (pid ppid pgid elapsed state command), then the pipes and FIFOs each held:\n{}\nWhat it printed so far:\n{}",
        CALL_LIMIT.as_secs(),
        left.report,
        said.join("\n---\n")
    );
}

/// The processes a hung call left, with lsof's pipes and FIFOs for each:
/// the call (while it runs) and its descendants, and the rig's own
/// processes and theirs, those whose program or script is in the rig's
/// folder (its `hollow-slot`, the stand-in ssh, the host's tool). Never a
/// process that only mentions the folder further on (a `tail -f` of a rig
/// file, an agent's prompt quoting it).
struct Left {
    pids: Vec<i32>,
    report: String,
}

fn still_running(call: Option<i32>, rig: Option<&Path>) -> Left {
    let ps = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid=,etime=,stat=,command="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let rig = rig.map(|r| r.display().to_string());
    // pid, ppid, the line, and whether its program or script is the rig's.
    let rows: Vec<(i32, i32, &str, bool)> = ps
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let p = f.next()?.parse().ok()?;
            let pp = f.next()?.parse().ok()?;
            let _group = f.next()?;
            let _elapsed = f.next()?;
            let _state = f.next()?;
            let own = rig
                .as_deref()
                .is_some_and(|r| f.take(2).any(|word| word.starts_with(&format!("{r}/"))));
            Some((p, pp, line, own))
        })
        .collect();
    let me = std::process::id() as i32;
    let mut keep: std::collections::BTreeSet<i32> = call.into_iter().collect();
    keep.extend(rows.iter().filter(|r| r.3).map(|r| r.0));
    loop {
        let before = keep.len();
        for (p, pp, ..) in &rows {
            if keep.contains(pp) {
                keep.insert(*p);
            }
        }
        if keep.len() == before {
            break;
        }
    }
    keep.remove(&me);
    let mut report: Vec<String> = rows
        .iter()
        .filter(|(p, ..)| keep.contains(p))
        .map(|(_, _, line, _)| line.chars().take(300).collect())
        .collect();
    let pids: Vec<i32> = keep.into_iter().filter(|p| *p > 1).collect();
    if !pids.is_empty() {
        let list = pids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let lsof = Command::new("/usr/sbin/lsof")
            .args(["-nP", "-p", &list])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        report.extend(
            lsof.lines()
                .filter(|l| l.contains(" FIFO ") || l.contains(" PIPE "))
                .take(80)
                .map(str::to_string),
        );
    }
    Left {
        pids,
        report: report.join("\n"),
    }
}

/// Stops what a hung call left, and the groups those processes lead (the
/// host's command groups), never this test process or its own group.
fn stop_all(left: &Left) {
    // SAFETY: getpgrp, getpgid, kill and killpg take plain integers and
    // touch no memory.
    let own_group = unsafe { libc::getpgrp() };
    let me = std::process::id() as i32;
    for &p in &left.pids {
        if p == me || p <= 1 {
            continue;
        }
        unsafe {
            if p != own_group && libc::getpgid(p) == p {
                libc::killpg(p, libc::SIGKILL);
            }
            libc::kill(p, libc::SIGKILL);
        }
    }
}

/// The exact tree goes along (an edit, a new file, a deletion, nothing
/// ignored), the command runs in the same subfolder on the host with that
/// worktree's own build folder and temp folder and none of this Mac's
/// variables, and its exit code comes back. The snapshot commit never lands
/// in this checkout's repository.
#[test]
fn a_remote_run_sends_the_exact_tree_and_returns_the_exit_code() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join("kept.txt"), "edited, not committed\n").unwrap();
    std::fs::write(repo.join("sub").join("new.txt"), "untracked\n").unwrap();
    std::fs::write(repo.join("ignored.txt"), "stays here\n").unwrap();
    std::fs::remove_file(repo.join("gone.txt")).unwrap();
    let refs_before = rig.git(&repo, &["for-each-ref"]);
    let out = rig.run(
        &repo.join("sub"),
        &[
            "--on",
            "fakehost",
            "sh",
            "-c",
            "pwd; cat ../kept.txt new.txt; for f in ../gone.txt ../ignored.txt; do [ -e $f ] && echo \"has $f\"; done; \
             echo \"target=$CARGO_TARGET_DIR\"; echo \"tmp=$TMPDIR\"; echo \"port=$PORT\"; echo \"local=${RIG_LOCAL_ONLY:-unset}\"; \
             echo \"commit=$(git rev-parse HEAD)\"; exit 7",
        ],
    );
    let (text, said) = (stdout(&out), stderr(&out));
    assert_eq!(
        out.status.code(),
        Some(7),
        "the command's own exit code; {said}"
    );
    let work = rig.host_root().join("work");
    let ids = rig.host_work();
    assert_eq!(ids.len(), 1, "one folder for this worktree: {ids:?}");
    let id = &ids[0];
    assert!(id.starts_with("repo-"), "named after the worktree: {id}");
    let pwd = text.lines().next().unwrap_or_default();
    assert!(
        pwd.ends_with(&format!("/hollow-remote/work/{id}/sub")),
        "the same subfolder, on the host: {pwd}"
    );
    assert!(
        text.contains("edited, not committed\nuntracked\n"),
        "{text}"
    );
    assert!(
        !text.contains("has "),
        "deleted and ignored files stay away: {text}"
    );
    let target = rig.host_root().join("targets").join(id);
    assert!(
        text.contains(&format!("target={}\n", target.display())),
        "the worktree's own CARGO_TARGET_DIR: {text}"
    );
    assert!(
        text.contains(&format!(
            "tmp={}/hs-",
            rig.path().join("host-tmp").display()
        )),
        "a short TMPDIR of its own: {text}"
    );
    let port: u16 = text
        .lines()
        .find_map(|l| l.strip_prefix("port="))
        .and_then(|p| p.parse().ok())
        .expect("a PORT");
    assert!((2300..2700).contains(&port), "its own PORT: {port}");
    assert!(
        text.contains("local=unset"),
        "no variable of this Mac's: {text}"
    );
    assert!(
        said.contains("running on fakehost, not this Mac")
            && said.contains("plus this checkout's uncommitted changes")
            && said.contains("finished on fakehost: exit 7"),
        "it says where it ran: {said}"
    );
    let snapshot = text
        .lines()
        .find_map(|l| l.strip_prefix("commit="))
        .unwrap()
        .to_string();
    assert_ne!(snapshot, rig.git(&repo, &["rev-parse", "HEAD"]));
    let here = Command::new("git")
        .current_dir(&repo)
        .args(["cat-file", "-e", &snapshot])
        .status()
        .unwrap();
    assert!(
        !here.success(),
        "the snapshot commit stays out of this repository"
    );
    assert_eq!(
        rig.git(&repo, &["for-each-ref"]),
        refs_before,
        "no new refs here"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("kept.txt")).unwrap(),
        "edited, not committed\n",
        "this checkout is left as it was"
    );
    assert!(work.join(format!("{id}.used")).exists());

    // A committed, clean tree is sent as HEAD itself.
    rig.commit(&repo, "second");
    let out = rig.run(&repo, &["--on", "fakehost", "git", "rev-parse", "HEAD"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), rig.git(&repo, &["rev-parse", "HEAD"]));
    assert!(!stderr(&out).contains("uncommitted"), "{}", stderr(&out));
}

/// Arguments arrive exactly as given: spaces, quotes, `$`, globs, newlines.
#[test]
fn arguments_survive_the_trip_to_the_host() {
    let rig = Remote::new();
    let args = [
        "printf",
        "[%s]\\n",
        "it's",
        "a  b",
        "$HOME",
        "*",
        "two\nlines",
        "'quoted'",
        "",
        "back\\slash",
    ];
    let here = Command::new(args[0])
        .args(&args[1..])
        .output_within()
        .unwrap();
    let mut remote_args = vec!["--on", "fakehost"];
    remote_args.extend(args);
    let out = rig.run(&rig.repo(), &remote_args);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), stdout(&here));
}

/// Anything that keeps the command from running on the host runs it here,
/// saying why, and nothing ran there: the host down (ssh's 255), the host's
/// preflight saying no, a folder outside any git checkout, no runner set up,
/// or the runner turned off.
#[test]
fn a_run_the_host_cant_take_runs_here_instead() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Remote::new();
    let repo = rig.repo();
    // Only this Mac has RIG_LOCAL_ONLY: the host gets none of its variables.
    let here = "echo \"local=${RIG_LOCAL_ONLY:-unset}\"; exit 5";

    let out = rig.run(&repo, &["--on", "down", "sh", "-c", here]);
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(stdout(&out), "local=this Mac only\n", "ran here");
    let said = stderr(&out);
    assert!(
        said.contains("down can't be reached (ssh: connect to host down port 22: Connection refused); running on this Mac instead"),
        "{said}"
    );
    // What to check for that error (overnight N2).
    assert!(
        said.contains(
            "hollow-slot: it is on but not taking SSH connections: Remote Login is off there"
        ),
        "{said}"
    );
    assert!(
        said.contains("got a build slot; running it now"),
        "in one of this Mac's slots: {said}"
    );

    let preflight = rig.path().join("host-bin").join("hollow-run-preflight");
    std::fs::write(&preflight, REFUSING_PREFLIGHT).unwrap();
    std::fs::set_permissions(&preflight, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = rig.run(&repo, &["--on", "fakehost", "sh", "-c", here]);
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(stdout(&out), "local=this Mac only\n", "ran here");
    assert!(
        stderr(&out).contains("fakehost isn't taking work now (hollow-run-preflight: not taking work: only 3 GB free); running on this Mac instead"),
        "{}",
        stderr(&out)
    );
    assert!(rig.host_work().is_empty(), "nothing ran on the host");
    std::fs::remove_file(&preflight).unwrap();

    let outside = tempfile::Builder::new()
        .prefix("no-git-")
        .tempdir()
        .unwrap();
    let out = rig.run(outside.path(), &["--on", "fakehost", "true"]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("isn't in a git checkout")
            && stderr(&out).contains("running on this Mac instead"),
        "{}",
        stderr(&out)
    );

    let out = rig.run(&repo, &["--remote", "true"]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("no remote runner is set up"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, &["--remote", "true"])
        .env("HOLLOW_SLOT_REMOTE", "off")
        .output_within()
        .unwrap();
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("the remote runner is turned off (off)"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(&repo, &["--on", "-oProxyCommand=x", "true"]);
    assert!(
        stderr(&out).contains("is not an SSH host name"),
        "{}",
        stderr(&out)
    );
    assert!(rig.host_work().is_empty(), "still nothing ran on the host");

    // Usage: a dev server stays here, --fetch needs a runner and a path
    // inside the checkout, and never a .git (the host's would replace this
    // repository).
    for args in [
        &["--remote", "--dev-server", "true"][..],
        &["--fetch", "out", "true"][..],
        &["--on", "fakehost", "--fetch", "../out", "true"][..],
        &["--on", "fakehost", "--fetch", "/tmp/out", "true"][..],
        &["--on", "fakehost", "--fetch", ".git", "true"][..],
        &["--on", "fakehost", "--fetch", "sub/.git/", "true"][..],
        &["--on", "fakehost", "--fetch", ".GIT", "true"][..],
        &["--on", "fakehost", "--fetch", "sub/.Git/x", "true"][..],
        &["--remote"][..],
    ] {
        assert_eq!(rig.run(&repo, args).status.code(), Some(2), "{args:?}");
    }
    assert!(repo.join(".git").join("HEAD").exists());
}

/// The runner set up in `~/.config/hollow/slot-remote` (comments and blank
/// lines skipped) is the one `--remote` uses, and a plain build then says
/// where it runs and why (smart routing, 2026-11-03; before, a build here
/// named `--remote`).
#[test]
fn the_configured_runner_is_used_and_named() {
    let rig = Remote::new();
    let config = rig.path().join("home").join(".config").join("hollow");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("slot-remote"),
        "# the runner, as runner\n\n  fakehost  # its alias\n",
    )
    .unwrap();
    let out = rig.run(&rig.repo(), &["--remote", "pwd"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("/hollow-remote/work/"),
        "{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("running on fakehost"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(&rig.repo(), &["true"]);
    assert!(
        stderr(&out).contains(
            "hollow-slot: on this Mac (true: no timing for it yet, so it runs here this time)"
        ),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("hollow-slot --remote <command>"),
        "the routing line replaces the old hint: {}",
        stderr(&out)
    );
    // HOLLOW_SLOT_REMOTE wins over the file.
    let out = rig
        .cmd(&rig.repo(), &["--remote", "true"])
        .env("HOLLOW_SLOT_REMOTE", "down")
        .output_within()
        .unwrap();
    assert!(
        stderr(&out).contains("down can't be reached"),
        "{}",
        stderr(&out)
    );
}

/// `--fetch` copies what the command left on the host back over this
/// worktree's (paths are the command's folder's), and a gate record the
/// command wrote for a clean tree comes back to this checkout's gate records;
/// one for another commit, or from a tree with uncommitted changes, doesn't.
#[test]
fn fetched_paths_and_a_clean_trees_gate_record_come_back() {
    let rig = Remote::new();
    let repo = rig.repo();
    let common = repo.join(".git");
    // A report folder is ignored, like test-results/: an old one here never
    // goes to the host, so the one that comes back replaces it whole.
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nreport/\n").unwrap();
    std::fs::create_dir_all(repo.join("sub").join("report")).unwrap();
    std::fs::write(repo.join("sub").join("report").join("old.txt"), "stale").unwrap();
    let write = "c=$(git rev-parse --path-format=absolute --git-common-dir)/hollow-gate-records; mkdir -p \"$c\"; \
                 printf '{\\n  \"commit\": \"%s\"\\n}\\n' \"$(git rev-parse HEAD)\" > \"$c/$(printf %064d 0 | tr 0 a).json\"; \
                 printf '{\\n  \"commit\": \"%s\"\\n}\\n' 0123 > \"$c/$(printf %064d 0 | tr 0 b).json\"; \
                 mkdir -p report; echo fresh > report/new.txt; echo one > one.txt";
    let records = common.join("hollow-gate-records");
    let good = records.join(format!("{}.json", "a".repeat(64)));
    let other = records.join(format!("{}.json", "b".repeat(64)));

    // Uncommitted changes: the files come back, the record doesn't.
    std::fs::write(repo.join("kept.txt"), "edited\n").unwrap();
    let out = rig.run(
        &repo.join("sub"),
        &[
            "--on", "fakehost", "--fetch", "report", "--fetch", "one.txt", "--fetch", "missing",
            "sh", "-c", write,
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let sub = repo.join("sub");
    assert_eq!(
        std::fs::read_to_string(sub.join("report").join("new.txt")).unwrap(),
        "fresh\n"
    );
    assert!(
        !sub.join("report").join("old.txt").exists(),
        "replaced, not merged"
    );
    assert_eq!(
        std::fs::read_to_string(sub.join("one.txt")).unwrap(),
        "one\n"
    );
    assert!(
        stderr(&out).contains("sub/missing isn't on"),
        "{}",
        stderr(&out)
    );
    assert!(!good.exists(), "no record for a tree that isn't committed");

    // Clean: the record for HEAD comes back, the other one doesn't.
    std::fs::remove_dir_all(sub.join("report")).unwrap();
    std::fs::remove_file(sub.join("one.txt")).unwrap();
    rig.commit(&repo, "second");
    let out = rig.run(&repo, &["--on", "fakehost", "--", "sh", "-c", write]);
    assert!(out.status.success(), "{}", stderr(&out));
    let head = rig.git(&repo, &["rev-parse", "HEAD"]);
    assert!(
        std::fs::read_to_string(&good).unwrap().contains(&head),
        "the record for this tree came back"
    );
    assert!(!other.exists(), "not one for another commit");
    assert!(
        stderr(&out).contains("the gate record for this tree came back"),
        "{}",
        stderr(&out)
    );
    assert!(
        rig.git(&repo, &["status", "--porcelain"]).is_empty(),
        "nothing else changed here"
    );

    // Never through a symlinked folder here: `linked` points outside the
    // checkout (ignored, so the host makes a real folder of its own).
    let outside = rig.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nreport/\nlinked\n").unwrap();
    rig.commit(&repo, "third");
    std::os::unix::fs::symlink(&outside, repo.join("linked")).unwrap();
    let out = rig.run(
        &repo,
        &[
            "--on",
            "fakehost",
            "--fetch",
            "linked/x.txt",
            "sh",
            "-c",
            "mkdir -p linked; echo x > linked/x.txt",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("not copying linked/x.txt back: a folder on its way here is a symlink"),
        "{}",
        stderr(&out)
    );
    assert!(!outside.join("x.txt").exists(), "nothing written outside");
}

/// `--fetch` brings back only what this run wrote (2026-11-03): an ignored
/// folder stays on the host between runs (`git clean -d` keeps it), so a run
/// that never wrote it (a build that failed first) brought back the earlier
/// run's copy and said "copied ... back". A path inside another fetched path
/// goes with it: copied back after it, the folder replaced it and lost it.
#[test]
fn only_what_this_run_wrote_comes_back_and_nested_fetches_keep_their_files() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nout/\n").unwrap();
    rig.commit(&repo, "ignore out");
    let read = |path: &str| std::fs::read_to_string(repo.join(path)).unwrap();
    let write = "mkdir -p out; echo report > out/report.json; echo log > out/log.txt";
    let out = rig.run(
        &repo,
        &[
            "--on",
            "fakehost",
            "--fetch",
            "out/report.json",
            "--fetch",
            "out",
            "--fetch",
            "out/",
            "sh",
            "-c",
            write,
        ],
    );
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(read("out/report.json"), "report\n", "{said}");
    assert_eq!(read("out/log.txt"), "log\n");
    assert_eq!(said.matches("copied out back").count(), 1, "{said}");
    assert!(!said.contains("copied out/report.json back"), "{said}");

    // A run that writes nothing there: the host's copy is the earlier run's.
    std::fs::remove_dir_all(repo.join("out")).unwrap();
    let out = rig.run(
        &repo,
        &[
            "--on",
            "fakehost",
            "--fetch",
            "out",
            "--fetch",
            "out/report.json",
            "sh",
            "-c",
            "exit 3",
        ],
    );
    let said = stderr(&out);
    assert_eq!(out.status.code(), Some(3), "{said}");
    assert!(
        !repo.join("out").exists(),
        "the stale copy stays there: {said}"
    );
    assert!(
        said.contains("hollow-slot: out on ")
            && said.contains(
                "is older than this run (the command didn't write it), so it wasn't copied back"
            ),
        "{said}"
    );
    assert!(!said.contains("copied out"), "{said}");

    // Anything in it written by this run brings the folder back whole, even
    // with an old modification time (tar -x, cp -p): ctime says it changed.
    let touch = "echo new > out/log.txt; touch -t 202001010000 out/log.txt out";
    let out = rig.run(
        &repo,
        &["--on", "fakehost", "--fetch", "out", "sh", "-c", touch],
    );
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(read("out/log.txt"), "new\n", "{said}");
    assert_eq!(read("out/report.json"), "report\n");
}

/// `--fetch` and `--send` paths are read in one spelling (2026-11-03):
/// repeated slashes squeezed, a leading `./` and a trailing slash dropped.
/// The list was tidied as typed, so `a//b/c` beside `a/b` stayed in it, came
/// back after the folder and replaced it with a copy holding only `c`.
#[test]
fn fetch_paths_are_tidied_in_one_spelling() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join(".gitignore"), "ignored.txt\na/\n").unwrap();
    rig.commit(&repo, "ignore a");
    let write = "mkdir -p a/b; echo c > a/b/c; echo d > a/b/d";
    let out = rig.run(
        &repo,
        &[
            "--on", "fakehost", "--fetch", "a//b/c", "--fetch", "./a/b/", "--fetch", ".//a//b",
            "sh", "-c", write,
        ],
    );
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(
        std::fs::read_to_string(repo.join("a/b/c")).unwrap(),
        "c\n",
        "{said}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a/b/d")).unwrap(),
        "d\n",
        "{said}"
    );
    assert_eq!(said.matches("copied a/b back").count(), 1, "{said}");
    assert!(!said.contains("copied a/b/c back"), "{said}");
    assert!(!said.contains("a//b"), "{said}");
    // A path that is only ./ and slashes is no path; ../ stays refused.
    for bad in ["./", ".//", "./../x", "a/./b"] {
        let out = rig.run(&repo, &["--on", "fakehost", "--fetch", bad, "true"]);
        assert_eq!(out.status.code(), Some(2), "{bad}: {}", stderr(&out));
    }
}

/// A copy back that fails puts the old copy back whole (2026-11-03): it
/// waits beside the path under a dot name, not in the call's scratch folder
/// (another volume when TMPDIR is, where moving it aside was a full copy),
/// and what a failed move in left is deleted first, so the old folder isn't
/// moved inside it. When even that fails, the call says where it is.
#[test]
fn a_failed_copy_back_restores_the_old_copy_or_says_where_it_is() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nout/\n").unwrap();
    rig.commit(&repo, "ignore out");
    std::fs::create_dir(repo.join("out")).unwrap();
    std::fs::write(repo.join("out").join("old.txt"), "old\n").unwrap();
    // An mv that fails to move the new copy in after writing part of it, and
    // with FAKE_MV_KEEP set, fails to put the old one back too.
    let fake = rig.path().join("fake-mv");
    std::fs::create_dir(&fake).unwrap();
    make_executable(
        &fake.join("mv"),
        "#!/bin/sh\ncase \"$1\" in\n  */back/wt/*) mkdir -p \"$2\" && echo part > \"$2/part.txt\"; echo 'mv: no room' >&2; exit 1 ;;\n  *.hollow-slot-old.*) [ -z \"$FAKE_MV_KEEP\" ] || { echo 'mv: refused' >&2; exit 1; } ;;\nesac\nexec /bin/mv \"$@\"\n",
    );
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", fake.display());
    let write = "mkdir -p out; echo new > out/new.txt";
    let args = ["--on", "fakehost", "--fetch", "out", "sh", "-c", write];
    let out = rig
        .cmd(&repo, &args)
        .env("PATH", &path)
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("couldn't copy out back from fakehost"),
        "{said}"
    );
    assert!(!said.contains("nor put the old copy back"), "{said}");
    let mut names: Vec<String> = std::fs::read_dir(repo.join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["old.txt"], "the old copy back alone: {said}");
    assert_eq!(
        std::fs::read_to_string(repo.join("out/old.txt")).unwrap(),
        "old\n"
    );
    let aside = |repo: &Path| -> Vec<String> {
        std::fs::read_dir(repo)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("hollow-slot-old"))
            .collect()
    };
    assert!(aside(&repo).is_empty(), "nothing left aside: {said}");

    let out = rig
        .cmd(&repo, &args)
        .env("PATH", &path)
        .env("FAKE_MV_KEEP", "1")
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    let left = aside(&repo);
    assert_eq!(left.len(), 1, "{said}");
    assert!(left[0].starts_with(".out.hollow-slot-old."), "{left:?}");
    let kept = repo.join(&left[0]);
    // git names the checkout by its real path (/private/var on macOS).
    assert!(
        said.contains(
            "couldn't copy out back from fakehost, nor put the old copy back: it is at /"
        ) && said.contains(&format!("/repo/{}\n", left[0])),
        "{said}"
    );
    assert_eq!(
        std::fs::read_to_string(kept.join("old.txt")).unwrap(),
        "old\n"
    );
    assert!(!repo.join("out").exists(), "the part copy is gone: {said}");
}

/// A name the old copy would wait under that is already taken is skipped,
/// not emptied (2026-11-03): it can hold an old copy an earlier run said it
/// couldn't put back. The old copy this run set aside is deleted once the
/// swap is over, so nothing of its own is left beside the path.
#[test]
fn a_copy_back_skips_a_taken_aside_name_and_deletes_its_own_old_copy() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(
        repo.join(".gitignore"),
        "ignored.txt\nout/\n.out.hollow-slot-old.*\n",
    )
    .unwrap();
    rig.commit(&repo, "ignore out");
    std::fs::create_dir(repo.join("out")).unwrap();
    std::fs::write(repo.join("out").join("old.txt"), "old\n").unwrap();
    let args = [
        "--on",
        "fakehost",
        "--fetch",
        "out",
        "sh",
        "-c",
        "mkdir -p out; echo new > out/new.txt",
    ];
    // The call keeps this shell's pid (exec), so the first name it tries,
    // .out.hollow-slot-old.<pid>.1, is taken before it starts.
    let base = rig.cmd(&repo, &args);
    let mut cmd = Command::new("/bin/sh");
    for (k, v) in base.get_envs() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let out = cmd
        .current_dir(&repo)
        .arg("-c")
        .arg("d=.out.hollow-slot-old.$$.1; mkdir \"$d\" && echo earlier > \"$d/old.txt\" && exec /bin/sh \"$0\" \"$@\"")
        .arg(rig.path().join("hollow-slot"))
        .args(args)
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert!(said.contains("copied out back from fakehost"), "{said}");
    assert_eq!(
        std::fs::read_to_string(repo.join("out/new.txt")).unwrap(),
        "new\n"
    );
    assert!(!repo.join("out/old.txt").exists(), "replaced: {said}");
    let left: Vec<String> = std::fs::read_dir(&repo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("hollow-slot-old"))
        .collect();
    assert_eq!(left.len(), 1, "only the earlier one: {left:?} {said}");
    assert!(left[0].ends_with(".1"), "{left:?}");
    assert_eq!(
        std::fs::read_to_string(repo.join(&left[0]).join("old.txt")).unwrap(),
        "earlier\n"
    );
}

/// A `git status` that fails here is no clean tree (2026-11-03): read as one,
/// HEAD went to the host alone and the uncommitted changes were left out.
#[test]
fn a_failing_git_status_runs_here_instead_of_sending_head_alone() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join("kept.txt"), "edited, not committed\n").unwrap();
    std::fs::write(repo.join(".git").join("index"), "not an index").unwrap();
    let out = rig.run(&repo, &["--on", "fakehost", "sh", "-c", "cat kept.txt"]);
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(stdout(&out), "edited, not committed\n", "ran here: {said}");
    assert!(
        said.contains("git status failed here, so the uncommitted changes can't be told (")
            && said.contains("running on this Mac instead"),
        "{said}"
    );
    assert!(!said.contains("running on fakehost"), "{said}");
}

/// The host runs branch code, so a fetched path that is a symlink there, or
/// a folder holding a link that could point out of it (absolute, or with a
/// `..` part), never comes back (2026-11-03): `out.json -> ~/.zshrc` would
/// have been installed here. Links that stay inside a folder come back.
#[test]
fn fetched_symlinks_that_could_point_out_never_come_back() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nout*\n").unwrap();
    rig.commit(&repo, "ignore out");
    let script = "ln -s \"$HOME/.zshrc\" out.json; mkdir -p outa/in outb outc; echo a > outa/in/a.txt; \
                  ln -s in/a.txt outa/link; ln -s ../../kept.txt outb/up; ln -s /etc/hosts outc/abs";
    let out = rig.run(
        &repo,
        &[
            "--on", "fakehost", "--fetch", "out.json", "--fetch", "outa", "--fetch", "outb",
            "--fetch", "outc", "sh", "-c", script,
        ],
    );
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("not copying out.json back: it is a symlink on fakehost"),
        "{said}"
    );
    for name in ["outb", "outc"] {
        assert!(
            said.contains(&format!(
                "not copying {name} back: it holds a symlink that points out of it"
            )),
            "{said}"
        );
        assert!(!repo.join(name).exists(), "{name}");
    }
    assert!(std::fs::symlink_metadata(repo.join("out.json")).is_err());
    assert_eq!(
        std::fs::read_link(repo.join("outa").join("link")).unwrap(),
        Path::new("in/a.txt")
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("outa").join("link")).unwrap(),
        "a\n"
    );
}

/// A worktree folder named with a leading dot gets a host folder named
/// with `_` instead (2026-11-03): `ls` and the `*.used` globs skip dot
/// names, so such a folder was never pruned. One a host tool from before
/// named with a dot is pruned too.
#[test]
fn a_dot_named_worktree_gets_a_host_folder_that_is_pruned() {
    let rig = Remote::new();
    let repo = rig.repo();
    let dotted = rig.path().join(".dotted");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &dotted.to_string_lossy(),
            "HEAD",
        ],
    );
    let out = rig.run(&dotted, &["--on", "fakehost", "true"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let ids = rig.host_work();
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert!(ids[0].starts_with("_dotted-"), "{ids:?}");
    let work = rig.host_root().join("work");
    let legacy = work.join(".legacy-0123456789ab");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(work.join(".legacy-0123456789ab.used"), "").unwrap();
    // Keep none: every other worktree's folder goes.
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "true"])
        .env("FAKE_KEEP", "0")
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    until("the dot-named worktree's folders are dropped", || {
        !work.join(&ids[0]).exists()
            && !work.join(format!("{}.used", ids[0])).exists()
            && !legacy.exists()
            && !work.join(".legacy-0123456789ab.used").exists()
    });
    let left = rig.host_work();
    assert!(left.len() == 1 && left[0].starts_with("repo-"), "{left:?}");
}

/// A new build folder is cloned only from the same repository's folders on
/// the host (its id in `<id>.used`), and only for a checkout that tracks a
/// Cargo.toml (2026-11-03): before, the newest folder of any repository
/// seeded it, Rust or not.
#[test]
fn a_build_folder_is_seeded_only_from_the_same_repository_and_for_rust() {
    let rig = Remote::new();
    let repo = rig.repo();
    let no_rust = rig.git(&repo, &["rev-parse", "HEAD"]);
    with_cargo_toml(&rig);
    let build = "mkdir -p \"$CARGO_TARGET_DIR\"; [ -e \"$CARGO_TARGET_DIR/built\" ] && echo seeded; echo x > \"$CARGO_TARGET_DIR/built\"";
    let run = |cwd: &Path| {
        let out = rig.run(cwd, &["--on", "fakehost", "sh", "-c", build]);
        assert!(out.status.success(), "{}", stderr(&out));
        out
    };
    let add = |name: &str, at: &str| {
        let tree = rig.path().join(name);
        rig.git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                &tree.to_string_lossy(),
                at,
            ],
        );
        tree
    };
    assert_eq!(stdout(&run(&repo)), "");
    // This repository at a commit with no Cargo.toml: no clone.
    let out = run(&add("no-rust", &no_rust));
    assert_eq!(stdout(&out), "", "{}", stderr(&out));
    assert!(!stderr(&out).contains("as a clone of"), "{}", stderr(&out));
    // Another repository with Rust of its own: none of this one's.
    let other = rig.path().join("other-repo");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("Cargo.toml"), "[package]\nname = \"o\"\n").unwrap();
    rig.git(&other, &["init", "-q", "-b", "main"]);
    rig.commit(&other, "other");
    let out = run(&other);
    assert_eq!(stdout(&out), "", "{}", stderr(&out));
    assert!(!stderr(&out).contains("as a clone of"), "{}", stderr(&out));
    // A worktree of this one that tracks one: the newest of this
    // repository's folders, past the other repository's (used since).
    let out = run(&add("rusty", "HEAD"));
    assert_eq!(stdout(&out), "seeded\n", "{}", stderr(&out));
    assert!(
        stderr(&out).contains("as a clone of no-rust-"),
        "{}",
        stderr(&out)
    );
}

/// Whatever the command leaves running is ended when it exits: it would
/// hold the session's output, so the call would wait for it.
#[test]
fn what_a_command_leaves_running_never_holds_the_call() {
    let rig = Remote::new();
    let pid_file = rig.path().join("left-pid");
    let script = format!("sleep 41 & echo $! > '{}'; echo done", pid_file.display());
    let started = Instant::now();
    let out = rig.run(&rig.repo(), &["--on", "fakehost", "sh", "-c", &script]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "done\n");
    // The leftover sleeps 41 s: a call that waited for it would take that long.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the call didn't wait for the leftover"
    );
    let left = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    until("the leftover is ended", || !alive(&left));
}

/// A host folder that is broken (or another repository's) is made again
/// rather than failing every call, and a linked worktree's uncommitted
/// changes travel too. Its own index is the snapshot's start (found through
/// `--git-path`): a tracked file that `.gitignore` matches would be left out
/// of a snapshot built from an empty index.
#[test]
fn a_broken_host_folder_is_made_again() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join("ignored.txt"), "tracked anyway\n").unwrap();
    rig.git(&repo, &["add", "-f", "ignored.txt"]);
    rig.commit(&repo, "force-add an ignored file");
    let linked = rig.path().join("linked-tree");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &linked.to_string_lossy(),
            "HEAD",
        ],
    );
    std::fs::write(linked.join("kept.txt"), "linked edit\n").unwrap();
    std::fs::write(linked.join("fresh.txt"), "linked new\n").unwrap();
    let show = &[
        "--on",
        "fakehost",
        "cat",
        "kept.txt",
        "fresh.txt",
        "ignored.txt",
    ];
    let out = rig.run(&linked, show);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "linked edit\nlinked new\ntracked anyway\n");
    let ids = rig.host_work();
    assert_eq!(ids.len(), 1);
    std::fs::write(
        rig.host_root().join("work").join(&ids[0]).join(".git"),
        "gitdir: /nowhere\n",
    )
    .unwrap();
    let out = rig.run(&linked, show);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "linked edit\nlinked new\ntracked anyway\n");
    assert!(stderr(&out).contains("making it again"), "{}", stderr(&out));
}

/// Stopping the call stops the command on the host, whether the call gets a
/// TERM (it says so and exits 143) or is killed outright (ssh's stdin ends
/// with it). Two calls from one worktree take turns with its host folder.
#[test]
fn stopping_the_call_stops_the_command_on_the_host() {
    for no_lockf in super::tests::lock_modes() {
        for signal in ["TERM", "KILL"] {
            let rig = Remote::new();
            let repo = rig.repo();
            let pid_file = rig.path().join("host-pid");
            let script = format!("echo $$ > '{}'; exec sleep 43", pid_file.display());
            let mut cmd = rig.cmd(&repo, &["--on", "fakehost", "sh", "-c", &script]);
            if no_lockf {
                cmd.env("FAKE_NO_LOCKF", "1")
                    .env("HOLLOW_SLOT_NO_LOCKF", "1");
            }
            let err = std::fs::File::create(rig.path().join("first.err")).unwrap();
            let mut first = cmd
                .stdout(std::process::Stdio::null())
                .stderr(err)
                .spawn()
                .unwrap();
            until("the command starts on the host", || {
                std::fs::read_to_string(&pid_file)
                    .map(|p| !p.trim().is_empty())
                    .unwrap_or(false)
            });
            let host_pid = std::fs::read_to_string(&pid_file)
                .unwrap()
                .trim()
                .to_string();
            assert!(alive(&host_pid));
            let holders: String = ["slot-0", "slot-1"]
                .iter()
                .map(|s| {
                    std::fs::read_to_string(
                        rig.path()
                            .join("host-slots")
                            .join(format!("{s}.lock.holder")),
                    )
                    .unwrap_or_default()
                })
                .collect();
            assert!(
                holders.contains("sleep 43"),
                "it holds one of the host's slots: {holders}"
            );

            // A second call from the same worktree waits for the folder.
            let mut second_cmd = rig.cmd(&repo, &["--on", "fakehost", "echo", "second"]);
            if no_lockf {
                second_cmd
                    .env("FAKE_NO_LOCKF", "1")
                    .env("HOLLOW_SLOT_NO_LOCKF", "1");
            }
            let second_err = rig.path().join("second.err");
            let second = second_cmd
                .stdout(std::process::Stdio::piped())
                .stderr(std::fs::File::create(&second_err).unwrap())
                .spawn()
                .unwrap();
            until("the second call says it waits", || {
                std::fs::read_to_string(&second_err)
                    .unwrap_or_default()
                    .contains("another run from this checkout is using its folder")
            });

            Command::new("/bin/kill")
                .args([&format!("-{signal}"), &first.id().to_string()])
                .status()
                .unwrap();
            let status = first.wait_within().unwrap();
            until("the command on the host stops", || !alive(&host_pid));
            if signal == "TERM" {
                assert_eq!(status.code(), Some(143), "no_lockf {no_lockf}");
                let said = std::fs::read_to_string(rig.path().join("first.err")).unwrap();
                assert!(
                    said.contains("the command on fakehost was stopped too"),
                    "{said}"
                );
            }
            let out = second.wait_with_output_within().unwrap();
            assert!(out.status.success(), "no_lockf {no_lockf} {signal}");
            assert_eq!(stdout(&out), "second\n", "then the second one ran");
        }
    }
}

/// A call stopped while it waits for its worktree's host folder never runs
/// its command there: once it gets the folder, the host sees the caller is
/// gone and gives up.
#[test]
fn a_call_stopped_while_it_waits_never_runs_its_command() {
    let rig = Remote::new();
    let repo = rig.repo();
    let pid_file = rig.path().join("host-pid");
    let ran = rig.path().join("ran");
    let hold = format!("echo $$ > '{}'; exec sleep 44", pid_file.display());
    let mut first = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", &hold])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until("the first command starts", || {
        std::fs::read_to_string(&pid_file)
            .map(|p| !p.trim().is_empty())
            .unwrap_or(false)
    });
    let host_pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    let queued_err = rig.path().join("queued.err");
    let touch = format!("touch '{}'", ran.display());
    let mut queued = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", &touch])
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&queued_err).unwrap())
        .spawn()
        .unwrap();
    until("the second call waits", || {
        std::fs::read_to_string(&queued_err)
            .unwrap_or_default()
            .contains("another run from this checkout is using its folder")
    });
    Command::new("/bin/kill")
        .args(["-KILL", &queued.id().to_string()])
        .status()
        .unwrap();
    queued.wait_within().unwrap();
    Command::new("/bin/kill")
        .args(["-TERM", &first.id().to_string()])
        .status()
        .unwrap();
    first.wait_within().unwrap();
    until("the first command stops", || !alive(&host_pid));
    // The host side of the stopped call still writes to its stderr file.
    until("the host gives the stopped call up", || {
        std::fs::read_to_string(&queued_err)
            .unwrap_or_default()
            .contains("stopped before the command started")
    });
    assert!(!ran.exists(), "the stopped call's command never ran");
}

/// The rig's repository tracks a Cargo.toml from here on: only a checkout
/// that does gets a build folder cloned from another.
fn with_cargo_toml(rig: &Remote) {
    let repo = rig.repo();
    std::fs::create_dir_all(repo.join("crate")).unwrap();
    std::fs::write(
        repo.join("crate").join("Cargo.toml"),
        "[package]\nname = \"t\"\n",
    )
    .unwrap();
    rig.commit(&repo, "a crate");
}

/// A new worktree's build folder on the host starts as a clone of the newest
/// one of the same repository's there (it tracks a Cargo.toml), and the host
/// keeps only its newest `HOLLOW_REMOTE_KEEP` worktree folders (with their
/// build folders), plus no build folder left without a worktree.
#[test]
fn a_host_keeps_its_newest_worktrees_and_clones_a_build_folder_for_a_new_one() {
    let rig = Remote::new();
    let repo = rig.repo();
    with_cargo_toml(&rig);
    let other = rig.path().join("other-tree");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &other.to_string_lossy(),
            "HEAD",
        ],
    );
    let build = "mkdir -p \"$CARGO_TARGET_DIR\"; [ -e \"$CARGO_TARGET_DIR/built\" ] && echo seeded; echo x > \"$CARGO_TARGET_DIR/built\"";
    let keep_one = |cwd: &Path| {
        let out = rig
            .cmd(cwd, &["--on", "fakehost", "sh", "-c", build])
            .env("FAKE_KEEP", "1")
            .output_within()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        out
    };
    keep_one(&repo);
    let first = rig.host_work();
    assert_eq!(first.len(), 1);
    let out = keep_one(&other);
    assert_eq!(
        stdout(&out),
        "seeded\n",
        "cloned from the first worktree's build folder"
    );
    assert!(stderr(&out).contains("as a clone of"), "{}", stderr(&out));
    let orphan = rig.host_root().join("targets").join("left-behind");
    std::fs::create_dir_all(&orphan).unwrap();
    // The third run is the other worktree's again: past the newest one, the
    // first worktree's folders go, and so does the orphan.
    keep_one(&other);
    let gone = |name: &str| {
        !rig.host_root().join("work").join(name).exists()
            && !rig.host_root().join("targets").join(name).exists()
            && !rig
                .host_root()
                .join("work")
                .join(format!("{name}.port"))
                .exists()
    };
    until("the oldest worktree's folders are dropped", || {
        gone(&first[0])
    });
    until("the orphan build folder is dropped", || !orphan.exists());
    assert_eq!(rig.host_work().len(), 1, "{:?}", rig.host_work());
    until("the trash is emptied", || {
        std::fs::read_dir(rig.host_root().join("trash"))
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    });
}

/// A build folder cloned from another one names its own folder in its
/// build scripts' outputs (cargo writes absolute paths there; tauri's
/// permission lists read them back), so pruning the folder it came from
/// breaks nothing; the outputs keep their times.
#[test]
fn a_cloned_build_folder_names_itself_in_its_build_outputs() {
    let rig = Remote::new();
    let repo = rig.repo();
    with_cargo_toml(&rig);
    let tree = rig.path().join("tree-b");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &tree.to_string_lossy(),
            "HEAD",
        ],
    );
    let write = "o=\"$CARGO_TARGET_DIR/debug/build/t-1/out\"; mkdir -p \"$o\"; printf '%s\\n' \"$o/a.toml\" > \"$o/list.txt\"; { printf '%s\\n' \"$o\"; head -c 70000 /dev/zero | tr '\\000' x; printf 'bin\\000'; } > \"$o/bin\"; touch -t 202001010000 \"$o/list.txt\"";
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", write])
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let read = "cat \"$CARGO_TARGET_DIR/debug/build/t-1/out/list.txt\"; stat -f %Sm -t %Y \"$CARGO_TARGET_DIR/debug/build/t-1/out/list.txt\"; echo \"own=$CARGO_TARGET_DIR\"";
    let out = rig
        .cmd(&tree, &["--on", "fakehost", "sh", "-c", read])
        .output_within()
        .unwrap();
    let said = stdout(&out);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("as a clone of"), "{}", stderr(&out));
    let own = said
        .lines()
        .find_map(|l| l.strip_prefix("own="))
        .unwrap()
        .to_string();
    assert!(own.contains("/targets/tree-b-"), "{said}");
    assert_eq!(
        said.lines().next().unwrap(),
        format!("{own}/debug/build/t-1/out/a.toml"),
        "{said}"
    );
    assert_eq!(said.lines().nth(1).unwrap(), "2020", "{said}");
    // A binary file is left as it was.
    let bin = std::fs::read(format!("{own}/debug/build/t-1/out/bin")).unwrap();
    assert!(!String::from_utf8_lossy(&bin).contains("/targets/tree-b-"));
}

/// A stand-in `df` for the host (`HOLLOW_REMOTE_DF`): each call answers the
/// next line of `<dir>/df.seq` as the free KB (the last line once they run
/// out), counting its calls in `<dir>/df.n`.
pub(crate) fn fake_df(dir: &Path) -> PathBuf {
    let df = dir.join("fake-df");
    make_executable(
        &df,
        &format!(
            "#!/bin/sh\nd='{}'\nn=$(cat \"$d/df.n\" 2>/dev/null || echo 0)\necho $((n + 1)) > \"$d/df.n\"\nkb=$(sed -n \"$((n + 1))p\" \"$d/df.seq\")\n[ -n \"$kb\" ] || kb=$(tail -n 1 \"$d/df.seq\")\nprintf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n/dev/disk3s5 999999999 1 %s 1%% /\\n' \"$kb\"\n",
            dir.display()
        ),
    );
    df
}

/// Set the stand-in `df`'s answers (KB, one per call) and restart its count.
pub(crate) fn df_answers(dir: &Path, kb: &[u64]) {
    let lines: Vec<String> = kb.iter().map(|k| k.to_string()).collect();
    std::fs::write(dir.join("df.seq"), lines.join("\n") + "\n").unwrap();
    let _ = std::fs::remove_file(dir.join("df.n"));
}

const GB: u64 = 1_048_576;

/// Free space on the host (overnight N5): while its disk is under
/// `HOLLOW_REMOTE_MIN_FREE_GB`, a run drops the least recently used worktree
/// folders unused for an hour, with their build folders, saying so, until
/// the disk is back over it; never its own, one another call holds, or one
/// used within the hour; and a remote session's build folder
/// (`targets/session-*`) is never the tool's to drop.
#[test]
fn a_host_under_its_disk_floor_drops_the_least_recently_used_worktrees() {
    let rig = Remote::new();
    let repo = rig.repo();
    let mut trees = vec![repo.clone()];
    for name in ["tree-b", "tree-c", "tree-d"] {
        let tree = rig.path().join(name);
        rig.git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                &tree.to_string_lossy(),
                "HEAD",
            ],
        );
        trees.push(tree);
    }
    let df = fake_df(rig.path());
    let build = "mkdir -p \"$CARGO_TARGET_DIR\"; echo x > \"$CARGO_TARGET_DIR/built\"";
    let run = |cwd: &Path, free: &[u64]| {
        df_answers(rig.path(), free);
        let mut cmd = rig.cmd(cwd, &["--on", "fakehost", "sh", "-c", build]);
        cmd.env("FAKE_MIN_FREE_GB", "40").env("FAKE_DF", &df);
        let out = cmd.output_within().unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        out
    };
    let folder = |prefix: &str| {
        rig.host_work()
            .into_iter()
            .find(|n| n.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix}: {:?}", rig.host_work()))
    };
    // Plenty of room (100 GB): all four stay.
    for tree in &trees {
        run(tree, &[100 * GB]);
    }
    assert_eq!(rig.host_work().len(), 4, "{:?}", rig.host_work());
    // Last used 4, 3 and 2 hours ago (repo, tree-b, tree-c); tree-d just now.
    let work = rig.host_root().join("work");
    for (prefix, hours) in [("repo-", 4), ("tree-b-", 3), ("tree-c-", 2)] {
        let when = (chrono::Local::now() - chrono::Duration::hours(hours))
            .format("%Y%m%d%H%M")
            .to_string();
        let used = work.join(format!("{}.used", folder(prefix)));
        assert!(Command::new("/usr/bin/touch")
            .args(["-t", &when])
            .arg(&used)
            .status()
            .unwrap()
            .success());
    }
    let session = rig.host_root().join("targets").join("session-abc");
    std::fs::create_dir_all(&session).unwrap();
    // Trash a cut-short deletion left an hour ago goes too.
    let stale = rig.host_root().join("trash").join("123");
    std::fs::create_dir_all(stale.join("work-x")).unwrap();
    let hour_ago = (chrono::Local::now() - chrono::Duration::hours(1))
        .format("%Y%m%d%H%M")
        .to_string();
    assert!(Command::new("/usr/bin/touch")
        .args(["-t", &hour_ago])
        .arg(&stale)
        .status()
        .unwrap()
        .success());
    // 1 GB free until one folder has gone (the check, then one per folder):
    // the least recently used goes, and nothing more once there is room.
    let out = run(&trees[3], &[GB, GB, 100 * GB]);
    let said = stderr(&out);
    assert!(said.contains("had 1 GB free, under its 40 GB floor: removed the least recently used worktree folder there (repo-"), "{said}");
    assert_eq!(
        said.matches("removed the least recently used").count(),
        1,
        "{said}"
    );
    assert_eq!(rig.host_work().len(), 3, "{:?}", rig.host_work());
    assert!(!stale.exists(), "old trash stays");
    // 1 GB free and it never rises: every folder unused for an hour goes,
    // except one another call holds (its slot lock); the call's own and
    // tree-d (used just now) stay, and so does the session's folder.
    let held = folder("tree-c-");
    let lock = work.join(format!("{held}.lock"));
    let mut holder = Command::new("/usr/bin/perl")
        .args([
            "-e",
            "use Fcntl ':flock'; open(F, '>>', $ARGV[0]) or die; flock(F, LOCK_EX) or die; $| = 1; print \"held\\n\"; sleep 30",
        ])
        .arg(&lock)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(holder.stdout.as_mut().unwrap()),
        &mut line,
    )
    .unwrap();
    assert_eq!(line, "held\n");
    let out = run(&trees[3], &[GB]);
    let said = stderr(&out);
    assert_eq!(
        said.matches("removed the least recently used").count(),
        1,
        "{said}"
    );
    assert!(said.contains("(tree-b-"), "{said}");
    let left = rig.host_work();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.contains(&held), "{left:?}");
    let _ = holder.kill();
    let _ = holder.wait();
    // Released, it goes at the next call; tree-d, used within the hour, stays.
    let out = run(&trees[1], &[GB]);
    let said = stderr(&out);
    assert_eq!(
        said.matches("removed the least recently used").count(),
        1,
        "{said}"
    );
    let left = rig.host_work();
    assert!(!left.contains(&held), "{left:?}");
    assert!(left.iter().any(|n| n.starts_with("tree-d-")), "{left:?}");
    assert!(session.exists(), "a remote session's build folder stays");
}

/// A host whose preflight refuses (its disk under the preflight's own
/// floor) is pruned once and asked again, so it recovers by itself; the
/// call that asked keeps its folder.
#[test]
fn a_host_whose_preflight_refuses_is_pruned_and_asked_again() {
    let rig = Remote::new();
    let repo = rig.repo();
    let other = rig.path().join("tree-o");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &other.to_string_lossy(),
            "HEAD",
        ],
    );
    let df = fake_df(rig.path());
    df_answers(rig.path(), &[100 * GB]);
    for tree in [&other, &repo] {
        let out = rig
            .cmd(tree, &["--on", "fakehost", "true"])
            .env("FAKE_DF", &df)
            .output_within()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let work = rig.host_root().join("work");
    let used = rig
        .host_work()
        .into_iter()
        .find(|n| n.starts_with("tree-o-"))
        .unwrap();
    let when = (chrono::Local::now() - chrono::Duration::hours(2))
        .format("%Y%m%d%H%M")
        .to_string();
    assert!(Command::new("/usr/bin/touch")
        .args(["-t", &when])
        .arg(work.join(format!("{used}.used")))
        .status()
        .unwrap()
        .success());
    // The host's preflight says no while the other folder is there.
    let bin = rig.path().join("host-bin");
    std::fs::create_dir_all(&bin).unwrap();
    make_executable(
        &bin.join("hollow-run-preflight"),
        &format!(
            "#!/bin/sh\nif [ -d '{}' ]; then echo 'no host=fake free=12GB under 20GB'; exit 1; fi\necho 'ok host=fake free=50GB'\n",
            work.join(&used).display()
        ),
    );
    df_answers(rig.path(), &[12 * GB]);
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", "echo ran-there"])
        .env("FAKE_DF", &df)
        .env("FAKE_MIN_FREE_GB", "40")
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert!(stdout(&out).contains("ran-there"), "{said}");
    assert!(
        said.contains(&format!(
            "removed the least recently used worktree folder there ({used})"
        )),
        "{said}"
    );
    assert!(said.contains("ok host=fake free=50GB"), "{said}");
    assert_eq!(rig.host_work().len(), 1, "{:?}", rig.host_work());
}

fn make_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The `.start` markers of runs under way on the host.
fn starting(rig: &Remote) -> usize {
    std::fs::read_dir(rig.host_root().join("work"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".start"))
                .count()
        })
        .unwrap_or(0)
}

/// Memory floor (2026-11-01): a run here, a fallback included, doesn't start
/// while less than 30% of memory is free. Once it holds its slot it waits,
/// saying so once, starts once memory is back, and refuses with 69 (freeing
/// the slot) after more than `HOLLOW_SLOT_MEMORY_WAIT` seconds under the
/// floor; a dev server the same. Time queued behind both slots doesn't count.
/// The reading falls back to `memory_pressure -Q` where `sysctl` is refused
/// (Codex's sandbox). A floor of 0, or no reading, never waits. Stand-ins for
/// `sysctl` and `memory_pressure` read the level from files.
#[test]
fn a_run_here_waits_for_memory_and_refuses_after_its_wait() {
    let rig = Remote::new();
    let repo = rig.repo();
    let mem = rig.path().join("mem");
    std::fs::create_dir(&mem).unwrap();
    let sysctl = mem.join("sysctl");
    make_executable(&sysctl, "#!/bin/sh\ncat \"$(dirname \"$0\")/level\"\n");
    let pressure = mem.join("memory_pressure");
    make_executable(
        &pressure,
        "#!/bin/sh\necho 'The system has 1 (1 pages).'\necho \"System-wide memory free percentage: $(cat \"$(dirname \"$0\")/pressure-level\")%\"\n",
    );
    let level = mem.join("level");
    std::fs::write(&level, "12\n").unwrap();
    std::fs::write(mem.join("pressure-level"), "90").unwrap();
    let ran = rig.path().join("ran");
    let touch = format!("touch '{}'", ran.display());
    let floor = |args: &[&str], wait: &str| {
        let mut cmd = rig.cmd(&repo, args);
        cmd.env_remove("HOLLOW_SLOT_MIN_FREE_PCT")
            .env("HOLLOW_SLOT_SYSCTL", &sysctl)
            .env("HOLLOW_SLOT_MEMORY_PRESSURE", &pressure)
            .env("HOLLOW_SLOT_MEMORY_WAIT", wait);
        cmd
    };

    let started = Instant::now();
    let out = floor(&["sh", "-c", &touch], "2").output_within().unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(started.elapsed() >= Duration::from_secs(2), "it waited");
    let said = stderr(&out);
    assert!(
        said.contains("12% of its memory free, under its 30% floor for builds"),
        "{said}"
    );
    assert!(
        said.contains("not starting it:") && said.contains("for over 2 s (now 12%)"),
        "{said}"
    );
    assert!(
        !said.contains("hollow-slot --remote"),
        "no runner set up, so none suggested: {said}"
    );
    assert_eq!(said.matches("waiting:").count(), 1, "said once: {said}");
    assert!(!ran.exists(), "refused: the command never ran");

    let out = floor(&["--dev-server", "sh", "-c", &touch], "1")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    let out = floor(&["--on", "down", "sh", "-c", &touch], "1")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("running on this Mac instead"),
        "a fallback too: {}",
        stderr(&out)
    );
    assert!(!ran.exists());

    // sysctl refused (Codex's sandbox): memory_pressure's reading counts.
    make_executable(
        &sysctl,
        "#!/bin/sh\necho 'Operation not permitted' >&2\nexit 1\n",
    );
    std::fs::write(mem.join("pressure-level"), "9").unwrap();
    let out = floor(&["sh", "-c", &touch], "1").output_within().unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("9% of its memory free"),
        "{}",
        stderr(&out)
    );
    make_executable(&sysctl, "#!/bin/sh\ncat \"$(dirname \"$0\")/level\"\n");
    std::fs::write(mem.join("pressure-level"), "90").unwrap();

    // Queued behind both slots, the wait doesn't run: memory comes back after
    // longer than the wait, while the call still waits for a slot, and it runs.
    let slots = rig.path().join("slots");
    std::fs::create_dir_all(&slots).unwrap();
    // Each holder says when it has its lock, then holds it 5 s.
    let hold = |n: u8| {
        Command::new("/usr/bin/perl")
            .args([
                "-MFcntl=:flock",
                "-e",
                "open(my $f, '>>', $ARGV[0]) or die; flock($f, LOCK_EX) or die; open(my $r, '>', $ARGV[1]) or die; close $r; sleep 5",
            ])
            .arg(slots.join(format!("slot-{n}.lock")))
            .arg(rig.path().join(format!("held-{n}")))
            .spawn()
            .unwrap()
    };
    let mut holders = vec![hold(0), hold(1)];
    until("both slots are held", || {
        rig.path().join("held-0").exists() && rig.path().join("held-1").exists()
    });
    let queued_err = rig.path().join("queued.err");
    let mut queued = floor(&["sh", "-c", &touch], "1")
        .env("HOLLOW_SLOT_NO_LOCKF", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&queued_err).unwrap())
        .spawn()
        .unwrap();
    // Well past the 1 s wait (the old design refused at about 2 s), still
    // before the holders let go.
    std::thread::sleep(Duration::from_millis(3500));
    assert!(!ran.exists(), "still queued");
    std::fs::write(&level, "64\n").unwrap();
    assert!(
        queued.wait_within().unwrap().success(),
        "{}",
        std::fs::read_to_string(&queued_err).unwrap_or_default()
    );
    assert!(ran.exists(), "it ran once it had a slot");
    for holder in &mut holders {
        holder.wait_within().unwrap();
    }
    std::fs::remove_file(&ran).unwrap();
    std::fs::write(&level, "12\n").unwrap();

    // A Rust build waits at the cargo lock (holding both locks), is refused
    // the same way, and leaves both free; the variable never reaches a command.
    let out = floor(&["sh", "-c", &touch], "1")
        .env("HOLLOW_SLOT_CARGO", "1")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(!ran.exists());
    std::fs::write(&level, "64\n").unwrap();
    let show = "echo \"mem=${HOLLOW_SLOT_MEM-unset}\"";
    for cargo in ["0", "1"] {
        let out = floor(&["sh", "-c", show], "1")
            .env("HOLLOW_SLOT_CARGO", cargo)
            .output_within()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stdout(&out), "mem=unset\n", "cargo {cargo}");
    }
    std::fs::write(&level, "12\n").unwrap();

    // Memory comes back while it waits: it starts.
    let err = rig.path().join("wait.err");
    let mut waiting = floor(&["sh", "-c", &touch], "60")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&err).unwrap())
        .spawn()
        .unwrap();
    until("it says it waits", || {
        std::fs::read_to_string(&err)
            .unwrap_or_default()
            .contains("waiting:")
    });
    assert!(!ran.exists());
    std::fs::write(&level, "64\n").unwrap();
    assert!(waiting.wait_within().unwrap().success());
    assert!(ran.exists(), "then it ran");

    std::fs::write(&level, "5\n").unwrap();
    let out = floor(&["true"], "60")
        .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
        .output_within()
        .unwrap();
    assert!(out.status.success());
    assert!(!stderr(&out).contains("waiting:"), "{}", stderr(&out));
    std::fs::write(&level, "unknown\n").unwrap();
    std::fs::write(mem.join("pressure-level"), "unknown").unwrap();
    let out = floor(&["true"], "60").output_within().unwrap();
    assert!(out.status.success());
    assert!(!stderr(&out).contains("waiting:"), "{}", stderr(&out));
}

/// The host looks again right before the command starts: getting the folder
/// ready (here a slow `npm ci`) can take minutes, and a call stopped
/// meanwhile never starts its command.
#[test]
fn a_call_stopped_while_its_folder_is_made_ready_never_runs_its_command() {
    let rig = Remote::new();
    let repo = rig.repo();
    std::fs::write(repo.join("package-lock.json"), "{}\n").unwrap();
    rig.commit(&repo, "a lock file");
    let npm_started = rig.path().join("npm-started");
    make_executable(
        &rig.path().join("host-bin").join("npm"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nsleep 2\nmkdir -p node_modules\n",
            npm_started.display()
        ),
    );
    let ran = rig.path().join("ran");
    let err = rig.path().join("call.err");
    let touch = format!("touch '{}'", ran.display());
    let mut call = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", &touch])
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&err).unwrap())
        .spawn()
        .unwrap();
    until("npm ci starts on the host", || npm_started.exists());
    Command::new("/bin/kill")
        .args(["-KILL", &call.id().to_string()])
        .status()
        .unwrap();
    call.wait_within().unwrap();
    until("the host gives the stopped call up", || {
        std::fs::read_to_string(&err)
            .unwrap_or_default()
            .contains("stopped before the command started on")
    });
    let said = std::fs::read_to_string(&err).unwrap();
    assert!(said.contains("while its folder was made ready"), "{said}");
    assert!(!ran.exists(), "the stopped call's command never ran");
}

/// The watchdog waits for the command's process group before signalling it:
/// a stop that comes while the command is still starting (held here 1.5 s
/// before perl's `setpgrp` by a module every perl on the host loads) still
/// ends it at once, not at the KILL 10 s later.
#[test]
fn a_stop_while_the_command_starts_still_reaches_its_group() {
    let rig = Remote::new();
    let repo = rig.repo();
    let lib = rig.path().join("perl-lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(
        lib.join("HollowSlowStart.pm"),
        "package HollowSlowStart;\nselect(undef, undef, undef, 1.5);\n1;\n",
    )
    .unwrap();
    let pid_file = rig.path().join("host-pid");
    let script = format!("echo $$ > '{}'; exec sleep 46", pid_file.display());
    let mut call = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", &script])
        .env("FAKE_PERL5LIB", &lib)
        .env("FAKE_PERL5OPT", "-MHollowSlowStart")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until("the command is being started", || starting(&rig) > 0);
    let stopped = Instant::now();
    Command::new("/bin/kill")
        .args(["-KILL", &call.id().to_string()])
        .status()
        .unwrap();
    call.wait_within().unwrap();
    until("the run on the host ends", || starting(&rig) == 0);
    assert!(
        stopped.elapsed() < Duration::from_secs(8),
        "the TERM reached the group, not the KILL 10 s later: {:?}",
        stopped.elapsed()
    );
    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        until("no command is left", || !alive(pid.trim()));
    }
}

/// Each worktree folder on the host keeps a port of its own: one no other
/// folder there holds (by hash alone, two shared one about once in 400), the
/// same one run after run, given to the command as PORT and PW_PORT.
#[test]
fn each_host_worktree_keeps_a_port_no_other_one_has() {
    let rig = Remote::new();
    let repo = rig.repo();
    let work = rig.host_root().join("work");
    std::fs::create_dir_all(&work).unwrap();
    // Every port but 2345 belongs to another folder.
    for port in 2300..2700u16 {
        if port != 2345 {
            std::fs::write(work.join(format!("other-{port}.port")), format!("{port}\n")).unwrap();
        }
    }
    let show = &["--on", "fakehost", "sh", "-c", "echo \"$PORT $PW_PORT\""];
    let out = rig.run(&repo, show);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2345 2345\n");
    let ids = rig.host_work();
    assert_eq!(ids.len(), 1);
    assert_eq!(
        std::fs::read_to_string(work.join(format!("{}.port", ids[0])))
            .unwrap()
            .trim(),
        "2345"
    );
    let out = rig.run(&repo, show);
    assert_eq!(stdout(&out), "2345 2345\n", "kept run after run");
}

/// A gate's run record (`scripts/gate-run.mjs`) comes back to this
/// checkout's gate history whether the tree was clean or not, so gate-stats
/// here counts gates run there; a name gate-run.mjs wouldn't write doesn't.
/// The command gets this checkout's own HEAD (`HOLLOW_REMOTE_HEAD`, never a
/// snapshot's) and whether it had uncommitted changes.
#[test]
fn a_gates_run_record_comes_back_clean_tree_or_not() {
    let rig = Remote::new();
    let repo = rig.repo();
    let runs = repo.join(".git").join("hollow-gate-runs");
    let good = "2026-11-01T22-15-03-123Z-0123abcd.json";
    let others = "2026-11-01T22-15-04-000Z-0123abcd.json";
    // Records name their checkout (gate-run.mjs's `checkout`); one from
    // another worktree on the host, which shares the repository, stays there.
    let write = format!(
        "c=$(git rev-parse --path-format=absolute --git-common-dir)/hollow-gate-runs; mkdir -p \"$c\"; \
         printf '{{\"commit\": \"%s\", \"dirty\": %s, \"checkout\": \"%s\"}}\\n' \"$HOLLOW_REMOTE_HEAD\" \"$HOLLOW_REMOTE_DIRTY\" \"$PWD\" > \"$c/{good}\"; \
         printf '{{\"checkout\": \"/elsewhere\"}}\\n' > \"$c/{others}\"; \
         echo x > \"$c/not-a-run.json\""
    );
    let head = rig.git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("kept.txt"), "edited\n").unwrap();
    let out = rig.run(&repo, &["--on", "fakehost", "sh", "-c", &write]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(runs.join(good))
            .unwrap()
            .starts_with(&format!("{{\"commit\": \"{head}\", \"dirty\": 1, ")),
        "this checkout's HEAD, not the snapshot commit"
    );
    assert!(!runs.join("not-a-run.json").exists());
    assert!(!runs.join(others).exists(), "not another worktree's");
    assert!(
        stderr(&out).contains("the gate's run record came back"),
        "{}",
        stderr(&out)
    );

    std::fs::remove_file(runs.join(good)).unwrap();
    rig.commit(&repo, "second");
    let head = rig.git(&repo, &["rev-parse", "HEAD"]);
    let out = rig.run(&repo, &["--on", "fakehost", "sh", "-c", &write]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(std::fs::read_to_string(runs.join(good))
        .unwrap()
        .starts_with(&format!("{{\"commit\": \"{head}\", \"dirty\": 0, ")));
}

/// The host runs branch code, so its results pack is not trusted: one whose
/// top folder (`runs`, `records` or `wt`) is a symlink would move files from
/// wherever it points into this checkout. Nothing is copied back then.
#[test]
fn a_results_pack_with_a_symlinked_folder_is_refused() {
    let rig = Remote::new();
    let repo = rig.repo();
    let victim = rig.path().join("victim");
    std::fs::create_dir(&victim).unwrap();
    let name = "2026-11-01T22-15-03-123Z-0123abcd.json";
    std::fs::write(victim.join(name), "{}\n").unwrap();
    std::fs::write(victim.join("out.txt"), "theirs\n").unwrap();
    for linked in ["records", "runs", "wt"] {
        let pack_dir = rig.path().join(format!("pack-{linked}"));
        std::fs::create_dir(&pack_dir).unwrap();
        for top in ["records", "runs", "wt"] {
            if top == linked {
                std::os::unix::fs::symlink(&victim, pack_dir.join(top)).unwrap();
            } else {
                std::fs::create_dir(pack_dir.join(top)).unwrap();
            }
        }
        let tar = rig.path().join(format!("evil-{linked}.tar"));
        let made = Command::new("/usr/bin/tar")
            .arg("-C")
            .arg(&pack_dir)
            .arg("-cf")
            .arg(&tar)
            .args(["records", "runs", "wt"])
            .status()
            .unwrap();
        assert!(made.success());
        let out = rig
            .cmd(&repo, &["--on", "fakehost", "--fetch", "out.txt", "true"])
            .env("FAKE_COLLECT_TAR", &tar)
            .output_within()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("its results pack holds a symlinked folder"),
            "{linked}: {}",
            stderr(&out)
        );
        assert!(victim.join(name).exists(), "{linked}: left where it was");
        assert!(victim.join("out.txt").exists(), "{linked}");
        assert!(!repo.join("out.txt").exists(), "{linked}");
        assert!(!repo
            .join(".git")
            .join("hollow-gate-runs")
            .join(name)
            .exists());
    }
}

/// A Codex turn in any mode but bypass runs in a sandbox with no network:
/// `--remote` says so at once and runs here (no ssh timeout first), and a
/// plain build's line doesn't suggest `--remote` there.
#[test]
fn a_sandbox_without_network_runs_here_and_says_why() {
    let rig = Remote::new();
    let repo = rig.repo();
    let here = "echo \"local=${RIG_LOCAL_ONLY:-unset}\"";
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", here])
        .env("CODEX_SANDBOX_NETWORK_DISABLED", "1")
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "local=this Mac only\n");
    assert!(
        stderr(&out).contains(
            "this turn's sandbox has no network (a Codex turn in a mode other than bypass), so it can't reach fakehost; running on this Mac instead"
        ),
        "{}",
        stderr(&out)
    );
    assert!(rig.host_work().is_empty(), "nothing ran on the host");

    let config = rig.path().join("home").join(".config").join("hollow");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("slot-remote"), "fakehost\n").unwrap();
    let out = rig
        .cmd(&repo, &["true"])
        .env("CODEX_SANDBOX_NETWORK_DISABLED", "1")
        .output_within()
        .unwrap();
    assert!(
        stderr(&out).contains("got a build slot"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("hollow-slot --remote"),
        "{}",
        stderr(&out)
    );
}

/// The seconds a step took, from the `<what> took N s ...: a 2 s, b 1 s` line.
fn step_seconds(said: &str, what: &str, step: &str) -> Option<u64> {
    let line = said
        .lines()
        .find(|l| l.starts_with(&format!("hollow-slot: {what} took ")))?;
    let list = line.split_once(": ").map(|(_, rest)| rest)?;
    let list = list.split_once(": ").map(|(_, rest)| rest)?;
    list.split(", ")
        .find_map(|item| item.strip_prefix(&format!("{step} ")))
        .and_then(|n| n.strip_suffix(" s"))
        .and_then(|n| n.parse().ok())
}

/// Overnight N3 (2026-11-02): one Phase 1 gate call spent 20 s before its
/// command with nothing said. Each setup step is timed on both sides, and
/// when they add up to `HOLLOW_SLOT_SETUP_SAY` (5) seconds or more the host
/// says one line naming the steps of a second or more: this side's (a slow
/// preflight, here) and the host's own (waiting for the folder another run
/// from this checkout holds). A quick call says nothing.
#[test]
fn a_slow_setup_says_which_steps_took_the_time() {
    let rig = Remote::new();
    let repo = rig.repo();
    let preflight = rig.path().join("host-bin").join("hollow-run-preflight");
    make_executable(
        &preflight,
        "#!/bin/sh\nsleep 3\necho 'hollow-run-preflight: ok'\n",
    );
    let env = "echo ran; env | grep '^HOLLOW_SLOT_SETUP' || echo none";
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", env])
        .env("HOLLOW_SLOT_SETUP_SAY", "2")
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    // None of the timing variables reach the command.
    assert_eq!(stdout(&out), "ran\nnone\n");
    assert!(
        said.contains("before the command on "),
        "the setup line: {said}"
    );
    let slow = step_seconds(&said, "setup", "ssh and preflight").unwrap_or(0);
    assert!(slow >= 2, "the preflight's 3 s are named: {said}");
    // Said before the command, which runs once setup is over.
    let line = said
        .lines()
        .position(|l| l.starts_with("hollow-slot: setup took "))
        .unwrap();
    let slot = said
        .lines()
        .position(|l| l.contains("got a build slot"))
        .unwrap();
    assert!(line < slot, "{said}");
    std::fs::remove_file(&preflight).unwrap();

    // The host's own step: waiting for this checkout's folder, held until
    // the second call has waited 3 s (whatever the load, its wait is longer).
    let pid_file = rig.path().join("holder-pid");
    let release = rig.path().join("release");
    let hold = format!(
        "echo $$ > '{}'; n=0; while [ ! -e '{}' ] && [ $n -lt 900 ]; do sleep 0.1; n=$((n + 1)); done",
        pid_file.display(),
        release.display()
    );
    let mut first = rig
        .cmd(&repo, &["--on", "fakehost", "sh", "-c", &hold])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until("the first command starts", || {
        std::fs::read_to_string(&pid_file)
            .map(|p| !p.trim().is_empty())
            .unwrap_or(false)
    });
    let second_err = rig.path().join("second.err");
    let second = rig
        .cmd(&repo, &["--on", "fakehost", "echo", "second"])
        .env("HOLLOW_SLOT_SETUP_SAY", "2")
        .stdout(std::process::Stdio::piped())
        .stderr(std::fs::File::create(&second_err).unwrap())
        .spawn()
        .unwrap();
    until("the second call waits", || {
        std::fs::read_to_string(&second_err)
            .unwrap_or_default()
            .contains("another run from this checkout is using its folder")
    });
    std::thread::sleep(Duration::from_secs(3));
    std::fs::write(&release, "").unwrap();
    let out = second.wait_with_output_within().unwrap();
    first.wait_within().unwrap();
    let said = std::fs::read_to_string(&second_err).unwrap();
    assert!(out.status.success(), "{said}");
    assert_eq!(stdout(&out), "second\n");
    let waited = step_seconds(&said, "setup", "waiting for its folder").unwrap_or(0);
    assert!(waited >= 2, "the wait for the folder is named: {said}");

    // The run's own ssh login, timed from this side's clock.
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "true"])
        .env("HOLLOW_SLOT_SETUP_SAY", "2")
        .env("FAKE_LOGIN_SLEEP", "2")
        .output_within()
        .unwrap();
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    let login = step_seconds(&said, "setup", "ssh login").unwrap_or(0);
    assert!(login >= 2, "the slow login is named: {said}");

    // A quick call says nothing (the threshold, not the default 5 s, so a
    // loaded machine can't make it slow enough to say it).
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "true"])
        .env("HOLLOW_SLOT_SETUP_SAY", "60")
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!stderr(&out).contains(" took "), "{}", stderr(&out));
}

/// Wrapping up is timed too: a leftover that ignores TERM holds the host for
/// its 5 s grace before it is killed, and the host says so.
#[test]
fn a_slow_wrap_up_says_what_held_it() {
    let rig = Remote::new();
    let pid_file = rig.path().join("left-pid");
    let script = format!(
        "(trap '' TERM; exec sleep 42) & echo $! > '{}'; echo done",
        pid_file.display()
    );
    let out = rig.run(&rig.repo(), &["--on", "fakehost", "sh", "-c", &script]);
    let said = stderr(&out);
    assert!(out.status.success(), "{said}");
    assert_eq!(stdout(&out), "done\n");
    assert!(said.contains("after the command on "), "{said}");
    let held = step_seconds(&said, "wrapping up", "ending what it left running").unwrap_or(0);
    assert!(held >= 5, "the leftover's grace is named: {said}");
    let left = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    until("the leftover is ended", || !alive(&left));
}

/// The host's job folders (`~/hollow-remote/jobs/job-*`).
fn host_jobs(rig: &Remote) -> Vec<String> {
    std::fs::read_dir(rig.host_root().join("jobs"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("job-") && !n.contains('.'))
                .collect()
        })
        .unwrap_or_default()
}

/// Jobs (overnight N4, 2026-11-02): `--send` runs a command that needs no
/// checkout, such as a transcription or a render, from any folder. The sent
/// files and folders go to a folder of the job's own on the host, the
/// command runs there with at most 2 render workers and none of this Mac's
/// variables, its exit code comes back, the `--fetch` paths come back here,
/// and the host's job folder is gone afterwards.
#[test]
fn a_job_sends_its_files_runs_there_and_brings_back_what_it_fetches() {
    let rig = Remote::new();
    let media = rig.path().join("media");
    std::fs::create_dir_all(media.join("in")).unwrap();
    std::fs::write(media.join("clip.txt"), "clip\n").unwrap();
    std::fs::write(media.join("in").join("a.txt"), "a\n").unwrap();
    std::fs::write(media.join("not-sent.txt"), "stays here\n").unwrap();
    let script = "pwd; cat clip.txt in/a.txt > out.txt; mkdir -p renders evil/.git; echo r > renders/r.txt; echo x > evil/.git/config; \
                  [ -e not-sent.txt ] && echo 'has not-sent'; \
                  echo \"workers=$PRODUCER_MAX_WORKERS pin=$HYPERFRAMES_NO_UPDATE_CHECK local=${RIG_LOCAL_ONLY:-unset} tmp=$TMPDIR\"; exit 3";
    let out = rig.run(
        &media,
        &[
            "--on", "fakehost", "--send", "clip.txt", "--send", "in/", "--fetch", "out.txt",
            "--fetch", "renders", "--fetch", "evil", "sh", "-c", script,
        ],
    );
    let (text, said) = (stdout(&out), stderr(&out));
    assert_eq!(out.status.code(), Some(3), "{said}");
    assert!(said.contains("as a job with the files it sends"), "{said}");
    let pwd = text.lines().next().unwrap_or_default();
    assert!(
        pwd.contains("/host/hollow-remote/jobs/job-"),
        "ran in the job's folder: {text}"
    );
    assert!(!text.contains("has not-sent"), "only what it sends: {text}");
    assert!(text.contains("workers=2 pin=1 local=unset tmp="), "{text}");
    assert_eq!(
        std::fs::read_to_string(media.join("out.txt")).unwrap(),
        "clip\na\n"
    );
    assert_eq!(
        std::fs::read_to_string(media.join("renders").join("r.txt")).unwrap(),
        "r\n"
    );
    // A tree holding a .git never comes back: it would be a repository here.
    assert!(
        said.contains("not copying evil back: it holds a .git"),
        "{said}"
    );
    assert!(!media.join("evil").exists());
    assert!(host_jobs(&rig).is_empty(), "the job's folder is gone");
    // No gate records or worktree folders for a job.
    assert!(rig.host_work().is_empty(), "{:?}", rig.host_work());
}

/// `--send` needs a runner and paths inside the folder; a path that isn't
/// here, a symlinked folder on the way, or a host that can't be reached
/// runs the command here instead, where the files already are.
#[test]
fn a_job_the_host_cant_take_runs_here_instead() {
    let rig = Remote::new();
    let media = rig.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("clip.txt"), "clip\n").unwrap();
    let out = rig.run(&media, &["--send", "clip.txt", "cat", "clip.txt"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("--fetch and --send go with --remote or --on"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(&media, &["--on", "fakehost", "--send", "../x", "true"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("--send takes a path inside this folder"),
        "{}",
        stderr(&out)
    );

    // Mistakes stop the call: a long job never runs here by accident.
    let here = "cat clip.txt; echo \"local=${RIG_LOCAL_ONLY:-unset}\"";
    let out = rig.run(
        &media,
        &[
            "--on",
            "fakehost",
            "--send",
            "missing.txt",
            "sh",
            "-c",
            here,
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("--send missing.txt: it isn't in this folder"),
        "{}",
        stderr(&out)
    );
    let outside = rig.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("x.txt"), "x\n").unwrap();
    std::os::unix::fs::symlink(&outside, media.join("link")).unwrap();
    let out = rig.run(
        &media,
        &["--on", "fakehost", "--send", "link/x.txt", "sh", "-c", here],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--send link/x.txt: a folder on the way to it is a symlink"),
        "{}",
        stderr(&out)
    );
    // A folder here the job wouldn't get whole is never replaced by its copy.
    std::fs::create_dir_all(media.join("footage")).unwrap();
    std::fs::write(media.join("footage").join("day1.mov"), "1\n").unwrap();
    std::fs::write(media.join("footage").join("day2.mov"), "2\n").unwrap();
    let out = rig.run(
        &media,
        &[
            "--on",
            "fakehost",
            "--send",
            "footage/day1.mov",
            "--fetch",
            "footage",
            "true",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--fetch footage would replace the folder footage here"),
        "{}",
        stderr(&out)
    );
    assert!(media.join("footage").join("day2.mov").exists());
    // Nothing comes back through a symlink here.
    let out = rig.run(
        &media,
        &[
            "--on",
            "fakehost",
            "--send",
            "clip.txt",
            "--fetch",
            "link/out.txt",
            "true",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("--fetch link/out.txt: it, or a folder on the way to it, is a symlink here"),
        "{}",
        stderr(&out)
    );
    // Sent whole, it may come back.
    let out = rig.run(
        &media,
        &[
            "--on",
            "fakehost",
            "--send",
            "footage",
            "--fetch",
            "footage",
            "sh",
            "-c",
            "echo 3 > footage/day3.txt",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    for f in ["day1.mov", "day2.mov", "day3.txt"] {
        assert!(media.join("footage").join(f).exists(), "{f}");
    }

    let out = rig.run(
        &media,
        &["--on", "down", "--send", "clip.txt", "sh", "-c", here],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "clip\nlocal=this Mac only\n");
    assert!(
        stderr(&out).contains("down can't be reached"),
        "{}",
        stderr(&out)
    );
    assert!(host_jobs(&rig).is_empty(), "{:?}", host_jobs(&rig));
}

/// One job at a time on the host: a second waits, saying so, and runs once
/// the first is done; each in its own folder.
#[test]
fn jobs_on_the_host_run_one_at_a_time() {
    let rig = Remote::new();
    let media = rig.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("clip.txt"), "clip\n").unwrap();
    let pid_file = rig.path().join("job-pid");
    let first_done = rig.path().join("first-done");
    let release = rig.path().join("release-job");
    let hold = format!(
        "echo $$ > '{}'; n=0; while [ ! -e '{}' ] && [ $n -lt 900 ]; do sleep 0.1; n=$((n + 1)); done; touch '{}'",
        pid_file.display(),
        release.display(),
        first_done.display()
    );
    let mut first = rig
        .cmd(
            &media,
            &["--on", "fakehost", "--send", "clip.txt", "sh", "-c", &hold],
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until("the first job starts", || {
        std::fs::read_to_string(&pid_file)
            .map(|p| !p.trim().is_empty())
            .unwrap_or(false)
    });
    let check = format!(
        "[ -e '{}' ] && echo after-first; cat clip.txt",
        first_done.display()
    );
    let second_err = rig.path().join("second-job.err");
    let second = rig
        .cmd(
            &media,
            &["--on", "fakehost", "--send", "clip.txt", "sh", "-c", &check],
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::fs::File::create(&second_err).unwrap())
        .spawn()
        .unwrap();
    until("the second job waits", || {
        std::fs::read_to_string(&second_err)
            .unwrap_or_default()
            .contains("another job is running on")
    });
    std::fs::write(&release, "").unwrap();
    let out = second.wait_with_output_within().unwrap();
    first.wait_within().unwrap();
    assert!(
        out.status.success(),
        "{}",
        std::fs::read_to_string(&second_err).unwrap_or_default()
    );
    assert_eq!(stdout(&out), "after-first\nclip\n");
    assert!(host_jobs(&rig).is_empty(), "{:?}", host_jobs(&rig));
}

/// Stop during a job's upload ends the call at once (both ends of the upload
/// run in the background, so the trap runs), and nothing is left on the
/// host; the upload here would otherwise wait out a 30 s late receive.
#[test]
fn stopping_a_job_while_it_uploads_ends_it_at_once() {
    let rig = Remote::new();
    let media = rig.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    // More than a pipe holds, so tar waits for the host to read.
    std::fs::write(media.join("big.bin"), vec![7u8; 2 << 20]).unwrap();
    let mut call = rig
        .cmd(&media, &["--on", "fakehost", "--send", "big.bin", "true"])
        .env("FAKE_RECEIVE_SLEEP", "30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let tmp = rig.path().join("tmp");
    until("the upload starts", || {
        std::fs::read_dir(&tmp)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .any(|e| e.path().join("up").exists())
            })
            .unwrap_or(false)
    });
    std::thread::sleep(Duration::from_millis(300));
    let stopped = Instant::now();
    Command::new("/bin/kill")
        .args(["-TERM", &call.id().to_string()])
        .status()
        .unwrap();
    let status = call.wait_within().unwrap();
    assert_eq!(status.code(), Some(143));
    assert!(
        stopped.elapsed() < Duration::from_secs(10),
        "the upload didn't hold the stop"
    );
    assert!(host_jobs(&rig).is_empty(), "{:?}", host_jobs(&rig));
}

/// The host's side of a dropped job: a drop deletes a folder nothing has
/// claimed and leaves a tombstone, so a receive still to come refuses and a
/// run for it never starts; a folder a run has claimed stays.
#[test]
fn a_dropped_job_never_runs_and_a_started_one_is_never_dropped() {
    let rig = Remote::new();
    let media = rig.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("clip.txt"), "clip\n").unwrap();
    // One job puts this tool on the host.
    let out = rig.run(&media, &["--on", "fakehost", "--send", "clip.txt", "true"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let host = rig.path().join("host");
    let jobs = rig.host_root().join("jobs");
    let tool = rig.host_root().join("bin").join("hollow-slot");
    // Spawned, not `output_within`, which reads from /dev/null: a late
    // receive gets its pack (review of #1169).
    let host_tool = |args: &[&str], stdin: std::process::Stdio| {
        Command::new("/bin/sh")
            .arg(&tool)
            .args(args)
            .env_clear()
            .env("HOME", &host)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(stdin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
            .wait_with_output_within()
            .unwrap()
    };
    // Received, not claimed: dropped.
    std::fs::create_dir_all(jobs.join("job-1-2")).unwrap();
    let out = host_tool(&["--job-drop", "job-1-2"], std::process::Stdio::null());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!jobs.join("job-1-2").exists());
    assert!(jobs.join("job-1-2.drop").is_dir(), "a tombstone");
    // A run for it refuses.
    let out = host_tool(
        &["--job-run", "job-1-2", "1-2", "", "--", "true"],
        std::process::Stdio::null(),
    );
    // 247: the host's own setup failed, so nothing ran (smart routing).
    assert_eq!(out.status.code(), Some(247), "{}", stderr(&out));
    assert!(stderr(&out).contains("isn't on"), "{}", stderr(&out));
    // So does a late receive, and it keeps nothing.
    let tar = rig.path().join("one.tar");
    Command::new("tar")
        .args([
            "-cf",
            tar.to_str().unwrap(),
            "-C",
            media.to_str().unwrap(),
            "clip.txt",
        ])
        .status()
        .unwrap();
    let out = host_tool(
        &["--job-receive", "job-1-2"],
        std::fs::File::open(&tar).unwrap().into(),
    );
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(!jobs.join("job-1-2").exists());
    // Claimed by a run: left to the run.
    std::fs::create_dir_all(jobs.join("job-3-4")).unwrap();
    std::fs::create_dir_all(jobs.join("job-3-4.run")).unwrap();
    let out = host_tool(&["--job-drop", "job-3-4"], std::process::Stdio::null());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(jobs.join("job-3-4").is_dir(), "a started job stays");
}

/// 11-02: the command there runs with the git identity this Mac uses for the
/// checkout (here the repository's own config), so a commit it makes is this
/// Mac's, not a guess from the runner account. The identity never rides a
/// command line, and the file that carries it there is gone before the
/// command starts. With none configured here, git's own default there stays,
/// and a job (no checkout) gets none.
#[test]
fn the_command_there_commits_as_this_macs_identity() {
    let rig = Remote::new();
    let repo = rig.repo();
    rig.git(&repo, &["config", "user.name", "Slot Person"]);
    rig.git(&repo, &["config", "user.email", "slot@example.com"]);
    let argv = rig.path().join("argv.log");
    let script =
        "git commit -q --allow-empty -m there && git log -1 --format='%an <%ae> | %cn <%ce>'; \
                  ls \"$HOME/hollow-remote/work\" | grep -c 'ident$'; true";
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "/bin/sh", "-c", script])
        .env("FAKE_ARGV_LOG", &argv)
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "Slot Person <slot@example.com> | Slot Person <slot@example.com>\n0\n",
        "{}",
        stderr(&out)
    );
    let log = std::fs::read_to_string(&argv).unwrap();
    assert!(log.contains("--remote-run"), "{log}");
    assert!(
        !log.contains("slot@example.com") && !log.contains("Slot Person"),
        "never on a command line: {log}"
    );

    // None here: nothing goes, and git's own default there stays.
    rig.git(&repo, &["config", "--unset", "user.name"]);
    rig.git(&repo, &["config", "--unset", "user.email"]);
    let out = rig.run(
        &repo,
        &[
            "--on",
            "fakehost",
            "/bin/sh",
            "-c",
            "env | grep -c '^GIT_AUTHOR'; git var GIT_AUTHOR_IDENT; true",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let said = stdout(&out);
    assert!(said.starts_with("0\n"), "{said}");
    assert!(!said.contains("slot@example.com"), "{said}");
    let left: Vec<String> = std::fs::read_dir(rig.host_root().join("work"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".ident"))
        .collect();
    assert!(left.is_empty(), "{left:?}");

    // A job runs outside any checkout: no identity goes with it.
    rig.git(&repo, &["config", "user.name", "Slot Person"]);
    rig.git(&repo, &["config", "user.email", "slot@example.com"]);
    std::fs::write(repo.join("in.txt"), "x\n").unwrap();
    let out = rig.run(
        &repo,
        &[
            "--on",
            "fakehost",
            "--send",
            "in.txt",
            "/bin/sh",
            "-c",
            "env | grep -c '^GIT_AUTHOR'; true",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "0\n", "{}", stderr(&out));
}

/// Smart routing (2026-11-03): with a runner set up, a plain call decides.
/// The rig's runner is `fakehost`, named in the fake home's slot-remote.
fn with_runner(rig: &Remote, host: &str) {
    let config = rig.path().join("home").join(".config").join("hollow");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("slot-remote"), format!("{host}\n")).unwrap();
}

fn route_log(rig: &Remote) -> String {
    std::fs::read_to_string(rig.path().join("slots").join("route-log")).unwrap_or_default()
}

fn route_history(rig: &Remote) -> String {
    std::fs::read_to_string(rig.path().join("slots").join("route-history")).unwrap_or_default()
}

/// Ran on the host: its worktree folders live under `hollow-remote/work`.
fn ran_there(out: &Output) -> bool {
    stdout(out).contains("/hollow-remote/work/")
}

/// Lock files held by perl for `secs` seconds; killed when dropped, so a
/// failing test never leaves them behind.
struct Holders(Vec<std::process::Child>);

impl Drop for Holders {
    fn drop(&mut self) {
        for h in self.0.iter_mut() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }
}

fn hold_locks(paths: &[PathBuf], secs: u32) -> Holders {
    use std::io::Read;
    let mut held = Holders(Vec::new());
    for path in paths {
        let child = Command::new("/usr/bin/perl")
            .args([
                "-e",
                "use Fcntl qw(:flock); open(my $f, '>>', $ARGV[0]) or die; flock($f, LOCK_EX) or die; $| = 1; print qq(held\\n); sleep $ARGV[1]",
            ])
            .arg(path)
            .arg(secs.to_string())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        held.0.push(child);
        let mut b = [0u8; 5];
        held.0
            .last_mut()
            .unwrap()
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut b)
            .unwrap();
    }
    held
}

/// Heavy work (a Rust build, a whole test suite) goes to the runner, quick
/// work (one test file) stays here, and a command that needs this Mac (git,
/// gh, this Mac's database, a file outside the checkout, a writer) stays
/// here whatever its kind. Each says where and why in one line, and each
/// decision is a line of route-log.
#[test]
fn auto_routing_sends_heavy_work_there_and_keeps_the_rest_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();

    let out = rig.run(&repo, &["sh", "-c", "pwd; : cargo test --lib"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("hollow-slot: on the runner (cargo test"),
        "{}",
        stderr(&out)
    );

    let out = rig.run(&repo, &["sh", "-c", "pwd; : npx vitest run src/a.test.ts"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("hollow-slot: on this Mac (vitest on named files; sending it to the runner would take about as long)"),
        "{}",
        stderr(&out)
    );

    for (args, why) in [
        (
            &["sh", "-c", "pwd; : git status; : cargo test"][..],
            "it runs git, which works on this checkout here",
        ),
        (
            &["sh", "-c", "pwd; : gh pr view; : npx vitest run"][..],
            "it runs gh, and only this Mac is signed in to GitHub",
        ),
        (
            &[
                "env",
                "DATABASE_URL=postgresql://localhost/x",
                "sh",
                "-c",
                "pwd; : npx vitest run",
            ][..],
            "it gives the command DATABASE_URL, a database this Mac reaches",
        ),
        (
            &[
                "env",
                "HOLLOW_REGENERATE_PROMPTS=1",
                "sh",
                "-c",
                "pwd; : cargo test",
            ][..],
            "it writes files into this checkout (HOLLOW_REGENERATE_PROMPTS)",
        ),
        (
            &["sh", "-c", "pwd; : cargo fmt"][..],
            "cargo fmt writes files here",
        ),
        (
            &["sh", "-c", "pwd; : npm ci"][..],
            "npm ci installs packages into this checkout",
        ),
        (
            &["sh", "-c", "pwd; : npm run tauri:build"][..],
            "an app build, which stays on this Mac",
        ),
        (
            &["sh", "-c", "pwd; : npm run lint:fix"][..],
            "run lint:fix writes files into this checkout",
        ),
        (
            &["sh", "-c", "pwd; : cargo test > test.log"][..],
            "it writes test.log with a > redirect, which would stay where it runs",
        ),
        (
            &["sh", "-c", "pwd; : mlx_whisper talk.m4a"][..],
            "a transcription reads and writes media here; hollow-slot --send <its files> --fetch <its output> runs it on the runner",
        ),
    ] {
        let out = rig.run(&repo, args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert!(!ran_there(&out), "{args:?}: {}", stdout(&out));
        assert!(
            stderr(&out).contains(&format!("hollow-slot: on this Mac ({why})")),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    // This Mac's own path into the checkout isn't the runner's (review r2).
    let abs = format!("cd {} && pwd; : cargo test", repo.join("sub").display());
    let out = rig.run(&repo, &["sh", "-c", &abs]);
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out)
            .contains("this Mac's own path into the checkout (the runner's copy is elsewhere; write it relative)"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(&repo, &["sh", "-c", "pwd; : npm test -- src/a.test.ts"]);
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("on this Mac (tests on named files;"),
        "{}",
        stderr(&out)
    );
    // A file outside the checkout isn't on the runner; a system file is.
    let outside = rig.path().join("tmp").join("elsewhere.txt");
    std::fs::write(&outside, "x\n").unwrap();
    let out = rig.run(
        &repo,
        &[
            "sh",
            "-c",
            &format!(
                "pwd; : npx vitest run; cat {} >/dev/null",
                outside.display()
            ),
        ],
    );
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("outside this checkout"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(
        &repo,
        &[
            "sh",
            "-c",
            "pwd; : npx vitest run; cat /etc/hosts >/dev/null",
        ],
    );
    assert!(ran_there(&out), "{}", stderr(&out));

    let log = route_log(&rig);
    assert!(log.lines().count() >= 10, "{log}");
    let first = log.lines().next().unwrap();
    let fields: Vec<&str> = first.split('\t').collect();
    assert_eq!(fields.len(), 6, "{first}");
    assert_eq!(fields[1], "fakehost", "{first}");
    assert!(fields[2].starts_with("cargo test"), "{first}");
    assert_eq!(fields[3], "cargo test", "{first}");
    assert!(fields[4].starts_with("repo-"), "{first}");
    assert_eq!(fields[5], "auto", "{first}");
    assert!(log.contains("\there\tit runs git"), "{log}");
}

/// --here keeps a run here, HOLLOW_SLOT_ROUTE (Builds and tests, once Settings has it)
/// moves a run either way (needs still win: git stays here), --send asks
/// for the runner, and contradictory flags are refused. Without a runner,
/// nothing is decided or said.
#[test]
fn flags_and_the_setting_choose_where_and_say_so() {
    let rig = Remote::new();
    let repo = rig.repo();
    let out = rig.run(&repo, &["sh", "-c", "pwd; : cargo test"]);
    assert!(out.status.success());
    assert!(!ran_there(&out));
    assert!(
        !stderr(&out).contains("hollow-slot: on "),
        "no runner, no line: {}",
        stderr(&out)
    );
    assert!(route_log(&rig).is_empty());

    with_runner(&rig, "fakehost");
    let out = rig.run(&repo, &["--here", "sh", "-c", "pwd; : cargo test"]);
    assert!(!ran_there(&out));
    assert!(
        stderr(&out).contains("hollow-slot: on this Mac (asked for with --here)"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, &["sh", "-c", "pwd; : cargo test"])
        .env("HOLLOW_SLOT_ROUTE", "this-mac")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out));
    assert!(
        stderr(&out).contains("on this Mac (Builds and tests is set to This Mac)"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, &["sh", "-c", "pwd; : npx vitest run src/a.test.ts"])
        .env("HOLLOW_SLOT_ROUTE", "mac-mini")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on the runner (Builds and tests is set to Remote runner)"),
        "{}",
        stderr(&out)
    );
    // M6: the runner's new name reads the same (Hollow still sets mac-mini,
    // which older installed copies understand).
    let out = rig
        .cmd(&repo, &["sh", "-c", "pwd; : npx vitest run src/a.test.ts"])
        .env("HOLLOW_SLOT_ROUTE", "runner")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on the runner (Builds and tests is set to Remote runner)"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, &["sh", "-c", "pwd; : git log"])
        .env("HOLLOW_SLOT_ROUTE", "mac-mini")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));
    // --auto overrides the setting.
    let out = rig
        .cmd(&repo, &["--auto", "sh", "-c", "pwd; : cargo build"])
        .env("HOLLOW_SLOT_ROUTE", "this-mac")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    // --remote is as before, and logged.
    let out = rig.run(&repo, &["--remote", "pwd"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        route_log(&rig).contains("\tfakehost\tasked for with --remote\t"),
        "{}",
        route_log(&rig)
    );

    let media = rig.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("clip.txt"), "clip\n").unwrap();
    let out = rig.run(&media, &["--send", "clip.txt", "cat", "clip.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "clip\n");
    assert!(
        stderr(&out).contains("on the runner (a job with the files it sends)"),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("as a job with the files it sends"),
        "{}",
        stderr(&out)
    );

    for args in [
        &["--here", "--remote", "true"][..],
        &["--remote", "--here", "true"][..],
        &["--here", "--fetch", "out", "true"][..],
        &["--bogus", "true"][..],
    ] {
        let out = rig.run(&repo, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    let out = rig.run(&repo, &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout(&out).starts_with("usage: hollow-slot"),
        "{}",
        stdout(&out)
    );
    assert!(
        !stderr(&out).contains("got a build slot"),
        "--help takes no slot: {}",
        stderr(&out)
    );
}

/// History: a command of no known kind runs here until it is known; once
/// its usual time is past HOLLOW_SLOT_LONG_SECS it goes to the runner. One
/// that changed the checkout when it ran stays here, and a run there that
/// changed the checkout says which paths stayed there (not --fetch paths)
/// and is remembered the same way.
#[test]
fn history_moves_slow_commands_there_and_keeps_writers_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let slow = &["sh", "-c", "pwd"][..];
    let out = rig
        .cmd(&repo, slow)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out));
    assert!(
        stderr(&out).contains("on this Mac (pwd: no timing for it yet, so it runs here this time)"),
        "{}",
        stderr(&out)
    );
    let history = route_history(&rig);
    let line = history.lines().last().unwrap();
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(fields.len(), 7, "{line}");
    assert_eq!(
        (fields[2], fields[3], fields[5], fields[6]),
        ("pwd", "here", "0", "0"),
        "{line}"
    );
    // One passing run is not enough for a command of no known kind.
    let out = rig
        .cmd(&repo, slow)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(", timed once so far, so it runs here again)"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, slow)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on the runner (pwd, usually "),
        "{}",
        stderr(&out)
    );
    // A heavy kind whose usual time is under HOLLOW_SLOT_QUICK_SECS stays.
    let heavy = &["sh", "-c", "pwd; : cargo check"][..];
    assert!(ran_there(&rig.run(&repo, heavy)));
    // 60, not the default 5: under a full gate's load the stand-in run
    // there can take several seconds.
    let out = rig
        .cmd(&repo, heavy)
        .env("HOLLOW_SLOT_QUICK_SECS", "60")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on this Mac (cargo check, usually ")
            && stderr(&out).contains(" s, less than sending it to the runner takes)"),
        "{}",
        stderr(&out)
    );
    // The default threshold keeps it here.
    let out = rig.run(&repo, slow);
    assert!(!ran_there(&out));
    assert!(
        stderr(&out).contains("on this Mac (pwd, usually ")
            && stderr(&out).contains(" s; sending it to the runner would take about as long)"),
        "{}",
        stderr(&out)
    );

    // A writer stays here once it is known to write.
    let writer = &["sh", "-c", "echo w; cp kept.txt made-here.txt"][..];
    let out = rig
        .cmd(&repo, writer)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out));
    let out = rig
        .cmd(&repo, writer)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("changed this checkout when it ran, so it runs where its changes land"),
        "{}",
        stderr(&out)
    );

    // A run there that writes says so, and a fetched path doesn't count.
    let out = rig.run(
        &repo,
        &[
            "sh",
            "-c",
            ": cargo test; cp kept.txt made-there.txt; mkdir -p out; cp gone.txt out/r.txt",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let said = stderr(&out);
    assert!(said.contains("on the runner (cargo test"), "{said}");
    assert!(said.contains("the command changed 2 path(s) in the checkout on fakehost, which don't come back here (made-there.txt out/r.txt); to change them here, run it with hollow-slot --here"), "{said}");
    // Known to write now, so it stays here, heavy or not (review of #1113).
    let out = rig.run(&repo, &["sh", "-c", ": cargo test; true"]);
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on this Mac (cargo test changed this checkout when it ran"),
        "{}",
        stderr(&out)
    );
    let out = rig.run(
        &repo,
        &[
            "--fetch",
            "out",
            "sh",
            "-c",
            ": cargo build; mkdir -p out; cp gone.txt out/r.txt; cp kept.txt ignored.txt",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        !stderr(&out).contains("the command changed"),
        "{}",
        stderr(&out)
    );
    // What git ignores comes back only by --fetch: the call says so.
    assert!(
        stderr(&out)
            .contains("it also wrote ignored.txt on fakehost, which stay there (ignored files)"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("out").join("r.txt")).unwrap(),
        "deleted here\n"
    );
    let history = route_history(&rig);
    let there: Vec<&str> = history
        .lines()
        .filter(|l| l.contains("\tfakehost\t"))
        .collect();
    assert!(there.iter().any(|l| l.ends_with("\t1")), "{history}");
}

/// Pressure: this Mac under its memory floor, both of its slots taken, or on
/// battery sends a command of a known kind to the runner; one of no known
/// kind stays (it may need this Mac).
#[test]
fn pressure_here_sends_known_work_there() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let quick = &["sh", "-c", "pwd; : npx vitest run src/a.test.ts"][..];
    let sysctl = rig.path().join("bin").join("fake-sysctl");
    make_executable(&sysctl, "#!/bin/sh\necho 12\n");
    let out = rig
        .cmd(&repo, quick)
        .env("HOLLOW_SLOT_MIN_FREE_PCT", "30")
        .env("HOLLOW_SLOT_SYSCTL", &sysctl)
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(stderr(&out).contains("on the runner (vitest on named files; this Mac has 12% of its memory free, under its 30% floor)"), "{}", stderr(&out));
    let out = rig
        .cmd(&repo, &["sh", "-c", "pwd; : some-tool"])
        .env("HOLLOW_SLOT_MIN_FREE_PCT", "30")
        .env("HOLLOW_SLOT_SYSCTL", &sysctl)
        .env("HOLLOW_SLOT_MEMORY_WAIT", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));

    std::fs::write(rig.path().join("bin").join("FAKE_BATTERY"), "").unwrap();
    let out = rig.run(&repo, quick);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("vitest on named files; this Mac is on battery)"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_file(rig.path().join("bin").join("FAKE_BATTERY")).unwrap();

    let slots = rig.path().join("slots");
    let held = hold_locks(&[slots.join("slot-0.lock"), slots.join("slot-1.lock")], 60);
    let out = rig.run(&repo, quick);
    drop(held);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("vitest on named files; both of this Mac's build slots are in use)"),
        "{}",
        stderr(&out)
    );
}

/// A runner that didn't answer is skipped by automatic runs for
/// HOLLOW_SLOT_DOWN_SECS (300) without waiting on ssh again; --remote still
/// tries, and an answer clears the note.
#[test]
fn a_runner_that_didnt_answer_is_skipped_for_a_while() {
    let rig = Remote::new();
    with_runner(&rig, "down");
    let repo = rig.repo();
    let argv = rig.path().join("argv.log");
    let heavy = &["sh", "-c", "pwd; : cargo test"][..];
    let out = rig
        .cmd(&repo, heavy)
        .env("FAKE_ARGV_LOG", &argv)
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on the runner (cargo test"),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("down can't be reached"),
        "{}",
        stderr(&out)
    );
    let calls = std::fs::read_to_string(&argv).unwrap().lines().count();
    assert!(calls >= 1);
    let out = rig
        .cmd(&repo, heavy)
        .env("FAKE_ARGV_LOG", &argv)
        .output_within()
        .unwrap();
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("on this Mac (the runner didn't answer ")
            && stderr(&out).contains(" s ago (ssh: connect to host down port 22: Connection refused); asking it again 5 min after that)"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(&argv).unwrap().lines().count(),
        calls,
        "no ssh call"
    );
    assert!(
        route_log(&rig).contains("\there\tdown can't be reached"),
        "{}",
        route_log(&rig)
    );
    let out = rig
        .cmd(&repo, heavy)
        .env("FAKE_ARGV_LOG", &argv)
        .env("HOLLOW_SLOT_DOWN_SECS", "0")
        .output_within()
        .unwrap();
    assert!(
        stderr(&out).contains("down can't be reached"),
        "{}",
        stderr(&out)
    );

    with_runner(&rig, "fakehost");
    std::fs::write(
        rig.path().join("slots").join("runner-down"),
        "9999999999 fakehost old\n",
    )
    .unwrap();
    let out = rig.run(&repo, &["--remote", "pwd"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        !rig.path().join("slots").join("runner-down").exists(),
        "an answer clears it"
    );
}

/// A call from inside a slot's own command stays in its caller's slot,
/// wherever that is: no second decision.
#[test]
fn a_call_inside_a_slot_is_not_routed_again() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let tool = rig.path().join("hollow-slot");
    let inner = format!("/bin/sh {} sh -c 'pwd; : npx vitest run'", tool.display());
    let out = rig.run(&repo, &["--here", "sh", "-c", &inner]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!ran_there(&out), "{}", stdout(&out));
    let said = stderr(&out);
    assert_eq!(said.matches("hollow-slot: on ").count(), 1, "{said}");
    assert!(
        said.contains("running in the build slot of the command that called it"),
        "{said}"
    );
}

/// Smart routing 2 (2026-11-03): each preparation reports the runner's tools
/// and load ("caps ...", kept here in runner-caps), and a routed command that
/// needs a tool the runner lacks runs here: first by the report it just got,
/// then at once by the kept one while it is fresh. `--remote` only warns.
#[test]
fn a_runner_without_a_tool_the_command_needs_keeps_it_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let pg = &["sh", "-c", "pwd; : npx vitest run; : pg_ctl status"][..];
    let out = rig.run(&repo, pg);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("hollow-slot: the runner has no Postgres, which this command needs; running on this Mac instead"),
        "{}",
        stderr(&out)
    );
    let caps = std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    assert!(
        caps.contains(" fakehost rust=1.96.0 node=26.0.0 "),
        "{caps}"
    );
    assert!(caps.contains(" postgres= "), "{caps}");
    assert!(
        caps.trim_end()
            .ends_with("slots=0/2 cargo=free lanes=2 keys=0 others=0 owner=none"),
        "{caps}"
    );
    let out = rig.run(&repo, pg);
    assert!(!ran_there(&out));
    assert!(
        stderr(&out).contains("on this Mac (the whole vitest suite needs Postgres, which the runner doesn't have (it said so "),
        "{}",
        stderr(&out)
    );
    // --remote goes anyway, and says so.
    let out = rig.run(&repo, &["--remote", "sh", "-c", "pwd; : pg_ctl status"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "fakehost reports no Postgres, which this command needs; running it there anyway"
        ),
        "{}",
        stderr(&out)
    );
    // Once the runner has it (its tools are re-read after 10 minutes; here
    // the kept list is removed), the command goes there.
    let pgdir = rig.path().join("pg");
    std::fs::create_dir_all(&pgdir).unwrap();
    make_executable(
        &pgdir.join("pg_ctl"),
        "#!/bin/sh\necho 'pg_ctl (PostgreSQL) 17.6'\n",
    );
    std::fs::remove_file(rig.host_root().join("caps-tools")).unwrap();
    let out = rig
        .cmd(&repo, pg)
        .env("FAKE_PG_DIRS", &pgdir)
        .env("HOLLOW_SLOT_CAPS_SECS", "0")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    let caps = std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    assert!(caps.contains(" postgres=17.6 "), "{caps}");
}

/// Remote runners 2 (2026-11-03): each preparation counts the keys the
/// runner account lets in against the owner key its owner.json records, and
/// a call warns when another key can log in there, or several with no
/// owner recorded.
#[test]
fn a_runner_that_lets_other_keys_in_is_warned_about() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let home = rig.path().join("host");
    let mut public = Vec::new();
    let mut fps = Vec::new();
    for name in ["one", "two"] {
        let key = rig.path().join(format!("key-{name}"));
        assert!(Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
            .arg(&key)
            .status()
            .unwrap()
            .success());
        let pubfile = key.with_extension("pub");
        public.push(std::fs::read_to_string(&pubfile).unwrap());
        let out = Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(&pubfile)
            .output_within()
            .unwrap();
        fps.push(stdout(&out).split_whitespace().nth(1).unwrap().to_string());
    }
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    // Key one twice (as two files may hold it): it counts once.
    std::fs::write(home.join(".ssh/authorized_keys"), public.concat()).unwrap();
    std::fs::write(home.join(".ssh/authorized_keys2"), &public[0]).unwrap();
    let warned = |out: &std::process::Output| {
        stderr(out)
            .lines()
            .filter(|l| l.starts_with("hollow-slot: warning: fakehost lets "))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let out = rig.run(&repo, &["--remote", "sh", "-c", "pwd"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    let said = warned(&out);
    assert_eq!(said.len(), 1, "{}", stderr(&out));
    assert!(
        said[0].contains("lets 2 SSH keys in and has no owner recorded"),
        "{said:?}"
    );
    let caps = std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    assert!(
        caps.trim_end().ends_with(" keys=2 others=2 owner=none"),
        "{caps}"
    );

    std::fs::write(
        rig.host_root().join("owner.json"),
        format!("{{\"v\":1,\"installId\":\"0123456789abcdef0123456789abcdef\",\"key\":\"{}\",\"at\":\"2026-11-03T19:00:00Z\"}}\n", fps[0]),
    )
    .unwrap();
    let out = rig.run(&repo, &["--remote", "sh", "-c", "pwd"]);
    let said = warned(&out);
    assert_eq!(said.len(), 1, "{}", stderr(&out));
    assert!(
        said[0].contains("lets 1 SSH key(s) in besides its owner's"),
        "{said:?}"
    );

    std::fs::write(home.join(".ssh/authorized_keys"), &public[0]).unwrap();
    std::fs::remove_file(home.join(".ssh/authorized_keys2")).unwrap();
    let out = rig.run(&repo, &["--remote", "sh", "-c", "pwd"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(warned(&out).is_empty(), "{}", stderr(&out));
    let caps = std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    assert!(
        caps.trim_end().ends_with(" keys=1 others=0 owner=yes"),
        "{caps}"
    );
}

/// A runner whose two slots are taken, or whose Rust build lock is taken
/// for a Rust build, sends an automatic run back here when this Mac has a
/// free slot; work that doesn't need what is taken still goes, and so does
/// work while this Mac is busy too.
#[test]
fn a_busy_runner_sends_automatic_work_back_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let host = rig.path().join("host-slots");
    let here = rig.path().join("slots");

    let held = hold_locks(&[host.join("slot-0.lock"), host.join("slot-1.lock")], 60);
    let out = rig.run(&repo, &["sh", "-c", "pwd; : npx vitest run"]);
    drop(held);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("the runner is busy (both its build slots are taken) and this Mac has a free build slot; running on this Mac instead"),
        "{}",
        stderr(&out)
    );
    assert!(
        route_log(&rig).contains("\there\tthe runner is busy"),
        "{}",
        route_log(&rig)
    );

    // --auto is the default, busy bounce included (review r2).
    let held = hold_locks(&[host.join("slot-0.lock"), host.join("slot-1.lock")], 60);
    let out = rig.run(&repo, &["--auto", "sh", "-c", "pwd; : cargo clippy"]);
    drop(held);
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("the runner is busy"),
        "{}",
        stderr(&out)
    );

    // One Rust lane taken is not busy: the runner has a second (R0).
    let one = hold_locks(&[host.join("cargo.lock")], 60);
    let rust = rig.run(&repo, &["sh", "-c", "pwd; : cargo test"]);
    drop(one);
    assert!(ran_there(&rust), "{}", stderr(&rust));
    assert!(!stderr(&rust).contains("is busy"), "{}", stderr(&rust));

    let held = hold_locks(&[host.join("cargo.lock"), host.join("cargo-1.lock")], 60);
    let rust = rig.run(&repo, &["sh", "-c", "pwd; : cargo test"]);
    let other = rig.run(&repo, &["sh", "-c", "pwd; : npx vitest run"]);
    drop(held);
    assert!(!ran_there(&rust), "{}", stderr(&rust));
    assert!(
        stderr(&rust).contains("the runner is busy (its Rust build lock is taken)"),
        "{}",
        stderr(&rust)
    );
    assert!(ran_there(&other), "{}", stderr(&other));

    // This Mac busy too: it stays there, and waits there for the runner's
    // Rust lock, let go 2 s later.
    let local = hold_locks(&[here.join("slot-0.lock"), here.join("slot-1.lock")], 60);
    let briefly = hold_locks(&[host.join("cargo.lock"), host.join("cargo-1.lock")], 2);
    let out = rig.run(&repo, &["sh", "-c", "pwd; : cargo check"]);
    drop(briefly);
    drop(local);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(!stderr(&out).contains("is busy"), "{}", stderr(&out));
}

/// R0 (2026-11-14): a runner has two Rust lanes. Two gates from two
/// checkouts run on the host at once (each waits, in its own command, for
/// the other to have started: one lane alone would time it out), each with 5
/// cargo jobs and a build folder of its own.
#[test]
fn two_gates_run_at_once_on_a_runner_with_a_build_folder_each() {
    let rig = Remote::new();
    with_cargo_toml(&rig);
    let repo = rig.repo();
    let other = rig.path().join("other-tree");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &other.to_string_lossy(),
            "HEAD",
        ],
    );
    let marks = rig.path().join("marks");
    std::fs::create_dir(&marks).unwrap();
    // Says it started, then waits (30 s at most) for the other to have too.
    let gate = format!(
        "d={}; touch \"$d/$$\"; n=0; while [ \"$(ls \"$d\" | wc -l)\" -lt 2 ] && [ \"$n\" -lt 300 ]; do n=$((n + 1)); sleep 0.1; done; echo \"both=$(ls \"$d\" | wc -l | tr -d ' ') jobs=$CARGO_BUILD_JOBS target=$CARGO_TARGET_DIR\"; : cargo test",
        marks.display()
    );
    let spawn = |cwd: &Path| {
        rig.cmd(cwd, &["--on", "fakehost", "sh", "-c", &gate])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = spawn(&repo);
    let second = spawn(&other);
    let outs = [first, second].map(|c| c.wait_with_output_within().unwrap());
    let mut targets = Vec::new();
    for out in &outs {
        assert!(out.status.success(), "{}", stderr(out));
        let said = stdout(out);
        assert!(
            said.contains("both=2 jobs=5 target="),
            "{said}{}",
            stderr(out)
        );
        targets.push(said.split("target=").nth(1).unwrap().trim().to_string());
    }
    assert_ne!(targets[0], targets[1], "a build folder each: {targets:?}");
    // The host's caps and its lanes: both free again.
    let host = rig.path().join("host-slots");
    assert!(!super::status_in(&host).cargo.held);
    assert!(!super::status_in(&host).cargo_lane_1.held);
}

/// R0: the memory watch. While the host has under 30% of its memory free the
/// second lane is off (a gate waits for lane 0 and says why), and it starts
/// the moment memory is back. The count the preparation reads
/// (`--remote-lanes`) follows the reading, and `HOLLOW_SLOT_RUST_LANES=1` in
/// the runner's own login keeps it to one.
#[test]
fn a_low_memory_runner_falls_back_to_one_rust_lane() {
    let rig = Remote::new();
    with_cargo_toml(&rig);
    let repo = rig.repo();
    let other = rig.path().join("other-tree");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &other.to_string_lossy(),
            "HEAD",
        ],
    );
    let level = rig.path().join("level");
    std::fs::write(&level, "80\n").unwrap();
    let sysctl = rig.path().join("bin").join("fake-sysctl");
    make_executable(&sysctl, "#!/bin/sh\ncat \"$(dirname \"$0\")/../level\"\n");
    let marks = rig.path().join("marks");
    std::fs::create_dir(&marks).unwrap();
    let with_floor = |cwd: &Path, gate: &str| {
        let mut cmd = rig.cmd(cwd, &["--on", "fakehost", "sh", "-c", gate]);
        cmd.env("FAKE_MIN_FREE_PCT", "30")
            .env("FAKE_SYSCTL", &sysctl)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd
    };
    // The first gate holds lane 0 until the release file appears.
    let first_gate = format!(
        "d={}; touch \"$d/a.in\"; n=0; while [ ! -e \"$d/release\" ] && [ \"$n\" -lt 300 ]; do n=$((n + 1)); sleep 0.1; done; echo \"a jobs=$CARGO_BUILD_JOBS\"; : cargo test",
        marks.display()
    );
    let second_gate = format!(
        "d={}; touch \"$d/b.in\"; echo \"b jobs=$CARGO_BUILD_JOBS\"; : cargo test",
        marks.display()
    );
    let first = with_floor(&repo, &first_gate).spawn().unwrap();
    until("the first gate has lane 0", || marks.join("a.in").exists());
    let host = rig.path().join("host-slots");
    assert!(super::status_in(&host).cargo.held);

    // Memory drops: the second gate waits although lane 1 is free.
    std::fs::write(&level, "12\n").unwrap();
    let second = with_floor(&other, &second_gate).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(4000));
    assert!(
        !marks.join("b.in").exists(),
        "the second gate started with memory under the floor"
    );
    assert!(!super::status_in(&host).cargo_lane_1.held);

    // Memory is back: it takes lane 1, beside the first, with 5 jobs.
    std::fs::write(&level, "80\n").unwrap();
    until("the second gate takes lane 1", || {
        marks.join("b.in").exists()
    });
    assert!(!marks.join("release").exists());
    std::fs::write(marks.join("release"), "").unwrap();
    let outs = [first, second].map(|c| c.wait_with_output_within().unwrap());
    for out in &outs {
        assert!(out.status.success(), "{}", stderr(out));
    }
    assert!(
        stdout(&outs[0]).contains("a jobs=5"),
        "{}",
        stdout(&outs[0])
    );
    assert!(
        stdout(&outs[1]).contains("b jobs=5"),
        "{}",
        stdout(&outs[1])
    );
    assert!(
        stderr(&outs[1]).contains(
            "waiting for the other Rust build to finish (one at a time while this runner's memory is under its floor)"
        ),
        "{}",
        stderr(&outs[1])
    );

    // The lane count the preparation reports.
    let one = Command::new("/bin/sh")
        .arg(rig.path().join("hollow-slot"))
        .arg("--remote-lanes")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin")
        .env("HOLLOW_SLOT_SYSCTL", &sysctl)
        .env("HOLLOW_SLOT_RUST_LANES", "1")
        .output_within()
        .unwrap();
    assert_eq!(stdout(&one), "1\n");
    let two = Command::new("/bin/sh")
        .arg(rig.path().join("hollow-slot"))
        .arg("--remote-lanes")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin")
        .env("HOLLOW_SLOT_SYSCTL", &sysctl)
        .output_within()
        .unwrap();
    assert_eq!(stdout(&two), "2\n", "a runner's default, memory 80%");
    std::fs::write(&level, "12\n").unwrap();
    let low = Command::new("/bin/sh")
        .arg(rig.path().join("hollow-slot"))
        .arg("--remote-lanes")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin")
        .env("HOLLOW_SLOT_SYSCTL", &sysctl)
        .output_within()
        .unwrap();
    assert_eq!(stdout(&low), "1\n", "memory 12%: lane 1 is off");
}

/// A command of no known kind that failed on the runner and passes here
/// stays here (review of #1113): its words can't show what it needs here.
#[test]
fn a_command_that_fails_there_and_passes_here_stays_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    // RIG_LOCAL_ONLY is this Mac's: the host never has it.
    let local = &["sh", "-c", "test -n \"$RIG_LOCAL_ONLY\""][..];
    for _ in 0..2 {
        let out = rig
            .cmd(&repo, local)
            .env("HOLLOW_SLOT_LONG_SECS", "0")
            .output_within()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let out = rig
        .cmd(&repo, local)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "failed there: {}", stderr(&out));
    assert!(
        stderr(&out).contains("finished on fakehost: exit 1"),
        "{}",
        stderr(&out)
    );
    let out = rig
        .cmd(&repo, local)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("failed on the runner last time (exit 1) and passes here, so it runs here)"),
        "{}",
        stderr(&out)
    );
}

/// HOLLOW_SLOT_ROUTE=mac-mini (the Builds and tests setting) also skips a runner that didn't
/// answer a moment ago, instead of waiting out ssh each call.
#[test]
fn the_setting_skips_a_runner_that_didnt_answer() {
    let rig = Remote::new();
    with_runner(&rig, "down");
    let repo = rig.repo();
    let argv = rig.path().join("argv.log");
    let quick = &["sh", "-c", "pwd; : npx vitest run src/a.test.ts"][..];
    let out = rig
        .cmd(&repo, quick)
        .env("HOLLOW_SLOT_ROUTE", "mac-mini")
        .env("FAKE_ARGV_LOG", &argv)
        .output_within()
        .unwrap();
    assert!(
        stderr(&out).contains("down can't be reached"),
        "{}",
        stderr(&out)
    );
    let calls = std::fs::read_to_string(&argv).unwrap().lines().count();
    let out = rig
        .cmd(&repo, quick)
        .env("HOLLOW_SLOT_ROUTE", "mac-mini")
        .env("FAKE_ARGV_LOG", &argv)
        .output_within()
        .unwrap();
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("on this Mac (the runner didn't answer "),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(&argv).unwrap().lines().count(),
        calls,
        "no ssh call"
    );
}

/// Review r2: a command that couldn't start on the runner (127) stays here
/// even without a pass here; a host whose own setup fails exits 247, and
/// the call runs the command here (nothing ran there).
#[test]
fn a_command_that_cant_start_there_or_a_failed_setup_runs_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let missing = &["sh", "-c", "pwd; : cargo test; hollow-no-such-tool-here"][..];
    let out = rig.run(&repo, missing);
    assert_eq!(out.status.code(), Some(127), "{}", stderr(&out));
    assert!(ran_there(&out), "{}", stderr(&out));
    let out = rig.run(&repo, missing);
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on this Mac (cargo test couldn't start on the runner last time (exit 127), so it runs here; hollow-slot --remote tries it there again)"),
        "{}",
        stderr(&out)
    );

    // A folder the host's copy doesn't have (ignored here): its setup fails.
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nonly-here/\n").unwrap();
    let here = repo.join("only-here");
    std::fs::create_dir_all(&here).unwrap();
    let out = rig.run(&here, &["sh", "-c", "pwd; : cargo check; exit 3"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "the command's own code, here: {}",
        stderr(&out)
    );
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("nothing ran on fakehost: it couldn't get the checkout ready (its line above says why); running on this Mac instead"),
        "{}",
        stderr(&out)
    );
    assert!(
        !route_history(&rig).contains("\tcargo check\tfakehost\t"),
        "{}",
        route_history(&rig)
    );
}

/// Review r2: a writer is remembered past the last 9 runs, and a rewrite of
/// a file that was already changed counts (the tree, not git status lines).
#[test]
fn a_writer_is_remembered_and_rewrites_count() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    std::fs::write(repo.join("kept.txt"), "already changed\n").unwrap();
    let rewrite = &["sh", "-c", "echo again; printf more | tee -a kept.txt"][..];
    let out = rig
        .cmd(&repo, rewrite)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out));
    let history = route_history(&rig);
    let line = history.lines().last().unwrap().to_string();
    assert!(line.ends_with("\t1"), "the rewrite counted: {line}");
    // Ten passing runs that changed nothing don't make it forget.
    let mut fields: Vec<String> = line.split('\t').map(str::to_string).collect();
    fields[6] = "0".into();
    let mut more = String::new();
    for _ in 0..10 {
        more.push_str(&fields.join("\t"));
        more.push('\n');
    }
    std::fs::write(
        rig.path().join("slots").join("route-history"),
        format!("{history}{more}"),
    )
    .unwrap();
    let out = rig
        .cmd(&repo, rewrite)
        .env("HOLLOW_SLOT_LONG_SECS", "0")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "echo again changed this checkout when it ran, so it runs where its changes land"
        ),
        "{}",
        stderr(&out)
    );
}

/// Review r3: a > redirect that --fetch brings back can go; -u that isn't a
/// test runner's (sort -u) and quoted > (an awk program) are no writers.
#[test]
fn fetched_redirects_and_other_lookalikes_can_go() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let out = rig.run(
        &repo,
        &[
            "--fetch",
            "out.log",
            "sh",
            "-c",
            ": cargo test > out.log; pwd",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("on the runner (cargo test"),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("copied out.log back"),
        "{}",
        stderr(&out)
    );
    for cmd in [
        ": cargo check 2>&1 | sort -u; pwd",
        ": cargo build; awk '$1 > 0' kept.txt; pwd",
    ] {
        let out = rig.run(&repo, &["sh", "-c", cmd]);
        assert!(ran_there(&out), "{cmd}: {}", stderr(&out));
    }
}

/// A Linux runner (2026-11-03): its preparation says `os=linux`, and work
/// that needs macOS (a gate or Rust build of a Tauri checkout, Playwright's
/// webkit) stays here, first by the report its own preparation just got,
/// then at once by the kept one. Other work still goes, `--remote` goes
/// anyway and says so, and a Mac runner (`os=darwin`), or a kept report that
/// names no system (a runner tool from before this), takes it. A stand-in
/// `uname` on the host's PATH makes the rig's host Linux.
#[test]
fn a_linux_runner_keeps_macos_work_here() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    std::fs::create_dir_all(repo.join("src-tauri")).unwrap();
    std::fs::write(repo.join("src-tauri").join("tauri.conf.json"), "{}\n").unwrap();
    rig.commit(&repo, "a Tauri app");
    let uname = rig.path().join("host-bin").join("uname");
    make_executable(&uname, "#!/bin/sh\necho Linux\n");
    let caps = || std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    let gate = &["sh", "-c", "pwd; : npm run gate"][..];

    let out = rig.run(&repo, gate);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("hollow-slot: the runner runs Linux and this command needs macOS; running on this Mac instead"),
        "{}",
        stderr(&out)
    );
    assert!(caps().contains(" os=linux "), "{}", caps());

    // By the kept report, at once.
    let out = rig.run(&repo, gate);
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out)
            .contains("on this Mac (the runner runs Linux and gate needs macOS (it said so "),
        "{}",
        stderr(&out)
    );
    for (cmd, label) in [
        ("pwd; : cargo test --lib", "cargo test"),
        ("pwd; : npx playwright test --project=webkit", "Playwright"),
        // Review of item 4: a quoted name inside sh -c, vitest's browser.
        (
            "pwd; : npx playwright test --project=\"webkit\"",
            "Playwright",
        ),
        (
            "pwd; : npx vitest run --browser.name=webkit",
            "the whole vitest suite",
        ),
    ] {
        let out = rig.run(&repo, &["sh", "-c", cmd]);
        assert!(!ran_there(&out), "{cmd}: {}", stdout(&out));
        assert!(
            stderr(&out).contains(&format!("runs Linux and {label} needs macOS")),
            "{cmd}: {}",
            stderr(&out)
        );
    }
    // MLX is Apple silicon's: a transcription job stays here too.
    let out = rig.run(
        &repo,
        &[
            "--send",
            "kept.txt",
            "sh",
            "-c",
            "pwd; : uvx --from mlx-whisper mlx_whisper kept.txt",
        ],
    );
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("runs Linux and transcription needs macOS"),
        "{}",
        stderr(&out)
    );
    // Work that doesn't need macOS still goes.
    let out = rig.run(&repo, &["sh", "-c", "pwd; : npx vitest run"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    // --remote goes anyway, and says so.
    let out = rig.run(&repo, &["--remote", "sh", "-c", "pwd; : npm run gate"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "fakehost runs Linux and this command needs macOS; running it there anyway, as --remote asked"
        ),
        "{}",
        stderr(&out)
    );

    // A Mac runner takes it (its tools are re-read: the kept list goes).
    std::fs::remove_file(&uname).unwrap();
    std::fs::remove_file(rig.host_root().join("caps-tools")).unwrap();
    let out = rig
        .cmd(&repo, gate)
        .env("HOLLOW_SLOT_CAPS_SECS", "0")
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    assert!(caps().contains(" os=darwin "), "{}", caps());
    // A kept report that names no system is a Mac's.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        rig.path().join("slots").join("runner-caps"),
        format!("{now} fakehost rust=1.96.0 node=26.0.0 slots=0/2 cargo=free\n"),
    )
    .unwrap();
    let out = rig.run(&repo, gate);
    assert!(ran_there(&out), "{}", stderr(&out));
}

/// A Linux runner's memory floor (2026-11-03): with no sysctl or
/// memory_pressure reading, MemAvailable over MemTotal in /proc/meminfo
/// (`HOLLOW_SLOT_MEMINFO` stands in for it) is the free percentage; a
/// meminfo with no MemAvailable is no reading, which never waits.
#[test]
fn the_memory_floor_reads_proc_meminfo_on_linux() {
    let rig = Remote::new();
    let repo = rig.repo();
    let meminfo = rig.path().join("meminfo");
    let ran = rig.path().join("ran");
    let touch = format!("touch '{}'", ran.display());
    let floor = || {
        let mut cmd = rig.cmd(&repo, &["sh", "-c", &touch]);
        cmd.env_remove("HOLLOW_SLOT_MIN_FREE_PCT")
            .env("HOLLOW_SLOT_SYSCTL", "/usr/bin/false")
            .env("HOLLOW_SLOT_MEMORY_PRESSURE", "/usr/bin/false")
            .env("HOLLOW_SLOT_MEMINFO", &meminfo)
            .env("HOLLOW_SLOT_MEMORY_WAIT", "1");
        cmd.output_within().unwrap()
    };
    std::fs::write(
        &meminfo,
        "MemTotal:       32000000 kB\nMemFree:          100000 kB\nMemAvailable:    3840000 kB\n",
    )
    .unwrap();
    let out = floor();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("12% of its memory free, under its 30% floor"),
        "{}",
        stderr(&out)
    );
    assert!(!ran.exists(), "refused");
    std::fs::write(
        &meminfo,
        "MemTotal: 32000000 kB\nMemAvailable: 16000000 kB\n",
    )
    .unwrap();
    let out = floor();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ran.exists(), "50% free: it ran");
    std::fs::remove_file(&ran).unwrap();
    std::fs::write(&meminfo, "MemTotal: 32000000 kB\nMemFree: 100000 kB\n").unwrap();
    let out = floor();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ran.exists(), "no reading: no wait");
}

/// util-linux flock(1) as hollow-slot calls it on a Linux runner, played by
/// perl: `[-o] [-n] [-E <code>] <file> <command...>`, a flock(2) lock (the
/// real one's kind, which perl's and lockf's see), the command's exit code,
/// and each call's arguments logged to FLOCK_LOG.
const FAKE_FLOCK: &str = r#"#!/usr/bin/perl
use Fcntl qw(:flock F_SETFD FD_CLOEXEC);
if (open(my $log, '>>', 'FLOCK_LOG')) { print $log "@ARGV\n"; close $log }
my ($close, $nb, $code) = (0, 0, 1);
while (@ARGV && $ARGV[0] =~ /\A-/) {
  my $o = shift;
  if ($o eq '-o') { $close = 1 } elsif ($o eq '-n') { $nb = 1 } elsif ($o eq '-E') { $code = shift } else { print STDERR "fake flock: $o\n"; exit 64 }
}
my $file = shift;
open(my $fh, '>>', $file) or exit 66;
flock($fh, $nb ? LOCK_EX | LOCK_NB : LOCK_EX) or exit $code;
fcntl($fh, F_SETFD, FD_CLOEXEC) if $close;
system { $ARGV[0] } @ARGV;
exit($? == -1 ? 127 : $? & 127 ? 128 + ($? & 127) : $? >> 8);
"#;

/// A Linux runner's locks (2026-11-03) are util-linux flock's, picked by
/// uname: a try is `-o -n -E 75` (the lock kept from the command, given up
/// at once with lockf -t 0's exit code), a wait `-o` alone. A stand-in flock
/// (`HOLLOW_SLOT_FLOCK`) and a `uname` that says Linux play it here, on this
/// side and the host's; the real flock ran in a Linux container (BUILD_LOG
/// 2026-11-03).
#[test]
fn a_linux_host_takes_its_locks_with_flock() {
    let rig = Remote::new();
    let repo = rig.repo();
    let linux = rig.path().join("linux-bin");
    std::fs::create_dir(&linux).unwrap();
    make_executable(&linux.join("uname"), "#!/bin/sh\necho Linux\n");
    let log = rig.path().join("flock.log");
    let flock = rig.path().join("bin").join("fake-flock");
    make_executable(
        &flock,
        &FAKE_FLOCK.replace("FLOCK_LOG", &log.display().to_string()),
    );
    let logged = || std::fs::read_to_string(&log).unwrap_or_default();
    let linux_cmd = |args: &[&str]| {
        let mut cmd = rig.cmd(&repo, args);
        cmd.env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", linux.display()),
        )
        .env("HOLLOW_SLOT_FLOCK", &flock);
        cmd.output_within().unwrap()
    };
    let slots = rig.path().join("slots");

    // Both slots taken for 2 s (perl's flock(2), which flock sees): it waits,
    // then runs in one.
    let held = hold_locks(&[slots.join("slot-0.lock"), slots.join("slot-1.lock")], 2);
    let out = linux_cmd(&["sh", "-c", "echo ran"]);
    drop(held);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "ran\n", "{}", stderr(&out));
    assert!(
        stderr(&out).contains("both build slots are in use; waiting"),
        "{}",
        stderr(&out)
    );
    assert!(
        logged().contains(&format!(
            "-o -n -E 75 {}/slot-0.lock /bin/sh -c ",
            slots.display()
        )),
        "{}",
        logged()
    );
    // The dev-server lock taken: refused with 75, as with lockf.
    let held = hold_locks(&[slots.join("dev-server.lock")], 30);
    let out = linux_cmd(&["--dev-server", "true"]);
    drop(held);
    assert_eq!(out.status.code(), Some(75), "{}", stderr(&out));

    // The host waits for its worktree folder with -o alone.
    let out = rig
        .cmd(&repo, &["--on", "fakehost", "pwd"])
        .env(
            "FAKE_PATH",
            format!(
                "{}:{}:/usr/bin:/bin:/usr/sbin:/sbin",
                linux.display(),
                rig.path().join("host-bin").display()
            ),
        )
        .env("FAKE_FLOCK", &flock)
        .output_within()
        .unwrap();
    assert!(ran_there(&out), "{}", stderr(&out));
    let work = format!("{}/", rig.host_root().join("work").display());
    let host = logged();
    assert!(
        host.lines()
            .any(|l| l.starts_with(&format!("-o {work}")) && l.contains(".lock /bin/sh ")),
        "{host}"
    );
    assert!(
        host.lines()
            .any(|l| l.starts_with(&format!("-o -n -E 75 {work}"))
                && l.ends_with(".lock /usr/bin/true")),
        "{host}"
    );
}

/// Review of item 4: a Linux runner seeds a new worktree's build folder only
/// with a reflink clone (`cp -a --reflink=always`); where its file system
/// can't clone (ext4) the folder starts empty, said once, instead of a whole
/// copy made while holding the other folder's lock. The host is Linux by a
/// stand-in `uname`, and this Mac's cp has no `--reflink`, so the probe of a
/// one-byte file fails as it does on ext4. macOS only, for that cp.
#[cfg(target_os = "macos")]
#[test]
fn a_linux_runner_without_reflinks_starts_a_build_folder_empty() {
    let rig = Remote::new();
    let repo = rig.repo();
    with_cargo_toml(&rig);
    let other = rig.path().join("other-tree");
    rig.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &other.to_string_lossy(),
            "HEAD",
        ],
    );
    make_executable(
        &rig.path().join("host-bin").join("uname"),
        "#!/bin/sh\necho Linux\n",
    );
    let build = "mkdir -p \"$CARGO_TARGET_DIR\"; [ -e \"$CARGO_TARGET_DIR/built\" ] && echo seeded; echo x > \"$CARGO_TARGET_DIR/built\"";
    let out = rig.run(&repo, &["--on", "fakehost", "sh", "-c", build]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = rig.run(&other, &["--on", "fakehost", "sh", "-c", build]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "not seeded: {}", stderr(&out));
    let said = stderr(&out);
    assert!(
        said.contains("Rust build folder on ")
            && said.contains(" empty: its file system can't clone "),
        "{said}"
    );
    assert!(!said.contains("as a clone of"), "{said}");
    let strays: Vec<String> = std::fs::read_dir(rig.host_root().join("targets"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".seed-"))
        .collect();
    assert!(strays.is_empty(), "the probe cleans up: {strays:?}");
}

/// Stand-ins for Postgres's tools in `dir` (smart routing 6): `postgres`
/// records its start and its stop (INT or TERM) in `log` and marks its data
/// folder running meanwhile; initdb makes the folder; pg_isready, createdb
/// and the version answers say yes.
fn fake_postgres(dir: &Path, log: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let log = log.display();
    // perl, not sh: a shell started with & ignores SIGINT for good, while
    // the real server sets its own handlers (fast shutdown on INT). The
    // handlers come first, then its socket (the wrapper waits for it), then
    // the start line; it records its process group too.
    make_executable(
        &dir.join("postgres"),
        &format!(
            "#!/usr/bin/perl\nuse IO::Socket::UNIX;\nif (($ARGV[0] // '') eq '--version') {{ print \"postgres (PostgreSQL) 17.0 (stand-in)\\n\"; exit 0 }}\nmy ($d, $p, $k) = ('', '', '');\nfor my $i (0 .. $#ARGV) {{ $d = $ARGV[$i + 1] if $ARGV[$i] eq '-D'; $p = $ARGV[$i + 1] if $ARGV[$i] eq '-p'; $k = $ARGV[$i + 1] if $ARGV[$i] eq '-k' }}\nsub note {{ open(my $f, '>>', '{log}') or die; print $f \"@_ $d\\n\"; close $f }}\n$SIG{{INT}} = $SIG{{TERM}} = sub {{ unlink \"$d/running\"; note('stop'); exit 0 }};\nmy $s = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Local => \"$k/.s.PGSQL.$p\", Listen => 1) or die \"socket: $!\";\nopen(my $r, '>', \"$d/running\"); close $r;\nnote('start pgid=' . getpgrp());\nsleep 1 while 1;\n"
        ),
    );
    make_executable(
        &dir.join("pg_ctl"),
        "#!/bin/sh\necho 'pg_ctl (PostgreSQL) 17.0'\n",
    );
    make_executable(&dir.join("initdb"), "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -D) mkdir -p \"$2\"; shift 2 ;; *) shift ;; esac; done\n");
    make_executable(&dir.join("createdb"), "#!/bin/sh\nexit 0\n");
    // Like the real one, steered elsewhere by PGSERVICE or PGSSLMODE.
    make_executable(
        &dir.join("pg_isready"),
        "#!/bin/sh\n[ -z \"$PGSERVICE$PGSSLMODE\" ] || exit 2\nexit 0\n",
    );
}

/// `--postgres` gives the command a throwaway database wherever it runs:
/// here (no runner) and on the runner, DATABASE_URL (with its password)
/// and PG* set for it, the server stopped and its folder gone after, the
/// command's exit code kept; it needs Postgres there, and a database
/// variable the command sets itself still keeps it here.
#[test]
fn postgres_gives_the_command_a_throwaway_database_wherever_it_runs() {
    let rig = Remote::new();
    let repo = rig.repo();
    let pg = rig.path().join("pg");
    let log = rig.path().join("pg.log");
    fake_postgres(&pg, &log);
    let local_path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", pg.display());
    let show = "echo \"url=$DATABASE_URL shadow=$SHADOW_DATABASE_URL host=$PGHOST db=$PGDATABASE workers=$VITEST_MAX_WORKERS service=${PGSERVICE-unset}\"; exit 4";
    let out = rig
        .cmd(&repo, &["--postgres", "sh", "-c", show])
        .env("PATH", &local_path)
        .env("HOLLOW_SLOT_PG_DIRS", "")
        .env("PGSERVICE", "a-real-one")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    let said = stdout(&out);
    assert!(said.starts_with("url=postgresql://postgres:"), "{said}");
    let pw = said
        .trim_start_matches("url=postgresql://postgres:")
        .split('@')
        .next()
        .unwrap();
    assert_eq!(pw.len(), 32, "a random password: {said}");
    assert!(
        said.contains("@127.0.0.1:") && said.contains("/hollow_test shadow=postgresql://postgres:"),
        "{said}"
    );
    assert!(
        said.contains("/hollow_test_shadow host=127.0.0.1 db=hollow_test workers=1 service=unset"),
        "{said}"
    );
    assert!(
        stderr(&out).contains("hollow-slot: --postgres: a throwaway Postgres 17.0 on "),
        "{}",
        stderr(&out)
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(lines.len(), 2, "{calls}");
    assert!(lines[0].starts_with("start pgid="), "{calls}");
    let folder = lines[0]
        .split(' ')
        .nth(2)
        .unwrap()
        .trim_end_matches("/data")
        .to_string();
    assert!(folder.starts_with("/tmp/hs-pg."), "{calls}");
    assert_eq!(lines[1], format!("stop {folder}/data"), "{calls}");
    assert!(!Path::new(&folder).exists(), "its folder is gone: {folder}");

    // No Postgres: it says so, exit 69, and the command never runs.
    let out = rig
        .cmd(&repo, &["--postgres", "sh", "-c", "echo ran"])
        .env("HOLLOW_SLOT_PG_DIRS", "")
        .output_within()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("--postgres: no Postgres on"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        rig.run(&repo, &["--postgres", "--dev-server", "true"])
            .status
            .code(),
        Some(2)
    );

    // On the runner: its preparation reports Postgres, so the command goes
    // there with --postgres, and its own server runs there.
    with_runner(&rig, "fakehost");
    let host_pg = rig.path().join("host-bin");
    fake_postgres(&host_pg, &log);
    std::fs::remove_file(&log).unwrap();
    let it = &["--postgres", "sh", "-c", "pwd; : npx prisma migrate deploy && : npx vitest run --config vitest.integration.config.ts; echo \"db=$PGDATABASE\""][..];
    let out = rig
        .cmd(&repo, it)
        .env("FAKE_PG_DIRS", &host_pg)
        .output_within()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ran_there(&out), "{}", stdout(&out));
    assert!(stdout(&out).contains("db=hollow_test"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("on the runner (the whole vitest suite"),
        "{}",
        stderr(&out)
    );
    let caps = std::fs::read_to_string(rig.path().join("slots").join("runner-caps")).unwrap();
    assert!(caps.contains(" postgres=17.0 "), "{caps}");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.starts_with("start pgid=") && calls.contains("\nstop /tmp/hs-pg."),
        "{calls}"
    );
    // Without --postgres, prisma migrate uses this Mac's database: here.
    let out = rig.run(&repo, &it[1..]);
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("prisma migrate writes into a database this Mac reaches"),
        "{}",
        stderr(&out)
    );
    // A database the command names itself isn't the throwaway one: here.
    let out = rig
        .cmd(
            &repo,
            &[
                "--postgres",
                "env",
                "DATABASE_URL=postgresql://localhost/x",
                "sh",
                "-c",
                "pwd; : npx vitest run",
            ],
        )
        .env("PATH", &local_path)
        .env("HOLLOW_SLOT_PG_DIRS", "")
        .output_within()
        .unwrap();
    assert!(!ran_there(&out), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("it gives the command DATABASE_URL, a database this Mac reaches"),
        "{}",
        stderr(&out)
    );
}

/// Review of #1121: the server is a direct child in the call's process
/// group, so a stopped call stops it, even one whose wrapper was killed
/// outright (pg_ctl's detached server outlived that); a folder an earlier,
/// killed wrapper left behind is swept by the next.
///
/// Its folders live in a /tmp folder of its own (HOLLOW_SLOT_PG_TMP): every
/// wrapper sweeps the dead-owner folders beside its own, so with the shared
/// /tmp a wrapper of the other Postgres test, or of another gate on the same
/// runner account, could sweep the killed wrapper's folder before the check
/// below that it is still there (11-04: #1166's gate failed that way and
/// passed alone).
#[test]
fn a_stopped_postgres_call_leaves_no_server_or_folder() {
    use std::os::unix::process::CommandExt;
    let rig = Remote::new();
    let repo = rig.repo();
    let pg = rig.path().join("pg");
    let log = rig.path().join("pg.log");
    fake_postgres(&pg, &log);
    let local_path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", pg.display());
    // Short, under /tmp: the server's socket path has a 103-byte limit.
    let pg_tmp = tempfile::Builder::new()
        .prefix("hsp")
        .tempdir_in("/tmp")
        .unwrap();
    let stale = tempfile::Builder::new()
        .prefix("hs-pg.")
        .tempdir_in(pg_tmp.path())
        .unwrap()
        .keep();
    // 999999 is past macOS's highest pid: a gone owner.
    std::fs::write(stale.join("owner"), "999999\n").unwrap();
    let call = || {
        rig.cmd(&repo, &["--postgres", "sh", "-c", "sleep 30"])
            .env("PATH", &local_path)
            .env("HOLLOW_SLOT_PG_DIRS", "")
            .env("HOLLOW_SLOT_PG_TMP", pg_tmp.path())
            .process_group(0)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut child = call();
    let group = child.id().to_string();
    until("the server started", || {
        std::fs::read_to_string(&log).is_ok_and(|l| l.contains("start "))
    });
    assert!(!stale.exists(), "an owner that's gone: its folder swept");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!("start pgid={group} ")),
        "the server is in the call's group: {calls}"
    );
    let folder = calls
        .lines()
        .next()
        .unwrap()
        .split(' ')
        .nth(2)
        .unwrap()
        .trim_end_matches("/data")
        .to_string();
    // Kill the wrapper alone, with no chance to clean up, then stop the
    // group as Hollow's Stop does: the server still gets it.
    let wrapper = Command::new("/usr/bin/pgrep")
        .args(["-g", &group, "-f", "hollow-slot-postgres"])
        .output_within()
        .unwrap();
    let wrapper = String::from_utf8_lossy(&wrapper.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(!wrapper.is_empty(), "the wrapper is running");
    Command::new("/bin/kill")
        .args(["-KILL", &wrapper])
        .status()
        .unwrap();
    Command::new("/bin/kill")
        .args(["-TERM", &format!("-{group}")])
        .status()
        .unwrap();
    let _ = child.wait_within();
    until("the server stopped", || {
        std::fs::read_to_string(&log).is_ok_and(|l| l.contains("stop "))
    });
    // Its folder stays (nobody was left to delete it) until the next call.
    assert!(Path::new(&folder).exists(), "{folder}");
    let mut next = call();
    until("the next call's server started", || {
        std::fs::read_to_string(&log).is_ok_and(|l| l.matches("start ").count() == 2)
    });
    until("the killed wrapper's folder went", || {
        !Path::new(&folder).exists()
    });
    // The second call's own server and folder: gone once it is stopped (no
    // process names its folder). Asserted on what is left, not on the
    // stand-in's stop line, which a full gate's load once kept from landing.
    let calls = std::fs::read_to_string(&log).unwrap();
    let second = calls
        .lines()
        .filter(|l| l.starts_with("start "))
        .nth(1)
        .and_then(|l| l.split(' ').nth(2))
        .unwrap()
        .trim_end_matches("/data")
        .to_string();
    Command::new("/bin/kill")
        .args(["-TERM", &format!("-{}", next.id())])
        .status()
        .unwrap();
    let _ = next.wait_within();
    until("the second call left nothing", || {
        let running = Command::new("/usr/bin/pgrep")
            .args(["-f", &second])
            .output_within()
            .unwrap();
        !Path::new(&second).exists() && running.stdout.is_empty()
    });
}

/// A step that readies the rest (npm ci, prisma generate) doesn't keep a
/// heavy command here; alone, or with an install that can change the lock
/// file, it does.
#[test]
fn setup_steps_go_with_the_heavy_work_they_ready() {
    let rig = Remote::new();
    with_runner(&rig, "fakehost");
    let repo = rig.repo();
    let out = rig.run(&repo, &["sh", "-c", "pwd; : npm ci && : cargo test"]);
    assert!(ran_there(&out), "{}", stderr(&out));
    let out = rig.run(
        &repo,
        &["sh", "-c", "pwd; : prisma generate && : tsc --noEmit"],
    );
    assert!(ran_there(&out), "{}", stderr(&out));
    let out = rig.run(
        &repo,
        &[
            "sh",
            "-c",
            "pwd; : pnpm install --frozen-lockfile && : pnpm test",
        ],
    );
    assert!(ran_there(&out), "{}", stderr(&out));
    for (cmd, why) in [
        (
            "pwd; : npm ci",
            "npm ci installs packages into this checkout",
        ),
        (
            "pwd; : npm ci; echo done",
            "npm ci installs packages into this checkout",
        ),
        (
            "pwd; : npm install && : cargo test",
            "npm install installs packages into this checkout",
        ),
        (
            "pwd; : prisma migrate dev && : npx vitest run",
            "prisma migrate dev writes into this checkout",
        ),
        (
            "pwd; : cargo test && : npm ci",
            "npm ci installs packages into this checkout",
        ),
    ] {
        let out = rig.run(&repo, &["sh", "-c", cmd]);
        assert!(!ran_there(&out), "{cmd}: {}", stdout(&out));
        assert!(stderr(&out).contains(why), "{cmd}: {}", stderr(&out));
    }
}
