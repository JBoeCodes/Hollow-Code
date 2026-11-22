//! The runner COO (M3b): its role, its record, its command, and a wake and a
//! follow-up queued on a headless core.

use super::*;
use crate::commands::AppState;
use crate::core_host::Core;
use crate::descendants::tests::{child_turn, record_child};
use crate::run_queue::RowStatus;
use crate::serve::ServeHost;
use chrono::{Duration, Utc};

fn state_in(dir: &Path) -> AppState {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.to_path_buf();
    config.claude_projects_path = dir.join("no-claude-projects");
    AppState::new(config, dir.join("config.json"))
}

/// The roles' table: only the app checks in; every role wakes its COO
/// (`main_agent::coo_pass`).
#[test]
fn each_role_fires_its_own_timers() {
    assert_eq!(role_for(false, false), Role::Single);
    assert_eq!(
        role_for(false, true),
        Role::Single,
        "the app never reads the record"
    );
    assert_eq!(role_for(true, true), Role::Runner);
    assert_eq!(role_for(true, false), Role::Off);
    assert!(Role::Single.check_ins());
    assert!(!Role::Runner.check_ins() && !Role::Off.check_ins());
    assert_eq!(
        [Role::Single, Role::Runner, Role::Off].map(Role::as_str),
        ["single", "runner", "off"]
    );
}

/// No runner: this process is the app (tests are), its single COO wakes and
/// checks in as before, its prompt's schedule line is byte-identical, and
/// its status keeps the saved check-ins.
#[test]
fn with_no_runner_the_single_coo_is_unchanged() {
    assert_eq!(role(), Role::Single);
    assert_eq!(schedule_note(), crate::main_agent::SCHEDULE_NOTE);
    let dir = tempfile::tempdir().unwrap();
    let saved = crate::main_agent::settings().check_ins;
    let now = Utc::now();
    let single = crate::main_agent::status_for(dir.path(), now, Role::Single);
    assert_eq!(single, crate::main_agent::status(dir.path(), now));
    assert_eq!(single.check_ins, saved);
    let runner = crate::main_agent::status_for(dir.path(), now, Role::Runner);
    assert!(!runner.check_ins && runner.next_check_in.is_none());
}

/// The runner's schedule line fits the COO prompt's budget and promises no
/// check-ins or cards.
#[test]
fn the_serve_schedule_lines_fit_and_promise_no_check_ins() {
    // Each role's own line; the app's is the app's.
    assert_ne!(
        schedule_note_for(Role::Off),
        crate::main_agent::SCHEDULE_NOTE
    );
    assert_ne!(
        schedule_note_for(Role::Off),
        schedule_note_for(Role::Runner)
    );
    assert!(schedule_note_for(Role::Runner).contains("you are the runner COO"));
    assert!(schedule_note_for(Role::Off).contains("not the runner"));
    for note in [
        schedule_note_for(Role::Runner),
        schedule_note_for(Role::Off),
    ] {
        check_serve_note(note);
    }
}

fn check_serve_note(note: &str) {
    assert!(note.len() <= SERVE_NOTE_MAX, "{}", note.len());
    assert!(note.starts_with("How you are run:"));
    for says in [
        "no check-ins",
        "sessions.wake_me",
        "nobody sees cards",
        "cost cap",
    ] {
        assert!(note.contains(says), "{says}");
    }
    assert!(!note.contains('\u{2014}') && !note.contains('\u{2013}'));
}

/// Absent is off; the record round-trips at 0600; one Hollow can't read is
/// an error, and serve reads it as off.
#[test]
fn the_record_is_off_unless_it_says_on() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read(dir.path()).unwrap(), Record::default());
    write(dir.path(), true, Utc::now()).unwrap();
    let record = read(dir.path()).unwrap();
    assert!(record.on && record.changed_at.is_some());
    let mode = std::fs::metadata(path_in(dir.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    write(dir.path(), false, Utc::now()).unwrap();
    assert!(!read(dir.path()).unwrap().on);
    std::fs::write(path_in(dir.path()), "on").unwrap();
    assert!(read(dir.path())
        .unwrap_err()
        .contains("not a runner COO record"));
}

/// `hollow serve runner-coo on|off|status`: parsed, and with no serve
/// running, saved on disk and reported.
#[test]
fn the_command_turns_it_on_and_off_and_reports() {
    use crate::serve_control::{parse, runner_line, runner_offline, Answer, Request};
    let words = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(crate::serve_control::COMMANDS.contains(&"runner-coo"));
    for (word, on) in [("on", Some(true)), ("off", Some(false)), ("status", None)] {
        let parsed = parse("runner-coo", &words(&[word])).unwrap();
        assert_eq!(
            parsed.what,
            crate::serve_control::What::Phone(Request::RunnerCoo { on }),
            "{word}"
        );
    }
    for bad in [&[][..], &["maybe"][..], &["on", "off"][..]] {
        assert!(parse("runner-coo", &words(bad)).is_err(), "{bad:?}");
    }
    let dir = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    runner_offline(dir.path(), Some(true), &mut out).unwrap();
    runner_offline(dir.path(), None, &mut out).unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains("The runner COO is on; hollow serve runs it when it starts."),
        "{said}"
    );
    assert!(
        said.contains("Runner COO: on (hollow serve is not running here)."),
        "{said}"
    );
    assert!(read(dir.path()).unwrap().on);
    // The running serve's answer, as the terminal prints it.
    let answer = |role: &str, key: Option<&str>| Answer {
        ok: true,
        role: Some(role.into()),
        chat_key: key.map(str::to_string),
        ..Answer::default()
    };
    assert_eq!(
        runner_line(&answer("runner", Some("s-1"))),
        "Runner COO: on, chat s-1."
    );
    assert!(runner_line(&answer("runner", None)).contains("no thread yet"));
    assert_eq!(runner_line(&answer("off", None)), "Runner COO: off.");
    // The wire shape stays one JSON line with a `cmd` tag.
    let line = serde_json::to_string(&Request::RunnerCoo { on: Some(true) }).unwrap();
    assert_eq!(line, r#"{"cmd":"runner-coo","on":true}"#);
}

/// Turning the role on prepares the COO's workspace, and the log line names
/// its chat once there is one.
#[test]
fn the_runner_gets_its_workspace_and_chat_key() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    assert!(crate::workspaces::main_agent_cwd(v).is_none());
    assert!(line(Role::Runner, v).contains("no thread yet"));
    let cwd = crate::workspaces::main_agent_cwd(v).expect("prepared");
    crate::session_store::start_turn(v, "s-runner-coo", "claude", &cwd, "R-rc-1", "hi").unwrap();
    assert_eq!(chat_key(v).as_deref(), Some("s-runner-coo"));
    assert_eq!(
        line(Role::Runner, v),
        "[serve] runner COO: on, chat s-runner-coo"
    );
    assert!(line(Role::Off, v).contains("runner-coo on"));
    // Not synced: no warning. A git checkout: the runner says so loudly.
    assert_eq!(synced_hint(v), None);
    std::fs::create_dir(v.join(".git")).unwrap();
    assert_eq!(synced_hint(v), Some("a git checkout"));
    let warned = line(Role::Runner, v);
    assert!(
        warned.contains("WARNING") && warned.contains("looks synced"),
        "{warned}"
    );
    assert!(!line(Role::Off, v).contains("WARNING"));
    assert_eq!(
        synced_hint(Path::new(
            "/Users/x/Library/Mobile Documents/com~apple~CloudDocs/v"
        )),
        Some("in iCloud Drive")
    );
}

/// The COO's thread in `v` with one ended chat it started: what a wake
/// reports.
fn coo_with_an_ended_child(v: &Path) -> (String, i64) {
    crate::workspaces::ensure_main_agent(v).unwrap();
    let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
    let parent = "s-runner-parent";
    crate::session_store::start_turn(v, parent, "claude", &cwd, "R-rc-main", "start a chat")
        .unwrap();
    let done = child_turn(v, "s-runner-child", "R-rc-child", RowStatus::Done);
    record_child(v, "R-rc-main", "s-runner-child", done);
    (parent.to_string(), done)
}

fn queued_for(v: &Path, key: &str) -> Vec<String> {
    crate::run_queue::live_rows(v)
        .unwrap()
        .into_iter()
        .filter(|r| r.source_id.as_deref() == Some(key))
        .map(|r| r.payload["hostAsker"].as_str().unwrap_or("").to_string())
        .collect()
}

/// The acceptance, at code level: on a headless core (serve's host, no
/// window), the runner COO's minute pass queues its wake for a chat it
/// started that ended, and a due `wake_me` follow-up of its thread queues
/// too.
#[tokio::test(flavor = "current_thread")]
async fn a_wake_and_a_follow_up_fire_on_a_headless_core() {
    crate::run_pump::TEST_ACCEPTS.with(|a| a.set(true));
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let (parent, _child) = coo_with_an_ended_child(v);
    let core = Core::new(ServeHost::new(state_in(v), false));
    assert!(core.gui().is_none(), "no window");

    // The runner: the wake, tagged as Hollow's.
    crate::main_agent::coo_pass(&core, Role::Runner).await;
    assert_eq!(
        queued_for(v, &parent),
        vec![crate::main_agent::ASKED_BY_WAKE]
    );
    // Once only: the same ended chat never wakes it twice.
    crate::main_agent::coo_pass(&core, Role::Runner).await;
    assert_eq!(queued_for(v, &parent).len(), 1);

    // Its follow-up: a wake_me the thread asked for, now due.
    let busy: Vec<i64> = crate::run_queue::live_rows(v)
        .unwrap()
        .into_iter()
        .filter(|r| r.source_id.as_deref() == Some(parent.as_str()))
        .map(|r| r.id)
        .collect();
    for id in busy {
        crate::run_queue::cancel(v, id, Utc::now()).ok();
    }
    let caller = crate::app_actions::Caller {
        run_id: "R-rc-main".into(),
        surface: "chat".into(),
        scope: parent.clone(),
        access: crate::app_mcp::AppAccess::Write,
        depth: 0,
        perm: "acceptEdits".into(),
    };
    crate::wake_me::schedule(
        v,
        &caller,
        1,
        "look at the build again",
        Utc::now() - Duration::minutes(2),
    )
    .unwrap();
    let config = core.try_state::<AppState>().unwrap().config.clone();
    crate::wake_me::run_due(&core, &config).await;
    assert_eq!(
        queued_for(v, &parent),
        vec![crate::main_agent::ASKED_BY_WAKE_ME],
        "the follow-up"
    );
    assert_eq!(crate::wake_me::load(v)[0].status, "queued");
}

/// PR 2: the COO's status says its role, and only the runner names its chat
/// for a peer; the machine capabilities gain `coo` only with one.
#[test]
fn the_status_and_capabilities_expose_only_the_runners_chat() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    prepare(v).unwrap();
    let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
    // No thread yet: nothing to reach.
    assert_eq!(peer_coo_for(Role::Runner, v), None);
    crate::session_store::start_turn(v, "s-runner-peer", "claude", &cwd, "R-rp", "hi").unwrap();
    let now = Utc::now();
    let runner = crate::main_agent::status_for(v, now, Role::Runner);
    assert_eq!(runner.role, Role::Runner);
    assert_eq!(runner.runner_chat_key.as_deref(), Some("s-runner-peer"));
    for role in [Role::Single, Role::Off] {
        let other = crate::main_agent::status_for(v, now, role);
        assert_eq!(
            (other.role, other.runner_chat_key),
            (role, None),
            "{role:?}"
        );
    }
    // The app's status (this process): `single`, no key, on the wire too.
    let wire = serde_json::to_value(crate::main_agent::status(v, now)).unwrap();
    assert_eq!(wire["role"], "single");
    assert!(wire["runnerChatKey"].is_null());
    assert_eq!(peer_coo(v), None, "the app is never the runner");

    let base = ["chats", "send", "stop", "cards", "events"];
    assert_eq!(capabilities(&base, None, false), base.to_vec());
    let with = capabilities(&base, Some("s-runner-peer"), true);
    assert_eq!(with.last(), Some(&CAPABILITY));
    assert_eq!(
        capabilities(&with, Some("s-runner-peer"), true),
        with,
        "once"
    );
    // M3a follow-up: the runner with no conversation yet may be started.
    let startable = capabilities(&base, None, true);
    assert_eq!(startable.last(), Some(&START_CAPABILITY));
    assert!(!startable.contains(&CAPABILITY));
}

/// Review medium 1: a serve that isn't the runner still wakes a COO already
/// in flight there when its chat ends (as every serve did before M3b), but
/// never checks in, and a vault with no COO gets none made.
#[tokio::test(flavor = "current_thread")]
async fn a_serve_that_is_not_the_runner_still_wakes_a_coo_in_flight() {
    crate::run_pump::TEST_ACCEPTS.with(|a| a.set(true));
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let (parent, _child) = coo_with_an_ended_child(v);
    let core = Core::new(ServeHost::new(state_in(v), false));
    crate::main_agent::coo_pass(&core, Role::Off).await;
    assert_eq!(
        queued_for(v, &parent),
        vec![crate::main_agent::ASKED_BY_WAKE]
    );
    let rows = crate::run_queue::live_rows(v).unwrap();
    assert!(
        rows.iter()
            .all(|r| r.payload["hostAsker"] != crate::main_agent::ASKED_BY_CHECKIN),
        "no check-in"
    );
    let empty = tempfile::tempdir().unwrap();
    let bare = Core::new(ServeHost::new(state_in(empty.path()), false));
    crate::main_agent::coo_pass(&bare, Role::Off).await;
    assert!(crate::workspaces::main_agent_cwd(empty.path()).is_none());
    assert!(crate::run_queue::live_rows(empty.path())
        .unwrap()
        .is_empty());
}

/// Review low 6: a record serve couldn't read shows in the status line.
#[test]
fn an_unreadable_record_shows_in_the_status() {
    use crate::serve_control::{runner_line, runner_offline, Answer};
    let answer = Answer {
        ok: true,
        role: Some("off".into()),
        record_error: Some("runner-coo.json is not a runner COO record".into()),
        ..Answer::default()
    };
    let said = runner_line(&answer);
    assert!(
        said.starts_with("Runner COO: off, runner-coo.json is not"),
        "{said}"
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(path_in(dir.path()), "on").unwrap();
    let mut out = Vec::new();
    runner_offline(dir.path(), None, &mut out).unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains("not a runner COO record") && said.contains("rewrites it"),
        "{said}"
    );
}

/// Review 2 low 4: a failed hardware UUID read is not cached; the next call
/// reads again, and once read the id is kept.
#[test]
fn a_failed_machine_id_read_is_asked_again() {
    let cell = std::sync::OnceLock::new();
    assert_eq!(owner_from(&cell, || None), None);
    assert!(cell.get().is_none(), "None is never cached");
    let id = owner_from(&cell, || Some([7u8; 16])).unwrap();
    assert!(id.starts_with("m-") && id.len() == 18, "{id}");
    let again = owner_from(&cell, || panic!("not read again once known"));
    assert_eq!(again.as_deref(), Some(id.as_str()));
    assert_ne!(
        owner_from(&std::sync::OnceLock::new(), || Some([8u8; 16])),
        Some(id)
    );
    assert!(machine_owner().is_some(), "this Mac has one");
}
