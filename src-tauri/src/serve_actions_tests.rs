//! Hollow app tools in `hollow serve` (M1 PR 5): serve's own host runs a
//! read and a write, refuses every card, and says plainly what needs the
//! window. Each test builds serve's host the way `serve.rs` does and calls
//! the broker's dispatch as an admitted request would.

use super::*;
use crate::app_broker::{proof_for, AppRequest, AUDIT_FILE};
use crate::app_mcp::{AppAccess, AppGrant};
use crate::core_host::Core;
use crate::serve::ServeHost;
use serde_json::{json, Map};
use std::path::Path;

fn state_in(dir: &Path) -> AppState {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.to_path_buf();
    config.claude_projects_path = dir.join("no-claude-projects");
    AppState::new(config, dir.join("config.json"))
}

fn serve(v: &Path) -> Core {
    Core::new(ServeHost::new(state_in(v), false))
}

/// An admitted request from `run_id` (a chat in `perm`, or an automation),
/// with a valid proof.
fn request(run_id: &str, surface: &str, perm: &str, action: &str, args: Value) -> AppRequest {
    let grant = AppGrant::new(AppAccess::Write, 0, perm);
    AppRequest {
        v: 1,
        action: action.into(),
        run_id: run_id.into(),
        surface: surface.into(),
        scope: "s-20261115-serve".into(),
        access: "write".into(),
        depth: 0,
        perm: perm.into(),
        proof: proof_for(run_id, surface, "s-20261115-serve", &grant),
        args,
        tool: String::new(),
        tool_args: Value::Null,
    }
}

fn call(core: &Core, v: &Path, req: &AppRequest) -> Result<Value, String> {
    dispatch(
        core.action_host().expect("serve has an action host"),
        v,
        req,
        true,
    )
}

/// The last audit line.
fn last_audit(v: &Path) -> Value {
    let log = std::fs::read_to_string(v.join(AUDIT_FILE)).unwrap();
    serde_json::from_str(log.lines().last().unwrap()).unwrap()
}

/// Every required parameter, typed (as `coo_parity_tests_g2` builds them).
fn dummy_args(id: &str) -> Value {
    let action = app_actions::find(id).unwrap();
    let mut args = Map::new();
    for p in action.params.iter().filter(|p| p.required) {
        use app_actions::Kind;
        let value = match p.kind {
            Kind::Str | Kind::Text | Kind::Secret | Kind::Any => json!("x"),
            Kind::Int(_) | Kind::Num => json!(1),
            Kind::Bool => json!(false),
            Kind::StrList => json!([]),
            Kind::OneOf(values) => json!(values[0]),
            Kind::Object => json!({}),
        };
        args.insert(p.name.to_string(), value);
    }
    Value::Object(args)
}

/// Serve's host holds the same state its core reads, and hands out an
/// action host with no GUI handle.
#[test]
fn serve_hands_out_an_action_host_over_its_own_state() {
    let dir = tempfile::tempdir().unwrap();
    let core = serve(dir.path());
    let host = core.action_host().expect("an action host");
    assert!(host.app().is_none());
    assert!(std::ptr::eq(
        host.app_state().inner(),
        core.try_state::<AppState>().unwrap()
    ));
    // A bare host has none, so the broker keeps refusing there.
    assert!(Core::new(crate::core_host::RecordingHost::default())
        .action_host()
        .is_none());
}

/// A read runs at once and is audited.
#[test]
fn a_read_runs_in_serve() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    crate::workspaces::create(v, "Runner", "Reads in serve").unwrap();
    let listed = call(
        &core,
        v,
        &request(
            "run-read",
            "chat",
            "acceptEdits",
            "workspaces.list_workspaces",
            json!({}),
        ),
    )
    .unwrap();
    assert!(listed.to_string().contains("Runner"), "{listed}");
    let audit = last_audit(v);
    assert_eq!(
        (
            audit["action"].as_str(),
            audit["ok"].as_bool(),
            audit["tier"].as_str()
        ),
        (Some("workspaces.list_workspaces"), Some(true), Some("read"))
    );
    // settings.get answers; a UI-owned key is marked unavailable, not fatal.
    let settings = call(
        &core,
        v,
        &request("run-read", "chat", "acceptEdits", "settings.get", json!({})),
    )
    .unwrap();
    assert!(settings["settings"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
}

/// A write runs inside the caller's live run and lands in its History; a
/// task (whose handler would emit to the window) is made too.
#[test]
fn a_write_runs_in_serve_and_lands_in_the_run() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let id = run.id.clone();
    call(
        &core,
        v,
        &request(
            &id,
            "chat",
            "acceptEdits",
            "vault.vault_create_note",
            json!({ "path": "notes/serve.md", "content": "# Serve\n" }),
        ),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(v.join("notes/serve.md")).unwrap(),
        "# Serve\n"
    );
    // The Task tools switch is process-wide (serve loads it at boot) and
    // another test flips it: held under `SWITCH_TESTS` while it matters.
    let _switch = crate::tasks_mcp::SWITCH_TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::tasks_mcp::set_enabled(true);
    let task = call(
        &core,
        v,
        &request(
            &id,
            "chat",
            "acceptEdits",
            "tasks.create_task",
            json!({ "idempotencyKey": "serve-1", "title": "Made in serve" }),
        ),
    )
    .unwrap();
    assert!(task.to_string().contains("Made in serve"), "{task}");
    let finished = crate::automated_history::finish_run(v, run, "success").unwrap();
    assert!(
        finished
            .changes
            .iter()
            .any(|c| c.vault_path == "notes/serve.md"),
        "{:?}",
        finished.changes
    );
}

/// The two handlers moved onto `Core` (M1 PR 5c) run in serve: the COO's
/// `sessions.stop` reaches the chat's cancel, and a notification lands in
/// the bell's file.
#[test]
fn stop_and_notify_run_on_serves_core() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    // It reaches `chat::cancel` on serve's core, which asks the run queue
    // (no pump runs in a unit test; the smoke stops a real turn).
    let stopped = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "acceptEdits",
            "sessions.stop",
            json!({ "session": "s-20261115-other" }),
        ),
    );
    match stopped {
        Ok(value) => assert_eq!(value, json!({ "stopped": "s-20261115-other" })),
        Err(error) => assert!(error.contains("run queue"), "{error}"),
    }
    let made = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "acceptEdits",
            "notifications.create_notification",
            json!({ "title": "From the runner", "body": "made in serve", "kind": "agent" }),
        ),
    )
    .unwrap();
    assert_eq!(made["title"], json!("From the runner"));
    let listed = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "acceptEdits",
            "notifications.list_notifications",
            json!({}),
        ),
    )
    .unwrap();
    assert!(listed.to_string().contains("From the runner"), "{listed}");
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// A confirm action is refused in every mode (acceptEdits, bypass, an
/// automation that named it in its allowed tools), shows no card, changes
/// nothing, and is audited with `card: null`.
#[test]
fn a_confirm_action_fails_closed_in_every_mode() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    crate::notifications::create(
        &core,
        v,
        crate::notifications::NotificationInput {
            title: "Keep me".into(),
            body: "a confirm in serve must not clear this".into(),
            kind: "info".into(),
            source: None,
            action_label: None,
            action_url: None,
            notify_native: false,
        },
    )
    .unwrap();
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let id = run.id.clone();
    crate::app_broker::register_workflow_run(
        &id,
        &["mcp__hollow_app__notifications.clear_notifications".to_string()],
        crate::tasks_mcp::TasksAccess::ReadOnly,
    );
    for (surface, perm) in [
        ("chat", "acceptEdits"),
        ("chat", "bypassPermissions"),
        ("chat", "auto"),
        ("workflow", "bypassPermissions"),
    ] {
        let error = call(
            &core,
            v,
            &request(
                &id,
                surface,
                perm,
                "notifications.clear_notifications",
                json!({}),
            ),
        )
        .unwrap_err();
        assert_eq!(
            error,
            FailClosed.refusal(app_actions::find("notifications.clear_notifications").unwrap()),
            "{surface} {perm}"
        );
        assert!(error.contains("hollow serve") && error.contains("nothing ran"));
        let audit = last_audit(v);
        assert_eq!(audit["ok"], json!(false));
        assert_eq!(audit["card"], Value::Null);
        assert_eq!(audit["tier"], json!("confirm"));
    }
    let left = call(
        &core,
        v,
        &request(
            &id,
            "chat",
            "acceptEdits",
            "notifications.list_notifications",
            json!({}),
        ),
    )
    .unwrap();
    assert!(left.to_string().contains("Keep me"), "{left}");
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// An external action fails closed the same way.
#[test]
fn an_external_action_fails_closed() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let error = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "bypassPermissions",
            "build.publish",
            dummy_args("build.publish"),
        ),
    )
    .unwrap_err();
    assert!(
        error.starts_with("build.publish needs the person's Allow on a card"),
        "{error}"
    );
    let audit = last_audit(v);
    assert_eq!(
        (
            audit["ok"].clone(),
            audit["card"].clone(),
            audit["tier"].clone()
        ),
        (json!(false), Value::Null, json!("external"))
    );
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// A route that can answer is asked (M2's seam): the card is shown and its
/// answer audited. Standing approvals stay off, so even bypass asks.
#[test]
fn a_live_card_route_is_asked_even_in_bypass() {
    struct Denies(std::sync::Mutex<Vec<String>>);
    impl CardRoute for Denies {
        fn live(&self) -> bool {
            true
        }
        fn ask(&self, card: &Card, _caller: &Caller) -> Verdict {
            self.0.lock().unwrap().push(card.title.clone());
            Verdict::Denied
        }
        fn refusal(&self, action: &Action) -> String {
            format!("{} unreachable", action.id)
        }
    }
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let host = core.action_host().unwrap();
    let make = || host.boxed();
    let asker = ServeAsker::new(Box::new(Denies(Default::default())));
    let req = request(
        &run.id,
        "chat",
        "bypassPermissions",
        "notifications.clear_notifications",
        json!({}),
    );
    let error =
        crate::app_broker::dispatch(&make, v, &req, true, &asker, &SERVE_POLICY).unwrap_err();
    assert!(error.contains("declined"), "{error}");
    assert_eq!(last_audit(v)["card"], json!("denied"));
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// What needs the window, the Keychain or a login session says so; an
/// action outside the first set says it isn't on the runner yet; a tracker
/// write is refused. Nothing hangs and nothing runs.
#[test]
fn what_serve_does_not_run_is_refused_plainly() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let refused = |id: &str| {
        call(
            &core,
            v,
            &request(&run.id, "chat", "bypassPermissions", id, dummy_args(id)),
        )
        .unwrap_err()
    };
    assert_eq!(
        refused("ui.navigate"),
        "ui.navigate needs Hollow's window, and this Hollow runs without one (hollow serve); nothing ran. Put it in your reply for the person to do in Hollow on their Mac."
    );
    assert!(refused("ui.vault_reveal_in_finder").contains("opens Finder"));
    assert!(refused("usage.claude_quota").contains("login Keychain"));
    assert!(refused("agents.agent_models").contains("isn't available on this runner yet"));
    assert!(refused("goals.create_goal").contains("isn't available on this runner yet"));
    assert!(refused("tracker.comment_issue").contains("task tracker"));
    assert!(refused("tracker.report_progress").contains("task tracker"));
    // A tracker key change is a card like any confirm.
    assert!(refused("tracker.set_key").contains("needs the person's Allow on a card"));
    let audit = last_audit(v);
    assert_eq!(
        (audit["ok"].clone(), audit["card"].clone()),
        (json!(false), Value::Null)
    );
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// A handler that asks for the running app is worded for serve; anything
/// else passes through unchanged.
#[test]
fn a_running_app_refusal_is_reworded_for_serve() {
    for said in [
        "stop_session needs the running app.",
        "Pressure tests need the running app. Nothing started.",
        "Hollow's window is unavailable.",
    ] {
        assert_eq!(
            reword("sessions.x", Err(said.into())).unwrap_err(),
            needs_window("sessions.x")
        );
    }
    assert_eq!(reword("a.b", Err("other".into())).unwrap_err(), "other");
    assert_eq!(reword("a.b", Ok(json!(1))).unwrap(), json!(1));
}

/// The lists name real actions of the right tier: every refused id exists,
/// every listed write is a write, and every write the unattended COO may
/// make runs in serve.
#[test]
fn the_serve_lists_name_real_actions() {
    for (id, _) in SERVE_REFUSED {
        assert!(app_actions::find(id).is_some(), "{id} is no action");
    }
    for id in WRITE_IDS {
        let action = app_actions::find(id).unwrap_or_else(|| panic!("{id} is no action"));
        assert_eq!(action.tier, Tier::Write, "{id}");
        assert!(screen(action).is_ok(), "{id}");
        assert!(!SERVE_REFUSED.iter().any(|(r, _)| r == id), "{id}");
    }
    for id in READ_IDS {
        let action = app_actions::find(id).unwrap_or_else(|| panic!("{id} is no action"));
        assert_eq!(action.tier, Tier::Read, "{id}");
    }
    for area in READ_AREAS {
        assert!(
            app_actions::ACTIONS
                .iter()
                .any(|a| a.area == *area && a.tier == Tier::Read),
            "{area} has no reads"
        );
    }
    for id in crate::main_agent::unattended::SAFE {
        assert!(WRITE_IDS.contains(id), "{id} is in the COO's safe set");
    }
}

/// Drift sweep, a heuristic: no action serve runs asks for the window in its
/// own handler (`cx.app(..)`, `host.app().ok_or..`, a UI setting). It reads
/// only the action's table entry as text, so a callee that needs the window
/// deeper down, or a handler written another way, slips past it; the
/// refusal wording (`reword`) still covers those at run time. Another lane
/// adding such an action to a serve area must list it in `SERVE_REFUSED`.
#[test]
fn no_action_serve_runs_needs_the_window_in_its_handler() {
    let src = include_str!("app_actions.rs");
    let table = &src[src.find("pub(crate) static ACTIONS").unwrap()..];
    let table = &table[..table.find("\n];\n").unwrap()];
    let starts = [
        "\n    act!(",
        "\n    read!(",
        "\n    verb!(",
        "\n    composite!(",
        "\n    Action {",
    ];
    let mut runs = 0;
    for action in app_actions::ACTIONS {
        if !matches!(action.tier, Tier::Read | Tier::Write) || screen(action).is_err() {
            continue;
        }
        runs += 1;
        let (area, name) = action.id.split_at(action.area.len());
        let name = name.trim_start_matches('.');
        let at = table
            .find(&format!("\"{}\"", action.id))
            .or_else(|| table.find(&format!("\"{area}\", {name},")))
            .unwrap_or_else(|| panic!("{} not found in the table", action.id));
        let rest = &table[at..];
        let end = starts
            .iter()
            .filter_map(|s| rest.find(s))
            .min()
            .unwrap_or(rest.len());
        let entry = &rest[..end];
        for needs in ["cx.app(", ".app().ok_or", "ui_set(", "ui_bridge("] {
            assert!(
                !entry.contains(needs),
                "{} runs in serve but its handler calls {needs}: list it in SERVE_REFUSED or move it onto Core",
                action.id
            );
        }
    }
    assert!(runs > 150, "{runs}");
}

/// The shell reaches Tauri only through the mock runtime's three calls and
/// the types a handler's `State` needs; serve.rs builds no Tauri app.
#[test]
fn the_state_shell_uses_only_the_mock_runtime() {
    let src = include_str!("serve_actions.rs");
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let allowed = [
        "tauri::test::MockRuntime",
        "tauri::test::mock_builder()",
        "tauri::test::mock_context(",
        "tauri::test::noop_assets()",
        "tauri::Manager",
        "tauri::AppHandle<",
        "tauri::AppHandle>",
        "tauri::App<",
        "tauri::State<",
    ];
    for (at, _) in code.match_indices("tauri::") {
        let rest = &code[at..];
        assert!(
            allowed.iter().any(|a| rest.starts_with(a)),
            "serve_actions reaches {}",
            &rest[..rest.len().min(40)]
        );
    }
    let serve = include_str!("serve.rs");
    assert!(!serve.contains(concat!("mock_", "builder")));
}

/// A write whose card escalates (a session in a stronger permission mode
/// than the asking chat's) is refused in serve, where the app would show
/// the card; nothing is queued.
#[test]
fn an_escalating_write_is_refused_in_serve() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let action = app_actions::find("sessions.start").unwrap();
    for perm in ["acceptEdits", "auto"] {
        let error = call(
            &core,
            v,
            &request(
                &run.id,
                "chat",
                perm,
                "sessions.start",
                json!({ "workspace": "vault", "prompt": "Go.", "agent": "claude", "permission": "bypassPermissions" }),
            ),
        )
        .unwrap_err();
        assert_eq!(error, FailClosed.refusal(action), "{perm}");
        assert_eq!(last_audit(v)["card"], Value::Null);
    }
    assert!(crate::run_queue::list(v, 1000)
        .unwrap_or_default()
        .is_empty());
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// An unattended COO turn (a check-in or a wake) gets no standing approval
/// in serve: a confirm is refused, and so is a start in its safe class
/// whose card the app's unattended rule would have approved unasked.
#[test]
fn an_unattended_coo_turn_gets_no_standing_approval_in_serve() {
    use crate::main_agent::unattended;
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let core = serve(v);
    let run = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    let id = crate::descendants::tests::child_turn(
        v,
        "s-20261115-wake",
        &run.id,
        crate::run_queue::RowStatus::Running,
    );
    unattended::register(id);
    assert!(unattended::is_unattended_run(v, &run.id));
    let error = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "bypassPermissions",
            "notifications.clear_notifications",
            json!({}),
        ),
    )
    .unwrap_err();
    assert!(error.contains("nothing"), "{error}");
    let audit = last_audit(v);
    assert_eq!(
        (audit["ok"].clone(), audit["card"].clone()),
        (json!(false), Value::Null)
    );
    let rows = crate::run_queue::list(v, 1000).unwrap().len();
    let error = call(
        &core,
        v,
        &request(
            &run.id,
            "chat",
            "plan",
            "sessions.start",
            json!({ "workspace": "quick-chats", "prompt": "Go.", "agent": "claude", "permission": "acceptEdits" }),
        ),
    )
    .unwrap_err();
    assert_eq!(
        error,
        FailClosed.refusal(app_actions::find("sessions.start").unwrap())
    );
    assert_eq!(last_audit(v)["card"], Value::Null);
    assert_eq!(
        crate::run_queue::list(v, 1000).unwrap().len(),
        rows,
        "nothing queued"
    );
    crate::automated_history::finish_run(v, run, "success").unwrap();
}

/// Through the socket: `handle_conn` on a stream pair over serve's `Core`
/// (the `(None, Some(host))` arm) answers a read `ok: true` and a confirm
/// with the card refusal, each audited. This process is the peer, so it is
/// registered as the run's leader.
#[test]
fn the_broker_socket_runs_serves_actions() {
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path().to_path_buf();
    let core = serve(&v);
    let run = crate::automated_history::try_begin_run(&v, "chat", None).unwrap();
    let peers = crate::run_auth::Peers::leaked();
    peers.register(
        "serve-socket-test",
        std::process::id() as i32,
        vec![run.id.clone()],
    );
    // The switch is process-wide and off in tests (serve loads it at boot);
    // nothing in the suite depends on it being off.
    crate::app_mcp::set_enabled(true);
    let ask = |action: &str| -> Value {
        let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        let (core, v) = (core.clone(), v.clone());
        let broker =
            std::thread::spawn(move || crate::app_broker::handle_conn(server, &core, &v, peers));
        let grant = AppGrant::new(AppAccess::Write, 0, "bypassPermissions");
        let line = json!({
            "v": 1, "action": action, "runId": run.id, "surface": "chat",
            "scope": "s-20261115-sock", "access": "write", "depth": 0,
            "perm": "bypassPermissions",
            "proof": proof_for(&run.id, "chat", "s-20261115-sock", &grant),
            "args": {},
        });
        writeln!(client, "{line}").unwrap();
        let mut answer = String::new();
        BufReader::new(&client).read_line(&mut answer).unwrap();
        broker.join().unwrap();
        serde_json::from_str(&answer).unwrap()
    };
    let read = ask("workspaces.list_workspaces");
    assert_eq!(read["ok"], json!(true), "{read}");
    let refused = ask("notifications.clear_notifications");
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(
        refused["error"],
        json!(FailClosed.refusal(app_actions::find("notifications.clear_notifications").unwrap()))
    );
    let log = std::fs::read_to_string(v.join(AUDIT_FILE)).unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");
    peers.release("serve-socket-test");
    crate::automated_history::finish_run(&v, run, "success").unwrap();
}
