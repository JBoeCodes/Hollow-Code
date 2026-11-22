//! The runner's state (overnight N1/N2). A stand-in `ssh` plays the runner:
//! it runs the probe with `zsh` in a fake home with only a login's
//! variables, and fails like ssh for the host names `down` (timed out),
//! `nodns` (no address) and `refused`; `hang` never answers.

use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const FAKE_SSH: &str = r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o | -p | -i | -l) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
host="$1"
shift
case "$host" in
  down) echo "ssh: connect to host down port 22: Operation timed out" >&2; exit 255 ;;
  nodns) echo "ssh: Could not resolve hostname nodns: nodename nor servname provided, or not known" >&2; exit 255 ;;
  refused) echo "ssh: connect to host refused port 22: Connection refused" >&2; exit 255 ;;
  hang) exec /bin/sleep 60 ;;
  broken) echo "zsh:1: no such file or directory: /usr/bin/perl" >&2; exit 3 ;;
esac
cd "$FAKE_HOME" || exit 255
exec env -i HOME="$FAKE_HOME" PATH=/usr/bin:/bin:/usr/sbin:/sbin /bin/zsh -f -c "$*"
"#;

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::Builder::new()
            .prefix("rst")
            .tempdir_in("/tmp")
            .unwrap();
        let rig = Rig { dir };
        std::fs::create_dir_all(rig.home()).unwrap();
        rig.exe(
            "bin/fake-ssh",
            &FAKE_SSH.replace("$FAKE_HOME", &rig.home().to_string_lossy()),
        );
        rig
    }
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }
    fn ssh(&self) -> String {
        self.dir
            .path()
            .join("bin/fake-ssh")
            .to_string_lossy()
            .into_owned()
    }
    fn exe(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
    fn write(&self, rel: &str, body: &str) {
        let p = self.home().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    fn preflight(&self, body: &str) {
        self.exe("home/.local/bin/hollow-run-preflight", body);
    }
}

fn gate_record(start: &str, checkout: &str, ok: bool) -> String {
    serde_json::json!({
        "schema": 1, "commit": "4c5517b304bb29619c90acd925a00b13539e4e37", "host": "fake",
        "checkout": checkout, "start": start, "end": "2026-11-02T07:09:55.432Z",
        "seconds": 150.72, "exitCode": if ok { 0 } else { 1 }, "ok": ok,
        "failedStep": if ok { serde_json::Value::Null } else { "cargo test".into() },
        "steps": [], "durations": {},
    })
    .to_string()
}

/// A process whose command line names `hollow-remote-run`, as a run's
/// supervisor's does; killed when dropped.
struct Supervisor(std::process::Child);
impl Supervisor {
    fn start(dir: &Path) -> Supervisor {
        let script = dir.join("hollow-remote-run");
        std::fs::write(&script, "sleep 60;\n").unwrap();
        Supervisor(
            std::process::Command::new("/usr/bin/perl")
                .arg(&script)
                .spawn()
                .unwrap(),
        )
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A runner that answers: its machine name, preflight line, the sessions
/// whose supervisor is alive (a dead one and a stray folder don't count)
/// and its newest gate run from any repository, named by its checkout.
#[test]
fn a_ready_runner_reports_its_preflight_sessions_and_last_gate() {
    let rig = Rig::new();
    rig.preflight("#!/bin/sh\necho checking\necho 'ok host=fake free=200GB mem=80% rust=1.96.0'\n");
    let live = Supervisor::start(rig.dir.path());
    rig.write(
        "hollow-remote/run/0123456789ab/pid",
        &format!("{}\n", live.0.id()),
    );
    rig.write("hollow-remote/run/ba9876543210/pid", "999999\n");
    rig.write(
        "hollow-remote/run/not-a-run/pid",
        &format!("{}\n", live.0.id()),
    );
    rig.write(
        "hollow-remote/repos/a.git/hollow-gate-runs/2026-11-02T06-59-14-711Z-5ac5c0ef.json",
        &gate_record(
            "2026-11-02T06:59:14.711Z",
            "/h/hollow-remote/work/ri27-mm4-4962a111ae86",
            false,
        ),
    );
    rig.write(
        "hollow-remote/repos/b.git/hollow-gate-runs/2026-11-02T07-07-24-710Z-58427102.json",
        &gate_record(
            "2026-11-02T07:07:24.710Z",
            "/h/hollow-remote/work/ri27-mm5-50734582c77e",
            true,
        ),
    );
    rig.write(
        "hollow-remote/repos/b.git/hollow-gate-runs/logs/zzzz.json",
        "{}",
    );

    let status = check(&rig.ssh(), "fake");
    assert_eq!(status.state, "ready", "{status:?}");
    assert_eq!(status.host.as_deref(), Some("fake"));
    assert!(status.message.is_none(), "{status:?}");
    assert_eq!(
        status.preflight.as_deref(),
        Some("ok host=fake free=200GB mem=80% rust=1.96.0")
    );
    assert!(status.machine.as_deref().is_some_and(|m| !m.is_empty()));
    assert_eq!(status.running, Some(1));
    // (gate_running isn't asserted: a gate running these tests on this
    // machine is a gate-run.mjs process of this account, and counts.)
    assert!(status.checked_at.is_some() && status.millis.is_some());
    let gate = status.last_gate.expect("the newest gate");
    assert_eq!(gate.start, "2026-11-02T07:07:24.710Z");
    assert_eq!(gate.checkout.as_deref(), Some("ri27-mm5"));
    assert_eq!(gate.ok, Some(true));
    assert_eq!(gate.exit_code, Some(0));
    assert_eq!(gate.seconds, Some(150.72));
    assert_eq!(gate.failed_step, None);
    assert_eq!(
        gate.commit.as_deref(),
        Some("4c5517b304bb29619c90acd925a00b13539e4e37")
    );
}

/// A gate running there (`node scripts/gate-run.mjs`, as `npm run gate`
/// starts it) counts; the pattern wants it at the start of a command line.
#[test]
fn a_gate_running_there_is_seen() {
    let rig = Rig::new();
    let mut gate = std::process::Command::new("/bin/bash")
        .args([
            "-c",
            "exec -a 'node scripts/gate-run.mjs npm run gate:all' /bin/sleep 30",
        ])
        .spawn()
        .unwrap();
    // Until bash has exec'd (its own command line doesn't count), 5 s at most.
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "command=", "-p", &gate.id().to_string()])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&out.stdout).starts_with("node scripts/gate-run.mjs") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = check(&rig.ssh(), "fake");
    let _ = gate.kill();
    let _ = gate.wait();
    assert_eq!(status.state, "ready", "{status:?}");
    assert!(status.gate_running, "{status:?}");
}

/// A preflight that says no: `refusing`, with its own line; a runner that
/// has never run anything has no sessions and no gate.
#[test]
fn a_refusing_preflight_is_reported_with_its_line() {
    let rig = Rig::new();
    rig.preflight("#!/bin/sh\necho 'no: free=12GB, under 20GB'\nexit 1\n");
    let status = check(&rig.ssh(), "fake");
    assert_eq!(status.state, "refusing", "{status:?}");
    assert_eq!(
        status.preflight.as_deref(),
        Some("no: free=12GB, under 20GB")
    );
    let message = status.message.unwrap();
    assert!(
        message.contains("isn't taking work now (no: free=12GB, under 20GB)"),
        "{message}"
    );
    assert_eq!(status.running, Some(0));
    assert_eq!(status.last_gate, None);
}

/// No preflight there: it answers, so it is ready, with no line to show.
#[test]
fn a_runner_without_a_preflight_is_ready() {
    let rig = Rig::new();
    let status = check(&rig.ssh(), "fake");
    assert_eq!(status.state, "ready", "{status:?}");
    assert_eq!(status.preflight, None);
}

/// ssh can't reach it: `unreachable`, with ssh's error and what to check
/// for that error (the FileVault screen after a restart, Tailscale when the
/// name doesn't resolve).
/// Remote runners 2: the probe reads the owner record and fingerprints the
/// keys the runner account lets in (both authorized_keys files, comments
/// and blank lines aside), and the check judges them against this Hollow.
#[test]
fn a_runner_reports_its_owner_and_the_keys_it_lets_in() {
    let rig = Rig::new();
    let keys = rig.dir.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let mut lines = vec!["# the owner".to_string(), String::new()];
    let mut fps = Vec::new();
    for name in ["one", "two"] {
        let key = keys.join(name);
        assert!(std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
            .arg(&key)
            .status()
            .unwrap()
            .success());
        let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        lines.push(public.trim().to_string());
        let out = std::process::Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(key.with_extension("pub"))
            .output()
            .unwrap();
        fps.push(
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string(),
        );
    }
    rig.write(".ssh/authorized_keys", &(lines[..3].join("\n") + "\n"));
    rig.write(".ssh/authorized_keys2", &format!("no-pty {}\n", lines[3]));
    let me = crate::runner_owner::install_id().unwrap();
    rig.write(
        "hollow-remote/owner.json",
        &serde_json::json!({ "v": 1, "installId": me, "key": fps[0], "at": "2026-11-03T19:00:00Z" })
            .to_string(),
    );
    let status = check(&rig.ssh(), "fake");
    assert_eq!(status.state, "ready", "{status:?}");
    let owner = status.owner.expect("an owner view");
    assert_eq!(owner.state, "you");
    assert_eq!(owner.recorded_at.as_deref(), Some("2026-11-03T19:00:00Z"));
    assert_eq!((owner.keys, owner.other_keys), (Some(2), Some(1)));
    assert!(
        owner.warnings[0].starts_with("1 other SSH key can log in"),
        "{:?}",
        owner.warnings
    );

    // No record and no keys file: nothing recorded, and no keys read is
    // said, not counted as zero.
    let bare = Rig::new();
    let none = check(&bare.ssh(), "fake").owner.expect("an owner view");
    assert_eq!(none.state, "none");
    assert_eq!((none.keys, none.other_keys), (None, None));
    assert!(
        none.warnings[0].starts_with("No key could be read"),
        "{:?}",
        none.warnings
    );
}

#[test]
fn an_unreachable_runner_says_what_to_check() {
    let rig = Rig::new();
    let down = check(&rig.ssh(), "down");
    assert_eq!(down.state, "unreachable");
    let message = down.message.unwrap();
    assert!(message.contains("Operation timed out"), "{message}");
    assert!(message.contains("FileVault unlock screen"), "{message}");
    let nodns = check(&rig.ssh(), "nodns").message.unwrap();
    assert!(nodns.contains("Tailscale"), "{nodns}");
    let refused = check(&rig.ssh(), "refused").message.unwrap();
    assert!(refused.contains("Remote Login"), "{refused}");
    let bad = check(&rig.ssh(), "-oProxyCommand=x");
    assert_eq!(bad.state, "error");
    assert!(bad.message.unwrap().contains("not an SSH host name"));
}

/// A check that never finishes is cut off at its limit: ssh is killed, and
/// the answer says it stalled and what to check.
#[test]
fn a_check_that_hangs_is_cut_off() {
    let rig = Rig::new();
    let started = std::time::Instant::now();
    assert!(matches!(
        run_bounded(&rig.ssh(), "hang", "true", Duration::from_secs(1)),
        Err(Bounded::Stalled)
    ));
    let status = check_within(&rig.ssh(), "hang", Duration::from_secs(1));
    assert_eq!(status.state, "unreachable", "{status:?}");
    let message = status.message.unwrap();
    assert!(
        message.contains("didn't finish the check in 1 s"),
        "{message}"
    );
    assert!(message.contains("FileVault"), "{message}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// The runner runs branch code, so a gate record with fields of the wrong
/// type loses those fields only, and long text is cut; the rest of the
/// answer stands.
#[test]
fn a_malformed_gate_record_loses_only_its_bad_fields() {
    let rig = Rig::new();
    rig.preflight(&format!(
        "#!/bin/sh\necho 'ok free=200GB {}'\n",
        "x".repeat(500)
    ));
    rig.write(
        "hollow-remote/repos/a.git/hollow-gate-runs/2026-11-02T07-07-24-710Z-58427102.json",
        &serde_json::json!({
            "start": "2026-11-02T07:07:24.710Z", "ok": 1, "seconds": "150",
            "commit": { "x": 1 }, "exitCode": 0, "failedStep": "s\u{e9}t\nnext", "checkout": ["no"],
        })
        .to_string(),
    );
    let status = check(&rig.ssh(), "fake");
    assert_eq!(status.state, "ready", "{status:?}");
    let preflight = status.preflight.unwrap();
    assert!(
        preflight.starts_with("ok free=200GB xxx") && preflight.chars().count() <= FIELD_MAX + 1,
        "{preflight}"
    );
    let gate = status.last_gate.expect("the record, minus its bad fields");
    assert_eq!(gate.start, "2026-11-02T07:07:24.710Z");
    assert_eq!(
        (gate.ok, gate.seconds, gate.commit, gate.checkout),
        (None, None, None, None)
    );
    assert_eq!(gate.exit_code, Some(0));
    assert_eq!(gate.failed_step.as_deref(), Some("s\u{e9}t next"));
}

/// It answered, but its check couldn't run there (no perl): `error`, with
/// what it said.
#[test]
fn a_check_that_fails_there_says_what_it_said() {
    let rig = Rig::new();
    let status = check(&rig.ssh(), "broken");
    assert_eq!(status.state, "error", "{status:?}");
    let message = status.message.unwrap();
    assert!(
        message.contains(
            "answered, but its check failed (zsh:1: no such file or directory: /usr/bin/perl)"
        ),
        "{message}"
    );
}

/// Each kind of ssh failure names its own likely cause; anything else gets
/// the general one, which names the FileVault screen after a restart.
#[test]
fn likely_causes_follow_the_ssh_error() {
    let cases = [
        ("ssh: Could not resolve hostname runner-host: nodename nor servname provided, or not known", "Tailscale"),
        ("ssh: connect to host x port 22: Connection refused", "Remote Login"),
        ("runner@x: Permission denied (publickey).", "authorized_keys"),
        ("Host key verification failed.", "known_hosts"),
        ("Too many authentication failures", "authorized_keys"),
        ("ssh: Could not resolve hostname x: Name or service not known", "Tailscale"),
        ("client_loop: send disconnect: Broken pipe", "failed logins"),
        ("Connection closed by 100.64.0.2 port 22", "failed logins"),
        ("kex_exchange_identification: read: Connection reset by peer", "failed logins"),
        ("ssh: connect to host x port 22: Operation timed out", "FileVault"),
        ("ssh: connect to host x port 22: No route to host", "FileVault"),
        ("", "FileVault"),
    ];
    for (error, expected) in cases {
        let cause = likely_causes(error);
        assert!(cause.contains(expected), "{error:?} -> {cause}");
    }
    // A turn's refusal carries the same cause.
    let turn = crate::remote_session::unreachable_message(
        "runner-host",
        "ssh: connect to host x port 22: Connection refused",
    );
    assert!(
        turn.contains("Remote Login") && turn.contains("Runs on to This Mac"),
        "{turn}"
    );
    let dots =
        crate::remote_session::unreachable_message("runner-host", "Host key verification failed.");
    assert!(
        dots.contains("failed. Nothing ran") && !dots.contains(".."),
        "{dots}"
    );
}

/// Overnight N6: a check under five minutes old that found the runner down
/// or refusing keeps unattended work here at once, saying when and why; a
/// ready, older, unchecked or other host's answer doesn't.
#[test]
fn a_recent_failed_check_keeps_unattended_work_here() {
    let rig = Rig::new();
    let down = check(&rig.ssh(), "refused");
    assert_eq!(down.state, "unreachable");
    let at = down.finished.unwrap();
    let within = Duration::from_secs(300);
    let why = unavailable_in(&down, "refused", within, later(at, 130, 130)).unwrap();
    assert!(
        why.starts_with("When checked 2 minutes ago, refused didn't answer: ssh: connect")
            && why.contains("Remote Login")
            && why.ends_with('.'),
        "{why}"
    );
    assert!(unavailable_in(&down, "refused", within, later(at, 301, 301)).is_none());
    assert!(unavailable_in(&down, "runner-host", within, at).is_none());
    assert!(unavailable_in(&down, "refused", within, at)
        .unwrap()
        .contains("under a minute ago"));
    rig.preflight("#!/bin/sh\necho 'no: free=12GB, under 20GB'\nexit 1\n");
    let refusing = check(&rig.ssh(), "fake");
    assert_eq!(refusing.state, "refusing", "{refusing:?}");
    let why = unavailable_in(&refusing, "fake", within, refusing.finished.unwrap()).unwrap();
    assert_eq!(
        why,
        "When checked under a minute ago, it wasn't taking work (no: free=12GB, under 20GB)."
    );
    let unchecked = RunnerStatus::new(Some("fake"), "unchecked");
    assert!(unavailable_in(&unchecked, "fake", within, Stamp::now()).is_none());
    let mut ready = refusing.clone();
    ready.state = "ready";
    assert!(unavailable_in(&ready, "fake", within, ready.finished.unwrap()).is_none());
}

/// Overnight N6: a preparation's ssh failing is kept as the runner's last
/// status, as a check would have found it, so the next unattended turn runs
/// here at once and Settings shows it (`note_failure` stores exactly this
/// in the process-wide last status, which other tests' ssh calls also write,
/// so the stored copy isn't read back here).
#[test]
fn a_failed_preparation_is_the_runners_last_status() {
    let host = "it-n6-note";
    let within = Duration::from_secs(300);
    // What the last check of this host found stays: the machine, its
    // sessions and its last gate (review r2: Settings said no gate had run).
    let mut before = RunnerStatus::new(Some(host), "ready");
    before.machine = Some("build-mac".into());
    before.running = Some(1);
    before.last_gate = Some(RunnerGate {
        start: "2026-11-02T04:00:00Z".into(),
        ok: Some(true),
        ..Default::default()
    });
    let down = failure_status(
        Some(&before),
        host,
        "unreachable",
        "Warning: x\nssh: connect to host it-n6-note port 22: Connection refused",
    );
    assert_eq!(down.state, "unreachable");
    assert!(down.noted && down.checked_at.is_some());
    assert_eq!(down.machine.as_deref(), Some("build-mac"));
    assert_eq!(down.running, Some(1));
    assert_eq!(down.last_gate, before.last_gate);
    // Another host's check carries nothing over.
    let other = failure_status(Some(&before), "it-n6-else", "unreachable", "ssh: x");
    assert_eq!((other.machine, other.last_gate), (None, None));
    let why = unavailable_in(&down, host, within, down.finished.unwrap()).unwrap();
    assert!(
        why.starts_with("When checked under a minute ago, it-n6-note didn't answer: ssh: connect")
            && why.contains("Remote Login"),
        "{why}"
    );
    let refusing = failure_status(None, host, "refusing", "no: free=12GB, under 20GB.\n");
    assert_eq!(
        refusing.preflight.as_deref(),
        Some("no: free=12GB, under 20GB")
    );
    assert_eq!(
        unavailable_in(&refusing, host, within, refusing.finished.unwrap()).unwrap(),
        "When checked under a minute ago, it wasn't taking work (no: free=12GB, under 20GB)."
    );
    assert!(unavailable_in(&refusing, "it-n6-other", within, refusing.finished.unwrap()).is_none());
}

/// `at` moved on by `mono` seconds of the monotonic clock (which stops
/// while the Mac sleeps) and `wall` seconds of the wall clock.
fn later(at: Stamp, mono: u64, wall: u64) -> Stamp {
    Stamp {
        mono: at.mono + Duration::from_secs(mono),
        wall: at.wall + Duration::from_secs(wall),
    }
}

/// Lane S9 (2026-11-03): a failure is recent only while both clocks say so,
/// and its age is the wall clock's. macOS's `Instant` stops during sleep, so
/// one found just before the lid closed read "under a minute ago" minutes
/// after wake and kept night work here.
#[test]
fn a_failure_is_recent_only_by_both_clocks() {
    let within = Duration::from_secs(300);
    let mut down = failure_status(
        None,
        "it-s9-clock",
        "unreachable",
        "ssh: Operation timed out",
    );
    let at = down.finished.unwrap();
    let said =
        |status: &RunnerStatus, now: Stamp| unavailable_in(status, "it-s9-clock", within, now);
    let why = |now: Stamp| said(&down, now);
    // Awake throughout: as before.
    assert!(why(later(at, 130, 130))
        .unwrap()
        .starts_with("When checked 2 minutes ago, it-s9-clock didn't answer"));
    // Slept past the window: 20 s awake, 20 minutes by the wall clock.
    assert_eq!(why(later(at, 20, 1200)), None);
    // Slept 4 minutes of the 5: still recent, worded from the wall clock.
    assert!(why(later(at, 10, 250))
        .unwrap()
        .starts_with("When checked 4 minutes ago,"));
    // Awake past the window while the wall clock was set back to just after.
    assert_eq!(why(later(at, 400, 10)), None);
    // The wall clock set back before the failure: its age is unknown, so it
    // isn't taken as recent (the next turn asks the runner itself).
    let back = Stamp {
        mono: at.mono + Duration::from_secs(10),
        wall: at.wall - Duration::from_secs(60),
    };
    assert_eq!(why(back), None);
    // A refusal the same way.
    down.state = "refusing";
    assert_eq!(said(&down, later(at, 20, 1200)), None);
    assert!(said(&down, later(at, 20, 70))
        .unwrap()
        .starts_with("When checked a minute ago, it wasn't taking work"));
}

/// Lane S9 (2026-11-03): any ssh call that gets through ends an unreachable
/// status, a check's as well as a noted one (Settings said Not answering
/// while a turn ran there); a refusal ends only when a preparation passes the
/// preflight, not with an unrelated call. `ended_by` is what `note_answered`
/// and `note_prepared` apply to the process-wide last status, which other
/// tests' ssh calls also write, so it is tested on its own.
#[test]
fn only_what_a_call_disproves_is_cleared() {
    let host = "it-s9-clear";
    let checked = |state: &'static str| RunnerStatus::new(Some(host), state);
    let noted = |state: &'static str| failure_status(None, host, state, "no: free=12GB");
    let ended = |status: &RunnerStatus, host: &str, prepared: bool| {
        ended_by(status, host, prepared, Instant::now())
    };
    for status in [checked("unreachable"), noted("unreachable")] {
        assert!(ended(&status, host, false), "{status:?}");
        assert!(ended(&status, host, true), "{status:?}");
        assert!(!ended(&status, "it-s9-other", true), "{status:?}");
    }
    for status in [checked("refusing"), noted("refusing")] {
        assert!(!ended(&status, host, false), "{status:?}");
        assert!(ended(&status, host, true), "{status:?}");
        assert!(!ended(&status, "it-s9-other", true), "{status:?}");
    }
    for state in ["ready", "error", "unchecked"] {
        assert!(!ended(&checked(state), host, false), "{state}");
        assert!(!ended(&checked(state), host, true), "{state}");
    }
}

/// Lane S10 (2026-11-03): a call ends only what was found by the time it
/// began. A refusal a check found while a preparation ran is newer than
/// that preparation's preflight, so it stays.
#[test]
fn a_call_ends_only_what_was_found_before_it_began() {
    let host = "it-s10-began";
    for state in ["unreachable", "refusing"] {
        let status = failure_status(None, host, state, "no: free=12GB");
        let at = status.finished.unwrap().mono;
        let second = Duration::from_secs(1);
        assert!(ended_by(&status, host, true, at), "{state}");
        assert!(ended_by(&status, host, true, at + second), "{state}");
        assert!(!ended_by(&status, host, true, at - second), "{state}");
    }
}

/// Lane S10 (2026-11-03): a check's answer becomes the last status only
/// when nothing newer came while it ran (up to 30 s): no other status was
/// kept or cleared, no call got through (for an unreachable answer) and no
/// preparation passed (for a refusing one). Both orders, with set times.
#[test]
fn a_check_keeps_its_answer_only_when_nothing_newer_came() {
    let host = "it-s10-slot";
    let began = Instant::now();
    let (before, after) = (
        began - Duration::from_secs(5),
        began + Duration::from_secs(5),
    );
    let slot = |changed: Option<Instant>,
                answered: Option<(&str, Instant)>,
                prepared: Option<(&str, Instant)>| Slot {
        status: None,
        changed,
        answered: answered.map(|(h, at)| (h.to_string(), at)),
        prepared: prepared.map(|(h, at)| (h.to_string(), at)),
    };
    let all = ["ready", "refusing", "unreachable", "error"];
    // Nothing since it began: any answer is kept.
    let quiet = slot(Some(before), Some((host, before)), Some((host, before)));
    for state in all {
        assert!(quiet.takes_check(host, state, began), "{state}");
    }
    // The status was kept or cleared since: none is.
    let changed = slot(Some(after), None, None);
    for state in all {
        assert!(!changed.takes_check(host, state, began), "{state}");
    }
    // A preparation passed since: its refusal and unreachable are stale.
    let prepared = slot(Some(before), Some((host, after)), Some((host, after)));
    assert!(!prepared.takes_check(host, "refusing", began));
    assert!(!prepared.takes_check(host, "unreachable", began));
    assert!(prepared.takes_check(host, "ready", began));
    // An unrelated call got through since: only unreachable is stale.
    let answered = slot(None, Some((host, after)), None);
    assert!(!answered.takes_check(host, "unreachable", began));
    assert!(answered.takes_check(host, "refusing", began));
    // Calls to another host say nothing about this one.
    let other = slot(
        None,
        Some(("it-s10-other", after)),
        Some(("it-s10-other", after)),
    );
    for state in all {
        assert!(other.takes_check(host, state, began), "{state}");
    }
}

/// A Linux runner (2026-11-03): the probe says which system it runs (perl's
/// `$^O`, so `darwin` here), kept as `os`, read leniently and clipped like
/// any field; an answer without one (an older probe) has none, and a noted
/// failure keeps the last check's.
#[test]
fn the_runners_system_is_reported() {
    let rig = Rig::new();
    let status = check(&rig.ssh(), "fakehost");
    assert_eq!(status.state, "ready", "{status:?}");
    let want = if cfg!(target_os = "linux") {
        "linux"
    } else {
        "darwin"
    };
    assert_eq!(status.os.as_deref(), Some(want), "{status:?}");

    let answer = rig.dir.path().join("answer.json");
    let canned = rig.exe(
        "bin/canned-ssh",
        &format!("#!/bin/sh\ncat '{}'\n", answer.display()),
    );
    let canned = canned.to_string_lossy().into_owned();
    let os_of = |json: &str| {
        std::fs::write(&answer, format!("{json}\n")).unwrap();
        let status = check(&canned, "fakehost");
        assert_eq!(status.state, "ready", "{json}: {status:?}");
        status
    };
    let linux = os_of(r#"{"machine":"box","os":"linux","running":0}"#);
    assert_eq!(linux.machine.as_deref(), Some("box"));
    assert_eq!(linux.os.as_deref(), Some("linux"));
    assert_eq!(serde_json::to_value(&linux).unwrap()["os"], "linux");
    assert_eq!(os_of(r#"{"machine":"box","os":7}"#).os, None);
    assert_eq!(os_of(r#"{"machine":"box"}"#).os, None);
    let long = os_of(&format!(r#"{{"os":"{}"}}"#, "x".repeat(400)))
        .os
        .unwrap();
    assert_eq!(
        long.chars().count(),
        FIELD_MAX + 1,
        "clipped, with an ellipsis"
    );

    let down = failure_status(
        Some(&linux),
        "fakehost",
        "unreachable",
        "ssh: connect to host fakehost port 22: Operation timed out",
    );
    assert_eq!(down.os.as_deref(), Some("linux"));
}

/// The real runner (`HOLLOW_REMOTE_IT_HOST=runner-host`, `--ignored`): it
/// answers through the person's own ssh, with its preflight line.
#[test]
#[ignore = "needs a real runner over SSH; HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib remote_status::tests::real_runner -- --ignored"]
fn real_runner_status() {
    let Ok(host) = std::env::var("HOLLOW_REMOTE_IT_HOST") else {
        return;
    };
    let status = check("ssh", &host);
    eprintln!("{}", serde_json::to_string_pretty(&status).unwrap());
    assert_eq!(status.state, "ready", "{status:?}");
    assert!(status.preflight.is_some(), "{status:?}");
    assert!(status.machine.is_some(), "{status:?}");
}
