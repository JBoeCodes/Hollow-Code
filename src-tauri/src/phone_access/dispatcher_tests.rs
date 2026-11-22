//! M3a: the dispatcher COO against this module's rig as the runner (the
//! mini, `runner_rig`), through the peer commands' hub, a stand-in ssh and a
//! real Unix-socket tunnel (`peer_api_tests::MacBook`). The acceptance test:
//! with a long turn running on the runner, the dispatcher's next question is
//! taken at once here and the long request is routed there, not run here.
use super::peer_api_tests::MacBook;
use super::*;
use crate::dispatcher_coo::{self, Runner, Target};
use crate::runner_coo::Role;

/// The runner COO's conversation in the stand-in runner's vault.
const RUNNER_COO: &str = "s-20261115-0000c000";
/// A chat on the runner that runs a long turn.
const LONG: &str = "s-20261115-0000a000";
/// The dispatcher COO's conversation on this Mac.
const DISPATCHER: &str = "s-20261115-0000d15a";

/// `vault`'s COO folder, made if needed.
fn coo_cwd(vault: &Path) -> String {
    crate::workspaces::ensure_main_agent(vault).unwrap();
    crate::main_agent::main_cwd(vault).expect("the COO's folder")
}

/// A turn for `key` in `cwd` accepted into `vault`'s queue now.
fn ask(vault: &Path, key: &str, cwd: &str, text: &str) -> crate::chat::AcceptedTurn {
    let input = ChatSendInput {
        prompt: text.into(),
        session_key: Some(key.into()),
        cwd: Some(cwd.into()),
        agent: Some("claude".into()),
        ..ChatSendInput::default()
    };
    crate::chat::accept_turn(vault, send_request(vault, input), Utc::now()).unwrap()
}

fn live(vault: &Path, key: &str) -> bool {
    crate::run_queue::newest_chat_row(vault, key)
        .unwrap()
        .is_some_and(|row| row.status.is_live())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_turn_on_the_runner_leaves_the_dispatcher_free_and_long_work_is_routed_there() {
    // The runner: its runner COO has a conversation, and another chat there
    // is busy with long work.
    let rig = runner_rig().await;
    let runner_vault = rig.vault.to_string_lossy().into_owned();
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning, nothing new overnight.")],
        "acceptEdits",
    );
    seed_chat(
        &rig.vault,
        LONG,
        "claude",
        &runner_vault,
        &[("Plan the release", "Planned.")],
        "acceptEdits",
    );
    rig.open_machine_window();

    // This Mac pairs with it and learns it runs the runner COO: its COO is
    // now the dispatcher.
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    let paired = book.machines.list().unwrap();
    assert_eq!(paired[0].coo.as_deref(), Some(RUNNER_COO));
    assert!(paired[0].coo_seen_at.is_some());
    let runner =
        dispatcher_coo::active_from(Role::Single, &paired, Utc::now()).expect("a dispatcher");
    assert_eq!(
        runner,
        Runner {
            machine_id: id.clone(),
            name: "Test Mac".into(),
        }
    );

    // A long turn runs on the runner (the person started it there).
    book.hub
        .send(&id, LONG, "Build and ship the release", "long-0001")
        .await
        .unwrap();
    assert!(live(&rig.vault, LONG));

    // This Mac: the dispatcher's own conversation, idle.
    let here_dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let here = here_dir.path();
    let here_coo = coo_cwd(here);
    seed_chat(
        here,
        DISPATCHER,
        "claude",
        &here_coo,
        &[("Hi", "Hi.")],
        "acceptEdits",
    );
    assert!(crate::chat_chain::is_main_conversation(here, DISPATCHER));

    // The person asks for long work: the dispatcher routes it at once.
    let started = Instant::now();
    let routed = dispatcher_coo::route(
        &book.hub,
        &runner,
        "MacBook Pro",
        Target::Coo,
        "Research the Q3 numbers and write a brief.",
    )
    .await
    .unwrap();
    assert!(started.elapsed() < dispatcher_coo::ROUTE_WAIT);
    assert_eq!(routed["routed"], true);
    assert_eq!(routed["to"], "coo");
    assert_eq!(routed["session"], RUNNER_COO);
    assert_eq!(routed["machine"]["id"], id.as_str());
    assert!(routed["tell"]
        .as_str()
        .unwrap()
        .contains("Nothing runs on this Mac."));
    // No remote words come back into this Mac's transcript (S26).
    assert!(!routed.to_string().contains("nothing new overnight"));

    // It landed in the runner's queue, as the runner COO's next turn, with
    // the line saying where it came from; nothing was stopped or waited for
    // there: the long turn is still running.
    assert!(live(&rig.vault, RUNNER_COO));
    let sent = rig.host.sent.lock().unwrap().clone();
    let last = sent.last().unwrap();
    assert_eq!(last.session_key.as_deref(), Some(RUNNER_COO));
    assert!(last.prompt.contains(
        "Routed by the dispatcher COO on MacBook Pro, not typed by the person in this chat."
    ));
    assert!(last
        .prompt
        .ends_with("Research the Q3 numbers and write a brief."));
    assert!(live(&rig.vault, LONG));
    assert!(rig.host.stopped.lock().unwrap().is_empty());

    // Not run here: this Mac's queue is empty, so the person's second
    // question is taken at once, with nothing ahead of it.
    assert!(crate::run_queue::live_rows(here).unwrap().is_empty());
    let second = ask(here, DISPATCHER, &here_coo, "What's running on the mini?");
    assert_eq!(second.waiting_for, None);
    assert!(live(here, DISPATCHER));
    assert!(crate::run_queue::newest_chat_row(here, RUNNER_COO)
        .unwrap()
        .is_none());

    // A local start of long work from this COO asks the person first.
    let scope = dispatcher_coo::scope_of(here, DISPATCHER);
    assert_eq!(scope, dispatcher_coo::Scope::Coo);
    assert!(dispatcher_coo::guard_for("sessions.start", scope, false, Some(&runner)).is_some());

    // A second long request while the runner COO's turn is still queued:
    // busy, and nothing more is sent or held for later.
    let before = rig.host.sent.lock().unwrap().len();
    let busy = dispatcher_coo::route(&book.hub, &runner, "MacBook Pro", Target::Coo, "Also this.")
        .await
        .unwrap_err();
    assert!(busy.contains("nothing is queued for later"), "{busy}");
    assert_eq!(rig.host.sent.lock().unwrap().len(), before);

    // A project whose lead never ran on the runner: nothing sent, and the
    // dispatcher is told to route it to the runner COO instead.
    let no_lead = dispatcher_coo::route(
        &book.hub,
        &runner,
        "MacBook Pro",
        Target::Lead("portal-1a2b".into()),
        "Fix the login bug.",
    )
    .await
    .unwrap_err();
    assert!(no_lead.contains("leave out workspace"), "{no_lead}");
    assert_eq!(rig.host.sent.lock().unwrap().len(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_that_is_not_the_runner_says_no_coo_and_nothing_dispatches() {
    // The plain rig: no `coo` in `GET /v1/machine`, so pairing records none
    // and this Mac's COO stays the single COO.
    let rig = rig().await;
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning.")],
        "acceptEdits",
    );
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    let paired = book.machines.list().unwrap();
    assert_eq!(paired[0].coo, None);
    assert_eq!(
        dispatcher_coo::active_from(Role::Single, &paired, Utc::now()),
        None
    );
    assert_eq!(book.hub.runner_coo(&id).await.unwrap(), None);
    // A route to it anyway (a stale record) sends nothing.
    let runner = Runner {
        machine_id: id,
        name: "Test Mac".into(),
    };
    let refused = dispatcher_coo::route(&book.hub, &runner, "MacBook Pro", Target::Coo, "Do it.")
        .await
        .unwrap_err();
    assert!(
        refused.contains("doesn't run a runner COO now"),
        "{refused}"
    );
    assert!(rig.host.sent.lock().unwrap().is_empty());
}

/// Round 2 fix 1: the hourly check renews a stale `cooSeenAt`, so the
/// dispatcher stays on while the runner answers (it lapsed a day after
/// pairing before).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hourly_check_keeps_a_live_runner_dispatching() {
    let rig = runner_rig().await;
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning.")],
        "acceptEdits",
    );
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    // Two days without an answer: no dispatcher, and the Mac is due.
    let mut stale = book.machines.list().unwrap().remove(0);
    stale.coo_seen_at = Some((Utc::now() - Span::days(2)).to_rfc3339());
    book.machines.upsert(stale).unwrap();
    let now = Utc::now();
    let paired = book.machines.list().unwrap();
    assert_eq!(
        dispatcher_coo::active_from(Role::Single, &paired, now),
        None
    );
    let due = dispatcher_coo::due_for_check(&paired, now);
    assert_eq!(due, std::slice::from_ref(&id));
    // The check asks it, and the record is fresh again.
    book.hub.refresh_coo(&due).await;
    let paired = book.machines.list().unwrap();
    assert_eq!(paired[0].coo.as_deref(), Some(RUNNER_COO));
    assert!(dispatcher_coo::active_from(Role::Single, &paired, Utc::now()).is_some());
    assert!(dispatcher_coo::due_for_check(&paired, Utc::now()).is_empty());
}

/// M3a PR2 review: with a runner COO paired, `agents.cooDispatcher` `off`
/// leaves this Mac's COO unguarded and unable to route: local work from its
/// own conversation asks no dispatcher card, and a route is refused with the
/// setting's words (nothing sent: the refusal comes before any hub). With
/// `auto` the same call is guarded.
#[tokio::test]
async fn the_setting_off_leaves_the_coo_unguarded_and_routing_refused() {
    use crate::dispatcher_coo::tests::{record, PAIRED, SETTING_OFF};
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let here = dir.path();
    let cwd = coo_cwd(here);
    seed_chat(
        here,
        DISPATCHER,
        "claude",
        &cwd,
        &[("Hi", "Hi.")],
        "acceptEdits",
    );
    assert!(crate::chat_chain::is_main_conversation(here, DISPATCHER));
    let caller = crate::app_actions::Caller {
        run_id: "run-dispatcher-off".into(),
        surface: "chat".into(),
        scope: DISPATCHER.into(),
        access: crate::app_mcp::AppAccess::Write,
        depth: 0,
        perm: "bypassPermissions".into(),
    };
    let args = serde_json::Map::new();
    let fresh = crate::peer_machines::MachineRecord {
        coo_seen_at: Some(Utc::now().to_rfc3339()),
        ..record("m-8d21c4be90f17a3e", "Mac mini", Some(RUNNER_COO))
    };
    PAIRED.with(|p| *p.borrow_mut() = vec![fresh]);
    // Auto: the COO's local start is guarded.
    assert!(dispatcher_coo::guard(here, "sessions.start", &caller, &args).is_some());
    // Off: no guard, and a route is refused before anything is sent.
    SETTING_OFF.with(|s| s.set(true));
    let guard = dispatcher_coo::guard(here, "sessions.start", &caller, &args);
    let refused =
        dispatcher_coo::route_action(here, &caller, None, "Build the release.", None).await;
    SETTING_OFF.with(|s| s.set(false));
    PAIRED.with(|p| p.borrow_mut().clear());
    assert!(guard.is_none());
    let refused = refused.unwrap_err();
    assert!(
        refused.contains("Settings, Devices, COO on this Mac"),
        "{refused}"
    );
    assert!(refused.contains("Nothing was sent"), "{refused}");
}

/// M3a follow-up: a routed turn is watched through the runner's chat list
/// (started for it, since no window follows it here), and when it ends this
/// Mac gets one notification whose Open opens that chat there; it carries
/// none of that chat's words. With nothing left to watch, the list stream
/// it started stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_routed_turn_that_ends_there_posts_one_notification_here() {
    let rig = runner_rig().await;
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning, nothing new overnight.")],
        "acceptEdits",
    );
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    let runner =
        dispatcher_coo::active_from(Role::Single, &book.machines.list().unwrap(), Utc::now())
            .expect("a dispatcher");
    let here_dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let here = here_dir.path();
    // The rig's live buffer has no worker: tick it.
    let ticker = {
        let live = rig.live.clone();
        tokio::spawn(async move {
            loop {
                live.tick(Instant::now());
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
    };

    let done = dispatcher_coo::route(
        &book.hub,
        &runner,
        "MacBook Pro",
        Target::Coo,
        "Write the brief.",
    )
    .await
    .unwrap();
    let turn =
        dispatcher_coo::routed_turn(&done, &runner, &Target::Coo, here).expect("a routed turn");
    assert_eq!(turn.key, RUNNER_COO);
    assert_eq!(book.hub.watched(&id), (false, None));
    book.hub.follow_routed(turn).await;
    assert_eq!(
        book.hub.watched(&id),
        (true, None),
        "the list is followed for it"
    );
    assert_eq!(book.hub.routed_waiting(), vec![RUNNER_COO.to_string()]);

    // Still queued there: nothing yet.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(crate::notifications::list(here).items.is_empty());

    // Its turn ends there.
    let row = crate::run_queue::newest_chat_row(&rig.vault, RUNNER_COO)
        .unwrap()
        .unwrap();
    crate::run_queue::finish(
        &rig.vault,
        row.id,
        crate::run_queue::RowStatus::Done,
        None,
        Utc::now(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let notice = loop {
        let items = crate::notifications::list(here).items;
        if let Some(n) = items.first() {
            break n.clone();
        }
        assert!(Instant::now() < deadline, "no notification");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        notice.action_url.as_deref(),
        Some(format!("peer-chat:{id}/{RUNNER_COO}").as_str())
    );
    assert_eq!(notice.action_label.as_deref(), Some("Open"));
    assert_eq!(notice.title, "Routed work ended on Test Mac");
    assert!(
        notice.body.starts_with("Test Mac's COO finished"),
        "{}",
        notice.body
    );
    assert!(!notice.body.contains("nothing new overnight"));
    assert!(book.hub.routed_waiting().is_empty());
    // The list stream it started stops; one notification only.
    let deadline = Instant::now() + Duration::from_secs(5);
    while book.hub.watched(&id).0 {
        assert!(Instant::now() < deadline, "the list stream kept running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(crate::notifications::list(here).items.len(), 1);
    ticker.abort();
}

/// Lane M: a routed turn is kept on disk. The app quits while it runs there;
/// when it has ended and the app starts again, the first list posts its one
/// notification, and a later start posts nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_routed_turn_survives_a_restart_and_notifies_once() {
    let rig = runner_rig().await;
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning, nothing new overnight.")],
        "acceptEdits",
    );
    rig.open_machine_window();
    let id = rig.machine_id();
    let here_dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let here = here_dir.path();
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir
        .path()
        .join("Peers")
        .join(crate::routed_store::FILE);
    let ticker = {
        let live = rig.live.clone();
        tokio::spawn(async move {
            loop {
                live.tick(Instant::now());
                // sleep-ok: the rig's live buffer has no worker; tick it.
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
    };

    // The first run of the app: routes a turn and keeps it on disk.
    let book = MacBook::new(&rig);
    book.hub.pair("host-up").await.unwrap();
    book.hub.keep_routed(file.clone());
    let runner =
        dispatcher_coo::active_from(Role::Single, &book.machines.list().unwrap(), Utc::now())
            .expect("a dispatcher");
    let done = dispatcher_coo::route(&book.hub, &runner, "MacBook Pro", Target::Coo, "Go.")
        .await
        .unwrap();
    let turn = dispatcher_coo::routed_turn(&done, &runner, &Target::Coo, here).unwrap();
    book.hub.follow_routed(turn).await;
    assert_eq!(crate::routed_store::Book::load(&file).pending().len(), 1);
    // Long enough that an idle list counts as ended even unseen busy.
    // sleep-ok: the settle time is wall clock (`ROUTED_SETTLE`).
    tokio::time::sleep(dispatcher_coo::ROUTED_SETTLE + Duration::from_millis(300)).await;
    assert!(crate::notifications::list(here).items.is_empty());

    // The app quits: memory, streams and the in-process book are gone.
    book.hub.forget_routed_in_memory(&id);
    assert!(book.hub.routed_waiting().is_empty());
    assert_eq!(book.hub.watched(&id), (false, None));
    // The turn ends while Hollow is closed.
    let row = crate::run_queue::newest_chat_row(&rig.vault, RUNNER_COO)
        .unwrap()
        .unwrap();
    crate::run_queue::finish(
        &rig.vault,
        row.id,
        crate::run_queue::RowStatus::Done,
        None,
        Utc::now(),
    )
    .unwrap();

    // The next run of the app reads the file and watches again.
    book.hub.keep_routed(file.clone());
    book.hub.resume_routed().await;
    book.hub.resume_routed().await; // idempotent: one watch, one turn
    let deadline = Instant::now() + Duration::from_secs(20);
    let notice = loop {
        if let Some(n) = crate::notifications::list(here).items.first() {
            break n.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no notification after the restart"
        );
        // sleep-ok: poll for the notification the list stream posts.
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        notice.action_url.as_deref(),
        Some(format!("peer-chat:{id}/{RUNNER_COO}").as_str())
    );
    assert!(book.hub.routed_waiting().is_empty());
    // sleep-ok: leave time for a second notification to show up if one would.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(crate::notifications::list(here).items.len(), 1);
    let kept = crate::routed_store::Book::load(&file);
    assert!(kept.pending().is_empty());
    assert_eq!(kept.entries[0].state, crate::routed_store::State::Delivered);

    // A third start finds nothing waiting: no stream, no second notice.
    book.hub.forget_routed_in_memory(&id);
    book.hub.keep_routed(file.clone());
    book.hub.resume_routed().await;
    assert_eq!(book.hub.watched(&id), (false, None));
    assert!(book.hub.routed_waiting().is_empty());
    // sleep-ok: leave time for a second notification to show up if one would.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(crate::notifications::list(here).items.len(), 1);
    ticker.abort();
}

/// A list the window follows is the window's: a routed turn rides on it,
/// and an unwatch while one waits keeps it for the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_windows_list_carries_a_routed_turn_and_outlives_its_unwatch() {
    let rig = runner_rig().await;
    seed_chat(
        &rig.vault,
        RUNNER_COO,
        "claude",
        &coo_cwd(&rig.vault),
        &[("Morning", "Morning.")],
        "acceptEdits",
    );
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    let runner =
        dispatcher_coo::active_from(Role::Single, &book.machines.list().unwrap(), Utc::now())
            .unwrap();
    let here_dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    book.hub.watch(&id, None).await.unwrap();
    let done = dispatcher_coo::route(&book.hub, &runner, "MacBook Pro", Target::Coo, "Go.")
        .await
        .unwrap();
    let turn = dispatcher_coo::routed_turn(&done, &runner, &Target::Coo, here_dir.path()).unwrap();
    book.hub.follow_routed(turn).await;
    // The window stops following: the turn still waits, so the list stays.
    book.hub.unwatch(&id, None);
    assert_eq!(book.hub.watched(&id), (true, None));
}

/// M3a follow-up: a runner whose runner COO has no conversation says so
/// (`coo-start`), so this Mac's COO isn't a dispatcher yet and its status
/// names that Mac; the person's Start its COO starts that conversation there
/// from this Mac, and the dispatcher is on at once. A second start sends
/// nothing and answers the same conversation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_starts_the_runner_coos_first_conversation_and_dispatches() {
    let rig = runner_rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = rig.machine_id();
    book.hub.pair("host-up").await.unwrap();
    let paired = book.machines.list().unwrap();
    assert_eq!(paired[0].coo, None);
    assert!(paired[0]
        .capabilities
        .contains(&crate::runner_coo::START_CAPABILITY.to_string()));
    assert!(dispatcher_coo::startable(&paired[0]));
    assert_eq!(
        dispatcher_coo::active_from(Role::Single, &paired, Utc::now()),
        None
    );
    let status = dispatcher_coo::dispatcher_from(Role::Single, false, &paired, Utc::now());
    assert_eq!(
        status.off,
        Some(dispatcher_coo::OffReason::RunnerNotStarted)
    );
    assert_eq!(status.machine_id.as_deref(), Some(id.as_str()));
    assert_eq!(status.machine.as_deref(), Some("Test Mac"));
    // A Mac that is due for the hourly check: it may start one meanwhile.
    assert_eq!(
        dispatcher_coo::due_for_check(&paired, Utc::now() + Span::hours(2)),
        vec![id.clone()]
    );

    let key = book.hub.start_coo(&id).await.unwrap();
    let sent = rig.host.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].cwd.as_deref(), Some(coo_cwd(&rig.vault).as_str()));
    assert!(sent[0]
        .prompt
        .contains("The person started your first conversation from MacBook Pro"));
    assert!(live(&rig.vault, &key));
    assert_eq!(
        crate::runner_coo::chat_key(&rig.vault).as_deref(),
        Some(key.as_str())
    );
    let paired = book.machines.list().unwrap();
    assert_eq!(paired[0].coo.as_deref(), Some(key.as_str()));
    assert!(dispatcher_coo::active_from(Role::Single, &paired, Utc::now()).is_some());

    // Started already (its first turn still queued): the same one, nothing sent.
    assert_eq!(book.hub.start_coo(&id).await.unwrap(), key);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}
