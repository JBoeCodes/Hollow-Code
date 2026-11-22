//! Pause a running chat (P1a, PER-15): the attempt loop ends a paused turn
//! only at a safe point, a Stop wins, a turn that finishes first is not
//! paused, and the resume continues the same native session. Attached from
//! `chat_tests.rs` as `#[path] mod pause`, so `super::` is the chat tests
//! (and through them `crate::chat`).
use super::*;
use crate::chat_pause;

/// A pause asked mid-call: a tool call opens, a call inside it returns, a
/// subagent and the main thread request again while it is open (neither is
/// a safe point), its own result, then the next main request (the safe
/// point), then text only an uncut attempt prints. The account is also past
/// its ceiling, so an unpaused turn would move to another account there.
fn mid_call_phases() -> Vec<String> {
    tg_over_ceiling_phases()
}

/// P1a: a pause asked while a tool call is open waits for the safe point,
/// ends the attempt there with every call finished, keeps the native
/// session, never moves the turn to another account or retries it, and is
/// recorded so the resume continues the same session.
#[cfg(unix)]
#[tokio::test]
async fn a_pause_ends_the_turn_at_the_safe_point_with_no_call_cut() {
    let key = "s-20261115-p1a00001";
    assert!(chat_pause::request_pause(key, true));
    let (dir, run, spawns, elapsed) = drive_guarded(
        key,
        &mid_call_phases(),
        &lines(&[FO_INIT, FO_DONE]),
        &StubAccounts::two(),
        None,
        CUT_PAUSE,
    )
    .await;
    // What the turn's stop registration does when it is dropped.
    chat_pause::clear_request(key);
    let vault = dir.path();
    assert_eq!(spawns.len(), 1, "no ceiling move, failover or retry");
    // TG_NEVER's text being absent proves the cut; a missed one takes 30 s.
    assert!(elapsed < Duration::from_secs(20), "no hang: {elapsed:?}");
    let AttemptsEnd::Driven {
        driven,
        stream,
        failover_stop,
        night_stop,
        budget_stop,
        paused,
        ..
    } = run.end
    else {
        panic!("driven");
    };
    assert_eq!(driven.ended, Ended::LimitReached);
    assert_eq!((failover_stop, night_stop, budget_stop), (None, None, None));
    let note = paused.expect("paused at the safe point");
    assert_eq!(note, "Was using Bash.");
    assert!(!run.sink.text().contains("Still on the first account."));
    assert_eq!(
        run.sink.of_type("toolResult").len(),
        2,
        "both calls finished before the pause"
    );
    assert!(run.sink.of_type("accountSwitched").is_empty());
    assert_eq!(stream.last_session_id.as_deref(), Some("id-fo-1"));
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);

    // What `execute_turn` records next, and the resume it allows.
    chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        stream.last_session_id.as_deref(),
        &note,
        1_000,
    );
    let record = chat_pause::get(vault, key).unwrap().expect("recorded");
    assert_eq!(record.native_session_id, "id-fo-1");
    assert_eq!(record.reason, chat_pause::PauseReason::User);
    assert_eq!(record.note, note);
    let input = chat_pause::resume_input(vault, key, false, 2_000, "pause-resume-a1").unwrap();
    assert_eq!(input.prompt, chat_pause::RESUME_PROMPT);
    assert_eq!(input.session_id.as_deref(), Some("id-fo-1"));
}

/// P1a: the resume's turn runs on the stored native session, so it keeps
/// the whole context, and announces the same id.
#[cfg(unix)]
#[tokio::test]
async fn the_resume_runs_on_the_paused_native_session() {
    let key = "s-20261115-p1a00002";
    let dir = chat_vault();
    let vault = dir.path();
    let fixtures = tempfile::tempdir().unwrap();
    let ord = conversation_with_prior_turn(vault, key);
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "Was using Bash.",
        5
    ));
    let input = chat_pause::resume_input(vault, key, false, 10, "pause-resume-a2").unwrap();
    let native = input.session_id.clone().expect("the stored session");
    let script = print_script(
        fixtures.path(),
        "resumed.jsonl",
        &lines(&[FO_INIT, FO_DONE_TEXT, FO_DONE]),
        false,
        0,
    );
    let spawns = std::cell::RefCell::new(Vec::new());
    let stop = StopSignal::new();
    let run = drive_stub_limited(
        vault,
        key,
        ord,
        &ResumePlan::Resume(native.clone()),
        &stop,
        |spec: &agent::RunSpec| {
            spawns
                .borrow_mut()
                .push((spec.resume.clone(), spec.prompt.clone()));
            stub_command(&script)
        },
        &StubAccounts::two(),
        None,
        None,
        None,
        &[],
    )
    .await;
    let spawns = spawns.into_inner();
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0].0.as_deref(), Some("id-fo-1"), "the same session");
    let AttemptsEnd::Driven {
        driven,
        stream,
        paused,
        ..
    } = run.end
    else {
        panic!("driven");
    };
    assert_eq!((driven.ended, paused), (Ended::Exited, None));
    assert_eq!(stream.last_session_id.as_deref(), Some(native.as_str()));
    let _ = run.ledger.finish(LedgerOutcome::Success);
    // The spawned turn cleared the record itself (review fix 4: at the
    // spawn, not at admission).
    assert_eq!(chat_pause::get(vault, key).unwrap(), None);
}

/// Review fix 4 with the delta's ownership: a resume stopped before its
/// agent spawned leaves the chat paused and gives back its own claim (every
/// exit of `execute_turn` releases by the turn's token), so it can be
/// resumed again at once; another turn's token releases nothing.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_that_never_spawns_leaves_the_chat_paused() {
    let key = "s-20261115-p1a00007";
    let dir = chat_vault();
    let vault = dir.path();
    let ord = conversation_with_prior_turn(vault, key);
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        5
    ));
    let token = chat_pause::new_resume_token();
    let input = chat_pause::resume_input(vault, key, false, 10, &token).unwrap();
    assert_eq!(
        input.client_turn_id.as_deref(),
        Some(token.as_str()),
        "the owner rides the send"
    );
    let stop = StopSignal::new();
    stop.request();
    let run = drive_stub_limited(
        vault,
        key,
        ord,
        &ResumePlan::Resume(input.session_id.clone().unwrap()),
        &stop,
        |_: &agent::RunSpec| stub_command("exit 0"),
        &StubAccounts::two(),
        None,
        None,
        None,
        &[],
    )
    .await;
    assert!(matches!(run.end, AttemptsEnd::StoppedBeforeSpawn));
    assert!(
        chat_pause::get(vault, key).unwrap().is_some(),
        "still paused"
    );
    let refused = chat_pause::resume_input(vault, key, false, 11, "pause-resume-b").unwrap_err();
    assert!(refused.contains("already being resumed"), "{refused}");
    // A normal message's exit (no resume token) or another resume's token
    // never frees this claim.
    chat_pause::release_claim(vault, key, None);
    chat_pause::release_claim(vault, key, Some("ct-normal-send"));
    chat_pause::release_claim(vault, key, Some("pause-resume-someone-else"));
    assert!(chat_pause::claim_held(vault, key, &token));
    // What every exit of the resume's own turn does (`finish_observed_run`).
    chat_pause::release_claim(vault, key, Some(&token));
    assert!(!chat_pause::claim_held(vault, key, &token));
    assert!(chat_pause::resume_input(vault, key, false, 12, "pause-resume-c").is_ok());
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
    let compact = |body: &str| body.split_whitespace().collect::<String>();
    let finish = compact(fn_body(
        chat_source(),
        concat!("async fn ", "finish_observed_run("),
    ));
    assert!(finish.contains(
        "crate::chat_pause::release_claim_as(vault,session_key,out.tag.client_turn_id.as_deref(),"
    ));
}

/// Delta review 1: a queued resume row that ends without its turn (cancelled
/// from the queue, refused or failed at admission) gives back its own claim;
/// a normal message's row ending never does. `end_row`, `withdraw_queued`
/// and `cancel_queued_run` each release by the row.
#[test]
fn a_resume_row_that_ends_unstarted_releases_only_its_own_claim() {
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-p1a00012";
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        5
    ));
    let token = chat_pause::new_resume_token();
    chat_pause::claim_resume(vault, key, 10, &token).unwrap();
    let row = |client: &str| crate::run_queue::QueueRow {
        id: 1,
        source: crate::run_queue::RunSource::Chat,
        source_id: Some(key.to_string()),
        label: String::new(),
        reason: String::new(),
        payload: json!({ "clientTurnId": client }),
        status: crate::run_queue::RowStatus::Queued,
        coalesced_count: 0,
        coalesced_into: None,
        idempotency_key: None,
        priority: 0,
        attempt: 0,
        retry_of: None,
        requested_at: String::new(),
        not_before: None,
        claimed_at: None,
        finished_at: None,
        history_run_id: None,
        error: None,
        phase_timings: None,
        cancel_requested: false,
        held_reason: None,
        wait: None,
        detail: None,
    };
    chat_pause::release_for_row(vault, &row("ct-normal-send"), true);
    assert!(
        chat_pause::claim_held(vault, key, &token),
        "a normal message's row"
    );
    chat_pause::release_for_row(vault, &row(&token), true);
    assert!(!chat_pause::claim_held(vault, key, &token), "its own row");
    assert!(
        chat_pause::get(vault, key).unwrap().is_some(),
        "still paused"
    );

    let pump = include_str!("run_pump.rs");
    let end_row = fn_body(pump, "fn end_row(");
    let released = end_row
        .find("chat_pause::release_for_row(")
        .expect("end_row releases");
    assert!(
        released < end_row.find("run_queue::finish(").unwrap(),
        "before the payload goes"
    );
    let withdraw = fn_body(chat_source(), "pub(crate) fn withdraw_queued(");
    assert!(withdraw.contains("chat_pause::release_for_row(vault, &row, false)"));
    let api = include_str!("commands/run_queue_api.rs");
    assert!(api.contains("crate::chat_pause::release_for_row(vault, row, false);"));
}

/// Delta review 2: when another turn of the chat spawns first (the person's
/// own message clears the pause), a resume still in the queue must not run
/// `RESUME_PROMPT` as a second turn: admission ends it as `Superseded`, a
/// cancelled queued row with no turn.
#[test]
fn a_queued_resume_whose_pause_is_gone_never_runs() {
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-p1a00013";
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        5
    ));
    let token = chat_pause::new_resume_token();
    chat_pause::claim_resume(vault, key, 10, &token).unwrap();
    assert!(
        chat_pause::claim_held(vault, key, &token),
        "its claim holds"
    );
    // A normal message spawned first.
    chat_pause::on_turn_start(vault, key);
    assert!(
        !chat_pause::claim_held(vault, key, &token),
        "the pause is gone"
    );
    // Its claim lapsed and another resume took it: not this one's either.
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        20
    ));
    chat_pause::claim_resume(vault, key, 30, "pause-resume-other").unwrap();
    assert!(!chat_pause::claim_held(vault, key, &token));

    let admit = fn_body(include_str!("chat_queue.rs"), "fn admit_checked_turn(");
    let checked = admit
        .find("chat_pause::claim_held(")
        .expect("checked at admission");
    assert!(admit[checked..].contains("return ChatAdmission::Superseded;"));
    assert!(
        checked < admit.find("try_begin_owned").unwrap(),
        "before the lease is taken"
    );
    let pump = include_str!("run_pump.rs");
    let arm = pump
        .find("ChatAdmission::Superseded => end_unstarted_chat(")
        .expect("pump arm");
    let arm = &pump[arm..arm + 220];
    assert!(
        arm.contains("RowStatus::Cancelled") && arm.contains("QueuedEnd::Cancelled"),
        "{arm}"
    );
}

/// Review fix 2: two resumes at once (a double click, the window and
/// `sessions.resume`) claim the chat once; the other is refused in words.
/// A failed send gives the claim back, a claim Hollow lost lapses, and a
/// new pause starts unclaimed.
#[test]
fn concurrent_resumes_claim_the_chat_once() {
    let dir = chat_vault();
    let vault = dir.path().to_path_buf();
    let key = "s-20261115-p1a00008";
    assert!(chat_pause::record_turn_pause(
        &vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        5
    ));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let results: Vec<(String, Result<chat_pause::PausedChat, String>)> = (0..4)
        .map(|_| {
            let vault = vault.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                let token = chat_pause::new_resume_token();
                barrier.wait();
                let claimed = chat_pause::claim_resume(&vault, key, 100, &token);
                (token, claimed)
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let winners: Vec<&String> = results
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(t, _)| t)
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    for (_, refused) in results.iter().filter(|(_, r)| r.is_err()) {
        assert!(refused
            .as_ref()
            .unwrap_err()
            .contains("already being resumed"));
    }
    chat_pause::release_claim(&vault, key, Some(winners[0]));
    assert!(
        chat_pause::claim_resume(&vault, key, 101, "pause-resume-x").is_ok(),
        "given back"
    );
    assert!(chat_pause::claim_resume(&vault, key, 102, "pause-resume-y").is_err());
    assert!(
        chat_pause::claim_resume(&vault, key, 101 + 120, "pause-resume-z").is_ok(),
        "a lost claim lapses"
    );
    assert!(chat_pause::record_turn_pause(
        &vault,
        key,
        "claude",
        Some("id-fo-2"),
        "m",
        300
    ));
    assert!(
        chat_pause::claim_resume(&vault, key, 301, "pause-resume-w").is_ok(),
        "a new pause is unclaimed"
    );
    let gone =
        chat_pause::claim_resume(&vault, "s-20261115-p1a00009", 1, "pause-resume-v").unwrap_err();
    assert!(gone.contains("isn't paused"), "{gone}");
}

/// Review fix 1: a Stop that came before the paused turn's close wins: no
/// record, and the turn closes as stopped. One that lands after the record
/// was written clears it (`cancel` through `stop_clears_pause`). Both are
/// decided while the turn's registration still holds its slot.
#[test]
fn a_stop_after_the_pause_cut_still_wins() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-p1a00010";
    let registration = register_stop(key);
    let stop = registration.signal();
    assert!(request_pause_for(key));
    stop.request();
    assert!(settle_pause(vault, key, &stop, "claude", Some("id-fo-1"), "n", 5).is_none());
    assert_eq!(chat_pause::get(vault, key).unwrap(), None, "nothing paused");
    assert!(!chat_pause::pause_requested(key));
    drop(registration);

    let registration = register_stop(key);
    let stop = registration.signal();
    assert_eq!(
        settle_pause(vault, key, &stop, "claude", Some("id-fo-1"), "n", 5),
        Some(chat_pause::PauseReason::User)
    );
    assert!(chat_pause::get(vault, key).unwrap().is_some());
    // The Stop lands now: the slot still reaches the turn, and the record goes.
    assert!(request_stop_for(key));
    assert!(stop_clears_pause(vault, key));
    assert_eq!(chat_pause::get(vault, key).unwrap(), None);
    drop(registration);

    let turn = fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    );
    let settled = turn.find("settle_pause(").expect("settled");
    let dropped = turn.find("drop(registration);").expect("dropped");
    assert!(
        settled < dropped,
        "decided while the Stop still reaches the turn"
    );
    let compact: String = turn.split_whitespace().collect();
    assert!(compact.contains("ifletSome(reason)=pause_kept{TurnClose::paused("));
    assert!(compact.contains("}else{TurnClose::cancelled("));
    let cancel = fn_body(chat_source(), concat!("pub async fn ", "cancel("));
    let stopped = cancel.find("stop_running_turn(").unwrap();
    let cleared = cancel.find("stop_clears_pause(").unwrap();
    assert!(
        stopped < cleared,
        "the turn is signalled before the record goes"
    );
}

/// Review fix 3: a pause never shows over a Stop's Cancelling.
#[test]
fn a_pause_never_overwrites_cancelling() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let _live = crate::agent_sessions::LIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let key = "s-20261115-p1a00011";
    crate::agent_sessions::begin(
        key,
        AgentKind::Claude,
        "run-p1a-11",
        crate::agent_sessions::SessionPhase::Running,
    );
    crate::agent_sessions::mark_pausing(key);
    assert_eq!(
        crate::agent_sessions::live_for_key(key).unwrap().phase,
        "pausing"
    );
    crate::agent_sessions::set_phase_for_key(key, crate::agent_sessions::SessionPhase::Cancelling);
    crate::agent_sessions::mark_pausing(key);
    assert_eq!(
        crate::agent_sessions::live_for_key(key).unwrap().phase,
        "cancelling"
    );
    crate::agent_sessions::clear_run("run-p1a-11");
}

/// P1a: a turn that finishes before reaching any safe point (no tool call,
/// so none comes) just finishes: nothing is paused.
#[cfg(unix)]
#[tokio::test]
async fn a_turn_that_finishes_first_is_not_paused() {
    let key = "s-20261115-p1a00003";
    assert!(chat_pause::request_pause(key, true));
    let phases = vec![
        lines(&[FO_INIT, FO_WORKING]),
        lines(&[FO_DONE_TEXT, FO_DONE]),
    ];
    let (dir, run, spawns, _) =
        drive_guarded(key, &phases, FO_DONE, &StubAccounts::two(), None, RUN_PAUSE).await;
    chat_pause::clear_request(key);
    assert_eq!(spawns.len(), 1);
    let AttemptsEnd::Driven {
        driven,
        stream,
        paused,
        ..
    } = run.end
    else {
        panic!("driven");
    };
    assert_eq!((driven.ended, paused), (Ended::Exited, None));
    assert!(stream.assistant.ends_with("The plan is done."));
    assert!(chat_pause::list(dir.path()).unwrap().is_empty());
    let _ = run.ledger.finish(LedgerOutcome::Success);
}

/// P1a: a Stop pressed while the turn is pausing (asked, not yet at its safe
/// point) wins: the turn ends stopped, never paused.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_while_pausing_wins() {
    let key = "s-20261115-p1a00004";
    assert!(chat_pause::request_pause(key, true));
    let dir = chat_vault();
    let vault = dir.path();
    let fixtures = tempfile::tempdir().unwrap();
    let ord = conversation_with_prior_turn(vault, key);
    // The safe point comes 1.2 s in; the Stop lands at 0.3 s.
    let phases = vec![
        lines(&[FO_INIT, &tg_tool_use("tu-1", "msg-tg1", 5)]),
        lines(&[TG_SUB_REQUEST]),
        lines(&[TG_SUB_REQUEST]),
        lines(&[&tg_tool_result("tu-1")]),
        lines(&[TG_REQUEST]),
        lines(&[TG_NEVER, FO_DONE]),
    ];
    let script = phased_script(fixtures.path(), "first", &phases, 0, CUT_PAUSE);
    let stop = StopSignal::new();
    let accounts = StubAccounts::two();
    let started = std::time::Instant::now();
    let drive = drive_stub_limited(
        vault,
        key,
        ord,
        &ResumePlan::New,
        &stop,
        |_: &agent::RunSpec| stub_command(&script),
        &accounts,
        None,
        None,
        None,
        &[],
    );
    let stopper = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.request();
    };
    let (run, ()) = tokio::join!(drive, stopper);
    chat_pause::clear_request(key);
    assert!(started.elapsed() < Duration::from_secs(20), "no hang");
    match run.end {
        // Under a loaded machine the Stop can land before the spawn.
        AttemptsEnd::StoppedBeforeSpawn => {}
        AttemptsEnd::Driven { driven, paused, .. } => {
            assert_eq!((driven.ended, paused), (Ended::Stopped, None));
        }
        _ => panic!("stopped"),
    }
    assert!(!run.sink.text().contains("Still on the first account."));
    assert!(chat_pause::list(vault).unwrap().is_empty());
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
}

/// P1a: a pause reaches only a registered running turn, never one already
/// stopping, and ends with the turn's registration; a new turn of the same
/// chat never inherits an earlier turn's request.
#[test]
fn a_pause_reaches_only_a_registered_turn_and_ends_with_it() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let key = "s-20261115-p1a00005";
    assert!(!request_pause_for(key), "no running turn");
    assert!(!chat_pause::pause_requested(key));

    let registration = register_stop(key);
    assert!(request_pause_for(key));
    assert!(chat_pause::pause_requested(key));
    drop(registration);
    assert!(!chat_pause::pause_requested(key), "ends with its turn");

    let registration = register_stop(key);
    registration.signal().request();
    assert!(!request_pause_for(key), "a Stop wins");
    drop(registration);

    assert!(chat_pause::request_pause(key, true));
    let registration = register_stop(key);
    assert!(
        !chat_pause::pause_requested(key),
        "a stale request never reaches the next turn"
    );
    drop(registration);
}

/// P1a: a paused turn closes neutral (status `cancelled`, the `cancelled`
/// usage outcome, no error), keeps its native session, writes a `paused`
/// transcript event with the note and ends with the `paused` chat event.
#[test]
fn a_paused_turn_closes_neutral_with_a_paused_notice() {
    let key = "s-20261115-p1a00006";
    let close = TurnClose::paused(
        LedgerOutcome::Cancelled,
        "Was using Bash.".into(),
        key,
        Some("id-fo-1".into()),
    );
    assert_eq!(close.status, "cancelled");
    assert_eq!(close.outcome, LedgerOutcome::Cancelled);
    assert_eq!(close.error, None);
    assert_eq!(close.session_id.as_deref(), Some("id-fo-1"));
    assert_eq!(close.event, Some(("paused", "Was using Bash.".to_string())));
    let notice = close.notice.clone().expect("the paused event");
    assert_eq!(notice["type"], "paused");
    assert_eq!(notice["sessionKey"], key);
    assert_eq!(notice["note"], "Was using Bash.");
    assert!(notice["at"].as_i64().is_some_and(|at| at > 0));
    assert!(close.is_pause());
    let stopped = TurnClose::cancelled(LedgerOutcome::Cancelled, "ok", cancelled_event(None), None);
    assert!(!stopped.is_pause());

    // `execute_turn` closes a pause through these, and a pause leaves the
    // turn's child task turns running.
    let compact = |body: &str| body.split_whitespace().collect::<String>();
    let turn = compact(fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    ));
    assert!(turn.contains("settle_pause("));
    assert!(turn.contains(
        "TurnClose::paused(LedgerOutcome::Cancelled,note,&session_key,last_session_id,)"
    ));
    let finish = compact(fn_body(
        chat_source(),
        concat!("async fn ", "finish_observed_run("),
    ));
    assert!(finish.contains("status==\"cancelled\"&&!close.is_pause()"));
}

// ── P1b: Pause all ─────────────────────────────────────────────────────────

/// P1b: Pause all asks every registered turn that isn't stopping, as `all`,
/// never the caller's own chat or one already asked (its own Pause keeps
/// `user`) and never a queued row (it has no slot). The asked chats form one
/// batch: the one that pauses records reason `all` with its note, the one
/// whose turn ends first settles as finished, and the last to settle closes
/// the batch with one summary naming both.
#[test]
fn pause_all_asks_running_turns_and_summarises_once_all_settle() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    chat_pause::reset_batch();
    let dir = chat_vault();
    let vault = dir.path();
    let (a, b, stopping, own, mine, child) = (
        "s-20261115-p1b00001",
        "s-20261115-p1b00002",
        "s-20261115-p1b00003",
        "s-20261115-p1b00004",
        "s-20261115-p1b00005",
        "s-20261115-p1b00006",
    );
    for key in [a, b] {
        conversation_with_prior_turn(vault, key);
    }
    let reg_a = register_stop(a);
    let reg_b = register_stop(b);
    let reg_child = register_stop(child);
    let reg_stopping = register_stop(stopping);
    reg_stopping.signal().request();
    let reg_own = register_stop(own);
    let reg_mine = register_stop(mine);
    // Review fix: only a person's own chat is asked; `child` (a child task,
    // unattended, bare or coordinator turn) never is.
    for reg in [&reg_a, &reg_b, &reg_stopping, &reg_own, &reg_mine] {
        reg.set_own_chat(true);
    }
    assert!(request_pause_for(mine), "the person's own Pause first");

    let asked = request_pause_all(vault, Some(own));
    assert_eq!(asked, vec![a.to_string(), b.to_string()]);
    assert_eq!(
        chat_pause::requested_reason(a),
        Some(chat_pause::PauseReason::All)
    );
    assert_eq!(
        chat_pause::requested_reason(mine),
        Some(chat_pause::PauseReason::User),
        "a chat already asked keeps its reason"
    );
    assert!(!chat_pause::pause_requested(own), "never the caller's chat");
    assert!(!chat_pause::pause_requested(stopping), "a Stop wins");
    assert!(
        !chat_pause::pause_requested(child),
        "never a turn that isn't a person's own chat"
    );

    // A pauses at its safe point: recorded as `all`, with its note.
    let stop_a = reg_a.signal();
    assert_eq!(
        settle_pause(
            vault,
            a,
            &stop_a,
            "claude",
            Some("id-fo-1"),
            "Writing the plan.",
            9
        ),
        Some(chat_pause::PauseReason::All)
    );
    let record = chat_pause::get(vault, a).unwrap().expect("paused");
    assert_eq!(record.reason, chat_pause::PauseReason::All);
    assert_eq!(record.note, "Writing the plan.");
    drop(reg_a);
    // B finishes first: the batch closes on it with both chats.
    let summary = chat_pause::settle_in_batch(b, chat_pause::Settled::Ended).expect("closed");
    assert_eq!(
        summary.settled,
        vec![
            (
                a.to_string(),
                chat_pause::Settled::Paused("Writing the plan.".into())
            ),
            (b.to_string(), chat_pause::Settled::Ended),
        ]
    );
    let note = chat_pause::summary_notification(&summary);
    assert_eq!(note.title, "Paused 1 chat; 1 finished first");
    let lines: Vec<&str> = note.body.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with(": Writing the plan."), "{}", lines[0]);
    assert!(lines[1].ends_with(": finished before it reached a safe point."));
    assert_eq!(
        note.action_url.as_deref(),
        Some(chat_pause::MISSION_CONTROL_ACTION)
    );
    assert_eq!(note.kind, "info");
    assert!(
        chat_pause::settle_in_batch(b, chat_pause::Settled::Ended).is_none(),
        "one summary per batch"
    );
    drop(reg_b);
    drop(reg_stopping);
    drop(reg_child);
    drop(reg_own);
    drop(reg_mine);
    // Nothing running: nothing asked, no batch.
    assert!(request_pause_all(vault, None).is_empty());
    assert!(chat_pause::settle_in_batch(a, chat_pause::Settled::Ended).is_none());
}

/// P1b: the turn's registration settles a Pause all chat that ended without
/// pausing, and a turn replaced by a newer one of the same chat settles too,
/// so a batch never waits on a turn that is gone.
#[test]
fn a_pause_all_chat_whose_turn_ends_settles_as_finished() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    chat_pause::reset_batch();
    let dir = chat_vault();
    let vault = dir.path();
    let (a, b) = ("s-20261115-p1b00011", "s-20261115-p1b00012");
    let reg_a = register_stop(a);
    let reg_b = register_stop(b);
    reg_a.set_own_chat(true);
    reg_b.set_own_chat(true);
    assert_eq!(request_pause_all(vault, None).len(), 2);
    drop(reg_a);
    // B is replaced by a newer turn of the same chat: settled, and the
    // batch (now empty) closed, so a later settle of B returns nothing.
    let newer = register_stop(b);
    assert!(chat_pause::settle_in_batch(b, chat_pause::Settled::Ended).is_none());
    assert!(
        !chat_pause::pause_requested(b),
        "the newer turn isn't asked"
    );
    drop(reg_b);
    drop(newer);
}

/// P1b with a fake agent: a turn Pause all asked pauses at its safe point
/// exactly as one the person paused, and the settle records reason `all`.
#[cfg(unix)]
#[tokio::test]
async fn a_pause_all_turn_pauses_at_the_safe_point_as_all() {
    let key = "s-20261115-p1b00021";
    assert!(chat_pause::request_pause_as(
        key,
        true,
        chat_pause::PauseReason::All
    ));
    let (dir, run, spawns, _) = drive_guarded(
        key,
        &mid_call_phases(),
        &lines(&[FO_INIT, FO_DONE]),
        &StubAccounts::two(),
        None,
        CUT_PAUSE,
    )
    .await;
    let AttemptsEnd::Driven { paused, stream, .. } = run.end else {
        panic!("driven");
    };
    assert_eq!(spawns.len(), 1);
    let note = paused.expect("paused");
    assert_eq!(
        chat_pause::requested_reason(key),
        Some(chat_pause::PauseReason::All)
    );
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let registration = register_stop_keep_request(key);
    let reason = settle_pause(
        dir.path(),
        key,
        &registration.signal(),
        "claude",
        stream.last_session_id.as_deref(),
        &note,
        7,
    );
    assert_eq!(reason, Some(chat_pause::PauseReason::All));
    assert_eq!(
        chat_pause::get(dir.path(), key).unwrap().unwrap().reason,
        chat_pause::PauseReason::All
    );
    drop(registration);
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
}

/// A stop registration that keeps the pause asked before it (the real turn
/// registered first; this test drove the attempt loop alone).
fn register_stop_keep_request(key: &str) -> StopRegistration {
    let reason = chat_pause::requested_reason(key);
    let registration = register_stop(key);
    if let Some(reason) = reason {
        chat_pause::request_pause_as(key, true, reason);
    }
    registration
}

// ── P1c: Resume at a chosen time ───────────────────────────────────────────

/// P1c: a paused chat with a time comes due at it (and stays due after a
/// restart: the time lives in the session store, nothing in memory), is
/// taken once through Resume's claim, so neither the person's Resume nor a
/// second tick can resume it again while that claim lives, and a scheduled
/// resume that lost to a later claim never runs (admission's claim check).
/// The resumed turn's spawn clears the record and its time.
#[cfg(unix)]
#[tokio::test]
async fn a_paused_chat_resumes_at_its_time_once() {
    let key = "s-20261115-p1c00001";
    let dir = chat_vault();
    let vault = dir.path();
    let ord = conversation_with_prior_turn(vault, key);
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-fo-1"),
        "n",
        100
    ));
    // Not paused: no time is set.
    assert_eq!(
        chat_pause::set_resume_at(vault, "s-20261115-p1c00009", Some(500)).unwrap(),
        None
    );
    let record = chat_pause::set_resume_at(vault, key, Some(500))
        .unwrap()
        .unwrap();
    assert_eq!(record.resume_at, Some(500));
    assert!(chat_pause::due(vault, 499).unwrap().is_empty(), "not yet");
    // "After a restart": a fresh read of the store, at a later time.
    let due = chat_pause::due(vault, 10_000).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].session_key, key);

    // The tick claims it (as `commands::resume_due_paused` does).
    let tick = chat_pause::new_resume_token();
    let input = chat_pause::resume_input(vault, key, false, 10_000, &tick).unwrap();
    assert!(
        chat_pause::due(vault, 10_001).unwrap().is_empty(),
        "claimed"
    );
    let err = chat_pause::resume_input(vault, key, false, 10_002, "pause-resume-hand").unwrap_err();
    assert!(err.contains("already being resumed"), "{err}");
    assert!(
        !chat_pause::definitive_refusal(&err),
        "a busy refusal keeps the time"
    );
    assert!(chat_pause::definitive_refusal(
        "This chat no longer exists. Nothing started."
    ));
    assert!(chat_pause::definitive_refusal(
        "This chat isn't paused. Nothing started."
    ));
    assert!(!chat_pause::definitive_refusal("queue write failed"));
    // A claim that lapsed comes due again and is taken by a new owner; the
    // first resume, still queued, would then be refused at admission.
    assert_eq!(chat_pause::due(vault, 10_000 + 120).unwrap().len(), 1);
    let later = chat_pause::new_resume_token();
    chat_pause::resume_input(vault, key, false, 10_000 + 120, &later).unwrap();
    assert!(!chat_pause::claim_held(vault, key, &tick));
    assert!(chat_pause::claim_held(vault, key, &later));

    // The later resume's turn runs on the paused native session; its spawn
    // clears the record, so nothing is due any more.
    let fixtures = tempfile::tempdir().unwrap();
    let script = print_script(
        fixtures.path(),
        "resumed.jsonl",
        &lines(&[FO_INIT, FO_DONE_TEXT, FO_DONE]),
        false,
        0,
    );
    let stop = StopSignal::new();
    let run = drive_stub_limited(
        vault,
        key,
        ord,
        &ResumePlan::Resume(input.session_id.clone().unwrap()),
        &stop,
        |_: &agent::RunSpec| stub_command(&script),
        &StubAccounts::two(),
        None,
        None,
        None,
        &[],
    )
    .await;
    assert!(matches!(run.end, AttemptsEnd::Driven { .. }));
    let _ = run.ledger.finish(LedgerOutcome::Success);
    assert_eq!(chat_pause::get(vault, key).unwrap(), None);
    assert!(chat_pause::due(vault, 99_999).unwrap().is_empty());
}

/// P1c: clearing the time keeps the chat paused with none; a Stop clears
/// the whole record, time included.
#[test]
fn clearing_the_time_or_a_stop_ends_the_scheduled_resume() {
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-p1c00002";
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-1"),
        "n",
        1
    ));
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    let cleared = chat_pause::set_resume_at(vault, key, None)
        .unwrap()
        .unwrap();
    assert_eq!(cleared.resume_at, None);
    assert!(chat_pause::due(vault, 1_000).unwrap().is_empty());
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    assert!(stop_clears_pause(vault, key));
    assert!(chat_pause::due(vault, 1_000).unwrap().is_empty());
    assert_eq!(chat_pause::get(vault, key).unwrap(), None);
}

// ── P1d: Pause at a usage limit ────────────────────────────────────────────

/// The facts `execute_turn` passes for a person's own chat that ran out.
fn own_limit<'a>(
    every_account: bool,
    native: Option<&'a str>,
    reset_at: Option<i64>,
    reset_label: Option<&'a str>,
) -> chat_pause::LimitFacts<'a> {
    chat_pause::LimitFacts {
        ran_out: true,
        every_account,
        failed: true,
        stopped: false,
        continued_elsewhere: false,
        guard_stop: false,
        own_chat: true,
        native,
        agent_label: "Claude",
        credit: false,
        reset_at,
        reset_label,
    }
}

/// P1d with a fake agent: the provider says its usage limit is reached and
/// when it resets (one account, no failover). The turn's facts give that
/// reset, the chat pauses (reason `usage_limit`, its native session kept, a
/// note naming the limit and the time) with `resume_at` 60 s after it, so
/// the scheduler resumes it then; Resume works on it at once too.
#[cfg(unix)]
#[tokio::test]
async fn a_usage_limit_with_a_reset_time_pauses_until_it() {
    let key = "s-20261115-p1d00001";
    let now = chrono::Utc::now().timestamp();
    let resets = now + 3_600;
    let limited = format!(
        r#"{{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|{resets}","session_id":"id-fo-1","total_cost_usd":0,"usage":{{"input_tokens":0,"output_tokens":0}}}}"#
    );
    let first = lines(&[FO_INIT, &limited]);
    let single = StubAccounts {
        failover: false,
        ..StubAccounts::two()
    };
    let (dir, _ord, run, spawns) = drive_failover(key, &first, FO_DONE, 0, &single).await;
    let vault = dir.path();
    assert_eq!(spawns.len(), 1, "no failover");
    let AttemptsEnd::Driven {
        driven,
        stream,
        failover_stop,
        ..
    } = run.end
    else {
        panic!("driven");
    };
    assert_eq!(failover_stop, None);
    let record = stream.results.record();
    let kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent: AgentKind::Claude,
        ended: driven.ended,
        exit_success: false,
        result: record.as_ref(),
        stderr_tail: &driven.stderr_tail,
        budget_stop: false,
        stalled_minutes: None,
    });
    assert_eq!(kind, crate::result_kind::ResultKind::ProviderQuota);
    let outcome = exit_outcome(kind, record.as_ref(), false, true, || "exit 1".into());
    let said = record
        .as_ref()
        .and_then(crate::result_kind::ResultRecord::error_text);
    let reset_at = limit_reset_at(
        false,
        || Some(1),
        stream.last_quota.as_ref(),
        &[outcome.error.as_deref(), said.as_deref()],
        now,
    );
    assert_eq!(reset_at, Some(resets), "the provider's own reset");
    let pause = chat_pause::limit_pause(&own_limit(
        false,
        stream.last_session_id.as_deref(),
        reset_at,
        Some("16:40"),
    ))
    .expect("paused, not failed");
    assert_eq!(
        pause.resume_at,
        Some(resets + chat_pause::RESET_MARGIN_SECS)
    );
    assert_eq!(
        pause.note,
        "Claude hit its usage limit. It resets at 16:40; the chat resumes then."
    );
    assert!(chat_pause::record_turn_pause_as(
        vault,
        key,
        "claude",
        stream.last_session_id.as_deref(),
        &pause.note,
        now,
        chat_pause::PauseReason::UsageLimit,
        pause.resume_at,
        0,
    ));
    let paused = chat_pause::get(vault, key).unwrap().unwrap();
    assert_eq!(paused.reason, chat_pause::PauseReason::UsageLimit);
    assert_eq!(paused.native_session_id, "id-fo-1", "nothing lost");
    assert!(
        chat_pause::due(vault, resets).unwrap().is_empty(),
        "the margin"
    );
    assert_eq!(chat_pause::due(vault, resets + 60).unwrap().len(), 1);
    // The Resume button works before the reset too.
    let input = chat_pause::resume_input(vault, key, false, now, "pause-resume-d1").unwrap();
    assert_eq!(input.session_id.as_deref(), Some("id-fo-1"));
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
}

/// P1d with a fake agent: every Claude account is out (R1 stops with no
/// eligible account). The soonest account reset is the time; the note says
/// every account hit its limit.
#[cfg(unix)]
#[tokio::test]
async fn every_account_out_pauses_until_the_soonest_reset() {
    let key = "s-20261115-p1d00002";
    let soonest = chrono::Utc::now().timestamp() + 7_200;
    let accounts = StubAccounts {
        eligible: vec!["acct-admin"],
        soonest: Some(soonest),
        ..StubAccounts::two()
    };
    let exhausted = lines(&[FO_INIT, FO_REJECTED, FO_LIMITED]);
    let (dir, _ord, run, _spawns) = drive_failover(key, &exhausted, FO_DONE, 0, &accounts).await;
    let AttemptsEnd::Driven {
        stream,
        failover_stop,
        ..
    } = run.end
    else {
        panic!("driven");
    };
    assert!(failover_stop.is_some(), "R1 stopped");
    let reset_at = limit_reset_at(
        true,
        || accounts.soonest_room(),
        stream.last_quota.as_ref(),
        &[],
        0,
    );
    assert_eq!(reset_at, Some(soonest));
    let pause = chat_pause::limit_pause(&own_limit(
        true,
        stream.last_session_id.as_deref(),
        reset_at,
        Some("Fri 01:20"),
    ))
    .unwrap();
    assert_eq!(
        pause.note,
        "Every Claude account hit its usage limit. It resets at Fri 01:20; the chat resumes then."
    );
    assert_eq!(pause.resume_at, Some(soonest + 60));
    assert!(
        chat_pause::list(dir.path()).unwrap().is_empty(),
        "the loop records nothing"
    );
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
}

/// P1d with a fake agent: a limit with no reset time anywhere (no
/// rate-limit event, no time in the provider's words) pauses with no time.
#[cfg(unix)]
#[tokio::test]
async fn a_usage_limit_without_a_reset_time_pauses_with_none() {
    let key = "s-20261115-p1d00003";
    let limited = r#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached","session_id":"id-fo-1","total_cost_usd":0,"usage":{"input_tokens":0,"output_tokens":0}}"#;
    let single = StubAccounts {
        failover: false,
        ..StubAccounts::two()
    };
    let (_dir, _ord, run, _) =
        drive_failover(key, &lines(&[FO_INIT, limited]), FO_DONE, 0, &single).await;
    let AttemptsEnd::Driven { driven, stream, .. } = run.end else {
        panic!("driven");
    };
    let record = stream.results.record();
    let kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent: AgentKind::Claude,
        ended: driven.ended,
        exit_success: false,
        result: record.as_ref(),
        stderr_tail: &driven.stderr_tail,
        budget_stop: false,
        stalled_minutes: None,
    });
    assert_eq!(kind, crate::result_kind::ResultKind::ProviderQuota);
    let outcome = exit_outcome(kind, record.as_ref(), false, true, || "exit 1".into());
    let now = chrono::Utc::now().timestamp();
    let said = record
        .as_ref()
        .and_then(crate::result_kind::ResultRecord::error_text);
    let reset_at = limit_reset_at(
        false,
        || None,
        stream.last_quota.as_ref(),
        &[outcome.error.as_deref(), said.as_deref()],
        now,
    );
    assert_eq!(reset_at, None);
    let pause = chat_pause::limit_pause(&own_limit(
        false,
        stream.last_session_id.as_deref(),
        None,
        None,
    ))
    .unwrap();
    assert_eq!(pause.resume_at, None);
    assert_eq!(
        pause.note,
        "Claude hit its usage limit and gave no reset time. Resume it when there is room."
    );
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
}

/// P1d: what keeps the old failure instead of a pause: a Stop, a guard, a
/// move to another agent, a child or unattended turn, no native session, a
/// turn that didn't fail or didn't run out.
#[test]
fn only_a_persons_own_failed_limit_turn_pauses() {
    let base = own_limit(false, Some("id-1"), Some(100), Some("01:20"));
    assert!(chat_pause::limit_pause(&base).is_some());
    let refused = [
        chat_pause::LimitFacts {
            ran_out: false,
            ..base
        },
        chat_pause::LimitFacts {
            failed: false,
            ..base
        },
        chat_pause::LimitFacts {
            stopped: true,
            ..base
        },
        chat_pause::LimitFacts {
            continued_elsewhere: true,
            ..base
        },
        chat_pause::LimitFacts {
            guard_stop: true,
            ..base
        },
        chat_pause::LimitFacts {
            own_chat: false,
            ..base
        },
        chat_pause::LimitFacts {
            native: None,
            ..base
        },
        chat_pause::LimitFacts {
            native: Some(" "),
            ..base
        },
        chat_pause::LimitFacts {
            credit: true,
            ..base
        },
    ];
    for facts in refused {
        assert!(chat_pause::limit_pause(&facts).is_none(), "{facts:?}");
    }
}

/// P1d: `execute_turn` pauses a limit turn through `limit_pause`, records
/// it as `usage_limit` with its time and closes it as a P1a pause.
#[test]
fn execute_turn_closes_a_limit_turn_as_a_pause() {
    let compact = |body: &str| body.split_whitespace().collect::<String>();
    let turn = compact(fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    ));
    assert!(turn.contains("crate::chat_pause::limit_pause(&crate::chat_pause::LimitFacts{"));
    assert!(turn.contains(
        "letown_chat=app_depth==0&&!unattended&&!bare&&!crate::app_sessions::is_coordinator("
    ));
    assert!(turn.contains("registration.set_own_chat(own_chat);"));
    assert!(
        turn.contains(".paused_for(crate::chat_pause::PauseReason::UsageLimit,limit.resume_at)")
    );
    // The pause is decided before the error is emitted or stored.
    let pause_at = turn.find("limit_pause(").unwrap();
    assert!(pause_at < turn.find("exit_error_records(").unwrap());
}

/// P1b to P1d: the `paused` event says why and when it resumes on its own;
/// a plain pause says `user` and no time. The first ask's reason holds.
#[test]
fn the_paused_event_carries_its_reason_and_time() {
    let key = "s-20261115-p1d00010";
    let plain = TurnClose::paused(LedgerOutcome::Cancelled, "n".into(), key, None);
    let notice = plain.notice.clone().unwrap();
    assert_eq!(notice["reason"], "user");
    assert!(notice["resumeAt"].is_null());
    let limit = TurnClose::paused(LedgerOutcome::Cancelled, "n".into(), key, None)
        .paused_for(chat_pause::PauseReason::UsageLimit, Some(1_900_000_000));
    let notice = limit.notice.clone().unwrap();
    assert_eq!(notice["reason"], "usage_limit");
    assert_eq!(notice["resumeAt"], 1_900_000_000_i64);
    assert!(limit.is_pause());

    let asked = "s-20261115-p1d00011";
    assert!(chat_pause::request_pause_as(
        asked,
        true,
        chat_pause::PauseReason::User
    ));
    assert!(chat_pause::request_pause_as(
        asked,
        true,
        chat_pause::PauseReason::All
    ));
    assert_eq!(
        chat_pause::requested_reason(asked),
        Some(chat_pause::PauseReason::User)
    );
    chat_pause::clear_request(asked);
    assert_eq!(chat_pause::requested_reason(asked), None);
}

// ── Review fixes (Sonnet, 2026-11-15) ──────────────────────────────────────

/// Items 1 and 2: an automatic resume's token names its attempt and its
/// claim counts it. Released without a spawn (withdrawn, refused, failed
/// before start), it drops the time so a past time never re-fires; a
/// transient send failure keeps the time for the next tick until the cap,
/// then drops it. A person's own Resume token releases nothing but its claim.
#[test]
fn automatic_resumes_drop_their_time_on_release_and_stop_at_the_cap() {
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-rvw00001";
    conversation_with_prior_turn(vault, key);
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-1"),
        "n",
        1
    ));
    assert_eq!(chat_pause::auto_attempt("pause-resume-0123"), None);
    let first = chat_pause::new_auto_token(1);
    assert_eq!(chat_pause::auto_attempt(&first), Some(1));
    assert!(chat_pause::is_resume_token(Some(&first)));

    // Released before a spawn: the time goes, the chat stays paused.
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    chat_pause::resume_input(vault, key, false, 100, &first).unwrap();
    assert_eq!(
        chat_pause::get(vault, key).unwrap().unwrap().auto_attempts,
        1
    );
    chat_pause::release_claim(vault, key, Some(&first));
    let record = chat_pause::get(vault, key).unwrap().expect("still paused");
    assert_eq!(record.resume_at, None);
    assert!(
        chat_pause::due(vault, 1_000).unwrap().is_empty(),
        "never re-fires"
    );

    // A person's Resume keeps the time when released.
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    chat_pause::resume_input(vault, key, false, 300, "pause-resume-hand").unwrap();
    chat_pause::release_claim(vault, key, Some("pause-resume-hand"));
    assert_eq!(
        chat_pause::get(vault, key).unwrap().unwrap().resume_at,
        Some(50)
    );

    // Transient failures retry until the cap, then the time goes.
    let second = chat_pause::new_auto_token(2);
    chat_pause::resume_input(vault, key, false, 500, &second).unwrap();
    assert!(!chat_pause::release_for_retry(vault, key, &second));
    let record = chat_pause::get(vault, key).unwrap().unwrap();
    assert_eq!((record.resume_at, record.auto_attempts), (Some(50), 2));
    assert_eq!(
        chat_pause::due(vault, 600).unwrap().len(),
        1,
        "retried next tick"
    );
    let third = chat_pause::new_auto_token(chat_pause::MAX_AUTO_ATTEMPTS);
    chat_pause::resume_input(vault, key, false, 700, &third).unwrap();
    assert!(chat_pause::release_for_retry(vault, key, &third), "gave up");
    let record = chat_pause::get(vault, key).unwrap().expect("still paused");
    assert_eq!(record.resume_at, None);
    let (title, body) = chat_pause::stays_paused_text("Plan", &chat_pause::gave_up_why());
    assert_eq!(title, "“Plan” stays paused");
    assert!(
        body.contains("3 times") && body.ends_with("Resume it from Running."),
        "{body}"
    );
}

/// Item 4: Resume all leaves a chat whose resume time is still ahead (a
/// limit pause waiting for its reset) and lists it as skipped.
#[tokio::test]
async fn resume_all_skips_a_chat_whose_time_is_ahead() {
    let dir = chat_vault();
    let vault = dir.path();
    let (waiting, gone) = ("s-20261115-rvw00011", "s-20261115-rvw00012");
    assert!(chat_pause::record_turn_pause(
        vault,
        waiting,
        "claude",
        Some("id-1"),
        "n",
        1
    ));
    assert!(chat_pause::record_turn_pause(
        vault,
        gone,
        "claude",
        Some("id-2"),
        "n",
        2
    ));
    let ahead = chrono::Utc::now().timestamp() + 3_600;
    chat_pause::set_resume_at(vault, waiting, Some(ahead)).unwrap();
    let mut config = crate::config::Config::placeholder();
    config.vault_path = vault.to_path_buf();
    let outcome = crate::commands::resume_all_paused(None, &config, None)
        .await
        .unwrap();
    assert_eq!(outcome.skipped, vec![waiting.to_string()]);
    assert!(outcome.resumed.is_empty());
    assert_eq!(
        outcome.refused.len(),
        1,
        "no conversation row: refused in words"
    );
    assert_eq!(outcome.refused[0].0, gone);
    assert!(chat_pause::get(vault, waiting).unwrap().is_some());
}

/// Item 5: a limit pause is recorded only while no Stop came, under the
/// slot lock with the turn's registration still held; `execute_turn`
/// drops the registration only after that settle.
#[test]
fn a_stop_beats_a_limit_pause() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-rvw00021";
    let limit = chat_pause::LimitPause {
        note: "Claude hit its usage limit.".into(),
        resume_at: Some(900),
    };
    let registration = register_stop(key);
    let stop = registration.signal();
    stop.request();
    assert_eq!(
        settle_limit_pause(vault, key, &stop, "claude", Some("id-1"), &limit, 0, 5),
        Some(false)
    );
    assert_eq!(chat_pause::get(vault, key).unwrap(), None, "nothing paused");
    drop(registration);
    let registration = register_stop(key);
    let stop = registration.signal();
    assert_eq!(
        settle_limit_pause(vault, key, &stop, "claude", Some("id-1"), &limit, 2, 5),
        Some(true)
    );
    let record = chat_pause::get(vault, key).unwrap().unwrap();
    assert_eq!(
        (record.reason, record.resume_at, record.auto_attempts),
        (chat_pause::PauseReason::UsageLimit, Some(900), 2)
    );
    drop(registration);

    let turn: String = fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    )
    .split_whitespace()
    .collect();
    let settled = turn.find("settle_limit_pause(").expect("settled");
    let dropped = turn.rfind("drop(registration);").expect("dropped");
    assert!(
        settled < dropped,
        "decided while the Stop still reaches the turn"
    );
}

/// Item 7: the limit pause's close writes the turn's usage row: it goes
/// through `finish_observed_run` with the `cancelled` usage outcome, the
/// funnel that finishes the ledger row (`every_post_lease_chat_exit_
/// finishes_its_usage_row` pins the funnel itself).
#[test]
fn a_limit_pause_writes_its_usage_row() {
    let turn: String = fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    )
    .split_whitespace()
    .collect();
    let settled = turn.find("settle_limit_pause(").unwrap();
    let close = &turn[settled..];
    let call = close
        .find(concat!("returnfinish_", "observed_run("))
        .unwrap();
    let args = &close[call..call + close[call..].find(".await").unwrap()];
    assert!(args.contains("ledger,live,"), "{args}");
    assert!(args.contains("TurnClose::paused(LedgerOutcome::Cancelled,limit.note,"));
    assert!(
        args.contains("TurnClose::cancelled(LedgerOutcome::Cancelled,"),
        "a Stop's close too"
    );
}

/// Item 7: a limit hit right after an automatic resume, before any reply or
/// tool, counts that attempt; at the cap the pause keeps no time. A turn
/// that got going starts again from 0.
#[test]
fn a_limit_right_after_an_auto_resume_counts_and_stops_at_the_cap() {
    let limit = chat_pause::LimitPause {
        note: "n".into(),
        resume_at: Some(100),
    };
    let two = chat_pause::new_auto_token(2);
    assert_eq!(
        chat_pause::count_auto_attempt(limit.clone(), "Claude", Some(&two), false),
        (limit.clone(), 2, false)
    );
    assert_eq!(
        chat_pause::count_auto_attempt(limit.clone(), "Claude", Some(&two), true),
        (limit.clone(), 0, false),
        "it got going: the count resets"
    );
    assert_eq!(
        chat_pause::count_auto_attempt(limit.clone(), "Claude", Some("pause-resume-hand"), false).1,
        0
    );
    let last = chat_pause::new_auto_token(chat_pause::MAX_AUTO_ATTEMPTS);
    let (paused, attempts, gave_up) =
        chat_pause::count_auto_attempt(limit, "Claude", Some(&last), false);
    assert!(gave_up);
    assert_eq!((attempts, paused.resume_at), (3, None));
    assert_eq!(
        paused.note,
        "Claude hit its usage limit again right after resuming 3 times. Resume it when there is room."
    );
    // A due chat already at the cap is not resumed again by the scheduler
    // (`commands::resume_due_paused` checks `auto_attempts + 1`).
    let body: String = fn_body(
        include_str!("commands.rs"),
        concat!("pub(crate) async fn ", "resume_due_paused("),
    )
    .split_whitespace()
    .collect();
    assert!(body.contains("ifattempt>chat_pause::MAX_AUTO_ATTEMPTS{"));
    assert!(body.contains("chat_pause::notify_resumed(vault,&chat);"));
}

/// Item 6 with a fake agent: credit exhaustion classifies as a usage limit
/// (`ProviderQuota`) but is never paused: no reset reopens it, so the turn
/// keeps its old failure with the provider's words.
#[cfg(unix)]
#[tokio::test]
async fn credit_exhaustion_fails_instead_of_pausing() {
    let key = "s-20261115-rvw00031";
    let broke = r#"{"type":"result","subtype":"success","is_error":true,"result":"Your credit balance is too low to access the Anthropic API.","session_id":"id-fo-1","total_cost_usd":0,"usage":{"input_tokens":0,"output_tokens":0}}"#;
    let single = StubAccounts {
        failover: false,
        ..StubAccounts::two()
    };
    let (_dir, _ord, run, _) =
        drive_failover(key, &lines(&[FO_INIT, broke]), FO_DONE, 0, &single).await;
    let AttemptsEnd::Driven { driven, stream, .. } = run.end else {
        panic!("driven");
    };
    let record = stream.results.record();
    let kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent: AgentKind::Claude,
        ended: driven.ended,
        exit_success: false,
        result: record.as_ref(),
        stderr_tail: &driven.stderr_tail,
        budget_stop: false,
        stalled_minutes: None,
    });
    assert_eq!(kind, crate::result_kind::ResultKind::ProviderQuota);
    let said = record
        .as_ref()
        .and_then(crate::result_kind::ResultRecord::error_text)
        .unwrap();
    assert!(crate::result_kind::is_credit_exhaustion(&said));
    let facts = chat_pause::LimitFacts {
        credit: true,
        ..own_limit(false, stream.last_session_id.as_deref(), None, None)
    };
    assert_eq!(chat_pause::limit_pause(&facts), None);
    for text in [
        "out of credits",
        "Billing hard limit reached",
        "insufficient_quota",
    ] {
        assert!(crate::result_kind::is_credit_exhaustion(text), "{text}");
    }
    for text in [
        "Claude AI usage limit reached|1790374200",
        "You've hit your weekly limit",
    ] {
        assert!(!crate::result_kind::is_credit_exhaustion(text), "{text}");
    }
    let _ = run.ledger.finish(LedgerOutcome::Error);
}

// ── Delta review (Sonnet, 2026-11-15) ──────────────────────────────────────

/// Delta 1: a new resume time starts the automatic attempts again, and a
/// claim the person ended (a Stop, a cancel from the queue) clears the time
/// quietly, as a failure does with its warning.
#[test]
fn a_new_time_resets_attempts_and_a_persons_stop_clears_the_time() {
    let dir = chat_vault();
    let vault = dir.path();
    let key = "s-20261115-dlt00001";
    conversation_with_prior_turn(vault, key);
    assert!(chat_pause::record_turn_pause(
        vault,
        key,
        "claude",
        Some("id-1"),
        "n",
        1
    ));
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    let two = chat_pause::new_auto_token(2);
    chat_pause::resume_input(vault, key, false, 100, &two).unwrap();
    chat_pause::release_for_retry(vault, key, &two);
    assert_eq!(
        chat_pause::get(vault, key).unwrap().unwrap().auto_attempts,
        2
    );
    let record = chat_pause::set_resume_at(vault, key, Some(900))
        .unwrap()
        .unwrap();
    assert_eq!(record.auto_attempts, 0, "a new time starts again");
    let cleared = chat_pause::set_resume_at(vault, key, None)
        .unwrap()
        .unwrap();
    assert_eq!(cleared.resume_at, None);
    // A Stop of the queued automatic resume: quiet, time cleared.
    chat_pause::set_resume_at(vault, key, Some(50)).unwrap();
    let one = chat_pause::new_auto_token(1);
    chat_pause::resume_input(vault, key, false, 300, &one).unwrap();
    chat_pause::release_claim_as(vault, key, Some(&one), false);
    let record = chat_pause::get(vault, key).unwrap().expect("still paused");
    assert_eq!(record.resume_at, None);
    assert!(
        chat_pause::claim_resume(vault, key, 301, "pause-resume-x").is_ok(),
        "claim freed"
    );
    // The close of a stopped resume releases quietly; a failure warns.
    let finish: String = fn_body(chat_source(), concat!("async fn ", "finish_observed_run("))
        .split_whitespace()
        .collect();
    assert!(finish.contains("status!=\"cancelled\","));
}

/// Delta 2 with a fake agent: a provider that answers every automatic
/// resume with its limit text as an API-error assistant message never
/// counts as progress, so the cap of 3 is reached; a real reply does.
#[cfg(unix)]
#[tokio::test]
async fn a_limit_text_reply_is_not_progress_so_the_cap_triggers() {
    let error = r#"{"type":"assistant","is_api_error_message":true,"api_error_status":429,"message":{"content":[{"type":"text","text":"Claude AI usage limit reached|1790374200"}]},"session_id":"id-fo-1"}"#;
    let limited = r#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1790374200","session_id":"id-fo-1","total_cost_usd":0,"usage":{"input_tokens":0,"output_tokens":0}}"#;
    let single = StubAccounts {
        failover: false,
        ..StubAccounts::two()
    };
    let mut gave_up = false;
    for attempt in 1..=chat_pause::MAX_AUTO_ATTEMPTS {
        let key = format!("s-20261115-dlt0001{attempt}");
        let (_dir, _ord, run, _) = drive_failover(
            &key,
            &lines(&[FO_INIT, error, limited]),
            FO_DONE,
            0,
            &single,
        )
        .await;
        let AttemptsEnd::Driven { stream, .. } = run.end else {
            panic!("driven");
        };
        assert!(
            stream.assistant.contains("usage limit reached"),
            "the text is there"
        );
        assert!(!stream.model_replied, "but it is not the model's reply");
        let token = chat_pause::new_auto_token(attempt);
        let limit = chat_pause::LimitPause {
            note: "n".into(),
            resume_at: Some(100),
        };
        let (_, attempts, done) =
            chat_pause::count_auto_attempt(limit, "Claude", Some(&token), stream.model_replied);
        assert_eq!(attempts, attempt);
        gave_up = done;
        let _ = run.ledger.finish(LedgerOutcome::Cancelled);
    }
    assert!(gave_up, "the third resume that only hit the limit gives up");
    let (_dir, _ord, run, _) = drive_failover(
        "s-20261115-dlt00019",
        &lines(&[FO_INIT, FO_WORKING, limited]),
        FO_DONE,
        0,
        &single,
    )
    .await;
    let AttemptsEnd::Driven { stream, .. } = run.end else {
        panic!("driven");
    };
    assert!(stream.model_replied, "a real reply is progress");
    let _ = run.ledger.finish(LedgerOutcome::Cancelled);
    let turn: String = fn_body(
        chat_source(),
        concat!("pub(crate) async fn ", "execute_turn("),
    )
    .split_whitespace()
    .collect();
    assert!(turn
        .contains("letprogressed=evidence.tool_calls_seen().is_some_and(|n|n>0)||model_replied;"));
}

/// Delta 3: every credit or billing phrase keeps the old failure.
#[test]
fn credit_and_billing_phrases_never_pause() {
    for text in [
        "Your credit balance is too low",
        "credit balance too low",
        "insufficient credits",
        "402 Payment Required",
        "payment required",
        "You reached your monthly spend limit",
        "Monthly spending-limit reached",
        "Account spend-limit exceeded",
        "spend limit reached",
        "Quota exceeded for this project",
        "Billing hard limit has been reached",
    ] {
        assert!(crate::result_kind::is_credit_exhaustion(text), "{text}");
    }
    for text in [
        "Claude AI usage limit reached|1790374200",
        "You've hit your session limit",
        "5-hour limit reached",
    ] {
        assert!(!crate::result_kind::is_credit_exhaustion(text), "{text}");
    }
}

/// Delta 4: a chat Pause all asked that paused at its usage limit settles
/// as paused, with the limit's note, in the summary.
#[test]
fn a_pause_all_chat_that_limit_pauses_settles_as_paused() {
    let _turns = TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    chat_pause::reset_batch();
    let dir = chat_vault();
    let vault = dir.path();
    let (a, b) = ("s-20261115-dlt00031", "s-20261115-dlt00032");
    let reg_a = register_stop(a);
    let reg_b = register_stop(b);
    reg_a.set_own_chat(true);
    reg_b.set_own_chat(true);
    assert_eq!(request_pause_all(vault, None).len(), 2);
    let limit = chat_pause::LimitPause {
        note: "Claude hit its usage limit.".into(),
        resume_at: None,
    };
    assert_eq!(
        settle_limit_pause(
            vault,
            a,
            &reg_a.signal(),
            "claude",
            Some("id-1"),
            &limit,
            0,
            5
        ),
        Some(true)
    );
    drop(reg_a);
    let summary = chat_pause::settle_in_batch(b, chat_pause::Settled::Ended).expect("closed");
    assert_eq!(
        summary.settled[0],
        (
            a.to_string(),
            chat_pause::Settled::Paused("Claude hit its usage limit.".into())
        )
    );
    drop(reg_b);
}
