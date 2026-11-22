//! Remote sessions (the remote runner, Phase 2). A stand-in `ssh` plays the
//! host: it runs the remote command with `zsh` in a fake home with only a
//! login's variables, and does `-R`'s forward with a symlink to the local
//! socket. The agent is the runner's own probe (`hollow-remote-run --probe`),
//! so no agent sign-in is needed. The broker is a real `run_auth` peer check.
//!
//! `real_runner_round_trip_and_stop` runs the same flow against a real host
//! when `HOLLOW_REMOTE_IT_HOST` names one (`--ignored`): Alex's is
//! `runner-host`.

use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

const FAKE_SSH: &str = r#"#!/bin/sh
fwd=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -R) fwd="$2"; shift 2 ;;
    -o | -p | -i | -l) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
host="$1"
shift
if [ "$host" = down ]; then
  echo "ssh: connect to host down port 22: Operation timed out" >&2
  exit 255
fi
if [ -n "$fwd" ]; then
  remote="${fwd%%:*}"
  ln -s "${fwd#*:}" "$remote" || exit 255
fi
cd "$FAKE_HOME" || exit 255
# Like sshd, the command runs in a session of its own: stopping ssh here
# must not reach it, only end its connection.
/usr/bin/perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV or exit 127' env -i HOME="$FAKE_HOME" PATH="$FAKE_PATH" $FAKE_HOST_ENV /bin/zsh -f -c "$*"
rc=$?
[ -z "$fwd" ] || rm -f "$remote"
exit $rc
"#;

/// Reads one line from the broker connection, answers whether the run
/// tree check admitted the peer, then copies the reply back.
const PROBE_SERVER: &str = r#"#!/usr/bin/perl
use IO::Socket::UNIX;
my $s = IO::Socket::UNIX->new(Peer => $ENV{BROKER}) or die "no broker: $!";
my $l = <STDIN>; print $s $l; my $r = <$s> // "nothing\n"; print $r;
"#;

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        // Short: the forwarded socket's path must fit a socket address.
        let dir = tempfile::Builder::new()
            .prefix("rs")
            .tempdir_in("/tmp")
            .unwrap();
        let rig = Rig { dir };
        for sub in ["repo", "host", "bin"] {
            std::fs::create_dir(rig.path().join(sub)).unwrap();
        }
        // The host's paths written into the stand-in, so nothing here has to
        // set a variable for the whole test process.
        let fake_ssh = FAKE_SSH
            .replace("$FAKE_HOME", &rig.path().join("host").to_string_lossy())
            .replace("$FAKE_PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        rig.exe("bin/fake-ssh", &fake_ssh);
        rig.host_env("HOLLOW_REMOTE_MIN_FREE_GB=0");
        rig.exe("bin/probe-server", PROBE_SERVER);
        let repo = rig.repo();
        std::fs::write(repo.join("kept.txt"), "committed\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "to delete\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
        rig.git(&["init", "-q", "-b", "work"]);
        rig.git(&["add", "-A"]);
        rig.git(&["commit", "-q", "-m", "first"]);
        rig
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    /// The variables the host's commands get beside a login's (the disk
    /// floor is off unless a test sets it: this Mac's own free space must
    /// not decide what a test's host prunes).
    fn host_env(&self, vars: &str) {
        let fake_ssh = FAKE_SSH
            .replace("$FAKE_HOME", &self.path().join("host").to_string_lossy())
            .replace("$FAKE_PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .replace("$FAKE_HOST_ENV", vars);
        self.exe("bin/fake-ssh", &fake_ssh);
    }
    fn repo(&self) -> PathBuf {
        self.path().join("repo")
    }
    fn exe(&self, rel: &str, body: &str) {
        let p = self.path().join(rel);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .current_dir(self.repo())
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
    fn ssh(&self) -> String {
        self.path()
            .join("bin/fake-ssh")
            .to_string_lossy()
            .into_owned()
    }
    /// Run `f` (kept as a seam: the stand-in carries the host's paths).
    fn with_host<T>(&self, f: impl FnOnce() -> T) -> T {
        f()
    }
    /// A config file like the run's private one: one server, the probe
    /// server here, pointed at `broker`.
    fn config(&self, broker: &Path) -> PathBuf {
        let config = json!({ "mcpServers": { "hollow_probe": {
            "command": self.path().join("bin/probe-server"),
            "args": [],
            "env": { "BROKER": broker },
        }}});
        let p = self.path().join("run-config.json");
        std::fs::write(&p, config.to_string()).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        p
    }
}

/// A broker socket served by a thread: each connection's first line is
/// answered `admitted` or `refused` by the real peer check against `peers`
/// for run `run-1`.
fn broker(path: &Path, peers: &'static crate::run_auth::Peers) {
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let verdict =
                crate::run_auth::check_peer(peers, conn.as_raw_fd(), "test broker", Some("run-1"));
            let mut line = String::new();
            let _ = BufReader::new(&conn).read_line(&mut line);
            let answer = match verdict {
                Ok(()) => format!("admitted {}", line.trim()),
                Err(_) => "refused".into(),
            };
            let _ = writeln!(conn, "{answer}");
        }
    });
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Spawn `cmd` in a process group of its own (as Hollow's guard does),
/// register it as run `run-1`'s leader, and return it.
fn spawn_leader(
    rt: &tokio::runtime::Runtime,
    mut cmd: Command,
    peers: &'static crate::run_auth::Peers,
) -> tokio::process::Child {
    cmd.process_group(0);
    let child = rt.block_on(async { cmd.spawn().unwrap() });
    peers.register("tag-test", child.id().unwrap() as i32, vec!["run-1".into()]);
    child
}

fn wait_output(
    rt: &tokio::runtime::Runtime,
    child: tokio::process::Child,
) -> (i32, String, String) {
    let out = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
            .await
            .expect("no hang")
            .unwrap()
    });
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn probe_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), "t".repeat(40));
    env
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// The whole turn: the exact tree (a committed file, an uncommitted edit)
/// goes to the host and is checked out on the same branch; the agent runs
/// there with the token and the relay but none of this Mac's tracking
/// variables; a tool it calls is served here by a process the broker's peer
/// check admits as the run's own (a process outside the run is refused); and
/// its commit, new file and deletion come back, with this Mac's uncommitted
/// edit still uncommitted.
#[test]
fn a_turn_runs_on_the_host_with_tools_served_here_and_its_work_comes_back() {
    let rig = Rig::new();
    let repo = rig.repo();
    std::fs::write(repo.join("kept.txt"), "edited here, not committed\n").unwrap();
    let head = rig.git(&["rev-parse", "HEAD"]);
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
        .unwrap();
    assert_eq!(target.head, head);
    assert_ne!(target.snapshot, head, "a dirty tree goes as a snapshot");
    assert_eq!(target.branch.as_deref(), Some("work"));
    assert!(target
        .dir
        .ends_with(&format!("/hollow-remote/sessions/{}", target.wt_id)));
    assert!((2700..3000).contains(&target.port), "{}", target.port);
    let host_kept = std::fs::read_to_string(Path::new(&target.dir).join("kept.txt")).unwrap();
    assert_eq!(host_kept, "edited here, not committed\n");
    // The host's checkout: on the branch, at HEAD, the edit uncommitted.
    let host_status = std::process::Command::new("git")
        .args(["-C", &target.dir, "status", "--porcelain", "--branch"])
        .output()
        .unwrap();
    let host_status = String::from_utf8_lossy(&host_status.stdout);
    assert!(host_status.contains("## work"), "{host_status}");
    assert!(host_status.contains(" M kept.txt"), "{host_status}");

    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let args = [
        "where",
        "bridge:hollow_probe:hello",
        "write:new.txt:from the host",
        "commit:new.txt:the host's commit",
        "write:loose.txt:not committed there",
        "rm:gone.txt",
    ];
    let cmd = leader(
        "hollow-remote-probe",
        args.iter().map(|a| a.to_string()).collect(),
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    let where_line = out.lines().next().unwrap();
    let place: Value = serde_json::from_str(where_line).unwrap();
    assert_eq!(place["token_len"], 40, "the token reached the agent");
    assert_eq!(place["relay"], 1);
    assert_eq!(
        place["local_only"], "",
        "none of this Mac's tracking: {where_line}"
    );
    assert_eq!(place["port"], target.port.to_string());
    assert!(
        place["cwd"].as_str().unwrap().ends_with(&target.wt_id),
        "{where_line}"
    );
    assert!(
        out.contains("bridge hollow_probe: admitted hello"),
        "the tool here passed the broker's peer check: {out}\n{err}"
    );
    assert!(err.contains("hollow-remote: result "), "{err}");

    // A process outside the run is refused by the same broker.
    let mut outside = std::os::unix::net::UnixStream::connect(&broker_sock).unwrap();
    writeln!(outside, "sneak").unwrap();
    let mut answer = String::new();
    BufReader::new(&outside).read_line(&mut answer).unwrap();
    assert_eq!(answer.trim(), "refused");

    let collected = rig.with_host(|| collect(&target)).unwrap();
    let Collected::Applied { head: new_head } = collected else {
        panic!("{collected:?}");
    };
    assert_eq!(rig.git(&["rev-parse", "HEAD"]), new_head);
    assert_eq!(rig.git(&["log", "-1", "--format=%s"]), "the host's commit");
    assert_eq!(
        rig.git(&["rev-parse", "HEAD~1"]),
        head,
        "on top of the old HEAD"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("new.txt")).unwrap(),
        "from the host\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("loose.txt")).unwrap(),
        "not committed there\n"
    );
    assert!(!repo.join("gone.txt").exists());
    let status = rig.git(&["status", "--porcelain"]);
    // (`rig.git` trims, so the first line has lost its leading space.)
    assert!(status.contains("M kept.txt"), "still uncommitted: {status}");
    assert!(status.contains("D gone.txt"), "{status}");
    assert!(status.contains("?? loose.txt"), "{status}");
    assert!(!status.contains("new.txt"), "committed: {status}");
    peers.release("tag-test");
}

/// R5b (Q-R33): the runner hands the agent the prompt as its stdin and the
/// other private text on the descriptors the header names, and the agent's
/// argv and environment hold none of it. A stand-in `sh` reads them as Claude
/// Code would (stdin, `/dev/fd/7`, descriptor 8).
#[test]
fn the_runner_hands_the_agent_its_secrets_on_private_files_not_argv_or_env() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    std::fs::create_dir(rig.path().join("hostbin")).unwrap();
    rig.exe(
        "hostbin/claude",
        "#!/bin/sh\ncat; echo '|APPEND'; cat /dev/fd/7; echo '|TOKEN'; cat <&8; echo '|ARGV'; echo \"$*\"; echo '|ENV'; env\n",
    );
    rig.host_env(&format!(
        "HOLLOW_REMOTE_MIN_FREE_GB=0 PATH={}:/usr/bin:/bin:/usr/sbin:/sbin",
        rig.path().join("hostbin").display()
    ));
    let cmd = leader_with(
        "claude",
        vec!["-p".into()],
        BTreeMap::from([(
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_string(),
            "8".to_string(),
        )]),
        &target,
        Some(&config),
        false,
        Some("PROMPT-SECRET\nsecond line".into()),
        vec![
            HeaderFd {
                fd: 7,
                text: "APPENDED-SECRET".into(),
            },
            HeaderFd {
                fd: 8,
                text: "TOKEN-SECRET".into(),
            },
        ],
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(
        out.contains(
            "PROMPT-SECRET\nsecond line|APPEND\nAPPENDED-SECRET|TOKEN\nTOKEN-SECRET|ARGV\n-p\n|ENV"
        ),
        "{out}\n{err}"
    );
    let env = out.split("|ENV\n").nth(1).unwrap();
    for secret in ["PROMPT-SECRET", "APPENDED-SECRET", "TOKEN-SECRET"] {
        assert!(!env.contains(secret), "{secret} in the agent's environment");
    }
    peers.release("tag-test");
}

/// Stop kills the leader's process group here, and the run there goes on
/// (Phase 4: its supervisor outlives the connection) until the turn's finish
/// collects: that stops it, ending the agent's process group and every group
/// the agent started (a tool's command in a group of its own, as Claude's
/// Bash tool runs one), so nothing is left running there, and the work it did
/// before the stop comes back. And while it runs, its tool socket there
/// refuses a process outside the agent.
#[test]
fn stopping_the_turn_ends_the_agent_on_the_host() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let pid_file = rig.path().join("agent.pid");
    let group_file = rig.path().join("grouped.pid");
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "write:before-stop.txt:made before the stop".into(),
            format!("group:{}", group_file.display()),
            format!("pidfile:{}", pid_file.display()),
            "sleep:60".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let leader_pid = child.id().unwrap() as i32;
    let started = Instant::now();
    let agent = loop {
        if let Ok(p) = std::fs::read_to_string(&pid_file) {
            if let Ok(p) = p.trim().parse::<i32>() {
                break p;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the agent started"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(alive(agent));
    // The probe's `group` step forks a child that writes its own pid file
    // once it has its group, while `pidfile` writes at once, so under load
    // the agent's file can come first (11-04: a gate read the group file a
    // moment too early, "No such file or directory"). Wait for it too, on
    // its own clock (`wait_pid`, which also retries an empty file).
    let grouped = wait_pid(&group_file);
    assert!(alive(grouped));
    // Give the runner a sample of the agent's tree (once a second).
    std::thread::sleep(Duration::from_millis(1500));

    // A process outside the agent's tree is refused by its tool socket.
    let run_dir = rig.path().join("host/hollow-remote/run");
    let tools = std::fs::read_dir(&run_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".tools.sock"))
        .expect("the run's tool socket");
    let mut outsider = std::os::unix::net::UnixStream::connect(&tools).unwrap();
    writeln!(outsider, "hollow_probe").unwrap();
    outsider
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut got = String::new();
    let _ = std::io::Read::read_to_string(&mut outsider, &mut got);
    assert!(got.is_empty(), "refused: {got}");

    unsafe { libc::killpg(leader_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, child);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        alive(agent) && alive(grouped),
        "the run goes on without the connection"
    );
    // The turn's finish stops it there and brings the work done before.
    let stopped = Instant::now();
    let collected = rig.with_host(|| collect(&target)).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert!(
        !alive(agent) && !alive(grouped),
        "the agent and what it started there were ended"
    );
    assert!(stopped.elapsed() < Duration::from_secs(20));
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("before-stop.txt")).unwrap(),
        "made before the stop\n"
    );
    peers.release("tag-test");
}

/// The direct children of `pid` whose command line has `needle`.
fn children_of(pid: i32, needle: &str) -> Vec<i32> {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let p: i32 = it.next()?.parse().ok()?;
            let pp: i32 = it.next()?.parse().ok()?;
            (pp == pid && l.contains(needle)).then_some(p)
        })
        .collect()
}

/// The pid a probe's `pidfile:` action wrote, waiting for it.
fn wait_pid(path: &Path) -> i32 {
    let t0 = Instant::now();
    loop {
        if let Ok(p) = std::fs::read_to_string(path) {
            if let Ok(p) = p.trim().parse::<i32>() {
                return p;
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(30), "{}", path.display());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Phase 4: the connection drops mid-turn (the leader's ssh killed). The
/// run goes on there, the leader reconnects, and the output continues from
/// the byte it had: every line arrives exactly once, a tool call after the
/// drop is served here, the runner's marks stay with the leader, and the
/// work comes back.
#[test]
fn a_dropped_connection_reattaches_and_the_output_arrives_once() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    // A second server that stays open (an MCP server held by the agent).
    let mut both: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    both["mcpServers"]["held"] = json!({ "command": "/bin/cat", "args": [] });
    std::fs::write(&config, both.to_string()).unwrap();
    let rt = runtime();
    let mark = rig.path().join("dropped.pid");
    let held = rig.path().join("held.closed");
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "say:one".into(),
            "bridge:hollow_probe:before".into(),
            format!("hold:held:{}", held.display()),
            "err:hollow-remote: 0123456789abcdef done 0".into(),
            "err:said before the drop".into(),
            format!("pidfile:{}", mark.display()),
            "sleep:5".into(),
            // The connection held through the old forward was ended at the
            // reattach (its bridge would reconnect), not left hanging.
            format!("wait:{}", held.display()),
            "say:two".into(),
            "err:said after the drop".into(),
            "bridge:hollow_probe:after".into(),
            "write:across.txt:made across a drop".into(),
            "commit:across.txt:a commit across a drop".into(),
            "say:three".into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let leader_pid = child.id().unwrap() as i32;
    let agent = wait_pid(&mark);
    let ssh = children_of(leader_pid, "fake-ssh");
    assert_eq!(ssh.len(), 1, "{ssh:?}");
    unsafe { libc::kill(ssh[0], libc::SIGKILL) };
    assert!(alive(agent), "the run goes on there");
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    for line in [
        "one",
        "two",
        "three",
        "bridge hollow_probe: admitted before",
        "bridge hollow_probe: admitted after",
        "holding held",
        &format!("saw {}", held.display()),
    ] {
        assert_eq!(
            out.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{out}\n{err}"
        );
    }
    // Overnight R2: the agent's stderr arrives once too.
    for line in ["said before the drop", "said after the drop"] {
        assert_eq!(
            err.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{err}"
        );
    }
    assert!(err.contains("dropped; the run goes on there"), "{err}");
    assert!(err.contains("reconnected to fakehost"), "{err}");
    // The runner's marks (`hollow-remote: <nonce> in|err|done …`) stay with
    // the leader; a line in the agent's stderr shaped like one is just text.
    let marks = err
        .lines()
        .filter(|l| {
            let mut w = l.split(' ');
            w.next() == Some("hollow-remote:")
                && w.next()
                    .is_some_and(|n| n.len() == 16 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        marks,
        vec!["hollow-remote: 0123456789abcdef done 0"],
        "{err}"
    );
    assert!(err.contains("hollow-remote: result "), "{err}");
    let collected = rig.with_host(|| collect(&target)).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s"]),
        "a commit across a drop"
    );
    peers.release("tag-test");
}

/// Overnight R1: the turn's record as `begin_turn` keeps it, for a test's
/// own conversation (not in the in-memory table, as after a restart).
fn keep_record(target: &RemoteTarget) -> String {
    let key = format!("s-test-{}", random_hex(8));
    save_record(&turns_dir().unwrap(), &key, target, "claude").unwrap();
    key
}

/// Wait for the leader's state file to name the run's supervisor.
fn wait_state(turn: &str) -> Reattach {
    let path = state_path(&turns_dir().unwrap(), turn);
    let t0 = Instant::now();
    loop {
        if let Some(r) = read_state(&path) {
            return r;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "no state at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Overnight R1: Hollow quits mid-turn (its leader killed with its group,
/// as the quit's settle does) after handling the first line. The run goes
/// on there. The conversation's next turn, a Continue, finds the record and
/// reattaches through `--attach` from exactly the bytes Hollow handled: the
/// rest of the output arrives once (never the line it had), a tool call
/// after the restart is served here through the run's own relay token, and
/// the work comes back applied, not on a branch.
#[test]
fn a_restarted_hollow_reattaches_to_its_run_from_the_bytes_it_handled() {
    use tokio::io::AsyncBufReadExt;
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let key = keep_record(&target);
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let mark = rig.path().join("restart.pid");
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            r#"say:{"n":"one"}"#.into(),
            format!("pidfile:{}", mark.display()),
            "sleep:6".into(),
            r#"say:{"n":"two"}"#.into(),
            "bridge:hollow_probe:after the restart".into(),
            "stdin".into(),
            "write:restart.txt:made while Hollow was away".into(),
            "commit:restart.txt:a commit across a restart".into(),
            r#"say:{"n":"three"}"#.into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        true,
    )
    .unwrap();
    let mut first = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    // Its input stays open there (a steerable turn's), as Hollow left it.
    let first_in = first.stdin.take();
    let first_pid = first.id().unwrap() as i32;
    // Hollow handles "one" (and counts it, as a turn's drive does)...
    let dir = turns_dir().unwrap();
    let mut progress = Progress::start_in(&dir, &target.turn, 0).unwrap();
    let mut lines = tokio::io::BufReader::new(first.stdout.take().unwrap()).lines();
    let line = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(60), lines.next_line()).await })
        .expect("no hang")
        .unwrap()
        .unwrap();
    assert_eq!(line, r#"{"n":"one"}"#);
    progress.add(&line);
    let agent = wait_pid(&mark);
    let state = wait_state(&target.turn);
    // ...then quits: its leader and everything in its group go.
    unsafe { libc::killpg(first_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, first);
    peers.release("tag-test");
    drop(progress);
    drop(first_in);
    assert!(alive(agent), "the run goes on there without Hollow");

    // The next Hollow: no turn in its table, the record on disk.
    let stale = stale_turn(&key).expect("the earlier turn's record");
    let r = stale.reattach.clone().expect("what its leader knew");
    assert_eq!(r.run, state.run);
    assert_eq!(r.out, 12, "the bytes Hollow had handled: the first line");
    assert!(r.sup > 1);
    assert_eq!(
        stale_choice(
            &stale,
            crate::restart_resume::CONTINUE_PROMPT,
            crate::agent::AgentKind::Claude,
            Some("fakehost"),
            &rig.repo(),
        ),
        StaleChoice::Reattach
    );
    let settled = rig.with_host(|| settle_stale(&key, stale, StaleChoice::Reattach, true));
    let StaleOutcome::Reattach(again) = settled else {
        panic!("{settled:?}");
    };
    assert_eq!(again.reattach.as_ref().map(|r| r.out), Some(12));
    let cmd = leader(
        "hollow-remote-probe",
        vec!["say:never-run".into()],
        probe_env(),
        &again,
        Some(&config),
        true,
    )
    .unwrap();
    let mut second = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    // This turn's own message (the Continue) is not the run's: dropped. A
    // steer after it reaches the run, and the input's end lets it finish.
    {
        use tokio::io::AsyncWriteExt;
        let mut input = second.stdin.take().unwrap();
        rt.block_on(async {
            input.write_all(b"the Continue message\n").await.unwrap();
            input
                .write_all(b"a steer after the restart\n")
                .await
                .unwrap();
            input.flush().await.unwrap();
        });
        drop(input);
    }
    let (code, out, err) = wait_output(&rt, second);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(!out.contains(r#""one""#), "the line Hollow had:\n{out}");
    assert!(
        !out.contains("never-run"),
        "the new turn's command never runs:\n{out}"
    );
    assert!(
        !out.contains("the Continue message"),
        "this turn's own message isn't the run's:\n{out}"
    );
    for line in [
        r#"{"n":"two"}"#,
        r#"{"n":"three"}"#,
        "bridge hollow_probe: admitted after the restart",
        "stdin: a steer after the restart",
    ] {
        assert_eq!(
            out.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{out}\n{err}"
        );
    }
    assert!(!alive(agent));
    let collected = rig.with_host(|| collect(&again)).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s"]),
        "a commit across a restart"
    );
    forget_turn(&key, &again.turn);
    assert!(stale_turn(&key).is_none());
    peers.release("tag-test");
}

/// Overnight R1: after a restart, a message other than Continue doesn't
/// attach: the earlier turn's run is stopped there through its finish and
/// its work so far comes back applied before this turn prepares (today it
/// was stopped by the preparation and came back on a branch).
#[test]
fn another_message_after_a_restart_brings_the_earlier_turns_work_back() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let key = keep_record(&target);
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let mark = rig.path().join("other.pid");
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "write:before-quit.txt:made before Hollow quit".into(),
            format!("pidfile:{}", mark.display()),
            "sleep:60".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let first = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let first_pid = first.id().unwrap() as i32;
    let agent = wait_pid(&mark);
    let _ = wait_state(&target.turn);
    unsafe { libc::killpg(first_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, first);
    peers.release("tag-test");
    let stale = stale_turn(&key).unwrap();
    let choice = stale_choice(
        &stale,
        "Fix the other thing too",
        crate::agent::AgentKind::Claude,
        Some("fakehost"),
        &rig.repo(),
    );
    assert_eq!(choice, StaleChoice::BringBack);
    let settled = rig.with_host(|| settle_stale(&key, stale, choice, true));
    let StaleOutcome::BroughtBack { worktree, .. } = settled else {
        panic!("{settled:?}");
    };
    assert!(
        matches!(worktree.0, Ok(Collected::Applied { .. })),
        "{worktree:?}"
    );
    assert!(!alive(agent), "stopped there");
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("before-quit.txt")).unwrap(),
        "made before Hollow quit\n"
    );
    assert!(stale_turn(&key).is_none(), "its record went");
}

/// Overnight R1: a record whose run is no longer there (a preparation of
/// the folder since stopped and kept it, or a day passed) attaches to
/// nothing and goes; that preparation's salvage brings what it left.
#[test]
fn a_stale_turn_whose_run_is_gone_is_forgotten() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let key = keep_record(&target);
    let dir = turns_dir().unwrap();
    std::fs::write(
        state_path(&dir, &target.turn),
        format!(
            "run {}0000\ntoken {}\nsecret {}\nsup 4242\nerr 0\n",
            target.turn,
            "a".repeat(32),
            "b".repeat(32)
        ),
    )
    .unwrap();
    let stale = stale_turn(&key).unwrap();
    assert!(stale.reattach.is_some());
    let settled = rig.with_host(|| settle_stale(&key, stale, StaleChoice::Reattach, true));
    assert!(matches!(settled, StaleOutcome::Nothing), "{settled:?}");
    assert!(stale_turn(&key).is_none());
    assert!(!state_path(&dir, &target.turn).exists());
}

/// Overnight R1 review: a run that another conversation's preparation of
/// the same folder stopped and kept (its output still in `run/` for a day)
/// is no longer that folder's: attaching to it, or bringing it back, could
/// take the other turn's result. It counts as gone.
#[test]
fn a_run_another_preparation_took_over_is_gone() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let key = keep_record(&target);
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let mark = rig.path().join("taken.pid");
    let cmd = leader(
        "hollow-remote-probe",
        vec![format!("pidfile:{}", mark.display()), "sleep:60".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let first = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let first_pid = first.id().unwrap() as i32;
    let agent = wait_pid(&mark);
    let state = wait_state(&target.turn);
    unsafe { libc::killpg(first_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, first);
    peers.release("tag-test");
    // Another conversation's turn prepares the same worktree's folder: it
    // stops the run (no attacher) and keeps its result.
    let _other = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(!alive(agent), "the other preparation stopped it");
    assert!(
        rig.path()
            .join(format!("host/hollow-remote/run/{}/out", state.run))
            .exists(),
        "its output stays there for a day"
    );
    let stale = stale_turn(&key).unwrap();
    assert_eq!(
        rig.with_host(|| run_there(&stale.target, &state.run)),
        Ok(false)
    );
    let settled = rig.with_host(|| settle_stale(&key, stale, StaleChoice::Reattach, true));
    assert!(matches!(settled, StaleOutcome::Nothing), "{settled:?}");
    assert!(stale_turn(&key).is_none());
}

/// Overnight R1's rule: only a Continue of a Claude turn to the same
/// runner, in the folder it ran from, with what its leader knew, attaches.
#[test]
fn only_a_continue_of_a_claude_turn_in_place_reattaches() {
    use crate::agent::AgentKind;
    let target = RemoteTarget {
        host: "runner-host".into(),
        worktree: PathBuf::from("/w/one"),
        turn: "0a1b2c3d".into(),
        ..Default::default()
    };
    let stale = StaleTurn {
        target,
        agent: "claude".into(),
        reattach: Some(Reattach {
            run: "0a1b2c3d9f9f".into(),
            sup: 77,
            ..Default::default()
        }),
    };
    let go = crate::restart_resume::CONTINUE_PROMPT;
    let here = Path::new("/w/one");
    let choice = |prompt: &str, agent, host: Option<&str>, cwd: &Path| {
        stale_choice(&stale, prompt, agent, host, cwd)
    };
    assert_eq!(
        choice(go, AgentKind::Claude, Some("runner-host"), here),
        StaleChoice::Reattach
    );
    assert_eq!(
        choice(
            go,
            AgentKind::Claude,
            Some("runner-host"),
            Path::new("/w/one/src-tauri")
        ),
        StaleChoice::Reattach
    );
    assert_eq!(
        choice("Keep going", AgentKind::Claude, Some("runner-host"), here),
        StaleChoice::BringBack
    );
    assert_eq!(
        choice(go, AgentKind::Codex, Some("runner-host"), here),
        StaleChoice::BringBack
    );
    assert_eq!(
        choice(go, AgentKind::Claude, None, here),
        StaleChoice::BringBack
    );
    assert_eq!(
        choice(go, AgentKind::Claude, Some("other"), here),
        StaleChoice::BringBack
    );
    assert_eq!(
        choice(
            go,
            AgentKind::Claude,
            Some("runner-host"),
            Path::new("/w/two")
        ),
        StaleChoice::BringBack
    );
    let unknown = StaleTurn {
        reattach: None,
        ..stale.clone()
    };
    assert_eq!(
        stale_choice(&unknown, go, AgentKind::Claude, Some("runner-host"), here),
        StaleChoice::BringBack
    );
    // A Codex turn left there is never attached to, whatever runs now.
    let codex = StaleTurn {
        agent: "codex".into(),
        ..stale.clone()
    };
    assert_eq!(
        stale_choice(&codex, go, AgentKind::Claude, Some("runner-host"), here),
        StaleChoice::BringBack
    );
}

/// Overnight R1: the stdout count Hollow keeps is the leader's bytes (a
/// line, its newline; a line too long to keep, its own length), written in
/// place, and starts from a reattached run's earlier bytes.
#[test]
fn progress_counts_the_bytes_of_each_line_handled() {
    let dir = tempfile::Builder::new().prefix("rt").tempdir().unwrap();
    let read = || {
        std::fs::read_to_string(dir.path().join("0a1b2c3d.out"))
            .unwrap()
            .trim()
            .parse::<u64>()
            .unwrap()
    };
    let mut p = Progress::start_in(dir.path(), "0a1b2c3d", 10).unwrap();
    p.add("{\"type\":\"é\"}");
    p.add(&crate::process_group::dropped_line_event(40_000_000, 16));
    assert_eq!(read(), 10 + 14 + 40_000_001);
    // A line that isn't a whole event (cut off by a quit, or plain text)
    // counts only once another follows it.
    p.add("{\"type\":\"cut");
    assert_eq!(read(), 10 + 14 + 40_000_001);
    p.add("{}");
    assert_eq!(read(), 10 + 14 + 40_000_001 + 13 + 3);
}

/// A runner connection scripted for the leader alone (overnight R2), in
/// three plays (`$MODE`). `cut`: the first connection drops in the middle of
/// the run's stderr, after a line the last `err` mark doesn't cover and
/// inside another. `joined`: on a live connection, ssh's own words land on a
/// line cut short, then the rest of it and the next line follow. `split`:
/// ssh's words land in the first bytes of a connection's first line, so its
/// rest arrives without the start. The connection a play damages stays up
/// until the leader ends it (it notes the TERM in `termed`); the next one
/// (`--attach`) finishes. Every command it got is in `commands`.
const SCRIPTED_SSH: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my ($dir, $mode) = ('$DIR', '$MODE'); my $cmd = $ARGV[-1];
my $n = 0; if (open(my $c, '<', "$dir/count")) { $n = <$c> + 0; close $c }
open(my $c, '>', "$dir/count") or die; print $c $n + 1; close $c;
open(my $l, '>>', "$dir/commands") or die; print $l "$cmd\n"; close $l;
my ($line, $ch) = ('', '');
if ($cmd =~ / --run\z/) { while (sysread(STDIN, $ch, 1)) { last if $ch eq "\n" } }
while (sysread(STDIN, $ch, 1)) { last if $ch eq "\n"; $line .= $ch }
my (undef, $nonce) = split / /, $line;
my $m = "hollow-remote: $nonce";
$SIG{TERM} = sub { open(my $t, '>>', "$dir/termed"); print $t "$n\n"; close $t; exit 143 };
if ($n == 0 && $mode eq 'badin') {
  syswrite(STDOUT, "out one\n");
  syswrite(STDERR, "$m in 0 1 4242 $nonce\n$m e 0 alpha $nonce\n");
  exit 255;
}
if ($n == 1 && $mode eq 'badin') {
  syswrite(STDERR, "$m in 0 1 42Timeout, server fakehost not responding.\r\n42 $nonce\n$m e 6 beta $nonce\n$m err 11 $nonce\n");
  sleep 120; exit 255;
}
syswrite(STDERR, "$m in 0 1 4242 $nonce\n");
if ($n == 0 && $mode eq 'cut') {
  syswrite(STDOUT, "out one\n");
  syswrite(STDERR, "$m e 0 alpha $nonce\n$m e 6 beta $nonce\n$m err 11 $nonce\n$m e 11 gamma $nonce\n$m e 17 del");
  exit 255;
}
if ($n == 0 && $mode eq 'joined') {
  syswrite(STDOUT, "out one\n");
  syswrite(STDERR, "$m e 0 alpha $nonce\n$m e 6 gamTimeout, server fakehost not responding.\r\nma $nonce\n$m e 12 beta $nonce\n");
  sleep 120; exit 255;
}
if ($n == 0) {
  syswrite(STDOUT, "out one\n");
  syswrite(STDERR, "hollow-remTimeout, server fakehost not responding.\r\note: $nonce e 0 alpha $nonce\n$m err 6 $nonce\n");
  sleep 120; exit 255;
}
syswrite(STDOUT, "out two\n");
if ($mode eq 'cut') { syswrite(STDERR, "$m e 17 delta $nonce\n$m err 23 $nonce\n$m f 23 tail $nonce\n$m done 0 $nonce\n") }
elsif ($mode eq 'joined') { syswrite(STDERR, "$m e 6 gamma $nonce\n$m e 12 beta $nonce\n$m err 17 $nonce\n$m done 0 $nonce\n") }
elsif ($mode eq 'badin') { syswrite(STDERR, "$m e 6 beta $nonce\n$m err 11 $nonce\n$m done 0 $nonce\n") }
else { syswrite(STDERR, "$m e 0 alpha $nonce\n$m err 6 $nonce\n$m done 0 $nonce\n") }
exit 0;
"#;

/// The leader against [`SCRIPTED_SSH`] in `mode`: its exit code, stdout,
/// stderr, the `--attach` command's words after `--attach`, and which
/// connections the leader ended itself.
fn scripted_drop(mode: &str) -> (i32, String, String, Vec<String>, String) {
    let rig = Rig::new();
    rig.exe(
        "bin/scripted-ssh",
        &SCRIPTED_SSH
            .replace("$DIR", &rig.path().to_string_lossy())
            .replace("$MODE", mode),
    );
    let target = RemoteTarget {
        host: "fakehost".into(),
        ssh: rig
            .path()
            .join("bin/scripted-ssh")
            .to_string_lossy()
            .into_owned(),
        home: rig.path().join("host").to_string_lossy().into_owned(),
        worktree: rig.repo(),
        ..Default::default()
    };
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let cmd = leader(
        "hollow-remote-probe",
        vec![],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    peers.release("tag-test");
    let commands = std::fs::read_to_string(rig.path().join("commands")).unwrap();
    let attach = commands
        .lines()
        .find_map(|l| l.split_once("--attach "))
        .map(|(_, rest)| rest.split(' ').map(str::to_string).collect())
        .unwrap_or_else(|| panic!("no --attach:\n{commands}\n{err}"));
    let termed = std::fs::read_to_string(rig.path().join("termed")).unwrap_or_default();
    (code, out, err, attach, termed)
}

/// Overnight R2: the run's stderr arrives exactly once across a drop. The
/// leader counts each line it passed on (not only the runner's last `err`
/// mark), so the reconnect asks from the byte after `gamma`, and the line
/// cut by the drop comes whole after it, once; a last line without its
/// newline counts none.
#[test]
fn stderr_arrives_once_when_the_connection_drops_between_marks() {
    let (code, out, err, attach, _) = scripted_drop("cut");
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(out, "out one\nout two\n");
    for line in ["alpha", "beta", "gamma", "delta", "tail"] {
        assert_eq!(
            err.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{err}"
        );
    }
    assert!(
        !err.lines().any(|l| l.contains("del") && l != "delta"),
        "{err}"
    );
    assert!(
        !err.contains(" e ") && !err.contains(" f "),
        "the marks stay with the leader:\n{err}"
    );
    // `<run> <stdout bytes> <err offset> <relay> <supervisor pid> 1`.
    assert_eq!(&attach[1..3], ["8", "17"], "{attach:?}");
    assert_eq!(&attach[4..], ["4242", "1"], "{attach:?}");
}

/// Overnight R2 review: ssh's own words joined onto a line cut short, on a
/// connection that stays up, would pass for a line of the run's (or let the
/// next one be counted past a gap). The leader sees it isn't whole (no
/// nonce at its end), passes nothing more from that connection, ends it,
/// and the reconnect asks from the end of the last whole line: every line
/// arrives once and the fragment never shows.
#[test]
fn a_line_with_ssh_words_joined_on_is_asked_for_again() {
    let (code, out, err, attach, termed) = scripted_drop("joined");
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(out, "out one\nout two\n");
    assert_eq!(termed, "0\n", "the leader ended the damaged connection");
    for line in ["alpha", "gamma", "beta"] {
        assert_eq!(
            err.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{err}"
        );
    }
    assert!(
        !err.lines()
            .any(|l| l.starts_with("ma") || l.contains("gamTimeout")),
        "{err}"
    );
    assert!(
        err.contains("came through damaged; asking for it again"),
        "{err}"
    );
    assert_eq!(&attach[1..3], ["8", "6"], "{attach:?}");
}

/// Overnight R2 review round 3: a reconnect whose `in` arrives damaged
/// asks again (the input and the count wait for a whole one), instead of
/// dropping that connection's stderr while its `err` marks move the count.
#[test]
fn a_damaged_in_on_a_reconnect_is_asked_for_again() {
    let (code, out, err, _, termed) = scripted_drop("badin");
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(termed, "1\n", "the leader ended the second connection");
    for line in ["alpha", "beta"] {
        assert_eq!(
            err.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{err}"
        );
    }
}

/// Overnight R2 review round 2: ssh's words in the first bytes of a
/// connection's first line leave the rest of it without its start. That
/// rest has the nonce in it, so it is no line of the run's own: the leader
/// asks again instead of letting the next `err` mark count past it.
#[test]
fn a_line_split_in_its_first_bytes_is_asked_for_again() {
    let (code, out, err, attach, termed) = scripted_drop("split");
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(termed, "0\n", "the leader ended the damaged connection");
    assert_eq!(err.lines().filter(|l| *l == "alpha").count(), 1, "{err}");
    assert!(!err.lines().any(|l| l.starts_with("ote:")), "{err}");
    assert_eq!(&attach[1..3], ["8", "0"], "{attach:?}");
}

/// A steerable turn's input across a drop: what was sent before reaches the
/// agent once, and what is sent while the leader reconnects is held and
/// reaches it once after.
#[test]
fn input_sent_across_a_drop_reaches_the_agent_once() {
    use tokio::io::AsyncWriteExt;
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let mark = rig.path().join("first.pid");
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "stdin".into(),
            format!("pidfile:{}", mark.display()),
            "stdin".into(),
            "stdin".into(),
            "say:end".into(),
        ],
        probe_env(),
        &target,
        None,
        true,
    )
    .unwrap();
    let mut child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let leader_pid = child.id().unwrap() as i32;
    let mut stdin = child.stdin.take().unwrap();
    let mut send = |text: &'static [u8]| {
        rt.block_on(async {
            stdin.write_all(text).await.unwrap();
            stdin.flush().await.unwrap();
        })
    };
    send(b"first\n");
    wait_pid(&mark);
    let ssh = children_of(leader_pid, "fake-ssh");
    assert_eq!(ssh.len(), 1, "{ssh:?}");
    unsafe { libc::kill(ssh[0], libc::SIGKILL) };
    send(b"second\n");
    std::thread::sleep(Duration::from_millis(300));
    send(b"third\n");
    drop(stdin);
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    for line in ["stdin: first", "stdin: second", "stdin: third", "end"] {
        assert_eq!(
            out.lines().filter(|l| *l == line).count(),
            1,
            "{line} once:\n{out}\n{err}"
        );
    }
    peers.release("tag-test");
}

/// A run whose leader is gone (Hollow quit mid-turn) goes on there until the
/// next preparation of its folder, which stops it and keeps its result: the
/// next turn starts and brings that work as a branch.
#[test]
fn a_run_whose_leader_is_gone_is_stopped_at_the_next_preparation() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let mark = rig.path().join("orphan.pid");
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "write:orphan.txt:made by a run Hollow lost".into(),
            "commit:orphan.txt:an orphaned run's commit".into(),
            format!("pidfile:{}", mark.display()),
            "sleep:60".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let leader_pid = child.id().unwrap() as i32;
    let agent = wait_pid(&mark);
    unsafe { libc::killpg(leader_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, child);
    assert!(alive(agent), "the run goes on without its leader");
    let started = Instant::now();
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(25),
        "{:?}",
        started.elapsed()
    );
    assert!(!alive(agent), "stopped by the next preparation");
    assert_eq!(next.kept, vec!["work-mini-1".to_string()]);
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s", "work-mini-1"]),
        "an orphaned run's commit"
    );
    peers.release("tag-test");
}

/// A turn Hollow finished (Stop) before its run there started never starts
/// it: the finish leaves the turn's stop file, which the supervisor checks
/// before starting the agent.
#[test]
fn a_turn_finished_before_its_run_starts_never_starts_it() {
    let rig = Rig::new();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert_eq!(
        rig.with_host(|| collect(&target)).unwrap(),
        Collected::Unchanged
    );
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let cmd = leader(
        "hollow-remote-probe",
        vec!["write:never.txt:never".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 70, "{out}\n{err}");
    assert!(err.contains("stopped before it started"), "{err}");
    assert!(!Path::new(&target.dir).join("never.txt").exists());
    peers.release("tag-test");
}

/// The bridge (Phase 4): when the way to Hollow's tools drops, the request
/// waiting is answered with an error at once; the bridge reconnects and
/// replays the agent's `initialize` (its answer swallowed) and
/// `initialized`, and the agent's next call is served on the new connection.
#[test]
fn a_bridge_reconnects_and_replays_initialize() {
    let rig = Rig::new();
    rig.exe("bin/hollow-remote-run", REMOTE_RUN_PL);
    let sock = rig.path().join("tools.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let (seen_tx, seen_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let answer = |w: &mut std::os::unix::net::UnixStream, line: &str, result: Value| {
            let m: Value = serde_json::from_str(line).unwrap();
            let r = json!({ "jsonrpc": "2.0", "id": m["id"], "result": result });
            writeln!(w, "{r}").unwrap();
        };
        // The first connection: set up, then a call left unanswered as it drops.
        let (mut c1, _) = listener.accept().unwrap();
        let mut r1 = BufReader::new(c1.try_clone().unwrap());
        let mut line = String::new();
        for _ in 0..3 {
            line.clear();
            r1.read_line(&mut line).unwrap();
            seen_tx.send(format!("1 {}", line.trim())).unwrap();
            if line.contains("\"initialize\"") {
                answer(
                    &mut c1,
                    &line,
                    json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": { "name": "t", "version": "1" } }),
                );
            }
        }
        drop(r1);
        drop(c1);
        // The second: the replay, then calls answered.
        let (mut c2, _) = listener.accept().unwrap();
        let mut r2 = BufReader::new(c2.try_clone().unwrap());
        loop {
            line.clear();
            if r2.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            seen_tx.send(format!("2 {}", line.trim())).unwrap();
            if line.contains("\"initialize\"") {
                answer(
                    &mut c2,
                    &line,
                    json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": { "name": "t", "version": "1" } }),
                );
            } else if line.contains("tools/call") {
                answer(
                    &mut c2,
                    &line,
                    json!({ "content": [{ "type": "text", "text": "second connection" }] }),
                );
            }
        }
    });
    let mut bridge = std::process::Command::new("/usr/bin/perl")
        .arg(rig.path().join("bin/hollow-remote-run"))
        .args(["--bridge", "hollow_probe"])
        .env("HOLLOW_RELAY_SOCK", &sock)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut to = bridge.stdin.take().unwrap();
    let from = bridge.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(from).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = || -> Value {
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(15)).expect("an answer")).unwrap()
    };
    writeln!(to, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"agent","version":"1"}}}}}}"#).unwrap();
    assert_eq!(next()["id"], 1);
    writeln!(
        to,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(to, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"x","arguments":{{"id":"nested"}}}}}}"#).unwrap();
    let lost = next();
    assert_eq!(lost["id"], 2, "{lost}");
    assert!(
        lost["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unreachable"),
        "{lost}"
    );
    let mut served = None;
    for id in 3..20 {
        writeln!(to, r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"x","arguments":{{}}}}}}"#).unwrap();
        let got = next();
        assert_eq!(got["id"], id, "{got}");
        if got.get("result").is_some() {
            served = Some(got);
            break;
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    let served = served.expect("served after reconnecting");
    assert_eq!(served["result"]["content"][0]["text"], "second connection");
    let seen: Vec<String> = seen_rx.try_iter().collect();
    let replay = seen
        .iter()
        .find(|l| l.starts_with("2 ") && l.contains("\"initialize\""))
        .expect("initialize replayed");
    assert!(replay.contains("\"hollow-replay-1\""), "{replay}");
    assert!(
        seen.iter()
            .any(|l| l.starts_with("2 ") && l.contains("notifications/initialized")),
        "{seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|l| l.starts_with("2 ") && l.contains("\"initialize\""))
            .count(),
        1
    );
    drop(to);
    let t0 = Instant::now();
    while bridge.try_wait().unwrap().is_none() {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "the bridge ends with the agent's input"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        rx.try_iter().all(|l| !l.contains("hollow-replay")),
        "the replay's answer is swallowed"
    );
}

/// A result no one brought back (here: never collected) is kept at the next
/// preparation and comes here as a branch, never dropped.
#[test]
fn a_result_that_never_came_back_is_kept_on_a_branch() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "write:lost.txt:would be lost".into(),
            "commit:lost.txt:a commit no one collected".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    // No collect: the next turn's preparation finds the result.
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert_eq!(next.kept, vec!["work-mini-1".to_string()]);
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s", "work-mini-1"]),
        "a commit no one collected"
    );
    assert!(
        !rig.repo().join("lost.txt").exists(),
        "the worktree untouched"
    );
    peers.release("tag-test");
}

/// Run the probe with `actions` on a fresh preparation and return its target
/// (not collected).
fn probe_run(
    rig: &Rig,
    rt: &tokio::runtime::Runtime,
    peers: &'static crate::run_auth::Peers,
    actions: &[&str],
) -> RemoteTarget {
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let cmd = leader(
        "hollow-remote-probe",
        actions.iter().map(|a| a.to_string()).collect(),
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(rt, cmd, peers));
    let (code, out, err) = wait_output(rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    target
}

/// The host's kept refs for `target`'s worktree.
fn host_kept_refs(rig: &Rig, target: &RemoteTarget) -> String {
    host_refs(rig, target, "-kept-")
}

/// The host's session refs for `target`'s worktree whose name has `part`.
fn host_refs(rig: &Rig, target: &RemoteTarget, part: &str) -> String {
    let hash = target.wt_id.rsplit('-').next().unwrap();
    let bare = rig
        .path()
        .join(format!("host/hollow-remote/repos/{}.git", target.repo_id));
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&bare)
        .args([
            "for-each-ref",
            "--format=%(refname)",
            "refs/hollow-session/",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|r| r.contains(&format!("{hash}{part}")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every kept result is listed at each preparation and dropped on the host
/// only once it is here: a fetch that fails leaves it for the next turn,
/// which brings it, and one already brought here isn't made a second branch.
#[test]
fn kept_results_stay_listed_until_they_are_here() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let target = probe_run(
        &rig,
        &rt,
        peers,
        &[
            "write:lost.txt:would be lost",
            "commit:lost.txt:never collected",
        ],
    );
    // A ref in the way of where this Mac fetches a kept result: the fetch
    // fails, so the host must keep it.
    let hash = target.wt_id.rsplit('-').next().unwrap().to_string();
    let blocker = format!("refs/hollow-remote/{hash}/kept/x");
    rig.git(&["update-ref", &blocker, "HEAD"]);
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(next.kept.is_empty(), "{:?}", next.kept);
    assert!(
        !host_kept_refs(&rig, &target).is_empty(),
        "still kept on the host"
    );
    rig.git(&["update-ref", "-d", &blocker]);
    let again = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert_eq!(again.kept, vec!["work-mini-1".to_string()]);
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s", "work-mini-1"]),
        "never collected"
    );
    assert_eq!(host_kept_refs(&rig, &target), "", "dropped there once here");
    // The same commit kept again (a drop there that didn't happen): it is
    // here already, so no second branch and nothing to name; dropped there.
    let sha = rig.git(&["rev-parse", "work-mini-1"]);
    let bare = rig
        .path()
        .join(format!("host/hollow-remote/repos/{}.git", target.repo_id));
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&bare)
        .args([
            "update-ref",
            &format!("refs/hollow-session/{hash}-kept-1-1"),
            &sha,
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let third = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(third.kept.is_empty(), "{:?}", third.kept);
    assert!(std::process::Command::new("git")
        .current_dir(rig.repo())
        .args(["rev-parse", "--verify", "-q", "refs/heads/work-mini-2"])
        .output()
        .unwrap()
        .stdout
        .is_empty());
    assert_eq!(host_kept_refs(&rig, &target), "", "dropped there");
    peers.release("tag-test");
}

/// The files came back but the branch couldn't be moved (its ref is locked
/// here): the agent's commits get a branch before the result refs go.
#[test]
fn commits_that_cant_move_the_branch_are_kept_on_a_branch() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let target = probe_run(
        &rig,
        &rt,
        peers,
        &[
            "write:new.txt:from the host",
            "commit:new.txt:the host's commit",
        ],
    );
    let lock = rig.repo().join(".git/refs/heads/work.lock");
    std::fs::write(&lock, "").unwrap();
    let (collected, still_there) = rig.with_host(|| collect_placed(&target));
    let error = collected.unwrap_err();
    std::fs::remove_file(&lock).unwrap();
    assert!(error.contains("on branch work-mini-1"), "{error}");
    // Failed, but the work is here (the morning report says nothing).
    assert!(!still_there);
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s", "work-mini-1"]),
        "the host's commit"
    );
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("new.txt")).unwrap(),
        "from the host\n",
        "the files are here"
    );
    // The result refs went there only after the branch was made: nothing
    // left there, and the next preparation keeps nothing.
    assert_eq!(host_refs(&rig, &target, "-result"), "");
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(next.kept.is_empty(), "{:?}", next.kept);
    peers.release("tag-test");
}

/// A kept branch can't be made here (a ref is in the way of every name it
/// would take): the runner keeps the result, and a later turn brings it.
#[test]
fn a_result_that_cant_be_kept_here_stays_on_the_runner() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let target = probe_run(&rig, &rt, peers, &["write:aside.txt:from the host"]);
    // Changed here meanwhile, so the result is kept aside, and no branch can
    // be made here (the branch folder is read-only; a ref under
    // `work-mini-1/` alone would just be passed over).
    std::fs::write(rig.repo().join("kept.txt"), "edited here meanwhile\n").unwrap();
    rig.git(&["update-ref", "refs/heads/work-mini-1/x", "HEAD"]);
    let heads = rig.repo().join(".git/refs/heads");
    std::fs::set_permissions(&heads, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (collected, still_there) = rig.with_host(|| collect_placed(&target));
    std::fs::set_permissions(&heads, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = collected.unwrap_err();
    assert!(error.contains("still on fakehost"), "{error}");
    assert!(still_there, "the work is still on the runner");
    assert_ne!(
        host_refs(&rig, &target, "-result"),
        "",
        "kept on the runner"
    );
    rig.git(&["update-ref", "-d", "refs/heads/work-mini-1/x"]);
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert_eq!(next.kept, vec!["work-mini-1".to_string()]);
    assert_eq!(rig.git(&["show", "work-mini-1:aside.txt"]), "from the host");
    peers.release("tag-test");
}

/// Phase 3: a vault workspace chat's turn runs in its own copy of the
/// workspace folder there (no repository in it); its edits come back,
/// another chat's edits here meanwhile stay, a file changed on both sides
/// is kept twice, and a turn whose changes never came back has them applied
/// at the next preparation.
#[test]
fn a_workspace_folder_turn_runs_there_and_its_changes_come_back() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "# W1\n\n## Decisions\n- a\n\n## History\n- h\n",
    )
    .unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-1".into(),
    };
    let turn = |actions: &[&str]| {
        let target = rig
            .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
            .unwrap();
        let cmd = leader(
            "hollow-remote-probe",
            actions.iter().map(|a| a.to_string()).collect(),
            probe_env(),
            &target,
            None,
            false,
        )
        .unwrap();
        let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
        let (code, out, err) = wait_output(&rt, child);
        assert_eq!(code, 0, "{out}\n{err}");
        (target, out)
    };
    let (target, out) = turn(&[
        "where",
        "cat:PLAN.md",
        "write:PLAN.md:plan, from the runner",
        "write:new.md:made there",
    ]);
    assert!(!target.git_worktree);
    assert!(
        target.dir.contains("/hollow-remote/folders/w1-"),
        "{}",
        target.dir
    );
    assert!(out.contains("PLAN.md: plan"), "{out}");
    assert!(
        out.contains(&format!("folders/{}", target.wt_id)),
        "ran in the copy: {out}"
    );
    assert!(
        !Path::new(&target.dir).join(".git").exists(),
        "no repository there"
    );
    // Another chat edits the brief here meanwhile.
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "# W1\n\n## Decisions\n- a\n- b, another chat\n\n## History\n- h\n",
    )
    .unwrap();
    assert_eq!(
        collect(&target).unwrap(),
        Collected::Unchanged,
        "no worktree"
    );
    let back = rig.with_host(|| collect_folder(&target)).unwrap().unwrap();
    let mut changed = back.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["PLAN.md", "new.md"], "{back:?}");
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "plan, from the runner\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("new.md")).unwrap(),
        "made there\n"
    );
    assert!(std::fs::read_to_string(folder.join("WORKSPACE.md"))
        .unwrap()
        .contains("b, another chat"));

    // Both sides change the plan: this Mac's stays, the runner's beside it.
    let (target, _) = turn(&["write:PLAN.md:the runner's plan"]);
    std::fs::write(folder.join("PLAN.md"), "this Mac's plan\n").unwrap();
    let back = rig.with_host(|| collect_folder(&target)).unwrap().unwrap();
    assert_eq!(back.copies.len(), 1, "{back:?}");
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "this Mac's plan\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(&back.copies[0])).unwrap(),
        "the runner's plan\n"
    );

    // Never collected: the next preparation brings it.
    let (_lost, _) = turn(&["write:late.md:written late"]);
    let next = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let earlier = next
        .folder_earlier
        .clone()
        .expect("the earlier turn's changes");
    assert_eq!(earlier.changed, vec!["late.md"]);
    assert_eq!(
        std::fs::read_to_string(folder.join("late.md")).unwrap(),
        "written late\n"
    );
    // And the copy there now has it too? No: it was sent before they were
    // applied here; the next turn's copy will. Nothing is lost either way.
    peers.release("tag-test");
}

/// Overnight R3: what the workspace folder's own `.gitignore` names goes to
/// the runner and comes back like any file, both ways (the snapshot here,
/// the finish there), while secret-looking files, junk and a file over the
/// cap stay here, named in the preparation; a secret-looking file the
/// agent makes there is never taken back.
#[test]
fn a_workspace_folders_gitignored_files_go_and_come_back() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(folder.join("build")).unwrap();
    std::fs::create_dir_all(folder.join("node_modules")).unwrap();
    for (name, text) in [
        ("PLAN.md", "plan\n"),
        (".gitignore", "build/\n*.log\n.env\n"),
        ("build/out.txt", "built\n"),
        ("run.log", "logged\n"),
        (".env", "TOKEN=here\n"),
        ("secrets.pem", "pem\n"),
        ("id_ed25519", "key\n"),
        ("node_modules/x.js", "junk\n"),
    ] {
        std::fs::write(folder.join(name), text).unwrap();
    }
    let big = std::fs::File::create(folder.join("big.bin")).unwrap();
    big.set_len(crate::remote_folder::MAX_FILE_BYTES + 1)
        .unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-ignored".into(),
    };
    let target = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let left = &target.folder.as_ref().unwrap().left;
    assert_eq!(left.secret, vec![".env", "id_ed25519", "secrets.pem"]);
    assert_eq!(left.big, vec!["big.bin"]);
    assert!(left.repos.is_empty(), "{left:?}");
    // Made there by tools, as the agent's commands would (overnight R3
    // review): a venv and a cache that ignore themselves, a file over the
    // cap, and a name that would be a glob; none of the first three come
    // back, the last does, literally.
    let there = Path::new(&target.dir);
    for (name, text) in [
        (".venv/.gitignore", "# Created by venv\n*\n"),
        (".venv/lib/pandas.py", "x\n"),
        (
            ".ruff_cache/CACHEDIR.TAG",
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        ),
        (".ruff_cache/0.1/x", "x\n"),
        ("build/x*", "a file named x*\n"),
        ("build/x.hollow-tmp-1", "junk\n"),
    ] {
        std::fs::create_dir_all(there.join(name).parent().unwrap()).unwrap();
        std::fs::write(there.join(name), text).unwrap();
    }
    let huge = std::fs::File::create(there.join("build/huge.bin")).unwrap();
    huge.set_len(crate::remote_folder::MAX_FILE_BYTES + 1)
        .unwrap();
    let cmd = leader(
        "hollow-remote-probe",
        [
            "cat:build/out.txt",
            "cat:run.log",
            "cat:.env",
            "cat:secrets.pem",
            "cat:id_ed25519",
            "cat:big.bin",
            "cat:node_modules/x.js",
            "write:build/out.txt:rebuilt there",
            "write:build/new.txt:made there",
            "write:fresh.log:logged there",
            "write:.env:TOKEN=there",
            "write:api.key:made there",
            "write:.DS_Store:junk",
        ]
        .iter()
        .map(|a| a.to_string())
        .collect(),
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("build/out.txt: built"), "{out}");
    assert!(out.contains("run.log: logged"), "{out}");
    for stays in [
        ".env",
        "secrets.pem",
        "id_ed25519",
        "big.bin",
        "node_modules/x.js",
    ] {
        assert!(out.contains(&format!("no {stays}\n")), "{stays}: {out}");
    }
    let back = rig.with_host(|| collect_folder(&target)).unwrap().unwrap();
    let mut changed = back.changed.clone();
    changed.sort();
    assert_eq!(
        changed,
        vec!["build/new.txt", "build/out.txt", "build/x*", "fresh.log"],
        "{back:?}"
    );
    let mut refused = back.refused.clone();
    refused.sort();
    assert_eq!(refused, vec![".env", "api.key"], "{back:?}");
    assert!(back.skipped.is_empty(), "{back:?}");
    assert_eq!(back.too_big.first, vec!["build/huge.bin"], "{back:?}");
    for stays in [
        ".venv",
        ".ruff_cache",
        "build/x.hollow-tmp-1",
        "build/huge.bin",
    ] {
        assert!(!folder.join(stays).exists(), "{stays}");
    }
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).unwrap();
    assert_eq!(read("build/out.txt"), "rebuilt there\n");
    assert_eq!(read("build/new.txt"), "made there\n");
    assert_eq!(read("fresh.log"), "logged there\n");
    assert_eq!(read(".env"), "TOKEN=here\n");
    assert!(!folder.join("api.key").exists());
    assert!(!folder.join(".DS_Store").exists());
    assert!(folder.join("big.bin").exists());
    peers.release("tag-test");
}

/// Round 2 review of overnight R3: what the snapshot held and the runner
/// now leaves out (a sent file grown past the cap there, a sent folder that
/// became a cache there) goes back into its result as it was sent, so this
/// Mac keeps its copy instead of reading a removal; the file is still named
/// as left there. The same through the salvage of a run that died there.
#[test]
fn what_was_sent_and_now_stays_there_is_kept_here() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(folder.join("outputs")).unwrap();
    std::fs::create_dir_all(folder.join("notes")).unwrap();
    for (name, text) in [
        ("PLAN.md", "plan\n"),
        ("outputs/events.jsonl", "{}\n"),
        ("notes/a.md", "a\n"),
        ("notes/b.md", "b\n"),
    ] {
        std::fs::write(folder.join(name), text).unwrap();
    }
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-grown".into(),
    };
    // What the agent's tools do there.
    let change_there = |dir: &str| {
        let there = Path::new(dir);
        std::fs::OpenOptions::new()
            .write(true)
            .open(there.join("outputs/events.jsonl"))
            .unwrap()
            .set_len(crate::remote_folder::MAX_FILE_BYTES + 1)
            .unwrap();
        std::fs::write(there.join("notes/.gitignore"), "*\n").unwrap();
        std::fs::write(there.join("notes/a.md"), "a, changed there\n").unwrap();
    };
    let kept_here = || {
        let read = |name: &str| std::fs::read_to_string(folder.join(name)).unwrap();
        assert_eq!(read("outputs/events.jsonl"), "{}\n");
        assert_eq!(read("notes/a.md"), "a\n");
        assert_eq!(read("notes/b.md"), "b\n");
        assert!(!folder.join("notes/.gitignore").exists());
    };
    let target = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    change_there(&target.dir);
    let cmd = leader(
        "hollow-remote-probe",
        vec!["write:PLAN.md:plan, from the runner".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    let back = rig.with_host(|| collect_folder(&target)).unwrap().unwrap();
    assert_eq!(back.changed, vec!["PLAN.md"], "{back:?}");
    assert_eq!(back.too_big.first, vec!["outputs/events.jsonl"], "{back:?}");
    kept_here();

    let first = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    change_there(&first.dir);
    std::fs::write(Path::new(&first.dir).join("late.md"), "late\n").unwrap();
    leave_dead_marker(&first.dir);
    // A change here, so the next snapshot isn't the same commit as `first`'s
    // (two in one second of the same files are): a kept result's base is
    // never the turn's own snapshot, so it would come back only as copies.
    std::fs::write(folder.join("here.md"), "here\n").unwrap();
    let next = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let earlier = next.folder_earlier.expect("the dead run's changes");
    assert_eq!(earlier.changed, vec!["late.md"], "{earlier:?}");
    assert_eq!(earlier.too_big.first, vec!["outputs/events.jsonl"]);
    kept_here();
    peers.release("tag-test");
}

/// A session's build folder cloned from its worktree's names itself in its
/// build scripts' outputs (overnight N5: cargo writes absolute paths
/// there), at the preparation that clones it.
#[test]
fn a_sessions_cloned_build_folder_names_itself() {
    let rig = Rig::new();
    let first = rig
        .with_host(|| prepare_with("fakehost", &rig.ssh(), &rig.repo(), None))
        .unwrap();
    let root = rig.path().join("host/hollow-remote/targets");
    let seed = root.join(&first.wt_id).join("debug/build/t-1/out");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(
        seed.join("list.txt"),
        format!("{}/a.toml\n", seed.display()),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&first.target);
    let again = rig
        .with_host(|| prepare_with("fakehost", &rig.ssh(), &rig.repo(), None))
        .unwrap();
    let list =
        std::fs::read_to_string(Path::new(&again.target).join("debug/build/t-1/out/list.txt"))
            .unwrap();
    assert_eq!(
        list,
        format!("{}/debug/build/t-1/out/a.toml\n", again.target)
    );
    assert!(
        again.target.contains("/targets/session-"),
        "{}",
        again.target
    );
}

/// Phase 3 beside Phase 2: a build workspace's vault folder goes with its
/// worktree as an extra folder, and both come back.
#[test]
fn a_build_workspaces_vault_folder_goes_beside_its_worktree() {
    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/b1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-2".into(),
    };
    let target = rig
        .with_host(|| prepare_with("fakehost", &rig.ssh(), &rig.repo(), Some(&request)))
        .unwrap();
    assert!(target.git_worktree);
    let sync = target.folder.clone().unwrap();
    let plan_there = format!("{}/PLAN.md", sync.dir);
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "write:kept.txt:from the host".into(),
            format!("write:{plan_there}:plan, edited there"),
            // The run marks the vault folder as in use too (overnight N5).
            format!("cat:{}.running", sync.dir),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    let marked = format!("{}.running: ", sync.dir);
    let pid = out
        .lines()
        .find_map(|l| l.strip_prefix(marked.as_str()))
        .unwrap_or_else(|| panic!("no folder marker: {out}"));
    assert!(pid.parse::<u32>().is_ok(), "{out}");
    assert!(matches!(
        rig.with_host(|| collect(&target)).unwrap(),
        Collected::Applied { .. }
    ));
    assert!(!Path::new(&format!("{}.running", sync.dir)).exists());
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("kept.txt")).unwrap(),
        "from the host\n"
    );
    let back = rig.with_host(|| collect_folder(&target)).unwrap().unwrap();
    assert_eq!(back.changed, vec!["PLAN.md"]);
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "plan, edited there\n"
    );
    assert!(agent_note(&target).contains(&sync.dir));
    peers.release("tag-test");
}

/// The host preparation prunes remote sessions' folders (overnight N5): one
/// not used for over 14 days goes with its scratch and build folders and
/// small files; while the disk is under its floor, the least recently used
/// of those idle for 12 hours go, oldest first, until there is room, and
/// the preparation says how many; never one whose run is live (a build
/// turn's vault folder included) or one this turn uses, which is stamped as
/// used first. The last round reads the real `df`.
#[test]
fn host_preparation_prunes_old_sessions_and_frees_space() {
    let rig = Rig::new();
    let root = rig.path().join("host/hollow-remote");
    let hours = |h: i64| {
        (chrono::Local::now() - chrono::Duration::hours(h))
            .format("%Y%m%d%H%M")
            .to_string()
    };
    let touch = |rel: &str, when: &str| {
        let ok = std::process::Command::new("/usr/bin/touch")
            .args(["-t", when])
            .arg(root.join(rel))
            .status()
            .unwrap();
        assert!(ok.success(), "{rel}");
    };
    let ids = [
        ("old", 20 * 24),
        ("a", 72),
        ("b", 48),
        ("c", 30),
        ("recent", 2),
        ("live", 30 * 24),
        ("mine", 30 * 24),
    ];
    for (id, _) in ids {
        for sub in [
            format!("sessions/{id}"),
            format!("scratch/{id}"),
            format!("targets/session-{id}"),
        ] {
            std::fs::create_dir_all(root.join(sub).join("x")).unwrap();
        }
        std::fs::write(root.join(format!("sessions/{id}.port")), "2701\n").unwrap();
    }
    for f in ["fold", "bfold"] {
        std::fs::create_dir_all(root.join(format!("folders/{f}/sub"))).unwrap();
    }
    // A live run in "live", and a build turn's vault folder "bfold" that its
    // run marks: both markers name a running hollow-remote-run.
    let script = rig.path().join("hollow-remote-run");
    std::fs::write(&script, "sleep 60;\n").unwrap();
    let mut live = std::process::Command::new("/usr/bin/perl")
        .arg(&script)
        .spawn()
        .unwrap();
    for m in ["sessions/live.running", "folders/bfold.running"] {
        std::fs::write(root.join(m), format!("{}\nrun\n", live.id())).unwrap();
    }
    for (id, h) in ids {
        touch(&format!("sessions/{id}"), &hours(h));
        touch(&format!("scratch/{id}"), &hours(h));
    }
    touch("folders/fold", &hours(24));
    for m in [
        "folders/bfold",
        "folders/bfold.running",
        "sessions/live.running",
    ] {
        touch(m, &hours(30 * 24));
    }
    let df = rig.path().join("fake-df");
    std::fs::write(
        &df,
        format!(
            "#!/bin/sh\nd='{}'\nn=$(cat \"$d/df.n\" 2>/dev/null || echo 0)\necho $((n + 1)) > \"$d/df.n\"\nkb=$(sed -n \"$((n + 1))p\" \"$d/df.seq\")\n[ -n \"$kb\" ] || kb=$(tail -n 1 \"$d/df.seq\")\nprintf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n/dev/disk3s5 999999999 1 %s 1%% /\\n' \"$kb\"\n",
            rig.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&df, std::fs::Permissions::from_mode(0o755)).unwrap();
    let answers = |kb: &[u64]| {
        let lines: Vec<String> = kb.iter().map(|k| k.to_string()).collect();
        std::fs::write(rig.path().join("df.seq"), lines.join("\n") + "\n").unwrap();
        let _ = std::fs::remove_file(rig.path().join("df.n"));
    };
    const GB: u64 = 1_048_576;
    let exists = |rel: &str| root.join(rel).exists();
    rig.host_env(&format!(
        "HOLLOW_REMOTE_MIN_FREE_GB=40 HOLLOW_REMOTE_DF={}",
        df.display()
    ));
    // Plenty of room: only the old one goes, all of it; this turn's is
    // stamped as used now.
    answers(&[100 * GB]);
    let (_, pruned) = host_prep("fake", &rig.ssh(), &["r".to_string()], &["mine"]).unwrap();
    assert_eq!(pruned, None);
    for gone in [
        "sessions/old",
        "scratch/old",
        "targets/session-old",
        "sessions/old.port",
    ] {
        assert!(!exists(gone), "{gone}");
    }
    for id in ["a", "b", "c", "recent", "live", "mine"] {
        assert!(exists(&format!("sessions/{id}")), "{id}");
    }
    let stamp = std::fs::metadata(root.join("sessions/mine.prepared"))
        .unwrap()
        .modified()
        .unwrap();
    assert!(stamp.elapsed().unwrap() < Duration::from_secs(60));
    // 1 GB free for five readings (one per session weighed: live, bfold,
    // a, b, then c), then room: the two least recently used idle ones go.
    answers(&[GB, GB, GB, GB, 100 * GB]);
    let (_, pruned) = host_prep("fake", &rig.ssh(), &["r".to_string()], &["mine"]).unwrap();
    assert_eq!(
        pruned,
        Some(DiskPruned {
            count: 2,
            free_gb: 1,
            floor_gb: 40
        })
    );
    for id in ["a", "b"] {
        assert!(!exists(&format!("sessions/{id}")), "{id}");
        assert!(!exists(&format!("targets/session-{id}")), "{id}");
    }
    assert!(exists("sessions/c") && exists("folders/fold"));
    // The folders holding every session, last changed long ago, are no
    // session of their own (an empty id would have taken them all).
    for top in ["sessions", "scratch", "folders"] {
        touch(top, &hours(30 * 24));
    }
    // The real df, with a floor no disk meets: every idle one goes; one used
    // 2 hours ago, the live run's, the build turn's marked folder and this
    // turn's stay.
    rig.host_env("HOLLOW_REMOTE_MIN_FREE_GB=999999999");
    let (_, pruned) = host_prep("fake", &rig.ssh(), &["r".to_string()], &["mine"]).unwrap();
    let pruned = pruned.expect("the real df was read");
    assert_eq!((pruned.count, pruned.floor_gb), (2, 999_999_999));
    assert!(!exists("sessions/c") && !exists("folders/fold"));
    assert!(exists("sessions/recent") && exists("targets/session-recent"));
    assert!(exists("sessions/live") && exists("targets/session-live"));
    assert!(exists("folders/bfold"));
    assert!(exists("sessions/mine") && exists("scratch/mine"));
    let _ = live.kill();
    let _ = live.wait();
}

/// A host that doesn't answer: nothing ran there, and the message says what
/// to check (the FileVault unlock screen after a restart) and how to run
/// here instead.
#[test]
fn an_unreachable_host_says_why_and_what_to_do() {
    let rig = Rig::new();
    let error = rig
        .with_host(|| prepare("down", &rig.ssh(), &rig.repo()))
        .unwrap_err();
    assert!(error.contains("didn't answer"), "{error}");
    assert!(error.contains("Operation timed out"), "{error}");
    assert!(error.contains("FileVault"), "{error}");
    assert!(error.contains("Runs on to This Mac"), "{error}");
    assert!(!rig.path().join("host/hollow-remote").exists());
}

/// RT1: a turn sent to a runner that is forced unreachable falls back here,
/// and its record (the `stayed` notice the chat keeps and shows) carries
/// where it ran, why, and that it was a fallback, beside ssh's own words.
#[test]
fn a_forced_unreachable_runner_records_the_fallback_and_its_reason() {
    let rig = Rig::new();
    let error = rig
        .with_host(|| prepare("down", &rig.ssh(), &rig.repo()))
        .unwrap_err();
    let notice = stayed_notice("down", None, &error, "s-rt1");
    assert_eq!(notice["phase"], "stayed");
    assert_eq!(
        notice["route"],
        json!({
            "target": "here",
            "host": "down",
            "cause": "unreachable",
            "reason": "the runner did not answer",
            "fallback": true,
        })
    );
    let detail = notice["reason"].as_str().unwrap_or_default();
    assert!(detail.contains("Operation timed out"), "{detail}");
    assert!(!detail.contains("Runs on to This Mac"), "{detail}");
}

/// RT1: every reason a preferred turn stays here maps to its cause, each
/// sentence built by its real producer, so rewording one fails here.
#[test]
fn each_fallback_sentence_maps_to_its_cause() {
    use crate::remote_status::{recent_state_for_test, stalled_message};
    let cause = |m: &str| Route::fell_back("runner-host", None, m).cause;
    // Failed preparations, read from their errors.
    assert_eq!(
        cause(&unreachable_message(
            "runner-host",
            "ssh: connect to host x port 22: Connection refused"
        )),
        "unreachable"
    );
    assert_eq!(cause(&stalled_message("runner-host", 10)), "unreachable");
    let checkout = |e: &str| checkout_send_error("runner-host", e);
    assert_eq!(
        cause(&checkout(
            "ssh: connect to host runner-host port 22: Operation timed out"
        )),
        "unreachable"
    );
    assert_eq!(cause(&checkout("git push failed: rejected")), "prepare");
    assert_eq!(
        cause(&refusing_message("runner-host", "no: free=12GB")),
        "refusing"
    );
    assert_eq!(cause("the runner's preparation failed (panic)"), "prepare");
    // A recent status: its state is the cause, whatever its words, a
    // stalled check (unreachable) included.
    let recent = |state, message: Option<String>, preflight| {
        recent_state_for_test("runner-host", state, message, preflight).unwrap()
    };
    let stalled = recent(
        "unreachable",
        Some(stalled_message("runner-host", 10)),
        None,
    );
    assert_eq!(stalled.0, "unreachable");
    assert!(
        stalled.1.contains("didn't finish the check"),
        "{}",
        stalled.1
    );
    assert_eq!(cause(&stalled.1), "unreachable");
    let refusing = recent("refusing", None, Some("no: free=12GB"));
    assert_eq!(refusing.0, "refusing");
    assert_eq!(cause(&refusing.1), "refusing");
    // The blocker's own sentences carry their cause; the shared consts are
    // what it says.
    for (sentence, want) in [
        (BUSY_SENTENCE, "busy"),
        (CODEX_KEY_SENTENCE, "signIn"),
        (CLAUDE_CONNECTOR_SENTENCE, "signIn"),
        (CLAUDE_ACCOUNT_SENTENCE, "signIn"),
    ] {
        let notice = stayed_notice("runner-host", Some(want), sentence, "s-rt1");
        assert_eq!(notice["route"]["cause"], want, "{sentence}");
        assert_eq!(notice["route"]["fallback"], true);
    }
    // The way there says why too, and is never a fallback.
    let there = Route::there("runner-host", true, false);
    assert_eq!(
        (there.target, there.cause, there.fallback),
        ("runner", "preferred", false)
    );
    assert_eq!(Route::there("runner-host", false, false).cause, "chosen");
    assert_eq!(Route::there("runner-host", false, true).cause, "reattached");
}

/// The local worktree changed while the turn ran: the host's work is kept on
/// a branch of its own and nothing here is touched. A turn that changed
/// nothing comes back as unchanged.
#[test]
fn a_worktree_changed_here_meanwhile_keeps_the_hosts_work_aside() {
    let rig = Rig::new();
    let repo = rig.repo();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let run = |actions: Vec<String>| {
        let target = rig
            .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
            .unwrap();
        let cmd = leader(
            "hollow-remote-probe",
            actions,
            probe_env(),
            &target,
            None,
            false,
        )
        .unwrap();
        let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
        let (code, out, err) = wait_output(&rt, child);
        assert_eq!(code, 0, "{out}\n{err}");
        target
    };
    let target = run(vec!["where".into()]);
    assert_eq!(
        rig.with_host(|| collect(&target)).unwrap(),
        Collected::Unchanged
    );

    let target = run(vec!["write:kept.txt:the host's version".into()]);
    std::fs::write(repo.join("kept.txt"), "edited here meanwhile\n").unwrap();
    let collected = rig.with_host(|| collect(&target)).unwrap();
    assert_eq!(
        collected,
        Collected::KeptAside {
            branch: "work-mini-1".into()
        }
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("kept.txt")).unwrap(),
        "edited here meanwhile\n",
        "untouched here"
    );
    assert_eq!(
        rig.git(&["show", "work-mini-1:kept.txt"]),
        "the host's version"
    );
    peers.release("tag-test");
}

/// The agent command becomes the leader: the header carries the command line
/// and the environment minus this Mac's paths and tracking, Claude's tools
/// become the bridges, and a Claude run on the Claude Code login (no token
/// to hand over) is refused.
#[test]
fn the_agent_command_becomes_the_leader_with_nothing_local_in_its_header() {
    let target = RemoteTarget {
        host: "fakehost".into(),
        ssh: "ssh".into(),
        home: "/Users/runner".into(),
        dir: "/Users/runner/hollow-remote/sessions/w-1".into(),
        cwd: "/Users/runner/hollow-remote/sessions/w-1".into(),
        scratch: "/Users/runner/hollow-remote/scratch/w-1".into(),
        port: 2701,
        target: "/Users/runner/hollow-remote/targets/session-w-1".into(),
        kept: Vec::new(),
        worktree: std::env::temp_dir(),
        head: "a".repeat(40),
        snapshot: "a".repeat(40),
        snapshot_tree: "b".repeat(40),
        branch: Some("work".into()),
        repo_id: "r-1".into(),
        wt_id: "w-1".into(),
        git_worktree: true,
        folder: None,
        folder_earlier: None,
        turn: "0a1b2c3d".into(),
        pruned: None,
        aside: Vec::new(),
        identity: Some(GitIdentity {
            author_name: "Work Person".into(),
            author_email: "work@example.com".into(),
            committer_name: "Work Committer".into(),
            committer_email: "commit@example.com".into(),
        }),
        access: Default::default(),
        reattach: None,
    };
    let config = json!({ "mcpServers": {
        "hollow_app": { "command": "/Applications/Hollow.app/hollow", "args": ["app-mcp"], "env": { "HOLLOW_APP_PROOF": "secret-proof" } },
        "web": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer x" } },
        "open": { "url": "https://example.com/open" },
    }})
    .to_string();
    let bridges = bridge_config(&config, &target.home).unwrap();
    assert!(
        !bridges.contains("secret-proof"),
        "no proof goes to the host"
    );
    assert!(
        !bridges.contains("Bearer"),
        "no header goes to the host's argv"
    );
    let b: Value = serde_json::from_str(&bridges).unwrap();
    assert_eq!(b["mcpServers"]["hollow_app"]["command"], "/usr/bin/perl");
    assert_eq!(
        b["mcpServers"]["hollow_app"]["args"],
        json!([
            "/Users/runner/hollow-remote/bin/hollow-remote-run",
            "--bridge",
            "hollow_app"
        ])
    );
    assert!(b["mcpServers"].get("web").is_none());
    assert_eq!(b["mcpServers"]["open"]["url"], "https://example.com/open");

    let mut agent = Command::new("/usr/local/bin/claude");
    agent
        .args([
            "-p",
            "hello",
            "--mcp-config",
            "/Users/me/Library/Caches/hollow/run-mcp/x.json",
        ])
        .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-test-oauth-7f3a")
        .env("PATH", "/Users/me/bin")
        .env("HOLLOW_RUN_TAG", "hollow-run-1")
        .env("HOLLOW_BOUND_RUN_IDS", "run-1")
        .env("CARGO_TARGET_DIR", "/Users/me/target")
        .env("HOLLOW_BUILD_SLOT_DIR", "/tmp/hollow-build-slots-501")
        .env(
            "HOLLOW_THIS_MAC",
            "/Users/me/Library/Application Support/Hollow/bin/this-mac.md",
        )
        .env("BASH_MAX_OUTPUT_LENGTH", "30000");
    let leader = into_leader_with(
        &agent,
        crate::agent::AgentKind::Claude,
        &target,
        Some(Path::new("/tmp/x.json")),
        Some(&bridges),
        Handover {
            stdin: Some("PROMPT-SECRET-TEXT".into()),
            append: Some("APPENDED-SECRET-TEXT".into()),
        },
    )
    .unwrap();
    let std_leader = leader.as_std();
    assert_eq!(std_leader.get_program(), "/usr/bin/perl");
    let args: Vec<String> = std_leader
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.iter().all(|a| !a.contains("sk-test-oauth-7f3a")),
        "no token on argv"
    );
    // `-e <script> <config> <relay> <runner socket base> <run> -- <ssh> <options> <host>`:
    // the leader adds each connection's `-R` and command itself.
    let sep = args.iter().position(|a| a == "--").unwrap();
    let (local, base, run) = (&args[sep - 3], &args[sep - 2], &args[sep - 1]);
    assert!(
        local.starts_with("/tmp/hollow-relay-") && local.ends_with(".sock"),
        "{local}"
    );
    assert_eq!(base, &format!("/Users/runner/hollow-remote/run/{run}"));
    assert!(
        run.len() == 12 && run.starts_with("0a1b2c3d"),
        "the run id starts with the turn's: {run}"
    );
    assert_eq!(args.last().unwrap(), "fakehost");
    assert!(!args.contains(&"-R".to_string()));
    assert!(
        args.contains(&"-a".to_string()) && args.contains(&"ForwardAgent=no".to_string()),
        "no agent forwarding"
    );
    let env: BTreeMap<String, String> = std_leader
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().into_owned(),
                v?.to_string_lossy().into_owned(),
            ))
        })
        .collect();
    assert_eq!(
        env.get("HOLLOW_RUN_TAG").map(String::as_str),
        Some("hollow-run-1")
    );
    assert_eq!(
        env.get("HOLLOW_BOUND_RUN_IDS").map(String::as_str),
        Some("run-1")
    );
    let header: Value = serde_json::from_str(&env["HOLLOW_REMOTE_HEADER"]).unwrap();
    assert_eq!(header["program"], "claude");
    assert_eq!(header["cwd"], target.cwd);
    let hargs: Vec<String> = serde_json::from_value(header["args"].clone()).unwrap();
    let i = hargs.iter().position(|a| a == "--mcp-config").unwrap();
    assert_eq!(hargs[i + 1], bridges);
    let henv = header["env"].as_object().unwrap();
    // R5b (Q-R33): the appended prompt and the prompt reach the agent on
    // private files the runner opens, never as its argv or env. The token
    // stays in its environment (a login makes Claude ignore the descriptor).
    assert_eq!(henv["CLAUDE_CODE_OAUTH_TOKEN"], "sk-test-oauth-7f3a");
    assert!(
        henv.get("CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR")
            .is_none(),
        "{henv:?}"
    );
    assert_eq!(header["stdin_text"], "PROMPT-SECRET-TEXT");
    let fds: Vec<(i64, String)> = header["fds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["fd"].as_i64().unwrap(),
                f["text"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(fds, vec![(7, "APPENDED-SECRET-TEXT".to_string())]);
    assert!(
        hargs
            .windows(2)
            .any(|w| w == ["--append-system-prompt-file", "/dev/fd/7"]),
        "{hargs:?}"
    );
    for secret in ["PROMPT-SECRET-TEXT", "APPENDED-SECRET-TEXT"] {
        assert!(
            !hargs.iter().any(|a| a.contains(secret)),
            "{secret} in the agent's argv"
        );
        assert!(!henv
            .values()
            .any(|v| v.as_str().is_some_and(|v| v.contains(secret))));
    }
    assert_eq!(henv["BASH_MAX_OUTPUT_LENGTH"], "30000");
    // The supervisor sets the runner's own slot folder and machine note
    // (J4): this Mac's paths never travel.
    for local in [
        "PATH",
        "HOLLOW_RUN_TAG",
        "HOLLOW_BOUND_RUN_IDS",
        "CARGO_TARGET_DIR",
        "HOLLOW_BUILD_SLOT_DIR",
        "HOLLOW_THIS_MAC",
    ] {
        assert!(henv.get(local).is_none(), "{local} stays here");
    }
    assert!(!header.to_string().contains("this-mac.md"));
    assert_eq!(henv["TMPDIR"], format!("{}/", target.scratch));
    assert_eq!(header["token"].as_str().unwrap(), env["HOLLOW_RELAY_TOKEN"]);
    assert_eq!(header["sock"], format!("{base}.0.sock"));
    assert_eq!(header["run"], run.as_str());
    // This Mac's identity for the checkout goes in the header (the SSH
    // channel), never on argv and never as the leader's own variables.
    assert_eq!(henv["GIT_AUTHOR_NAME"], "Work Person");
    assert_eq!(henv["GIT_AUTHOR_EMAIL"], "work@example.com");
    assert_eq!(henv["GIT_COMMITTER_NAME"], "Work Committer");
    assert_eq!(henv["GIT_COMMITTER_EMAIL"], "commit@example.com");
    assert!(
        args.iter().all(|a| !a.contains("work@example.com")),
        "no identity on argv"
    );
    assert!(!env.contains_key("GIT_AUTHOR_EMAIL"));
    // None configured here: git's default there stays (nothing sent), and
    // one the agent command sets itself is kept.
    let mut none = target.clone();
    none.identity = None;
    let header_env = |leader: &Command| -> serde_json::Map<String, Value> {
        let raw = leader
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == "HOLLOW_REMOTE_HEADER")
            .and_then(|(_, v)| v)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let header: Value = serde_json::from_str(&raw).unwrap();
        header["env"].as_object().unwrap().clone()
    };
    let leader = into_leader(&agent, crate::agent::AgentKind::Claude, &none, None, None).unwrap();
    let henv = header_env(&leader);
    for key in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
    ] {
        assert!(henv.get(key).is_none(), "{key} not sent without one here");
    }
    let mut own = Command::new("/usr/local/bin/claude");
    own.arg("-p")
        .arg("x")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-test-oauth-7f3a")
        .env("GIT_AUTHOR_NAME", "Set By The Command");
    let leader = into_leader(&own, crate::agent::AgentKind::Claude, &target, None, None).unwrap();
    let henv = header_env(&leader);
    // What the command sets stays; the rest is this Mac's, as git here
    // would combine them (never the runner's guessed email).
    assert_eq!(henv["GIT_AUTHOR_NAME"], "Set By The Command");
    assert_eq!(henv["GIT_AUTHOR_EMAIL"], "work@example.com");
    assert_eq!(henv["GIT_COMMITTER_NAME"], "Work Committer");
    assert_eq!(henv["GIT_COMMITTER_EMAIL"], "commit@example.com");

    let mut login = Command::new("claude");
    login
        .arg("-p")
        .arg("x")
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN");
    let refused =
        into_leader(&login, crate::agent::AgentKind::Claude, &target, None, None).unwrap_err();
    assert!(refused.contains("added Claude accounts"), "{refused}");

    // ENG-33: a command that names the login is refused even with a token in
    // its environment, and one on an added account hands its account id to
    // the leader, so the usage ledger records the added account, never `login`.
    let mut named_login = Command::new("claude");
    named_login
        .arg("-p")
        .arg("x")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-test-oauth-7f3a")
        .env(crate::claude_accounts::ACCOUNT_ENV, "login");
    let refused = into_leader(
        &named_login,
        crate::agent::AgentKind::Claude,
        &target,
        None,
        None,
    )
    .unwrap_err();
    assert!(refused.contains("added Claude accounts"), "{refused}");
    let mut added = Command::new("claude");
    added
        .arg("-p")
        .arg("x")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-test-oauth-7f3a")
        .env(crate::claude_accounts::ACCOUNT_ENV, "acct-added-1");
    let leader = into_leader(&added, crate::agent::AgentKind::Claude, &target, None, None).unwrap();
    assert_eq!(
        crate::claude_accounts::account_of(&leader).as_deref(),
        Some("acct-added-1")
    );
    let vault = tempfile::tempdir().unwrap();
    let mut run = crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            vault.path(),
            crate::usage_ledger::Surface::Chat,
            Some("test".into()),
            Some("R0".into()),
        ),
        Some(crate::agent::AgentKind::Claude),
        None,
    );
    run.launched(&leader);
    let key = run.key().to_string();
    let _ = run.finish(crate::usage::LedgerOutcome::Success);
    let rows = crate::run_ledger::rows_between(
        vault.path(),
        "2000-01-01T00:00:00.000Z",
        "2100-01-01T00:00:00.000Z",
    )
    .unwrap();
    let row = rows.iter().find(|r| r.run_key == key).unwrap();
    assert_eq!(
        row.account,
        crate::run_attribution::account_ref("acct-added-1")
    );
    assert_ne!(row.account, "login", "a remote run never bills the login");
}

/// The runner's stand-in `gh`: whether it got a token (its hash) and the repo.
const FAKE_GH: &str = r#"#!/bin/sh
if [ -n "$GH_TOKEN" ]; then
  echo "gh token sha=$(printf %s "$GH_TOKEN" | shasum -a 256 | cut -c1-16) repo=$GH_REPO args=$*"
else
  echo "gh no token args=$*"
fi
"#;

/// The runner's stand-in `gcloud`: the token file's hash and mode, its path
/// kept in `$TMPDIR/gcloud-file` so the test can see it is gone afterwards.
const FAKE_GCLOUD: &str = r#"#!/bin/sh
f="$CLOUDSDK_AUTH_ACCESS_TOKEN_FILE"
if [ -n "$f" ] && [ -f "$f" ]; then
  echo "$f" >> "$GCLOUD_SEEN"
  echo "gcloud file sha=$(shasum -a 256 < "$f" | cut -c1-16) mode=$(stat -f %Lp "$f") args=$*"
else
  echo "gcloud no file args=$*"
fi
"#;

fn sha16(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Every file under `dir` (as bytes) that contains `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                if let Ok(bytes) = std::fs::read(&p) {
                    if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                        hits.push(p);
                    }
                }
            }
        }
    }
    hits
}

/// Remote runners (2026-11-03): with GitHub and Google Cloud on, the agent's
/// git (credential helper), gh and gcloud get the run's credential one call
/// at a time from the leader here, through the run's own tool socket; gh and
/// gcloud refuse what would print or store it, and limited Cloud access
/// refuses a production change. Afterwards no token is in any file on the
/// runner (the run's output, its scratch, the per-call gcloud file), in the
/// header, on the leader's argv, or in anything the leader printed; and a
/// process outside the agent gets nothing from the tool socket.
#[test]
fn a_runner_turn_gets_github_and_cloud_one_call_at_a_time_and_keeps_none() {
    use crate::runner_access::{CloudAccess, Credentials, TurnAccess};
    let rig = Rig::new();
    let repo = rig.repo();
    std::fs::create_dir(rig.path().join("hostbin")).unwrap();
    rig.exe("hostbin/gh", FAKE_GH);
    rig.exe("hostbin/gcloud", FAKE_GCLOUD);
    let seen = rig.path().join("gcloud-seen");
    let cloud_token = "ya29.fake-cloud-token-0123456789abcdef";
    rig.exe("bin/mint", &format!("#!/bin/sh\necho {cloud_token}\n"));
    // No system git configuration (Apple's names the Keychain's helper): the
    // runner's git sees only what the run sets.
    rig.host_env(&format!(
        "HOLLOW_REMOTE_MIN_FREE_GB=0 GIT_CONFIG_NOSYSTEM=1 GCLOUD_SEEN={} PATH={}:/usr/bin:/bin:/usr/sbin:/sbin",
        seen.display(),
        rig.path().join("hostbin").display()
    ));
    rig.git(&["remote", "add", "origin", "git@github.com:acme/widgets.git"]);
    let mut target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
        .unwrap();
    target.access = TurnAccess {
        github: true,
        cloud: CloudAccess::Limited,
        service_account: None,
        gh_repo: crate::runner_access::github_repo("git@github.com:acme/widgets.git"),
    };
    assert_eq!(target.access.gh_repo.as_deref(), Some("acme/widgets"));
    let token = "github_pat_FAKE0123456789abcdefTOKEN";
    let mut env = probe_env();
    crate::runner_access::header_env(&target.access, &mut env);

    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let held = rig.path().join("agent-pid");
    let args = [
        "gitcred",
        "sh:gh pr create --fill",
        "sh:gh auth token",
        "sh:gh auth status --show-token",
        "sh:gcloud run services list --project p-1",
        "sh:gcloud builds approve b-1",
        "sh:gcloud run deploy svc --no-traffic",
        "sh:gcloud run deploy svc",
        "sh:gcloud auth print-access-token",
        "sh:gcloud config config-helper --format=json",
        // The agent can't lift the guard: the level comes from the Mac.
        "sh:HOLLOW_RUNNER_CLOUD=full gcloud builds approve b-2",
        "sh:git config --get remote.origin.url",
        // origin is this checkout's only: another repository keeps its own.
        "sh:cd $TMPDIR && git init -q other && git -C other remote add origin /tmp/elsewhere && git -C other remote get-url --push --all origin | tr '\\n' ' '",
        // The runner's own global configuration still applies.
        "sh:git config --get user.name",
        &format!("pidfile:{}", held.display()),
        "sleep:3",
    ];
    // The runner account's global git config names a credential helper of
    // its own and a name: the run's file includes it, then drops the helper.
    std::fs::write(
        rig.path().join("host/.gitconfig"),
        "[credential]\n\thelper = \"!f() { echo username=u; echo password=from-the-global-helper; }; f\"\n[user]\n\tname = Global Person\n",
    )
    .unwrap();
    let mut cmd = leader(
        "hollow-remote-probe",
        args.iter().map(|a| a.to_string()).collect(),
        env,
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    // What `into_leader` adds: the secrets, only in the leader's environment.
    let creds = Credentials {
        github: Some(Ok(token.to_string())),
        gcloud_mint: Some(Ok(vec![rig
            .path()
            .join("bin/mint")
            .to_string_lossy()
            .into_owned()])),
        gcloud_level: Some(CloudAccess::Limited),
    };
    for (key, value) in crate::runner_access::leader_env(&creds) {
        cmd.env(key, value);
    }
    let std_cmd = cmd.as_std();
    for arg in std_cmd.get_args() {
        let arg = arg.to_string_lossy();
        assert!(
            !arg.contains(token) && !arg.contains(cloud_token),
            "argv: {arg}"
        );
    }
    let header = std_cmd
        .get_envs()
        .find(|(k, _)| *k == "HOLLOW_REMOTE_HEADER")
        .and_then(|(_, v)| v)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        !header.contains(token) && !header.contains("ya29."),
        "header: {header}"
    );
    assert!(header.contains("HOLLOW_RUNNER_GITHUB") && header.contains("GH_REPO"));

    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    // While the agent holds, a process outside it asks the run's tool
    // socket for the credential: it gets nothing.
    let pid = wait_pid(&held);
    assert!(pid > 0);
    let run_dir = rig.path().join("host/hollow-remote/run");
    let tools = std::fs::read_dir(&run_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".tools.sock"))
        .expect("the run's tool socket");
    let mut outside = std::os::unix::net::UnixStream::connect(&tools).unwrap();
    writeln!(outside, "hollow-credential-github").unwrap();
    outside
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut got = String::new();
    let _ = BufReader::new(&outside).read_line(&mut got);
    assert!(
        !got.contains(token) && !got.starts_with("ok"),
        "outside got: {got}"
    );

    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    let line = |prefix: &str| -> String {
        out.lines()
            .find(|l| l.contains(prefix))
            .unwrap_or_else(|| panic!("no {prefix} in {out}\n{err}"))
            .to_string()
    };
    assert_eq!(
        line("gitcred:"),
        format!("gitcred: len={} sha={}", token.len(), sha16(token))
    );
    let gh = line("args=pr create");
    assert!(
        gh.contains(&format!("gh token sha={} repo=acme/widgets", sha16(token))),
        "{gh}"
    );
    assert!(line("gh auth token").contains("isn't available on a runner"));
    assert!(line("doesn't show the token").contains("rc=1"));
    let list = line("args=run services list");
    assert!(
        list.contains(&format!("gcloud file sha={} mode=600", sha16(cloud_token))),
        "{list}"
    );
    assert!(line("gcloud builds approve").contains("production change"));
    assert!(line("args=run deploy svc --no-traffic").contains("gcloud file sha="));
    assert!(line("run deploy (without --no-traffic)").contains("production change"));
    assert!(line("print-access-token").contains("isn't available on a runner"));
    assert!(line("config config-helper").contains("isn't available on a runner"));
    assert!(out.contains("gcloud builds approve is a production change"));
    assert_eq!(
        out.matches("gcloud builds approve is a production change")
            .count(),
        2,
        "the agent's HOLLOW_RUNNER_CLOUD=full didn't lift it: {out}"
    );
    assert!(line("sh: https://github.com/acme/widgets.git").contains("rc=0"));
    let other = line("/tmp/elsewhere");
    assert!(!other.contains("github.com"), "{other}");
    assert!(line("Global Person").contains("rc=0"));

    // Each per-call gcloud file was gone when its call ended.
    let files = std::fs::read_to_string(&seen).unwrap();
    assert_eq!(files.lines().count(), 2, "{files}");
    for f in files.lines() {
        assert!(!Path::new(f).exists(), "{f} left behind");
    }
    // No token anywhere on the runner, nor in what the leader printed.
    for secret in [token, cloud_token] {
        let hits = files_containing(&rig.path().join("host"), secret);
        assert!(hits.is_empty(), "{secret} stored on the runner: {hits:?}");
        assert!(!out.contains(secret) && !err.contains(secret), "printed");
    }
    // Nor in the turn's record here.
    if let Ok(dir) = turns_dir() {
        let state = state_path(&dir, &target.turn);
        if let Ok(text) = std::fs::read_to_string(state) {
            assert!(!text.contains(token) && !text.contains(cloud_token));
        }
    }
    peers.release("tag-test");
}

/// `into_leader` reads the workspace's credential when it builds the leader
/// and puts it only in the leader's own environment: not on its argv, not in
/// the header (so never in the agent's environment there), and a token in
/// this Mac's environment never rides along.
#[test]
fn into_leader_puts_the_credential_only_in_the_leaders_environment() {
    use crate::runner_access::{GithubSource, TurnAccess};
    let _held = crate::runner_access::tests::SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let token = "github_pat_LEADERONLY0123456789abcdef";
    crate::runner_access::set_github_source(GithubSource::RunnerToken).unwrap();
    crate::runner_access::save_github_token(token).unwrap();
    let target = RemoteTarget {
        host: "fakehost".into(),
        home: "/Users/runner".into(),
        turn: "0a1b2c3d".into(),
        access: TurnAccess {
            github: true,
            gh_repo: Some("acme/widgets".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut agent = Command::new("/usr/local/bin/claude");
    agent
        .args(["-p", "x"])
        .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-test-oauth-7f3a")
        .env("GH_TOKEN", "gho_FROMTHISMACSENV0123");
    let leader = into_leader(&agent, crate::agent::AgentKind::Claude, &target, None, None);
    crate::runner_access::clear_github_token().unwrap();
    let leader = leader.unwrap();
    let std_leader = leader.as_std();
    let env_of = |name: &str| -> Option<String> {
        std_leader
            .get_envs()
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().into_owned())
    };
    assert_eq!(env_of("HOLLOW_CRED_GITHUB").as_deref(), Some(token));
    let header = env_of("HOLLOW_REMOTE_HEADER").unwrap();
    assert!(!header.contains(token), "{header}");
    assert!(!header.contains("gho_FROMTHISMACSENV0123"), "{header}");
    let h: Value = serde_json::from_str(&header).unwrap();
    assert_eq!(h["env"]["HOLLOW_RUNNER_GITHUB"], "on");
    assert_eq!(h["env"]["GH_REPO"], "acme/widgets");
    assert!(h["env"].get("GH_TOKEN").is_none());
    for arg in std_leader.get_args() {
        assert!(!arg.to_string_lossy().contains(token));
    }
    // The turn's record (kept on disk while it runs) holds no secret.
    let record = serde_json::to_string(&target).unwrap();
    assert!(!record.contains(token) && record.contains("acme/widgets"));
}

/// The same turn with nothing turned on: no credential helper, no flags, gh
/// and gcloud run as the runner's own (no token), and the helper says why.
#[test]
fn a_runner_turn_without_access_reaches_nothing() {
    let rig = Rig::new();
    let repo = rig.repo();
    std::fs::create_dir(rig.path().join("hostbin")).unwrap();
    rig.exe("hostbin/gh", FAKE_GH);
    rig.exe("hostbin/gcloud", FAKE_GCLOUD);
    rig.host_env(&format!(
        "HOLLOW_REMOTE_MIN_FREE_GB=0 GIT_CONFIG_NOSYSTEM=1 PATH={}:/usr/bin:/bin:/usr/sbin:/sbin",
        rig.path().join("hostbin").display()
    ));
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
        .unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let args = [
        "sh:git config --get-regexp '^credential' ; git config --get remote.origin.url",
        "sh:gh pr list",
        "sh:gcloud projects list",
        "sh:printf 'protocol=https\\nhost=github.com\\n\\n' | /usr/bin/perl $HOME/hollow-remote/bin/hollow-remote-run --credential get",
    ];
    let cmd = leader(
        "hollow-remote-probe",
        args.iter().map(|a| a.to_string()).collect(),
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    // No helper and no origin were set for the run.
    assert!(out.contains("sh: rc=1"), "{out}");
    assert!(!out.contains("hollow-remote-run --credential"), "{out}");
    assert!(out.contains("gh no token args=pr list"), "{out}");
    assert!(out.contains("gcloud no file args=projects list"), "{out}");
    assert!(out.contains("GitHub isn't turned on"), "{out}");
    // And the leader of such a turn holds nothing to hand out.
    let none = crate::runner_access::credentials(&target.access);
    assert!(crate::runner_access::leader_env(&none).is_empty());
    peers.release("tag-test");
}

/// The relay's socket file appears at bind, before it calls listen: a connect in that gap
/// is refused (the 11-13 gate flake). Retry a refused or missing socket for a few seconds.
fn connect_when_listening(path: &Path) -> std::os::unix::net::UnixStream {
    let t0 = Instant::now();
    loop {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(s) => return s,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) && t0.elapsed() < Duration::from_secs(10) =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("the relay socket never accepted: {e}"),
        }
    }
}

/// The relay serves only connections that carry the run's token and name a
/// server of its config; anything else is closed without starting anything.
#[test]
fn the_relay_refuses_a_connection_without_the_runs_token() {
    let rig = Rig::new();
    let rt = runtime();
    let started = rig.path().join("server-started");
    rig.exe(
        "bin/probe-server",
        &format!("#!/bin/sh\ntouch '{}'\ncat\n", started.display()),
    );
    let config = rig.config(&rig.path().join("unused.sock"));
    let local = rig.path().join("relay.sock");
    // A leader whose "ssh" just waits, so the relay stays up.
    rig.exe("bin/wait-ssh", "#!/bin/sh\nexec /bin/sleep 20\n");
    let mut cmd = Command::new("/usr/bin/perl");
    cmd.arg("-e")
        .arg(LEADER_PL)
        .arg(&config)
        .arg(&local)
        .arg(rig.path().join("host/run-base"))
        .arg("0123456789ab")
        .arg("--")
        .arg(rig.path().join("bin/wait-ssh"))
        .arg("fakehost")
        .env("HOLLOW_RELAY_TOKEN", "x".repeat(32))
        .env("HOLLOW_REMOTE_HEADER", "{}")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    cmd.process_group(0);
    let mut child = rt.block_on(async { cmd.spawn().unwrap() });
    let t0 = Instant::now();
    while !local.exists() {
        assert!(t0.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    let mode = std::fs::metadata(&local).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "the relay socket is private: {mode:o}");
    for line in [
        format!("HOLLOW-RELAY 1 {} hollow_probe", "y".repeat(32)),
        format!("HOLLOW-RELAY 1 {} not_a_server", "x".repeat(32)),
        "GET / HTTP/1.0".to_string(),
    ] {
        let mut s = connect_when_listening(&local);
        writeln!(s, "{line}").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(&mut s, &mut buf);
        assert!(buf.is_empty(), "{line}: {buf}");
    }
    assert!(!started.exists(), "no server started");
    let mut s = connect_when_listening(&local);
    writeln!(s, "HOLLOW-RELAY 1 {} hollow_probe", "x".repeat(32)).unwrap();
    writeln!(s, "echoed").unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut s, &mut buf).unwrap();
    assert_eq!(buf, "echoed\n", "the right token reaches the server");
    assert!(started.exists());
    unsafe { libc::killpg(child.id().unwrap() as i32, libc::SIGTERM) };
    let _ = rt.block_on(child.wait());
}

/// Phase 3 against a real runner (`HOLLOW_REMOTE_IT_HOST=runner-host`,
/// `--ignored`): a vault workspace chat's turn runs in its copy of the
/// folder there, its tool call is served here, its edits come back, an edit
/// made here meanwhile stays, and a file changed on both sides is kept twice.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_session::tests::real_runner -- --ignored"]
fn real_runner_workspace_folder_round_trip() {
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    let rig = Rig::new();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/it-folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "# IT\n\n## Decisions\n- a\n\n## History\n- h\n",
    )
    .unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    // Overnight R3: what the folder's `.gitignore` names goes and comes
    // back; a secret-looking file stays here.
    std::fs::create_dir_all(folder.join("build")).unwrap();
    std::fs::write(folder.join(".gitignore"), "build/\n.env\n").unwrap();
    std::fs::write(folder.join("build/out.txt"), "built\n").unwrap();
    std::fs::write(folder.join(".env"), "TOKEN=here\n").unwrap();
    let request = FolderRequest {
        vault,
        folder: folder.clone(),
        session_key: format!("it-{}", random_hex(8)),
    };
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let started = Instant::now();
    let target = prepare_folder(&host, "ssh", &request).unwrap();
    eprintln!(
        "prepared on {host} in {:?}: {}",
        started.elapsed(),
        target.dir
    );
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "where".into(),
            "bridge:hollow_probe:from the folder".into(),
            "cat:PLAN.md".into(),
            "write:PLAN.md:plan, from the runner".into(),
            "write:new.md:made on the runner".into(),
            "cat:build/out.txt".into(),
            "cat:.env".into(),
            "write:build/new.txt:made on the runner".into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("{out}{err}");
    assert_eq!(code, 0);
    assert!(
        out.contains("bridge hollow_probe: admitted from the folder"),
        "{out}"
    );
    assert!(out.contains("PLAN.md: plan"), "{out}");
    assert!(out.contains("build/out.txt: built"), "{out}");
    assert!(out.contains("no .env\n"), "{out}");
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "# IT\n\n## Decisions\n- a\n- b, made here meanwhile\n\n## History\n- h\n",
    )
    .unwrap();
    let started = Instant::now();
    let back = collect_folder(&target).unwrap().unwrap();
    eprintln!("came back in {:?}: {back:?}", started.elapsed());
    let mut changed = back.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["PLAN.md", "build/new.txt", "new.md"]);
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "plan, from the runner\n"
    );
    assert!(std::fs::read_to_string(folder.join("WORKSPACE.md"))
        .unwrap()
        .contains("made here meanwhile"));

    // Both sides change the plan: both kept.
    let target = prepare_folder(&host, "ssh", &request).unwrap();
    let cmd = leader(
        "hollow-remote-probe",
        vec!["write:PLAN.md:the runner's plan".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}{err}");
    std::fs::write(folder.join("PLAN.md"), "this Mac's plan\n").unwrap();
    let back = collect_folder(&target).unwrap().unwrap();
    eprintln!("conflict: {back:?}");
    assert_eq!(back.copies.len(), 1);
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "this Mac's plan\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(&back.copies[0])).unwrap(),
        "the runner's plan\n"
    );
    peers.release("tag-test");
}

/// Against a real runner (`HOLLOW_REMOTE_IT_HOST=runner-host`, `--ignored`):
/// prepare, a probe turn whose tool call is served here through the real
/// forwarded socket and admitted by the broker's peer check, the work coming
/// back, then a Stop that leaves no process on the host.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_session::tests::real_runner -- --ignored"]
fn real_runner_round_trip_and_stop() {
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    real_round_trip_and_stop(&host);
}

/// Remote runners (2026-11-03): a read-only Google Cloud call on the real
/// runner through a token this Mac's gcloud mints, handed over one request at
/// a time. `HOLLOW_REMOTE_IT_GCP_PROJECT` names the project to list Cloud Run
/// services in (Alex's: `example-project`). Nothing is created or changed.
#[test]
#[ignore = "needs a real runner and this Mac's gcloud sign-in; HOLLOW_REMOTE_IT_HOST=runner-host HOLLOW_REMOTE_IT_GCP_PROJECT=<project> cargo test --lib remote_session::tests::real_runner_cloud -- --ignored --nocapture"]
fn real_runner_cloud_read_through_a_forwarded_token() {
    let (Ok(host), Ok(project)) = (
        std::env::var("HOLLOW_REMOTE_IT_HOST"),
        std::env::var("HOLLOW_REMOTE_IT_GCP_PROJECT"),
    ) else {
        return;
    };
    use crate::runner_access::{CloudAccess, TurnAccess};
    let rig = Rig::new();
    let repo = rig.repo();
    let mut target = prepare(&host, "ssh", &repo).unwrap();
    target.access = TurnAccess {
        cloud: CloudAccess::Limited,
        ..Default::default()
    };
    let mut env = probe_env();
    crate::runner_access::header_env(&target.access, &mut env);
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let mut cmd = leader(
        "hollow-remote-probe",
        vec![
            "sh:gcloud auth list --format='value(account)'".into(),
            format!("sh:gcloud run services list --project {project} --format='value(metadata.name)' --limit 5"),
            format!("sh:gcloud builds approve x --project {project}"),
            "sh:ls -a $HOLLOW_SCRATCH_DIR".into(),
        ],
        env,
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let creds = crate::runner_access::credentials(&target.access);
    let mint = creds.gcloud_mint.as_ref().unwrap().clone().unwrap();
    let minted = std::process::Command::new(&mint[0])
        .args(&mint[1..])
        .output()
        .unwrap();
    let token = String::from_utf8_lossy(&minted.stdout).trim().to_string();
    assert!(token.len() > 20, "this Mac's gcloud must be signed in");
    for (key, value) in crate::runner_access::leader_env(&creds) {
        cmd.env(key, value);
    }
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("{out}{err}");
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[1].contains("rc=0"), "the read worked: {}", lines[1]);
    assert!(lines[2].contains("production change"), "{}", lines[2]);
    assert!(
        !lines[3].contains(".gcloud-token-"),
        "no token file left: {}",
        lines[3]
    );
    assert!(!out.contains(&token) && !err.contains(&token));
    // The needle goes on stdin, never a command line; only where a run
    // writes (its session, scratch and run folders), not the build folders.
    let left = ssh_call(
        "ssh",
        &host,
        &format!(
            "/usr/bin/grep -rlF -f /dev/stdin {} {} {home}/hollow-remote/run 2>/dev/null; ls {home}/hollow-remote/scratch/.gcloud-token-* 2>/dev/null; echo done",
            shell_quote(&target.dir),
            shell_quote(&target.scratch),
            home = shell_quote(&target.home)
        ),
        format!("{}\n", &token[..32]).as_bytes(),
    )
    .unwrap_or_default();
    assert_eq!(
        left.trim(),
        "done",
        "the token is in a file on the runner: {left}"
    );
    let _ = collect(&target);
    peers.release("tag-test");
}

fn real_round_trip_and_stop(host: &str) {
    let host = host.to_string();
    let rig = Rig::new();
    let repo = rig.repo();
    let target = prepare(&host, "ssh", &repo).unwrap();
    eprintln!("prepared on {host}: {}", target.dir);
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "where".into(),
            "bridge:hollow_probe:from the runner".into(),
            "write:new.txt:made on the runner".into(),
            "commit:new.txt:runner commit".into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("{out}{err}");
    assert_eq!(code, 0);
    assert!(
        out.contains("bridge hollow_probe: admitted from the runner"),
        "{out}"
    );
    let place: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_ne!(
        place["host"].as_str().unwrap(),
        String::from_utf8_lossy(
            &std::process::Command::new("hostname")
                .arg("-s")
                .output()
                .unwrap()
                .stdout
        )
        .trim()
    );
    let collected = collect(&target).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(rig.git(&["log", "-1", "--format=%s"]), "runner commit");
    peers.release("tag-test");

    // Stop: the agent's pid on the host, then the leader's group here.
    let target = prepare(&host, "ssh", &repo).unwrap();
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "pidfile:/tmp/hollow-remote-it.pid".into(),
            "sleep:120".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let leader_pid = child.id().unwrap() as i32;
    let ssh = |c: &str| {
        let o = std::process::Command::new("ssh")
            .args(["-o", "BatchMode=yes", &host, c])
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
    };
    let t0 = Instant::now();
    let agent = loop {
        let (ok, pid) = ssh("cat /tmp/hollow-remote-it.pid 2>/dev/null");
        if ok && !pid.is_empty() {
            break pid;
        }
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(300));
    };
    assert!(ssh(&format!("kill -0 {agent}")).0, "running on {host}");
    // Stop kills the leader's group here; the run there goes on (Phase 4)
    // until the turn's finish collects, which stops it.
    unsafe { libc::killpg(leader_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, child);
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        ssh(&format!("kill -0 {agent}")).0,
        "the run goes on on {host} without the connection"
    );
    let stopped = Instant::now();
    let collected = collect(&target);
    eprintln!("collected after the stop: {collected:?}");
    assert!(
        !ssh(&format!("kill -0 {agent} 2>/dev/null")).0,
        "ended on {host}"
    );
    let (_, left) = ssh("ps -axo pid,command | grep -c '[h]ollow-remote-run'");
    eprintln!(
        "stopped on {host} after {:?}; hollow-remote-run processes left: {left}",
        stopped.elapsed()
    );
    assert_eq!(left, "0");
    let _ = ssh("rm -f /tmp/hollow-remote-it.pid");
    peers.release("tag-test");
}

/// Phase 4 against a real runner (`HOLLOW_REMOTE_IT_HOST=runner-host`,
/// `--ignored`): the leader's ssh is killed mid-turn; the run goes on there,
/// the leader reconnects, every line arrives exactly once, a tool call after
/// the drop is served here through the new forward, and the work comes back.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_session::tests::real_runner -- --ignored"]
fn real_runner_reattach_after_a_drop() {
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    let rig = Rig::new();
    let repo = rig.repo();
    let target = prepare(&host, "ssh", &repo).unwrap();
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let pid_file = format!("/tmp/hollow-remote-it-drop-{}.pid", random_hex(6));
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "say:one".into(),
            "err:stderr before the drop".into(),
            "bridge:hollow_probe:before the drop".into(),
            format!("pidfile:{pid_file}"),
            "sleep:8".into(),
            "say:two".into(),
            "err:stderr after the drop".into(),
            "bridge:hollow_probe:after the drop".into(),
            "write:across.txt:made on the runner across a drop".into(),
            "commit:across.txt:runner commit across a drop".into(),
            "say:three".into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let leader_pid = child.id().unwrap() as i32;
    let ssh = |c: &str| {
        let o = std::process::Command::new("ssh")
            .args(["-o", "BatchMode=yes", &host, c])
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
    };
    let t0 = Instant::now();
    let agent = loop {
        let (ok, pid) = ssh(&format!("cat {pid_file} 2>/dev/null"));
        if ok && !pid.is_empty() {
            break pid;
        }
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(300));
    };
    let conn = children_of(leader_pid, "--run");
    assert_eq!(conn.len(), 1, "the leader's ssh: {conn:?}");
    unsafe { libc::kill(conn[0], libc::SIGKILL) };
    let dropped = Instant::now();
    assert!(
        ssh(&format!("kill -0 {agent}")).0,
        "the run goes on on {host} after the drop"
    );
    let (code, out, err) = wait_output(&rt, child);
    eprintln!(
        "finished {:?} after the drop\n--- stdout\n{out}--- stderr\n{err}",
        dropped.elapsed()
    );
    assert_eq!(code, 0);
    for line in [
        "one",
        "two",
        "three",
        "bridge hollow_probe: admitted before the drop",
        "bridge hollow_probe: admitted after the drop",
    ] {
        assert_eq!(out.lines().filter(|l| *l == line).count(), 1, "{line} once");
    }
    // Overnight R2: the run's stderr too, each line exactly once.
    for line in ["stderr before the drop", "stderr after the drop"] {
        assert_eq!(err.lines().filter(|l| *l == line).count(), 1, "{line} once");
    }
    assert!(err.contains("reconnected to"), "{err}");
    let collected = collect(&target).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s"]),
        "runner commit across a drop"
    );
    let _ = ssh(&format!("rm -f {pid_file}"));
    let (_, left) = ssh("ps -axo pid,command | grep -c '[h]ollow-remote-run'");
    eprintln!("hollow-remote-run processes left on {host}: {left}");
    assert_eq!(left, "0");
    peers.release("tag-test");
}

/// Overnight R1 against a real runner (`HOLLOW_REMOTE_IT_HOST`): this Mac's
/// leader is killed with its group mid-turn after Hollow handled the first
/// line (a Hollow that quit); the run goes on there; a new leader built from
/// the turn's record attaches from those bytes: the rest arrives once, a
/// tool call after the restart is served here, the work comes back applied,
/// and nothing is left running there.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_session::tests::real_runner -- --ignored"]
fn real_runner_reattach_after_a_restart() {
    use tokio::io::AsyncBufReadExt;
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    let rig = Rig::new();
    let target = prepare(&host, "ssh", &rig.repo()).unwrap();
    let key = keep_record(&target);
    let peers = crate::run_auth::Peers::leaked();
    let broker_sock = rig.path().join("broker.sock");
    broker(&broker_sock, peers);
    let config = rig.config(&broker_sock);
    let rt = runtime();
    let pid_file = format!("/tmp/hollow-remote-it-restart-{}.pid", random_hex(6));
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            r#"say:{"n":"one"}"#.into(),
            format!("pidfile:{pid_file}"),
            "sleep:8".into(),
            r#"say:{"n":"two"}"#.into(),
            "err:stderr after the restart".into(),
            "bridge:hollow_probe:after the restart".into(),
            "write:restart.txt:made on the runner while Hollow was away".into(),
            "commit:restart.txt:runner commit across a restart".into(),
            r#"say:{"n":"three"}"#.into(),
        ],
        probe_env(),
        &target,
        Some(&config),
        false,
    )
    .unwrap();
    let mut first = spawn_leader(&rt, cmd, peers);
    let first_pid = first.id().unwrap() as i32;
    let dir = turns_dir().unwrap();
    let mut progress = Progress::start_in(&dir, &target.turn, 0).unwrap();
    let mut lines = tokio::io::BufReader::new(first.stdout.take().unwrap()).lines();
    let line = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(60), lines.next_line()).await })
        .expect("no hang")
        .unwrap()
        .unwrap();
    assert_eq!(line, r#"{"n":"one"}"#);
    progress.add(&line);
    let sup = wait_state(&target.turn).sup;
    let quit = Instant::now();
    unsafe { libc::killpg(first_pid, libc::SIGKILL) };
    let _ = wait_output(&rt, first);
    peers.release("tag-test");
    drop(progress);
    let stale = stale_turn(&key).expect("the turn's record");
    assert_eq!(stale.reattach.as_ref().map(|r| r.out), Some(12));
    let StaleOutcome::Reattach(again) = settle_stale(&key, stale, StaleChoice::Reattach, true)
    else {
        panic!("the run is still there");
    };
    let cmd = leader(
        "hollow-remote-probe",
        vec![],
        probe_env(),
        &again,
        Some(&config),
        false,
    )
    .unwrap();
    let second = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, second);
    eprintln!(
        "finished {:?} after the quit\n--- stdout\n{out}--- stderr\n{err}",
        quit.elapsed()
    );
    assert_eq!(code, 0);
    assert!(!out.contains(r#""one""#), "{out}");
    for line in [
        r#"{"n":"two"}"#,
        r#"{"n":"three"}"#,
        "bridge hollow_probe: admitted after the restart",
    ] {
        assert_eq!(out.lines().filter(|l| *l == line).count(), 1, "{line} once");
    }
    assert_eq!(
        err.lines()
            .filter(|l| *l == "stderr after the restart")
            .count(),
        1,
        "{err}"
    );
    let collected = collect(&again).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(
        rig.git(&["log", "-1", "--format=%s"]),
        "runner commit across a restart"
    );
    forget_turn(&key, &again.turn);
    let ssh = |c: &str| {
        let o = std::process::Command::new("ssh")
            .args(["-o", "BatchMode=yes", &host, c])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let _ = ssh(&format!("rm -f {pid_file}"));
    // This run's supervisor is gone (other runs there are not this test's).
    let left = ssh(&format!(
        "kill -0 {sup} 2>/dev/null && echo alive || echo gone"
    ));
    eprintln!("this run's supervisor on {host}: {left}");
    assert_eq!(left, "gone");
    peers.release("tag-test");
}

/// One MCP tool, served on this Mac: it names the machine it runs on.
const MCP_WHERE: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP; $| = 1;
my $host = `hostname -s`; chomp $host;
while (my $line = <STDIN>) {
  my $m = eval { decode_json($line) } or next;
  next unless defined $m->{id};
  my $method = $m->{method} // '';
  my $r;
  if ($method eq 'initialize') { $r = { protocolVersion => ($m->{params}{protocolVersion} // '2025-06-18'), capabilities => { tools => {} }, serverInfo => { name => 'hollow_where', version => '1' } } }
  elsif ($method eq 'tools/list') { $r = { tools => [ { name => 'where_am_i', description => 'Names the machine this tool runs on.', inputSchema => { type => 'object', properties => {} } } ] } }
  elsif ($method eq 'tools/call') { $r = { content => [ { type => 'text', text => "served on $host" } ] } }
  else { print encode_json({ jsonrpc => '2.0', id => $m->{id}, error => { code => -32601, message => 'no such method' } }) . "\n"; next }
  print encode_json({ jsonrpc => '2.0', id => $m->{id}, result => $r }) . "\n";
}
"#;

/// A real Codex turn on a real runner (`HOLLOW_REMOTE_IT_HOST`, and
/// `HOLLOW_REMOTE_IT_CODEX=1` because it spends a little Codex usage),
/// built by Hollow's own command builder: this Mac's Codex sign-in goes over
/// the SSH channel only, Codex runs on the runner, and the tool it calls is
/// served here through the relay.
#[test]
#[ignore = "spends Codex usage on a real runner; HOLLOW_REMOTE_IT_HOST=runner-host HOLLOW_REMOTE_IT_CODEX=1 cargo test --lib remote_session::tests::real_codex -- --ignored --nocapture"]
fn real_codex_turn_on_the_runner() {
    let (Ok(host), Ok(_)) = (
        std::env::var("HOLLOW_REMOTE_IT_HOST"),
        std::env::var("HOLLOW_REMOTE_IT_CODEX"),
    ) else {
        return;
    };
    // A ChatGPT sign-in can't be handed over (only an API key can): then
    // the run is refused with the message the turn shows.
    if let Err(why) = codex_credential() {
        assert_eq!(why, CODEX_CHATGPT_REFUSAL);
        eprintln!("this Mac's Codex has no API key, so a remote Codex turn is refused: {why}");
        return;
    }
    let rig = Rig::new();
    rig.exe("bin/mcp-where", MCP_WHERE);
    let target = prepare(&host, "ssh", &rig.repo()).unwrap();
    let config = json!({ "mcpServers": { "hollow_where": {
        "command": rig.path().join("bin/mcp-where"), "args": [],
    }}})
    .to_string();
    let spec = crate::agent::RunSpec {
        prompt: "Call the where_am_i tool of the hollow_where server, then reply with exactly TOOL= followed by what it returned, and nothing else. Do not run shell commands.".into(),
        resume: None,
        model: std::env::var("HOLLOW_REMOTE_IT_CODEX_MODEL").ok(),
        perm: "bypassPermissions".into(),
        cwd: rig.repo(),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        interactive: false,
        mcp_config: Some(config),
        append_system_prompt: None,
        process_scope: crate::agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        extras: crate::agent::RunExtras {
            remote: Some(target.clone()),
            ..Default::default()
        },
    };
    let mut cmd =
        crate::agent::command_without_codex_listing(crate::agent::AgentKind::Codex, &spec);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(cmd.as_std().get_program(), "/usr/bin/perl", "{args:?}");
    let token_on_argv = args.iter().any(|a| a.contains("eyJ"));
    assert!(!token_on_argv, "no token on the leader's argv");
    cmd.process_group(0);
    let rt = runtime();
    let child = rt.block_on(async { cmd.spawn().unwrap() });
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("exit {code}\n--- stdout\n{out}\n--- stderr\n{err}");
    assert_eq!(code, 0);
    let here = String::from_utf8_lossy(
        &std::process::Command::new("hostname")
            .arg("-s")
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert!(
        out.contains(&format!("TOOL=served on {here}")),
        "Codex on the runner got the tool's answer from this Mac"
    );
    assert!(err.contains("hollow-remote: result "), "it ran there");
    assert_eq!(collect(&target).unwrap(), Collected::Unchanged);
}

/// Overnight N6: unattended work prefers the runner only in a build
/// workspace that follows the default, while Settings' Overnight and worker
/// chats is the runner, and only for Claude and Codex. A workspace's own
/// choice, a vault workspace (C-3) and other agents never move by it. Read
/// live, the settings are this process's, where it is absent: since M7 that
/// follows Runs on, so nothing moves even with a runner set up (the rule
/// with the runner chosen: `the_unattended_rule_follows_...`).
#[test]
fn unattended_work_prefers_the_runner_only_in_a_build_workspace_on_the_default() {
    use crate::agent::AgentKind;
    let vault = tempfile::tempdir().unwrap();
    let tree = vault.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    crate::build_workspaces::bind_repo_for_test(
        vault.path(),
        "bw",
        &vault.path().join("repo"),
        &tree,
    );
    assert_eq!(
        unattended_runner(vault.path(), &tree, AgentKind::Claude, PlaceReads::LIVE),
        None
    );
    assert_eq!(
        unattended_runner(vault.path(), &tree, AgentKind::Codex, PlaceReads::LIVE),
        None
    );
    assert_eq!(
        unattended_runner(
            vault.path(),
            &tree,
            AgentKind::Antigravity,
            PlaceReads::LIVE
        ),
        None
    );
    assert_eq!(
        unattended_runner(vault.path(), &tree, AgentKind::Grok, PlaceReads::LIVE),
        None
    );
    for own in ["this-mac", "mac-mini"] {
        crate::build_workspaces::set_runs_on(vault.path(), "bw", Some(own)).unwrap();
        assert_eq!(
            unattended_runner(vault.path(), &tree, AgentKind::Claude, PlaceReads::LIVE),
            None,
            "{own}"
        );
    }
    crate::build_workspaces::set_runs_on(vault.path(), "bw", None).unwrap();
    // Not a build workspace: a vault workspace's folder, the vault itself.
    let folder = vault.path().join("workspaces").join("w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: w1\nstatus: active\n---\n\n# w1\n",
    )
    .unwrap();
    assert_eq!(
        unattended_runner(vault.path(), &folder, AgentKind::Claude, PlaceReads::LIVE),
        None
    );
    assert_eq!(
        unattended_runner(
            vault.path(),
            vault.path(),
            AgentKind::Claude,
            PlaceReads::LIVE
        ),
        None
    );
}

/// Self-hosted models Q7: a turn on a self-hosted model runs on this Mac
/// whatever Runs on says, and records which runner it was kept from (its
/// first line's host). Each place Runs on is chosen: a build workspace's
/// own (its Changes tab, or a new conversation's empty state), a vault
/// workspace's own (its chats), Settings, Devices, Runs on, and Overnight
/// and worker chats' default; with This Mac nothing is noted. A Claude
/// model's turn with the same settings goes to the runner as before. Where
/// nothing goes to the runner, `execute_turn` prepares nothing there (its
/// `wanted` is `None`; pinned in `chat_self_hosted_tests.rs`). The settings
/// and the runner are given, not read: the settings file is shared by this
/// test process.
#[test]
fn a_self_hosted_turn_stays_here_whatever_runs_on_says() {
    use crate::agent::AgentKind;
    use crate::agent_settings::AgentSettings;
    let vault = tempfile::tempdir().unwrap();
    let tree = vault.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    crate::build_workspaces::bind_repo_for_test(
        vault.path(),
        "bw",
        &vault.path().join("repo"),
        &tree,
    );
    let folder = vault.path().join("workspaces").join("w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: w1\nstatus: active\n---\n\n# w1\n",
    )
    .unwrap();
    // Settings as saved by default (Runs on This Mac; Overnight and worker
    // chats follow it since M7), with a runner set up.
    let defaults = PlaceReads {
        settings: || Some(AgentSettings::default()),
        runner: || Some("runner-host".into()),
    };
    let settings_mini = PlaceReads {
        settings: || {
            Some(AgentSettings {
                runs_on: Some("mac-mini".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    let no_runner = PlaceReads {
        settings: || {
            Some(AgentSettings {
                runs_on: Some("mac-mini".into()),
                ..AgentSettings::default()
            })
        },
        runner: || None,
    };
    let v = vault.path();
    let place = |cwd: &Path, prefers: bool, self_hosted: bool, reads: PlaceReads| {
        turn_place(v, cwd, AgentKind::Claude, prefers, self_hosted, reads)
    };
    let kept = |host: &str| TurnPlace {
        chosen: None,
        preferred: None,
        self_hosted_here: Some(host.to_string()),
    };
    let chosen = TurnPlace {
        chosen: Some(Ok("runner-host".into())),
        ..TurnPlace::default()
    };

    // This Mac: here, no note, for both.
    for self_hosted in [true, false] {
        assert_eq!(
            place(&tree, false, self_hosted, defaults),
            TurnPlace::default()
        );
        assert_eq!(
            place(&folder, false, self_hosted, defaults),
            TurnPlace::default()
        );
    }
    crate::build_workspaces::set_runs_on(v, "bw", Some("this-mac")).unwrap();
    for (prefers, reads) in [(false, settings_mini), (true, defaults)] {
        assert_eq!(place(&tree, prefers, true, reads), TurnPlace::default());
        assert_eq!(place(&tree, prefers, false, reads), TurnPlace::default());
    }

    // The build workspace's own Runs on (Changes, or a new chat's empty state).
    crate::build_workspaces::set_runs_on(v, "bw", Some("mac-mini")).unwrap();
    assert_eq!(place(&tree, false, true, defaults), kept("runner-host"));
    assert_eq!(place(&tree, false, false, defaults), chosen);
    // An unattended turn there follows the workspace's own choice.
    assert_eq!(place(&tree, true, true, defaults), kept("runner-host"));
    assert_eq!(place(&tree, true, false, defaults), chosen);
    crate::build_workspaces::set_runs_on(v, "bw", None).unwrap();

    // A vault workspace's own Runs on (its chats).
    crate::remote_folder::set_runs_on(v, "w1", Some("mac-mini")).unwrap();
    assert_eq!(place(&folder, false, true, defaults), kept("runner-host"));
    assert_eq!(place(&folder, false, false, defaults), chosen);
    crate::remote_folder::set_runs_on(v, "w1", None).unwrap();

    // Settings, Devices, Runs on, for a build workspace following it.
    assert_eq!(
        place(&tree, false, true, settings_mini),
        kept("runner-host")
    );
    assert_eq!(place(&tree, false, false, settings_mini), chosen);
    // M6: saved under the runner's neutral name, the same place.
    let settings_runner = PlaceReads {
        settings: || {
            Some(AgentSettings {
                runs_on: Some("runner".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    assert_eq!(place(&tree, false, false, settings_runner), chosen);

    // Overnight and worker chats' default since M7: it follows Runs on,
    // so with This Mac nothing moves and nothing is noted.
    assert_eq!(place(&tree, true, true, defaults), TurnPlace::default());
    assert_eq!(place(&tree, true, false, defaults), TurnPlace::default());
    // Set to the runner (when it answers).
    let unattended_on = PlaceReads {
        settings: || {
            Some(AgentSettings {
                unattended_runs_on: Some("runner".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    assert_eq!(place(&tree, true, true, unattended_on), kept("runner-host"));
    assert_eq!(
        place(&tree, true, false, unattended_on),
        TurnPlace {
            preferred: Some("runner-host".into()),
            ..TurnPlace::default()
        }
    );
    assert_eq!(place(&tree, true, true, settings_mini), kept("runner-host"));

    // The runner chosen but none set up: a Claude model's turn is refused
    // with why; a self-hosted one runs here, its note's host empty (Q7
    // review: the chat gives that its own sentence, `remoteRunNotice.ts`,
    // never "the runner has no Google Cloud login"), unattended too.
    assert_eq!(place(&tree, false, true, no_runner), kept(""));
    assert_eq!(place(&tree, true, true, no_runner), kept(""));
    assert_ne!(place(&tree, false, true, no_runner), kept("runner-host"));
    assert!(matches!(
        place(&tree, false, false, no_runner).chosen,
        Some(Err(why)) if why.contains("no runner is set up")
    ));
}

/// Self-hosted models Q9 review: a Codex chat on an `openai` entry is a
/// self-hosted chat for placing (`self_hosted::chat_on_self_hosted`), so
/// with Runs on the runner it runs on this Mac with Q7's note, as a Claude
/// one does; an ordinary Codex model's turn goes to the runner.
#[test]
fn a_self_hosted_codex_turn_stays_here_too() {
    use crate::agent::AgentKind;
    use crate::agent_settings::AgentSettings;
    let vault = tempfile::tempdir().unwrap();
    let cwd = vault.path().join("tree");
    std::fs::create_dir_all(&cwd).unwrap();
    crate::build_workspaces::bind_repo_for_test(
        vault.path(),
        "bw",
        &vault.path().join("repo"),
        &cwd,
    );
    let settings_mini = PlaceReads {
        settings: || {
            Some(AgentSettings {
                runs_on: Some("mac-mini".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    let self_hosted =
        crate::self_hosted::chat_on_self_hosted(AgentKind::Codex, Some("self-hosted:qwen-codex"));
    assert!(self_hosted);
    let place = turn_place(
        vault.path(),
        &cwd,
        AgentKind::Codex,
        false,
        self_hosted,
        settings_mini,
    );
    assert_eq!(
        place,
        TurnPlace {
            chosen: None,
            preferred: None,
            self_hosted_here: Some("runner-host".into()),
        }
    );
    // The runner kept from is named whatever Codex's own sign-in here is
    // (its check is for turns that go there). An ordinary Codex model's turn
    // is placed as before: the runner, or why Codex can't go there.
    let ordinary = crate::self_hosted::chat_on_self_hosted(AgentKind::Codex, Some("gpt-6-luna"));
    assert!(!ordinary);
    let place = turn_place(
        vault.path(),
        &cwd,
        AgentKind::Codex,
        false,
        ordinary,
        settings_mini,
    );
    assert_eq!(place.self_hosted_here, None);
    assert!(place.chosen.is_some(), "{place:?}");
}

/// Self-hosted models Q7 review: Overnight and worker chats on Follow Runs
/// on (`agents.unattendedRunsOn` `same`) with Settings, Devices, Runs on
/// the Mac mini sends an unattended or worker turn by the person's Runs on
/// (`runs_on`, not the unattended default). A self-hosted chat's turn still
/// runs here with its note's host; a Claude model's goes there. With Runs
/// on This Mac neither moves and nothing is noted.
#[test]
fn an_unattended_self_hosted_turn_following_runs_on_stays_here() {
    use crate::agent::AgentKind;
    use crate::agent_settings::AgentSettings;
    let vault = tempfile::tempdir().unwrap();
    let tree = vault.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    crate::build_workspaces::bind_repo_for_test(
        vault.path(),
        "bw",
        &vault.path().join("repo"),
        &tree,
    );
    let follow_mini = PlaceReads {
        settings: || {
            Some(AgentSettings {
                runs_on: Some("mac-mini".into()),
                unattended_runs_on: Some("same".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    let follow_here = PlaceReads {
        settings: || {
            Some(AgentSettings {
                unattended_runs_on: Some("same".into()),
                ..AgentSettings::default()
            })
        },
        runner: || Some("runner-host".into()),
    };
    let v = vault.path();
    // The rule itself: Follow Runs on gives no unattended default.
    assert_eq!(
        unattended_runner(v, &tree, AgentKind::Claude, follow_mini),
        None
    );
    // An unattended or worker turn starting its native session (`prefers`).
    let unattended = |self_hosted: bool, reads| {
        turn_place(v, &tree, AgentKind::Claude, true, self_hosted, reads)
    };
    assert_eq!(
        unattended(true, follow_mini),
        TurnPlace {
            self_hosted_here: Some("runner-host".into()),
            ..TurnPlace::default()
        },
        "a self-hosted chat runs here, its first line naming the runner"
    );
    assert_eq!(
        unattended(false, follow_mini),
        TurnPlace {
            chosen: Some(Ok("runner-host".into())),
            ..TurnPlace::default()
        },
        "a Claude model's turn follows Runs on to the runner"
    );
    for self_hosted in [true, false] {
        assert_eq!(unattended(self_hosted, follow_here), TurnPlace::default());
    }
}

/// Overnight N6: what keeps a preferred turn here before anything is sent,
/// each said as a sentence, and a failed preparation's reason.
#[test]
fn a_preferred_turn_stays_here_when_the_runner_cannot_take_it() {
    use crate::agent::AgentKind;
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("wt");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    // The host name is this test's own, so no status check answers for it.
    let host = "it-n6-nowhere";
    // A remote turn of this Mac's already has the worktree there.
    begin_turn(
        "it-n6-busy",
        RemoteTarget {
            worktree: tree.clone(),
            git_worktree: true,
            ..Default::default()
        },
        AgentKind::Claude,
    );
    let busy = preferred_blocker(host, AgentKind::Claude, &tree.join("src"), false, false);
    take_turn("it-n6-busy");
    forget_turn("it-n6-busy", "");
    let busy = busy.unwrap();
    assert!(busy.message.starts_with("Another turn of this worktree"));
    // RT1: each blocker carries its cause.
    assert_eq!(busy.cause, "busy");
    // A claude.ai connector needs the login, which stays here.
    let login = preferred_blocker(host, AgentKind::Claude, &tree, true, true).unwrap();
    assert_eq!(login.cause, "signIn");
    let login = login.message;
    assert!(login.contains("Claude Code login"), "{login}");
    // No added account (no GUI registry here): the login would run it.
    let account = preferred_blocker(host, AgentKind::Claude, &tree, true, false).unwrap();
    assert_eq!(account.cause, "signIn");
    let account = account.message;
    assert!(account.contains("added accounts"), "{account}");
    for why in [login, account] {
        assert!(why.ends_with('.'), "a sentence: {why}");
    }
    // A failed preparation's message, without the advice to choose This Mac.
    let why = stay_reason(&unreachable_message(
        "runner-host",
        "ssh: connect to host x port 22: Connection refused",
    ));
    assert!(
        why.starts_with("The runner (runner-host) didn't answer"),
        "{why}"
    );
    assert!(
        why.contains("Remote Login") && !why.contains("This Mac") && why.ends_with('.'),
        "{why}"
    );
    assert_eq!(
        stay_reason("runner-host isn't taking work now (no: free=12GB)"),
        "runner-host isn't taking work now (no: free=12GB)."
    );
}

/// Overnight N6, the rule itself: only Claude and Codex, only a build
/// workspace following the default, only while the setting is the runner
/// (`runner` or `mac-mini`; since M7 not absent, not `same`), and only to a
/// runner that is set up.
#[test]
fn the_unattended_rule_follows_the_workspace_the_setting_and_the_runner() {
    use crate::agent::AgentKind;
    let runner = || Some("runner-host".to_string());
    let rule = |agent, own: Option<Option<&str>>, setting, runner: Option<String>| {
        unattended_runner_for(agent, own.map(|o| o.map(str::to_string)), setting, runner)
    };
    // A build workspace following the default goes on the runner by either
    // name (`runner`, or `mac-mini` from before M6); since M7 absent (or an
    // unknown value) follows Runs on, like `same`.
    for setting in [Some("runner"), Some(" mac-mini ")] {
        assert_eq!(
            rule(AgentKind::Claude, Some(None), setting, runner()),
            runner(),
            "{setting:?}"
        );
    }
    for setting in [None, Some("moon")] {
        assert_eq!(
            rule(AgentKind::Claude, Some(None), setting, runner()),
            None,
            "{setting:?}"
        );
    }
    let on = Some("runner");
    assert_eq!(rule(AgentKind::Codex, Some(None), on, runner()), runner());
    // Follow Runs on: not by this rule.
    assert_eq!(
        rule(AgentKind::Claude, Some(None), Some("same"), runner()),
        None
    );
    // No runner set up, another agent, a workspace's own choice, not a
    // build workspace: here.
    assert_eq!(rule(AgentKind::Claude, Some(None), on, None), None);
    assert_eq!(rule(AgentKind::Antigravity, Some(None), on, runner()), None);
    assert_eq!(rule(AgentKind::Grok, Some(None), on, runner()), None);
    for own in ["this-mac", "runner", "mac-mini"] {
        assert_eq!(
            rule(AgentKind::Claude, Some(Some(own)), on, runner()),
            None,
            "{own}"
        );
    }
    assert_eq!(rule(AgentKind::Claude, None, on, runner()), None);
}

/// Overnight N6 review r1: `start_turn` opens a stint before the turn reads
/// its resume state, so "starts its native session" means the open stint
/// holds no native id yet (and none was asked to resume), not that no stint
/// is open.
#[test]
fn a_turn_starts_its_native_session_until_one_is_recorded() {
    let vault = tempfile::tempdir().unwrap();
    let key = "s-20261102-0000beef";
    crate::session_store::start_turn(vault.path(), key, "claude", "/tmp/x", "run-n6-1", "hi")
        .unwrap();
    let state = crate::session_store::resume_state(vault.path(), key).unwrap();
    assert!(state.open.is_some(), "start_turn opened a stint");
    assert!(starts_native_session(None, &state));
    assert!(starts_native_session(Some("  "), &state));
    assert!(starts_native_session(Some("antigravity"), &state));
    assert!(!starts_native_session(Some("abc-123"), &state));
    crate::session_store::record_harness_id(vault.path(), key, "native-1").unwrap();
    let state = crate::session_store::resume_state(vault.path(), key).unwrap();
    assert!(!starts_native_session(None, &state));
    assert!(starts_native_session(
        None,
        &crate::session_store::ResumeState::default()
    ));
}

/// A run that died there leaves its marker naming a supervisor that is gone
/// (pid 999999 never exists on macOS) and no result.
fn leave_dead_marker(dir: &str) {
    std::fs::write(format!("{dir}.running"), "999999\nrun-died\n").unwrap();
}

/// Phase 3 leftover (2026-11-02): a run that died there (the runner
/// restarting mid-turn: its supervisor gone, its marker left, no result) is
/// kept at the next preparation instead of being reset away, and comes back
/// on a branch; a copy left exactly as it was sent keeps nothing, and a run
/// that finished normally isn't kept twice.
#[test]
fn a_run_that_died_there_is_kept_at_the_next_preparation() {
    let rig = Rig::new();
    // Uncommitted here, so what is sent isn't HEAD (review r1: a clean
    // checkout let "changed nothing" pass without the check).
    std::fs::write(rig.repo().join("gone.txt"), "dirty here\n").unwrap();
    let first = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    // Died having changed nothing: nothing to keep.
    leave_dead_marker(&first.dir);
    let second = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(second.kept.is_empty(), "{:?}", second.kept);
    assert_eq!(host_kept_refs(&rig, &second), "");
    // Died after adding one file and changing another.
    let there = Path::new(&second.dir);
    std::fs::write(there.join("salvaged.txt"), "kept\n").unwrap();
    std::fs::write(there.join("kept.txt"), "changed there\n").unwrap();
    leave_dead_marker(&second.dir);
    let third = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert_eq!(third.kept, vec!["work-mini-1".to_string()]);
    assert_eq!(rig.git(&["show", "work-mini-1:salvaged.txt"]), "kept");
    assert_eq!(rig.git(&["show", "work-mini-1:kept.txt"]), "changed there");
    assert!(
        !rig.repo().join("salvaged.txt").exists(),
        "the worktree untouched"
    );
    assert!(
        !Path::new(&format!("{}.running", third.dir)).exists(),
        "the stale marker went"
    );
    // The folder was reset for the new turn: what it kept is gone there.
    assert!(!Path::new(&third.dir).join("salvaged.txt").exists());
    // A marker with no change since: nothing more.
    leave_dead_marker(&third.dir);
    let fourth = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(fourth.kept.is_empty(), "{:?}", fourth.kept);
    // A run that finished (its result written, then brought back here) and
    // left its marker anyway, as one killed just after its result would:
    // its finish stamped `.done` with its run, so nothing is kept again.
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let ran = probe_run(&rig, &rt, peers, &["write:done.txt:finished there"]);
    let done = std::fs::read_to_string(format!("{}.done", ran.dir)).unwrap();
    assert!(matches!(collect(&ran).unwrap(), Collected::Applied { .. }));
    std::fs::write(format!("{}.running", ran.dir), format!("999999\n{done}")).unwrap();
    let fifth = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    assert!(fifth.kept.is_empty(), "{:?}", fifth.kept);
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("done.txt")).unwrap(),
        "finished there\n"
    );
    // Review r2: a later attempt's run in the same copy (a retry, an
    // account switch) that died after one finished there is still kept.
    let again = probe_run(&rig, &rt, peers, &["write:first.txt:first attempt"]);
    std::fs::write(Path::new(&again.dir).join("second.txt"), "second attempt\n").unwrap();
    std::fs::write(format!("{}.running", again.dir), "999999\nrun-second\n").unwrap();
    let sixth = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    // The first attempt's uncollected result and what the second left.
    let holds = |branch: &str| {
        std::process::Command::new("git")
            .current_dir(rig.repo())
            .args(["show", &format!("{branch}:second.txt")])
            .output()
            .unwrap()
            .stdout
            == b"second attempt\n"
    };
    assert_eq!(sixth.kept.len(), 2, "{:?}", sixth.kept);
    assert!(sixth.kept.iter().any(|b| holds(b)), "{:?}", sixth.kept);
    peers.release("tag-test");
}

/// The runner's salvage leaves out what a finish leaves out: its copy of
/// the junk list is `remote_folder::EXCLUDE`, the same text, and its bounds
/// are `remote_folder`'s, the same numbers (overnight R3 review).
#[test]
fn the_salvage_leaves_out_what_a_finish_leaves_out() {
    let perl = format!(
        "my $FOLDER_EXCLUDE = \"{}\";",
        crate::remote_folder::EXCLUDE.replace('\n', "\\n")
    );
    assert!(REMOTE_RUN_PL.contains(&perl), "{perl}");
    let bounds = format!(
        "my ($FOLDER_MAX_BYTES, $IGNORED_MAX_FILES, $IGNORED_MAX_BYTES) = ({}, {}, {});",
        crate::remote_folder::MAX_FILE_BYTES,
        crate::remote_folder::GITIGNORED_MAX_FILES,
        crate::remote_folder::GITIGNORED_MAX_BYTES
    );
    assert!(REMOTE_RUN_PL.contains(&bounds), "{bounds}");
}

/// The same for a vault workspace's folder copy: what a run that died there
/// left comes back here at the next preparation, merged like any kept
/// result, on top of the snapshot it was prepared from.
#[test]
fn a_folder_run_that_died_there_comes_back_at_the_next_preparation() {
    let rig = Rig::new();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    std::fs::write(folder.join("WORKSPACE.md"), "# W1\n").unwrap();
    std::fs::write(folder.join(".gitignore"), "*.log\n").unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-died".into(),
    };
    let first = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let there = Path::new(&first.dir);
    std::fs::write(there.join("PLAN.md"), "plan\nfrom the runner\n").unwrap();
    std::fs::write(there.join("new.md"), "new there\n").unwrap();
    // Overnight R3: one the copy's `.gitignore` names is kept too.
    std::fs::write(there.join("run.log"), "logged there\n").unwrap();
    std::fs::create_dir_all(there.join("node_modules")).unwrap();
    std::fs::write(there.join("node_modules/junk.js"), "x\n").unwrap();
    leave_dead_marker(&first.dir);
    // Changed here meanwhile: kept, the runner's merged in.
    std::fs::write(folder.join("WORKSPACE.md"), "# W1 here\n").unwrap();
    let next = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let earlier = next
        .folder_earlier
        .expect("the dead run's changes came back");
    assert_eq!(earlier.changed.len(), 3, "{earlier:?}");
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "plan\nfrom the runner\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("new.md")).unwrap(),
        "new there\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("run.log")).unwrap(),
        "logged there\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("WORKSPACE.md")).unwrap(),
        "# W1 here\n"
    );
    assert!(!folder.join("node_modules").exists(), "junk stays there");
}

/// Overnight P3L-5: a dead run's copy that can't be kept is tried again for
/// an hour counted from the first failed keep (its run was last heard hours
/// before, which used to give up at once), then moved aside there instead
/// of reset, and the start notice's `aside` names where.
#[test]
fn a_dead_runs_copy_that_cant_be_kept_is_moved_aside_after_an_hour() {
    let rig = Rig::new();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-aside".into(),
    };
    let first = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let there = Path::new(&first.dir).to_path_buf();
    std::fs::write(there.join("PLAN.md"), "plan\nfrom the runner\n").unwrap();
    leave_dead_marker(&first.dir);
    let ago = |path: &str, hours: i64| {
        let when = (chrono::Local::now() - chrono::Duration::hours(hours))
            .format("%Y%m%d%H%M")
            .to_string();
        let ok = std::process::Command::new("/usr/bin/touch")
            .args(["-t", &when, path])
            .status()
            .unwrap();
        assert!(ok.success(), "{path}");
    };
    // Its run was last heard five hours ago.
    ago(&format!("{}.running", first.dir), 5);
    // Keeps fail there: the bare repository refuses the `-died` ref.
    let sync = first.folder.clone().expect("a folder turn");
    let hook = rig.path().join(format!(
        "host/hollow-remote/repos/{}.git/hooks/reference-transaction",
        sync.repo_id
    ));
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\nif grep -e '-died$' >/dev/null; then exit 1; fi\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stamp = format!("{}.keep-failed", first.dir);
    let Err(refused) = rig.with_host(|| prepare_folder("fakehost", &rig.ssh(), &request)) else {
        panic!("not given up at once");
    };
    assert!(refused.contains("trying again next time"), "{refused}");
    assert!(
        Path::new(&stamp).exists(),
        "the first failed keep is stamped"
    );
    assert_eq!(
        std::fs::read_to_string(there.join("PLAN.md")).unwrap(),
        "plan\nfrom the runner\n"
    );
    // Still within the hour (30 minutes on): tried again, still there, and
    // the stamp is never refreshed by a later failure.
    ago(&stamp, 0);
    let half = (chrono::Local::now() - chrono::Duration::minutes(30))
        .format("%Y%m%d%H%M")
        .to_string();
    assert!(std::process::Command::new("/usr/bin/touch")
        .args(["-t", &half, &stamp])
        .status()
        .unwrap()
        .success());
    let stamped = std::fs::metadata(&stamp).unwrap().modified().unwrap();
    assert!(rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .is_err());
    assert_eq!(
        std::fs::metadata(&stamp).unwrap().modified().unwrap(),
        stamped
    );
    // An hour after the first failed keep: moved aside, started fresh.
    ago(&stamp, 2);
    let next = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    assert_eq!(next.aside.len(), 1, "{:?}", next.aside);
    let aside = Path::new(&next.aside[0]);
    assert!(
        next.aside[0].starts_with(&format!("{}/hollow-remote/aside/", next.home)),
        "{:?}",
        next.aside
    );
    assert_eq!(
        std::fs::read_to_string(aside.join("PLAN.md")).unwrap(),
        "plan\nfrom the runner\n"
    );
    assert_eq!(
        std::fs::read_to_string(there.join("PLAN.md")).unwrap(),
        "plan\n"
    );
    assert!(!Path::new(&stamp).exists());
    assert!(!Path::new(&format!("{}.running", first.dir)).exists());
    std::fs::remove_file(&hook).unwrap();
    // The next preparation has nothing more to say, and a stamp left with
    // no dead run beside it goes.
    std::fs::write(&stamp, "").unwrap();
    let calm = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    assert!(calm.aside.is_empty(), "{:?}", calm.aside);
    assert!(!Path::new(&stamp).exists(), "a stale stamp goes");
}

/// The same for a build workspace's worktree copy (review of #1084): the
/// copy moved aside is cut off from the session's git admin folder, which
/// the fresh checkout reuses, so git there can't move the live session; the
/// run's HEAD is written beside its files.
#[test]
fn a_dead_worktree_copy_moved_aside_is_cut_off_from_the_live_session() {
    let rig = Rig::new();
    let first = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    let there = Path::new(&first.dir).to_path_buf();
    std::fs::write(there.join("left.txt"), "left there\n").unwrap();
    leave_dead_marker(&first.dir);
    let hook = rig.path().join(format!(
        "host/hollow-remote/repos/{}.git/hooks/reference-transaction",
        first.repo_id
    ));
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\nif grep -e '-died$' >/dev/null; then exit 1; fi\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Err(refused) = rig.with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo())) else {
        panic!("tried again first");
    };
    assert!(refused.contains("trying again next time"), "{refused}");
    let stamp = format!("{}.keep-failed", first.dir);
    let when = (chrono::Local::now() - chrono::Duration::hours(2))
        .format("%Y%m%d%H%M")
        .to_string();
    assert!(std::process::Command::new("/usr/bin/touch")
        .args(["-t", &when, &stamp])
        .status()
        .unwrap()
        .success());
    let next = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &rig.repo()))
        .unwrap();
    std::fs::remove_file(&hook).unwrap();
    assert_eq!(next.aside.len(), 1, "{:?}", next.aside);
    let aside = Path::new(&next.aside[0]);
    assert_eq!(
        std::fs::read_to_string(aside.join("left.txt")).unwrap(),
        "left there\n"
    );
    assert!(!aside.join(".git").exists());
    assert!(aside.join(".git-was-worktree").exists());
    let head = std::fs::read_to_string(aside.join(".hollow-aside")).unwrap();
    assert!(
        head.starts_with("head ") && head.trim().len() >= 45,
        "{head}"
    );
    // Git in the aside copy finds no repository; the live session's does.
    let git_in = |dir: &Path| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
            .args(["rev-parse", "--git-dir"])
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(!git_in(aside), "the aside copy has no repository");
    assert!(git_in(Path::new(&next.dir)), "the live session works");
    assert!(!Path::new(&next.dir).join("left.txt").exists());
}

/// J4: preparation installs the runner's own machine note beside its
/// `hollow-slot`, byte for byte what `this_mac::runner_note` renders, where
/// the supervisor's `HOLLOW_THIS_MAC` points.
#[test]
fn host_preparation_installs_the_runners_machine_note() {
    let rig = Rig::new();
    host_prep("it-j4-note", &rig.ssh(), &["r".to_string()], &[]).unwrap();
    let bin = rig.path().join("host/hollow-remote/bin");
    assert!(bin.join("hollow-slot").is_file());
    assert_eq!(
        std::fs::read_to_string(bin.join(crate::this_mac::FILE_NAME)).unwrap(),
        crate::this_mac::runner_note()
    );
    assert!(REMOTE_RUN_PL.contains("$ENV{HOLLOW_THIS_MAC} = \"$root/bin/this-mac.md\""));
}

/// Copies moved aside go after HOLLOW_REMOTE_DAYS, and sooner (once 12
/// hours old, oldest first) while the volume is under its floor.
#[test]
fn host_preparation_prunes_copies_kept_aside() {
    let rig = Rig::new();
    let root = rig.path().join("host/hollow-remote");
    std::fs::create_dir_all(root.join("aside")).unwrap();
    let ago = |rel: &str, hours: i64| {
        std::fs::create_dir_all(root.join(rel)).unwrap();
        std::fs::write(root.join(rel).join("PLAN.md"), "kept\n").unwrap();
        let when = (chrono::Local::now() - chrono::Duration::hours(hours))
            .format("%Y%m%d%H%M")
            .to_string();
        let ok = std::process::Command::new("/usr/bin/touch")
            .args(["-t", &when])
            .arg(root.join(rel))
            .status()
            .unwrap();
        assert!(ok.success(), "{rel}");
    };
    ago("aside/old-1-1", 20 * 24);
    ago("aside/mid-2-2", 48);
    ago("aside/new-3-3", 1);
    let df = rig.path().join("fake-df");
    std::fs::write(
        &df,
        format!(
            "#!/bin/sh\nkb=$(cat '{}/df.kb')\nprintf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n/dev/disk3s5 999999999 1 %s 1%% /\\n' \"$kb\"\n",
            rig.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&df, std::fs::Permissions::from_mode(0o755)).unwrap();
    rig.host_env(&format!(
        "HOLLOW_REMOTE_MIN_FREE_GB=40 HOLLOW_REMOTE_DF={}",
        df.display()
    ));
    const GB: u64 = 1_048_576;
    let free =
        |gb: u64| std::fs::write(rig.path().join("df.kb"), format!("{}\n", gb * GB)).unwrap();
    free(100);
    host_prep("fake", &rig.ssh(), &["r".to_string()], &[]).unwrap();
    assert!(!root.join("aside/old-1-1").exists(), "past the days");
    assert!(root.join("aside/mid-2-2").exists());
    assert!(root.join("aside/new-3-3").exists());
    free(10);
    host_prep("fake", &rig.ssh(), &["r".to_string()], &[]).unwrap();
    assert!(!root.join("aside/mid-2-2").exists(), "under the floor");
    assert!(root.join("aside/new-3-3").exists(), "under 12 hours old");
}

/// Only a path under the runner's own `aside/` folder is named.
#[test]
fn only_the_runners_aside_folder_is_named() {
    let home = "/Users/runner";
    let named = |a: &str| aside_of(&json!({ "aside": a }), home);
    assert_eq!(
        named("/Users/runner/hollow-remote/aside/w-1a2b-1793612265-411").as_deref(),
        Some("/Users/runner/hollow-remote/aside/w-1a2b-1793612265-411")
    );
    // A repository in a dot folder (`~/.dotfiles`) names its copy so too.
    assert!(named("/Users/runner/hollow-remote/aside/.dotfiles-1a2b-1-2").is_some());
    for bad in [
        "",
        "/Users/runner/hollow-remote/aside/",
        "/Users/runner/hollow-remote/aside/.",
        "/Users/runner/hollow-remote/aside/..",
        "/Users/runner/hollow-remote/aside/../sessions",
        "/Users/runner/hollow-remote/aside/a/b",
        "/Users/other/hollow-remote/aside/x",
        "/Users/runner/hollow-remote/aside/x\ny",
    ] {
        assert_eq!(named(bad), None, "{bad:?}");
    }
    assert_eq!(aside_of(&json!({}), home), None);
}

/// Phase 3 leftover (2026-11-02): a kept folder result is applied three ways
/// only on top of a snapshot this Mac sent. One whose parent the runner made
/// up (here: the same files as what was sent, so "unchanged here" would let
/// its deletion and its edit through) writes nothing over and removes
/// nothing here: overnight P3L-4 brings what it changed only as copies
/// beside the files, says so, and then drops it there.
#[test]
fn a_kept_folder_result_on_a_base_this_mac_never_sent_comes_only_as_copies() {
    let rig = Rig::new();
    let vault = rig.path().join("vault");
    let folder = vault.join("workspaces/w1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("PLAN.md"), "plan\n").unwrap();
    std::fs::write(folder.join("WORKSPACE.md"), "# W1\n").unwrap();
    let request = FolderRequest {
        vault: vault.clone(),
        folder: folder.clone(),
        session_key: "s-forged".into(),
    };
    let first = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let sync = first.folder.clone().expect("a folder turn");
    let bare = rig
        .path()
        .join(format!("host/hollow-remote/repos/{}.git", sync.repo_id));
    let git = |args: &[&str], input: Option<&str>| -> String {
        use std::io::Write as _;
        let mut child = std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.unwrap_or("").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let tree = git(&["rev-parse", &format!("{}^{{tree}}", sync.snapshot)], None);
    let forged_base = git(&["commit-tree", &tree, "-m", "made up there"], None);
    let listing = git(&["ls-tree", &tree], None);
    let edited = git(&["hash-object", "-w", "--stdin"], Some("# W1 from there\n"));
    let added = git(&["hash-object", "-w", "--stdin"], Some("new there\n"));
    let same = git(&["hash-object", "-w", "--stdin"], Some("same\n"));
    let mut entries: String = listing
        .lines()
        .filter(|l| !l.ends_with("\tPLAN.md") && !l.ends_with("\tWORKSPACE.md"))
        .map(|l| format!("{l}\n"))
        .collect();
    entries.push_str(&format!("100644 blob {edited}\tWORKSPACE.md\n"));
    entries.push_str(&format!("100644 blob {added}\tnew.md\n"));
    // Already the same here: left alone, no copy.
    entries.push_str(&format!("100644 blob {same}\tsame.md\n"));
    std::fs::write(folder.join("same.md"), "same\n").unwrap();
    let changing = git(&["mktree"], Some(&entries));
    let result = git(
        &[
            "commit-tree",
            &changing,
            "-p",
            &forged_base,
            "-m",
            "deletes PLAN.md, edits WORKSPACE.md, adds new.md",
        ],
        None,
    );
    let kept = format!("refs/hollow-session/{}-kept-1-1-x", sync.hash());
    git(&["update-ref", &kept, &result], None);
    let next = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    let earlier = next.folder_earlier.expect("its changes came as copies");
    // Nothing here written over, removed or added under its own name.
    assert_eq!(
        std::fs::read_to_string(folder.join("PLAN.md")).unwrap(),
        "plan\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("WORKSPACE.md")).unwrap(),
        "# W1\n"
    );
    assert!(!folder.join("new.md").exists());
    assert!(
        earlier.changed.is_empty() && earlier.merged.is_empty(),
        "{earlier:?}"
    );
    assert!(earlier.copies.is_empty(), "{earlier:?}");
    // Its removal says nothing about this Mac's PLAN.md: not mentioned.
    assert!(earlier.kept_here.is_empty(), "{earlier:?}");
    // What it changed is beside the files, under copy names.
    assert_eq!(earlier.unsure.len(), 2, "{earlier:?}");
    let copy_of = |stem: &str, body: &str| {
        earlier.unsure.iter().any(|c| {
            c.starts_with(&format!("{stem} (conflicted copy"))
                && std::fs::read_to_string(folder.join(c)).unwrap() == body
        })
    };
    assert!(copy_of("WORKSPACE", "# W1 from there\n"), "{earlier:?}");
    assert!(copy_of("new", "new there\n"), "{earlier:?}");
    // Then dropped there (not listed again), and this turn's copy has them.
    let gone = std::process::Command::new("git")
        .arg("-C")
        .arg(&bare)
        .args(["rev-parse", "-q", "--verify", &kept])
        .output()
        .unwrap();
    assert!(!gone.status.success(), "dropped there");
    let there = Path::new(&next.dir);
    for copy in &earlier.unsure {
        assert!(there.join(copy).exists(), "{copy} sent with this turn");
    }
    // A drop that failed there (the ref back, as if it never went): the
    // next preparation only drops it again, no second set of copies.
    let brought = format!("refs/hollow-folder/{}/brought/{result}", sync.hash());
    let marked = |want: bool| {
        let ok = std::process::Command::new("git")
            .env("GIT_DIR", &sync.git_dir)
            .args(["rev-parse", "-q", "--verify", &brought])
            .output()
            .unwrap()
            .status
            .success();
        assert_eq!(ok, want, "brought mark");
    };
    marked(true);
    git(&["update-ref", &kept, &result], None);
    let copies_before = std::fs::read_dir(&folder).unwrap().count();
    let third = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    assert!(third.folder_earlier.is_none(), "{:?}", third.folder_earlier);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), copies_before);
    let gone = std::process::Command::new("git")
        .arg("-C")
        .arg(&bare)
        .args(["rev-parse", "-q", "--verify", &kept])
        .output()
        .unwrap();
    assert!(!gone.status.success(), "dropped there again");
    // Once the runner no longer lists it, the mark goes too.
    let fourth = rig
        .with_host(|| prepare_folder("fakehost", &rig.ssh(), &request))
        .unwrap();
    assert!(
        fourth.folder_earlier.is_none(),
        "{:?}",
        fourth.folder_earlier
    );
    marked(false);
}

/// Overnight (B2 r3 minor): when no branch can be made for the runner's
/// work, the message carries git's own error, cut short.
#[test]
fn keep_aside_says_what_git_said() {
    assert_eq!(
        no_branch_message(None),
        "No free branch name for the runner's work"
    );
    assert_eq!(
        no_branch_message(Some("  \n")),
        "No free branch name for the runner's work"
    );
    // On one line (git's hint lines folded in), no full stop of its own.
    assert_eq!(
        no_branch_message(Some(
            "git branch failed: fatal: cannot lock ref 'refs/heads/w-mini-1':\nhint:   File exists.\n"
        )),
        "No branch could be made for the runner's work (git branch failed: fatal: cannot lock ref 'refs/heads/w-mini-1': hint: File exists.)"
    );
    let long = "x".repeat(400);
    let said = no_branch_message(Some(&long));
    assert!(said.contains(&format!("({}…)", "x".repeat(300))), "{said}");
    // And keep_aside itself, in a repository where the commit isn't.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    for args in [
        &["init", "-q"][..],
        &["commit", "-q", "--allow-empty", "-m", "one"],
    ] {
        let ok = std::process::Command::new("git")
            .current_dir(repo)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(ok.success());
    }
    let missing = "1".repeat(40);
    let Err(message) = keep_aside(repo, &Some("w".into()), &missing) else {
        panic!("no branch at a commit that isn't there");
    };
    assert!(
        message.starts_with("No branch could be made for the runner's work (git branch failed:"),
        "{message}"
    );
}

/// A git command that sees only the config `global` and none of this
/// process's identity variables (a gate on the runner carries this Mac's).
fn isolated_git(home: &Path, global: &Path) -> impl Fn(&mut std::process::Command) {
    let (home, global) = (home.to_path_buf(), global.to_path_buf());
    move |cmd: &mut std::process::Command| {
        cmd.env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for key in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
            "GIT_DIR",
            "GIT_WORK_TREE",
        ] {
            cmd.env_remove(key);
        }
    }
}

/// `git <args>` in `dir`, in that folder's own repository only (never one an
/// outer `GIT_DIR` names, as in a hook) and without anyone's config.
fn plain_git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .status()
        .unwrap();
    assert!(ok.success(), "git {args:?}");
}

/// C5 review: the chat's attachments ignore themselves (a `.gitignore` of
/// `/*`), so the runner's snapshot adds them by name: a checkout whose only
/// change is an attachment isn't clean, and the snapshot holds the file.
#[test]
fn the_snapshot_takes_the_chats_attachments() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let dir = tempfile::tempdir().unwrap();
    let top = dir.path().canonicalize().unwrap();
    std::fs::write(top.join("README.md"), "hi\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "README.md"],
        &["commit", "-q", "-m", "one"],
    ] {
        let ok = std::process::Command::new("git")
            .current_dir(&top)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(ok.success(), "git {args:?}");
    }
    let head = git(&top, &["rev-parse", "HEAD"]).unwrap();
    let common = git(
        &top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .unwrap();
    let rel = attachments_in_checkout("");
    let scratch = tempfile::tempdir().unwrap();
    let head_tree = git(&top, &["rev-parse", "HEAD^{tree}"]).unwrap();
    let before = snapshot(&top, &common, &head, scratch.path(), Some(&rel)).unwrap();
    assert_eq!(
        before,
        (head.clone(), true, head_tree.clone()),
        "nothing attached: clean"
    );

    let vault = tempfile::tempdir().unwrap();
    let body = STANDARD.encode(b"name,email\n");
    let saved =
        crate::chat::save_attachment(&body, "text/csv", Some("customers.csv"), &top, vault.path())
            .unwrap();
    let status = git(&top, &["status", "--porcelain", "--untracked-files=all"]).unwrap();
    assert_eq!(status, "", "git itself ignores the attachment");
    let (commit, clean, compared) =
        snapshot(&top, &common, &head, scratch.path(), Some(&rel)).unwrap();
    assert!(!clean);
    // C5 third review: the attachments travel in the commit, never in the
    // tree a collect compares (nor, without them, in a collect's snapshot).
    assert_eq!(compared, head_tree, "the compared tree leaves them out");
    let collect_side = snapshot(&top, &common, &head, scratch.path(), None).unwrap();
    assert_eq!(collect_side, (head.clone(), true, head_tree.clone()));
    let objects = scratch.path().join("objects");
    let alternates = Path::new(&common).join("objects");
    let env = [
        ("GIT_OBJECT_DIRECTORY", objects.as_os_str()),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates.as_os_str()),
    ];
    let files = git_env(&top, &["ls-tree", "-r", "--name-only", &commit], &env).unwrap();
    let name = Path::new(&saved)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let listed = |path: &str| files.lines().any(|f| f == path);
    assert!(listed(&format!(".hollow-attachments/{name}")), "{files}");
    assert!(listed(".hollow-attachments/.gitignore"), "{files}");
    assert!(listed("README.md"), "{files}");
    // A chat in a folder of the checkout keeps its attachments there.
    let nested = attachments_in_checkout("app/web/");
    assert_eq!(nested, "app/web/.hollow-attachments");
}

/// C5 review: a chat's attachments (ignored by their own `.gitignore` of
/// `/*`) reach the runner by name, and the agent is told where this Mac's
/// paths are there. C5 third review: the result leaves them out and bringing
/// the work back leaves them alone here, one saved during the turn (for the
/// next message) included, which is no change here.
#[test]
fn a_chats_attachments_reach_the_runner_and_stay_here() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = tempfile::tempdir().unwrap();
    let body = STANDARD.encode(b"name,email\n");
    let saved = crate::chat::save_attachment(
        &body,
        "text/csv",
        Some("customers.csv"),
        &rig.repo(),
        vault.path(),
    )
    .unwrap();
    let name = Path::new(&saved)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let target = probe_run(&rig, &rt, peers, &["write:new.txt:from the host"]);
    let there = Path::new(&target.cwd)
        .join(".hollow-attachments")
        .join(&name);
    assert_eq!(std::fs::read_to_string(&there).unwrap(), "name,email\n");
    let note = agent_note(&target);
    let mapping = format!(
        "paths under {} on this Mac are at {} there",
        target.worktree.display(),
        target.dir
    );
    assert!(note.contains(&mapping), "{note}");
    // Its copy there is for reading: a change to it doesn't come back.
    assert!(
        note.contains("An attached file there is a copy to read"),
        "{note}"
    );
    let later = crate::chat::save_attachment(
        &STANDARD.encode(b"%PDF-1.4\n"),
        "application/pdf",
        Some("next.pdf"),
        &rig.repo(),
        vault.path(),
    )
    .unwrap();
    let collected = rig.with_host(|| collect(&target)).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "name,email\n");
    assert_eq!(std::fs::read_to_string(&later).unwrap(), "%PDF-1.4\n");
    assert_eq!(
        std::fs::read_to_string(rig.repo().join("new.txt")).unwrap(),
        "from the host\n"
    );
    let status = rig.git(&["status", "--porcelain"]);
    assert!(!status.contains(".hollow-attachments"), "{status}");
    peers.release("tag-test");
}

/// C5 third review: a result kept on a branch (this Mac changed while the
/// turn ran) never holds the chat's attachments, so merging that branch
/// can't commit them, and they stay here as they were.
#[test]
fn a_result_kept_aside_leaves_the_chats_attachments_out() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let rig = Rig::new();
    let rt = runtime();
    let peers = crate::run_auth::Peers::leaked();
    let vault = tempfile::tempdir().unwrap();
    let saved = crate::chat::save_attachment(
        &STANDARD.encode(b"name,email\n"),
        "text/csv",
        Some("customers.csv"),
        &rig.repo(),
        vault.path(),
    )
    .unwrap();
    let target = probe_run(&rig, &rt, peers, &["write:kept.txt:the host's version"]);
    std::fs::write(rig.repo().join("kept.txt"), "edited here meanwhile\n").unwrap();
    let collected = rig.with_host(|| collect(&target)).unwrap();
    let branch = match &collected {
        Collected::KeptAside { branch } => branch.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        rig.git(&["show", &format!("{branch}:kept.txt")]),
        "the host's version"
    );
    let files = rig.git(&["ls-tree", "-r", "--name-only", &branch]);
    assert!(!files.contains(".hollow-attachments"), "{files}");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "name,email\n");
    peers.release("tag-test");
}

fn git_init(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    plain_git(dir, &["init", "-q"]);
}

/// The identity is the checkout's own, as git here would use it: the global
/// one, a work identity an `includeIf` picks by the checkout's folder or by
/// its remote's URL, a repository's own config over both; None when nothing
/// is configured (git's guess from the account is never taken).
#[test]
fn the_identity_is_the_checkouts_own_with_includes() {
    let dir = tempfile::tempdir().unwrap();
    // Canonical, as git matches `gitdir:` against the resolved path.
    let root = dir.path().canonicalize().unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let global = root.join("global.gitconfig");
    std::fs::write(
        &global,
        format!(
            "[user]\n\tname = Personal Person\n\temail = personal@example.com\n\
             [includeIf \"gitdir:{work}/\"]\n\tpath = {work_inc}\n\
             [includeIf \"hasconfig:remote.*.url:git@github.com:Acme-Org/**\"]\n\tpath = {acme_inc}\n",
            work = root.join("work").display(),
            work_inc = root.join("work.inc").display(),
            acme_inc = root.join("acme.inc").display(),
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("work.inc"),
        "[user]\n\tname = Work Person\n\temail = work@example.com\n",
    )
    .unwrap();
    std::fs::write(
        root.join("acme.inc"),
        "[user]\n\temail = acme@example.com\n[committer]\n\tname = Acme Bot\n",
    )
    .unwrap();
    let git = isolated_git(&home, &global);
    let id = |d: &Path| identity_with(d, &git);

    let personal = root.join("personal");
    git_init(&personal);
    assert_eq!(
        id(&personal),
        Some(GitIdentity {
            author_name: "Personal Person".into(),
            author_email: "personal@example.com".into(),
            committer_name: "Personal Person".into(),
            committer_email: "personal@example.com".into(),
        })
    );
    // A subfolder of a checkout under the work folder: includeIf by gitdir.
    let work = root.join("work/repo");
    git_init(&work);
    std::fs::create_dir_all(work.join("src")).unwrap();
    let got = id(&work.join("src")).unwrap();
    assert_eq!(
        (got.author_name.as_str(), got.author_email.as_str()),
        ("Work Person", "work@example.com")
    );
    assert_eq!(got.committer_email, "work@example.com");
    // includeIf by the remote's URL, with a committer name of its own.
    let acme = root.join("acme");
    git_init(&acme);
    plain_git(
        &acme,
        &["remote", "add", "origin", "git@github.com:Acme-Org/app.git"],
    );
    assert_eq!(
        id(&acme),
        Some(GitIdentity {
            author_name: "Personal Person".into(),
            author_email: "acme@example.com".into(),
            committer_name: "Acme Bot".into(),
            committer_email: "acme@example.com".into(),
        })
    );
    // A repository's own config wins.
    plain_git(
        &personal,
        &["config", "user.email", "this-repo@example.com"],
    );
    assert_eq!(id(&personal).unwrap().author_email, "this-repo@example.com");
    // Nothing configured anywhere: None, not git's guess.
    let bare = isolated_git(&home, Path::new("/dev/null"));
    let fresh = root.join("fresh");
    git_init(&fresh);
    assert_eq!(identity_with(&fresh, &bare), None);
    // A name without an email is no identity either.
    let name_only = root.join("name-only.gitconfig");
    std::fs::write(&name_only, "[user]\n\tname = Only Name\n").unwrap();
    assert_eq!(identity_with(&fresh, isolated_git(&home, &name_only)), None);
    // An empty name is no identity (and git's message, which names the
    // email, is the expected kind, never logged).
    let empty_name = root.join("empty-name.gitconfig");
    std::fs::write(
        &empty_name,
        "[user]\n\tname =\n\temail = quiet@example.com\n",
    )
    .unwrap();
    assert_eq!(
        identity_with(&fresh, isolated_git(&home, &empty_name)),
        None
    );
    // A folder outside any checkout takes the global identity.
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(
        id(&plain).map(|i| i.author_email),
        Some("personal@example.com".into())
    );
    // And a folder that isn't there has none.
    assert_eq!(id(&root.join("missing")), None);
}

/// A failed `git var` is logged only when it isn't the expected "none", and
/// never with an email in it.
#[test]
fn a_failed_identity_lookup_logs_no_email() {
    let empty = "hint: x\nfatal: empty ident name (for <quiet@example.com>) not allowed\n";
    assert_eq!(identity_failure_note(empty), None);
    let none = "*** Please tell me who you are.\nfatal: no email was given and auto-detection is disabled\n";
    assert_eq!(identity_failure_note(none), None);
    assert_eq!(
        identity_failure_note("error: bad config line 3 in file /Users/me/.gitconfig\n"),
        Some("error: bad config line 3 in file /Users/me/.gitconfig".into())
    );
    let named =
        identity_failure_note("fatal: können <ana.maría@example.com> nicht lesen (ü)\n").unwrap();
    assert_eq!(named, "fatal: können <email> nicht lesen (ü)");
    assert!(!named.contains('@'));
    assert_eq!(
        identity_failure_note("fatal: odd > then <\n"),
        Some("fatal: odd > then <".into())
    );
}

#[test]
fn git_idents_parse_to_name_and_email() {
    assert_eq!(
        parse_ident("example-user <dev@example.com> 1793629447 -0500\n"),
        Some(("example-user".into(), "dev@example.com".into()))
    );
    assert_eq!(
        parse_ident("Ana María O'Neil <ana@example.com> 1 +0530"),
        Some(("Ana María O'Neil".into(), "ana@example.com".into()))
    );
    for bad in [
        "",
        "<a@b> 1 +0000",
        "Name <> 1 +0000",
        "Name <a@b>",
        "Name <a@b> x +0000",
        "Name <a@b> 1 0000",
        "Name <a@b> 1 +0000 extra",
        "Name a@b 1 +0000",
    ] {
        assert_eq!(parse_ident(bad), None, "{bad:?}");
    }
}

/// End to end through the stand-in host: the turn's agent sees this Mac's
/// identity for the checkout (not the runner account's guess), its commit
/// comes back carrying it, and without one here git's default there stays.
#[test]
fn the_agents_commits_there_carry_this_macs_identity() {
    let rig = Rig::new();
    let repo = rig.repo();
    rig.git(&["config", "user.name", "Rig Person"]);
    rig.git(&["config", "user.email", "rig@example.com"]);
    // What git here gives this checkout (this process's own variables win,
    // as they would for a commit here).
    let want = checkout_identity(&repo).expect("the rig's checkout has one");
    let target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
        .unwrap();
    assert_eq!(target.identity.as_ref(), Some(&want));
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let ident_line = format!(
        "ident: {} <{}> | {} <{}>",
        want.author_name, want.author_email, want.committer_name, want.committer_email
    );
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "ident".into(),
            "write:new.txt:from the host".into(),
            "commit:new.txt:the host's commit".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains(&ident_line), "{out}");
    let collected = rig.with_host(|| collect(&target)).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    assert_eq!(rig.git(&["log", "-1", "--format=%s"]), "the host's commit");
    assert_eq!(
        rig.git(&["log", "-1", "--format=%an <%ae> | %cn <%ce>"]),
        ident_line.trim_start_matches("ident: ")
    );

    // None here: nothing is sent, so the host's own default applies.
    let mut target = rig
        .with_host(|| prepare("fakehost", &rig.ssh(), &repo))
        .unwrap();
    target.identity = None;
    let cmd = leader(
        "hollow-remote-probe",
        vec!["ident".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = rig.with_host(|| spawn_leader(&rt, cmd, peers));
    let (code, out, err) = wait_output(&rt, child);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("ident: "), "{out}");
    assert!(!out.contains(&want.author_email), "{out}");
    let _ = rig.with_host(|| collect(&target));
    peers.release("tag-test");
}

/// Against a real runner (`HOLLOW_REMOTE_IT_HOST=runner-host`, `--ignored`):
/// the runner account has no git identity, so before 11-02 an agent's commit
/// there was `Hollow runner <runner@…>`. Now a commit made there comes
/// back with the identity this Mac's git uses for the checkout, and without
/// one git's default there stays.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_session::tests::real_runner -- --ignored"]
fn real_runner_commit_carries_this_macs_identity() {
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    let rig = Rig::new();
    let repo = rig.repo();
    let want = checkout_identity(&repo).expect("this Mac has a git identity for the checkout");
    let target = prepare(&host, "ssh", &repo).unwrap();
    assert_eq!(target.identity.as_ref(), Some(&want));
    let peers = crate::run_auth::Peers::leaked();
    let rt = runtime();
    let cmd = leader(
        "hollow-remote-probe",
        vec![
            "where".into(),
            "ident".into(),
            "write:new.txt:made on the runner".into(),
            "commit:new.txt:runner commit".into(),
        ],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("{out}{err}");
    assert_eq!(code, 0);
    let expected = format!(
        "{} <{}> | {} <{}>",
        want.author_name, want.author_email, want.committer_name, want.committer_email
    );
    assert!(out.contains(&format!("ident: {expected}")), "{out}");
    let collected = collect(&target).unwrap();
    assert!(
        matches!(collected, Collected::Applied { .. }),
        "{collected:?}"
    );
    let got = rig.git(&["log", "-1", "--format=%H %s | %an <%ae> | %cn <%ce>"]);
    eprintln!("came back from {host}: {got}");
    assert!(
        got.ends_with(&format!("runner commit | {expected}")),
        "{got}"
    );

    // Without one here: git's own default on the runner (what it was before).
    let mut target = prepare(&host, "ssh", &repo).unwrap();
    target.identity = None;
    let cmd = leader(
        "hollow-remote-probe",
        vec!["ident".into()],
        probe_env(),
        &target,
        None,
        false,
    )
    .unwrap();
    let child = spawn_leader(&rt, cmd, peers);
    let (code, out, err) = wait_output(&rt, child);
    eprintln!("without an identity here: {out}{err}");
    assert_eq!(code, 0);
    assert!(!out.contains(&want.author_email), "{out}");
    let _ = collect(&target);
    peers.release("tag-test");
}

/// Lane S10 (2026-11-03): `host_prep` ends a noted refusal once its
/// preflight passes, and an unrelated call that gets through doesn't (the
/// two notes swapped, or the preparation's dropped, passed every test
/// before). The runner's last status is process-wide and other tests' ssh
/// calls write it too, so it is read right after each call under a host
/// name of its own, a few times over.
#[test]
fn a_passing_preparation_ends_a_refusal_and_another_call_does_not() {
    use crate::remote_status::{note_failure, state_of};
    let rig = Rig::new();
    let host = "it-s10-prep";
    let mut kept = false;
    for _ in 0..5 {
        note_failure(host, "refusing", "no: free=12GB, under 20GB");
        host_prep(host, &rig.ssh(), &["r".to_string()], &[]).unwrap();
        assert_ne!(state_of(host), Some("refusing"), "a passing preparation");
        note_failure(host, "refusing", "no: free=12GB, under 20GB");
        ssh_call(&rig.ssh(), host, "true", b"").unwrap();
        if state_of(host) == Some("refusing") {
            kept = true;
            break;
        }
    }
    assert!(kept, "an unrelated call that got through ended a refusal");
}

/// Lane S10 (2026-11-03): an ssh killed by a signal has no exit code. It
/// counted as having got through and ended an unreachable status; it now
/// says nothing either way.
#[test]
fn an_ssh_killed_by_a_signal_leaves_unreachable_alone() {
    use crate::remote_status::{note_failure, state_of};
    let rig = Rig::new();
    rig.exe("bin/killed-ssh", "#!/bin/sh\nkill -9 $$\n");
    let ssh = rig.path().join("bin/killed-ssh");
    let host = "it-s10-killed";
    let mut kept = false;
    for _ in 0..5 {
        note_failure(
            host,
            "unreachable",
            "ssh: connect to host it-s10-killed port 22: Operation timed out",
        );
        let err = ssh_call(&ssh.to_string_lossy(), host, "true", b"").unwrap_err();
        assert!(err.contains("couldn't get the session ready"), "{err}");
        if state_of(host) == Some("unreachable") {
            kept = true;
            break;
        }
    }
    assert!(kept, "a killed ssh ended an unreachable status");
}

/// Both ends of a Unix socket, for `peer_pid` (`PEER_HARNESS <socket path>`,
/// after the sub): a child connects and reports what it sees (the server,
/// the process that made the listening socket); the server sees the child.
/// Prints `ok`, or what each saw.
const PEER_HARNESS: &str = r#"
my $path = shift;
my $srv = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Local => $path, Listen => 1) or die "listen: $!";
pipe(my $r, my $w) or die "pipe: $!";
my $kid = fork() // die "fork: $!";
if (!$kid) {
  close $r; close $srv;
  my $c = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => $path) or POSIX::_exit(2);
  syswrite($w, peer_pid($c) . "\n"); close $w;
  my $b; sysread($c, $b, 1);
  POSIX::_exit(0);
}
close $w;
my $conn = $srv->accept or die "accept: $!";
my $seen = peer_pid($conn);
my $theirs = <$r> // ''; chomp $theirs;
close $conn; waitpid($kid, 0);
print $seen == $kid && $theirs eq "$$" ? "ok\n" : "server saw $seen (the client is $kid), client saw $theirs (the server is $$)\n";
"#;

/// A Linux runner (2026-11-03): `peer_pid`, the supervisor's peer check in
/// `REMOTE_RUN_PL`, names the process at the other end of a Unix socket from
/// the kernel, on both ends: macOS's `LOCAL_PEERPID` here, Linux's
/// `SO_PEERCRED` there (this harness ran in a Linux container too,
/// BUILD_LOG 2026-11-03).
#[test]
fn the_peer_check_names_the_other_end() {
    let start = REMOTE_RUN_PL.find("\nsub peer_pid {").expect("peer_pid") + 1;
    let len = REMOTE_RUN_PL[start..].find("\n}\n").expect("its end") + 3;
    let sub = &REMOTE_RUN_PL[start..start + len];
    let dir = tempfile::Builder::new()
        .prefix("peer")
        .tempdir_in("/tmp")
        .unwrap();
    let script = format!(
        "use strict; use warnings; use IO::Socket::UNIX; use POSIX ();\n{sub}{PEER_HARNESS}"
    );
    let out = std::process::Command::new("/usr/bin/perl")
        .arg("-e")
        .arg(&script)
        .arg(dir.path().join("s.sock"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ok\n");
}

/// Review of item 4 (Linux run isolation): `--run` and `--attach` (and the
/// supervisor, which forks from `--run`) make themselves non-dumpable on
/// Linux (`no_dump`: prctl PR_SET_DUMPABLE 0) in a BEGIN block ahead of
/// every module, so the window in which the header and secret wait in their
/// stdin is as short as perl allows; a no-op on macOS. Checked here: the
/// syscall number comes from perl's own build (`archname`), not the kernel;
/// `no_dump` runs (on Linux leaving PR_GET_DUMPABLE at 0); and the BEGIN
/// block comes before the first `use`, for those two modes alone. That a
/// live supervisor's /proc entries turn root's is for the Debian container
/// check to show (BUILD_LOG 2026-11-03).
#[test]
fn runs_make_themselves_non_dumpable_on_linux() {
    let sub_of = |name: &str| {
        let start = REMOTE_RUN_PL.find(&format!("\nsub {name} {{")).expect(name) + 1;
        let len = REMOTE_RUN_PL[start..].find('\n').expect("one line") + 1;
        &REMOTE_RUN_PL[start..start + len]
    };
    let script = format!(
        "use strict; use warnings; use Config;\n{}{}{}",
        sub_of("prctl_nr"),
        sub_of("no_dump"),
        r#"my @m = map { prctl_nr($_) // 'none' } 'x86_64-linux-gnu-thread-multi', 'aarch64-linux-gnu-thread-multi', 'x86_64-linux-thread-multi', 'i686-linux-gnu-thread-multi', 'arm-linux-gnueabihf-thread-multi', 'darwin-thread-multi-2level', '';
no_dump();
my $nr = prctl_nr($Config{archname});
my $d = $^O eq 'linux' && defined $nr ? syscall($nr, 3, 0, 0, 0, 0) : 'n/a';
print "@m $d\n";
"#
    );
    let out = std::process::Command::new("/usr/bin/perl")
        .arg("-e")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let want = if cfg!(target_os = "linux") {
        "0"
    } else {
        "n/a"
    };
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("157 167 157 none none none none {want}\n")
    );
    let begin = REMOTE_RUN_PL
        .find("\nBEGIN { no_dump() if ($ARGV[0] // '') =~ /\\A--(?:run|attach)\\z/ }\n")
        .expect("the BEGIN block");
    let first_use = REMOTE_RUN_PL.find("\nuse ").expect("a use");
    assert!(begin < first_use, "BEGIN comes before every module");
    assert!(
        !REMOTE_RUN_PL.contains("  no_dump();"),
        "no later call stands in for it"
    );
}

/// A session's folder names agree with `hollow-slot`'s (`slug_of` in
/// `SLOT_TOOL`), so a session finds its worktree's build folder; a leading
/// dot is `_` in both (2026-11-03), since the host's `*.used` globs skip
/// dot names and such a folder was never pruned.
#[test]
fn folder_names_match_hollow_slots() {
    let line = crate::build_slots::SLOT_TOOL
        .lines()
        .find(|l| l.starts_with("slug_of() "))
        .unwrap();
    for path in [
        "/x/Hollow",
        "/x/.dotted",
        "/x/..two",
        "/x/sp ace+plus",
        "/x/a.b_c-d",
        "/x/a-very-long-folder-name-that-runs-past-forty-characters",
        // Not ASCII: the shell's tr works byte by byte (2026-11-03).
        "/x/caf\u{e9}",
        "/x/\u{65e5}\u{672c}-folder",
        "/x/\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}x",
    ] {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{line}\nslug_of \"$1\""))
            .arg("sh")
            .arg(path)
            .output()
            .unwrap();
        assert!(out.status.success());
        let shell = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        assert_eq!(shell, slug_of(path), "{path}");
        assert!(!shell.starts_with('.'), "{path}");
    }
    assert_eq!(slug_of("/x/caf\u{e9}"), "caf--");
}

/// Q-R7 (2026-11-13): the runner preparation's `status` and `add -A` on
/// this Mac carry the hardened helper's pinned flags, so a planted
/// submodule entry for a decoy with a `core.fsmonitor` payload runs nothing
/// in Hollow's process. Real git.
#[test]
fn the_snapshot_runs_no_planted_submodule_payload() {
    let git = |dir: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let top = root_path.join("repo");
    std::fs::create_dir_all(&top).unwrap();
    git(&top, &["init", "-q", "-b", "main"]);
    std::fs::write(top.join("a"), "a").unwrap();
    git(&top, &["add", "a"]);
    git(&top, &["commit", "-q", "-m", "init"]);
    // A decoy repository with a `core.fsmonitor` payload, and a planted
    // `160000` entry for it in the index.
    let decoy = top.join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    git(&decoy, &["init", "-q", "."]);
    git(&decoy, &["commit", "-q", "--allow-empty", "-m", "d"]);
    let sha = git(&decoy, &["rev-parse", "HEAD"]);
    let ran = root_path.join("RAN");
    git(
        &decoy,
        &[
            "config",
            "core.fsmonitor",
            &format!("touch {}; true", ran.display()),
        ],
    );
    git(
        &top,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sha},decoy"),
        ],
    );
    git(&top, &["status", "--porcelain"]);
    assert!(ran.exists(), "control: plain git status runs the payload");
    std::fs::remove_file(&ran).unwrap();
    let head = git(&top, &["rev-parse", "HEAD"]);
    let common = git(
        &top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let scratch = tempfile::tempdir().unwrap();
    let (_, clean, _) = snapshot(&top, &common, &head, scratch.path(), None).unwrap();
    assert!(!clean, "the planted entry is a change to send");
    assert!(!ran.exists(), "the snapshot ran the planted fsmonitor");
}
