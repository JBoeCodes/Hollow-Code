use super::*;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::test]
async fn an_unqualified_codex_never_advertises_native_steering() {
    // Tests must never probe an installed binary to qualify it.
    let table = support_table().await;
    let codex = table.iter().find(|row| row.agent == "codex").unwrap();
    assert_eq!(codex.support, SteerSupport::None);
    assert!(codex.note.contains("Agent updates"));
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-10-21T17:42:05Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[test]
fn codex_transport_qualification_requires_a_complete_stable_floor() {
    for version in [
        None,
        Some(""),
        Some("0.162.9"),
        Some("0.163"),
        Some("0.163.0-alpha.1"),
        Some("0.170.0-beta"),
        Some("0.163.0+build"),
        Some("codex-cli 0.163.0"),
        Some("0.163.0.1"),
        Some("0.163.-1"),
    ] {
        let row = support_info(AgentKind::Codex, version);
        assert_eq!(row.support, SteerSupport::None, "{version:?}");
        assert!(row.note.contains("Agent updates"));
    }
    for version in ["0.163.0", "0.163.1", "0.170.0", "1.0.0"] {
        assert_eq!(
            support_for(AgentKind::Codex, Some(version)),
            SteerSupport::Native
        );
    }
}

#[test]
fn admission_retains_each_turns_resolved_transport() {
    let old = support_for(AgentKind::Codex, Some("0.162.1"));
    let native = support_for(AgentKind::Codex, Some("0.163.0"));
    let _old = open(
        "s-st-old-version",
        "old",
        (AgentKind::Codex, old),
        1,
        PathBuf::new(),
        Map::new(),
    );
    let _native = open(
        "s-st-qualified",
        "native",
        (AgentKind::Codex, native),
        1,
        PathBuf::new(),
        Map::new(),
    );
    // Reading a different current version cannot alter either running turn.
    assert_eq!(
        support_for(AgentKind::Codex, Some("0.170.0")),
        SteerSupport::Native
    );
    assert_eq!(
        steer("s-st-old-version", "hello", "id-old", now()).unwrap_err(),
        STEER_UNSUPPORTED
    );
    assert_eq!(support_for(AgentKind::Codex, None), SteerSupport::None);
    assert!(steer("s-st-qualified", "hello", "id-new", now()).is_ok());
}

fn open_claude(key: &str, run: &str) -> LiveGuard {
    open(
        key,
        run,
        (AgentKind::Claude, SteerSupport::Native),
        3,
        PathBuf::from("/tmp/vault"),
        Map::new(),
    )
}

/// Accept, then publish (the session store write between them is chat's).
fn steer_pub(key: &str, text: &str, id: &str) -> Accepted {
    let accepted = steer(key, text, id, now()).unwrap();
    publish(key, id).unwrap();
    accepted
}

fn lifecycle(id: &str, state: &str) -> String {
    json!({ "type": "command_lifecycle", "command_uuid": id, "state": state }).to_string()
}

#[test]
fn claude_and_codex_have_a_true_steer_on_hollows_transport() {
    // Steering PR 3: Codex chat turns run `codex app-server` (`turn/steer`).
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        assert_eq!(
            support_for(agent, Some("0.163.0")),
            SteerSupport::Native,
            "{agent:?}"
        );
    }
    for agent in [AgentKind::Grok, AgentKind::Antigravity] {
        assert_eq!(
            support_for(agent, Some("0.163.0")),
            SteerSupport::None,
            "{agent:?}"
        );
    }
    let table: Vec<_> = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Grok,
        AgentKind::Antigravity,
    ]
    .into_iter()
    .map(|agent| support_info(agent, Some("0.163.0")))
    .collect();
    assert_eq!(table.len(), 4);
    let codex = table.iter().find(|row| row.agent == "codex").unwrap();
    assert!(codex.note.contains("at its next step"), "{}", codex.note);
    let grok = table.iter().find(|row| row.agent == "grok").unwrap();
    assert!(
        grok.note.contains("can't take a message mid-turn"),
        "{}",
        grok.note
    );
}

#[test]
fn a_steer_needs_a_running_turn_of_an_agent_that_can_take_one() {
    assert_eq!(
        steer("s-st-none", "hi", "id-1", now()).unwrap_err(),
        STEER_NOT_RUNNING
    );
    let _grok = open(
        "s-st-grok",
        "run-c",
        (AgentKind::Grok, SteerSupport::None),
        1,
        PathBuf::from("/tmp/vault"),
        Map::new(),
    );
    assert_eq!(
        steer("s-st-grok", "hi", "id-2", now()).unwrap_err(),
        STEER_UNSUPPORTED
    );
    let _claude = open_claude("s-st-empty", "run-e");
    assert_eq!(
        steer("s-st-empty", "   ", "id-3", now()).unwrap_err(),
        STEER_INVALID
    );
}

/// Edge case: a tool is running (or an approval card holds it). The steer is
/// written at once and stays `sent` until the agent says it took it.
#[test]
fn a_steer_while_a_tool_runs_is_written_and_waits_for_the_next_step() {
    let key = "s-st-tool";
    let _live = open_claude(key, "run-t");
    let (tx, mut rx) = unbounded_channel();
    assert_eq!(attach(key, "run-t", tx), Some(1));
    note_tool(key, "run-t");
    let accepted = steer(key, "  also check the tests ", "id-t", now()).unwrap();
    // Nothing reaches the agent before the record is stored.
    assert!(rx.try_recv().is_err());
    publish(key, "id-t").unwrap();
    assert_eq!(accepted.record.text, "also check the tests");
    assert_eq!(accepted.record.after_tools, 1);
    assert_eq!(accepted.record.state, SteerState::Sent);
    assert_eq!(accepted.record.at, "2026-10-21T17:42:05Z");
    assert_eq!(accepted.turn_ord, 3);
    let PipeMsg::Line(line) = rx.try_recv().unwrap() else {
        panic!("a line");
    };
    let written: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(written["type"], "user");
    assert_eq!(written["uuid"], "id-t");
    assert_eq!(
        written["message"]["content"][0]["text"],
        "also check the tests"
    );
    // Queued is not yet taken; another uuid is not ours.
    assert!(observe_line(key, "run-t", &lifecycle("id-t", "queued")).is_empty());
    assert!(observe_line(key, "run-t", &lifecycle("first", "started")).is_empty());
    let moved = observe_line(key, "run-t", &lifecycle("id-t", "started"));
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].state, SteerState::Delivered);
    // Completed after started moves nothing more.
    assert!(observe_line(key, "run-t", &lifecycle("id-t", "completed")).is_empty());
}

/// Edge case: two quick steers. Both written in order, both tracked.
#[test]
fn two_quick_steers_are_both_written_and_both_delivered() {
    let key = "s-st-two";
    let _live = open_claude(key, "run-2");
    let (tx, mut rx) = unbounded_channel();
    attach(key, "run-2", tx).unwrap();
    steer_pub(key, "first", "id-a");
    steer_pub(key, "second", "id-b");
    let lines: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(lines.len(), 2);
    assert!(matches!(&lines[0], PipeMsg::Line(l) if l.contains("id-a")));
    assert!(matches!(&lines[1], PipeMsg::Line(l) if l.contains("id-b")));
    assert!(matches!(
        on_result(key, "run-2", 0),
        AfterResult::Waiting(_)
    ));
    observe_line(key, "run-2", &lifecycle("id-a", "started"));
    observe_line(key, "run-2", &lifecycle("id-b", "started"));
    let (all, moved) = settle(key, "run-2");
    assert!(moved.is_empty());
    assert!(all.iter().all(|s| s.state == SteerState::Delivered));
}

/// Edge case: the turn ends as you steer. Once its answer is in and nothing
/// is pending, the input closes and a later steer is refused as ending, so
/// the composer sends it as the next message instead.
#[test]
fn a_steer_after_the_answer_ended_is_refused_as_ending() {
    let key = "s-st-end";
    let _live = open_claude(key, "run-x");
    let (tx, mut rx) = unbounded_channel();
    attach(key, "run-x", tx).unwrap();
    assert_eq!(on_result(key, "run-x", 0), AfterResult::Closed);
    assert_eq!(rx.try_recv().unwrap(), PipeMsg::Close);
    assert_eq!(
        steer(key, "too late", "id-late", now()).unwrap_err(),
        STEER_ENDING
    );
}

/// A steer written just before the answer's `result` keeps the input open:
/// the CLI runs it as one more step of this turn.
#[test]
fn a_steer_still_on_its_way_keeps_the_input_open_until_it_lands() {
    let key = "s-st-race";
    let _live = open_claude(key, "run-r");
    let (tx, mut rx) = unbounded_channel();
    attach(key, "run-r", tx).unwrap();
    steer_pub(key, "one more thing", "id-r");
    let _line = rx.try_recv().unwrap();
    assert!(matches!(
        on_result(key, "run-r", 0),
        AfterResult::Waiting(_)
    ));
    observe_line(key, "run-r", &lifecycle("id-r", "started"));
    // The CLI's own count also holds it open.
    assert!(matches!(
        on_result(key, "run-r", 1),
        AfterResult::Waiting(_)
    ));
    assert_eq!(on_result(key, "run-r", 0), AfterResult::Closed);
    assert_eq!(steer(key, "x", "id-r2", now()).unwrap_err(), STEER_ENDING);
}

/// Review: a late-steer timer closes only its own wait of its own attempt,
/// and only while no steer started a new step since that `result`; a steer
/// the input waited for that is cancelled closes it at once.
#[test]
fn the_late_steer_grace_closes_only_its_own_stalled_wait() {
    let key = "s-st-grace";
    let _live = open_claude(key, "run-gr");
    let (tx, mut rx) = unbounded_channel();
    let first = attach(key, "run-gr", tx).unwrap();
    steer_pub(key, "and the README", "id-gr");
    let _line = rx.try_recv().unwrap();
    let AfterResult::Waiting(wait_a) = on_result(key, "run-gr", 0) else {
        panic!("waits for the steer");
    };
    // An account switch moved the turn on before the timer fired.
    detach(key, "run-gr");
    let (tx2, mut rx2) = unbounded_channel();
    let second = attach(key, "run-gr", tx2).unwrap();
    assert_ne!(first, second);
    assert!(matches!(rx2.try_recv(), Ok(PipeMsg::Line(l)) if l.contains("id-gr")));
    assert!(
        !close_if_stalled(key, "run-gr", first, wait_a),
        "never the later attempt"
    );
    // A wait in the new attempt; the steer starts a step, then another
    // steer arrives during that step: the old wait's timer leaves it alone.
    let AfterResult::Waiting(wait_b) = on_result(key, "run-gr", 0) else {
        panic!("still waiting");
    };
    observe_line(key, "run-gr", &lifecycle("id-gr", "started"));
    steer_pub(key, "and the CHANGELOG", "id-gr2");
    assert!(
        !close_if_stalled(key, "run-gr", second, wait_b),
        "a step runs"
    );
    assert!(matches!(rx2.try_recv(), Ok(PipeMsg::Line(_))));
    assert!(rx2.try_recv().is_err(), "not closed");
    // That step's result waits for id-gr2; nothing happens: closed.
    let AfterResult::Waiting(wait_c) = on_result(key, "run-gr", 0) else {
        panic!("waits for id-gr2");
    };
    assert!(
        !close_if_stalled(key, "run-gr", second, wait_b),
        "an older wait"
    );
    assert!(close_if_stalled(key, "run-gr", second, wait_c));
    assert_eq!(rx2.try_recv(), Ok(PipeMsg::Close));
}

/// Review: the steer the input waited for is cancelled and no `result`
/// follows: the input closes at once.
#[test]
fn a_cancelled_steer_after_the_answer_closes_the_input() {
    let key = "s-st-cancel";
    let _live = open_claude(key, "run-cx");
    let (tx, mut rx) = unbounded_channel();
    attach(key, "run-cx", tx).unwrap();
    steer_pub(key, "never mind", "id-cx");
    let _line = rx.try_recv().unwrap();
    assert!(matches!(
        on_result(key, "run-cx", 0),
        AfterResult::Waiting(_)
    ));
    let moved = observe_line(key, "run-cx", &lifecycle("id-cx", "cancelled"));
    assert_eq!(moved[0].state, SteerState::NotDelivered);
    assert_eq!(rx.try_recv(), Ok(PipeMsg::Close));
    assert_eq!(steer(key, "x", "id-cx2", now()).unwrap_err(), STEER_ENDING);
}

/// Review: a steer accepted while its turn closes is refused at publish,
/// never written, never on the receipt; the composer sends it next.
#[test]
fn a_steer_whose_turn_closed_before_it_was_stored_is_refused_and_dropped() {
    let key = "s-st-unpub";
    let _live = open_claude(key, "run-u");
    let (tx, mut rx) = unbounded_channel();
    attach(key, "run-u", tx).unwrap();
    steer(key, "wait, use v2", "id-u", now()).unwrap();
    let (all, moved) = settle(key, "run-u");
    assert!(all.is_empty() && moved.is_empty());
    assert_eq!(publish(key, "id-u").unwrap_err(), STEER_ENDING);
    assert!(std::iter::from_fn(|| rx.try_recv().ok()).all(|m| m == PipeMsg::Close));
    // And one the store refused is forgotten.
    let _live2 = open_claude("s-st-forget", "run-f");
    steer("s-st-forget", "x y z", "id-f", now()).unwrap();
    forget("s-st-forget", "id-f");
    assert!(settle("s-st-forget", "run-f").0.is_empty());
}

/// Review: the stored record must stay whole, escapes included.
#[test]
fn a_steer_whose_json_would_outgrow_the_event_cap_is_refused() {
    let _live = open_claude("s-st-esc", "run-esc");
    let escapes = "\u{1b}".repeat(1_500);
    assert_eq!(
        steer("s-st-esc", &escapes, "id-esc", now()).unwrap_err(),
        STEER_INVALID
    );
    assert!(steer("s-st-esc", &"a".repeat(MAX_STEER_CHARS), "id-ok", now()).is_ok());
    assert_eq!(
        steer("s-st-esc", "again", "id-ok", now()).unwrap_err(),
        STEER_INVALID,
        "repeated id"
    );
}

/// P1-30: a steer the agent refused (Codex's `turn/steer` error) keeps the
/// agent's reason, so the chat shows it; a delivered one carries none.
#[test]
fn a_refused_steer_keeps_the_agents_reason() {
    let key = "s-st-refuse";
    let _live = open_claude(key, "run-r");
    let (tx, _rx) = unbounded_channel();
    attach(key, "run-r", tx).unwrap();
    steer_pub(key, "and lint it", "id-r1");
    steer_pub(key, "and test it", "id-r2");
    let refused = refuse(key, "run-r", "id-r1", "no active turn to steer");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].state, SteerState::NotDelivered);
    assert_eq!(
        refused[0].reason.as_deref(),
        Some("no active turn to steer")
    );
    let wire = serde_json::to_value(&refused[0]).unwrap();
    assert_eq!(wire["reason"], "no active turn to steer");
    let delivered = mark(key, "run-r", "id-r2", SteerState::Delivered);
    let wire = serde_json::to_value(&delivered[0]).unwrap();
    assert!(wire.get("reason").is_none(), "{wire}");
    // A stored record without the field still reads back.
    let old: SteerRecord = serde_json::from_value(json!({
        "id": "id-old", "text": "x", "at": "2026-10-21T17:42:05Z", "afterTools": 0, "state": "notDelivered"
    }))
    .unwrap();
    assert_eq!(old.reason, None);
    // A long reason is cut, never stored whole.
    steer_pub(key, "once more", "id-r3");
    let long = refuse(key, "run-r", "id-r3", &"x".repeat(2_000));
    assert!(long[0].reason.as_ref().unwrap().chars().count() <= MAX_REFUSAL_CHARS);
}

/// An account switch (or night guard) ended the attempt before it took the
/// steer: the next attempt writes it after its first message.
#[test]
fn an_undelivered_steer_is_written_again_by_the_next_attempt() {
    let key = "s-st-move";
    let _live = open_claude(key, "run-m");
    let (tx, _rx) = unbounded_channel();
    attach(key, "run-m", tx).unwrap();
    steer_pub(key, "keep going on the API", "id-m");
    detach(key, "run-m");
    // Between attempts a steer still lands in the record.
    steer_pub(key, "and the docs", "id-n");
    let (tx2, mut rx2) = unbounded_channel();
    attach(key, "run-m", tx2.clone()).unwrap();
    // Written in order, under the lock, before any newer steer.
    steer_pub(key, "and the tests", "id-o");
    let lines: Vec<_> = std::iter::from_fn(|| rx2.try_recv().ok()).collect();
    assert_eq!(lines.len(), 3);
    for (line, id) in lines.iter().zip(["id-m", "id-n", "id-o"]) {
        assert!(
            matches!(line, PipeMsg::Line(l) if l.contains(id)),
            "{line:?}"
        );
    }
}

/// Stopped (or ended) with a steer never taken: it says so, and the receipt
/// gets every steer.
#[test]
fn settling_marks_steers_the_agent_never_took() {
    let key = "s-st-stop";
    let _live = open_claude(key, "run-s");
    steer_pub(key, "use the other branch", "id-s");
    let (all, moved) = settle(key, "run-s");
    assert_eq!(all.len(), 1);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].state, SteerState::NotDelivered);
    assert_eq!(
        state_payload(&moved[0]),
        r#"{"id":"id-s","state":"notDelivered"}"#
    );
}

#[test]
fn a_newer_turn_is_never_removed_by_an_older_guard() {
    let key = "s-st-guard";
    let old = open_claude(key, "run-old");
    let _new = open_claude(key, "run-new");
    drop(old);
    assert!(steer(key, "still here", "id-g", now()).is_ok());
}

#[test]
fn held_sends_go_oldest_first_and_cancel_by_id() {
    let key = "s-st-held";
    let item = |id: &str, after: After, prompt: &str| Held {
        id: id.into(),
        after,
        at: "2026-10-21T18:30:00Z".into(),
        input: HeldInput {
            prompt: prompt.into(),
            ..Default::default()
        },
    };
    let input = |prompt: &str| prompt.to_string();
    let hold =
        |key: &str, id: &str, after: After, prompt: String| hold(key, item(id, after, &prompt));
    hold(key, "h-1", After::Queue, input("first")).unwrap();
    hold(key, "h-2", After::Stop, input("second")).unwrap();
    hold(key, "h-3", After::Queue, input("third")).unwrap();
    // The store is global: other tests' keys may sit beside this one.
    assert!(held_conversations().contains(&key.to_string()));
    // What survives a restart round-trips whole.
    let wire = serde_json::to_string(&item("h-9", After::Stop, "x")).unwrap();
    assert_eq!(
        serde_json::from_str::<Held>(&wire).unwrap(),
        item("h-9", After::Stop, "x")
    );
    assert_eq!(cancel_held(key, "h-2"), Ok(true));
    assert_eq!(cancel_held(key, "h-2"), Ok(false));
    let next = take_held(key).unwrap();
    assert_eq!(
        (next.id.as_str(), next.input.prompt.as_str()),
        ("h-1", "first")
    );
    put_back(key, next);
    assert_eq!(held_count(key), 2);
    assert!(settle_held(key, &take_held(key).unwrap().id));
    assert!(settle_held(key, &take_held(key).unwrap().id));
    assert!(take_held(key).is_none());
    for i in 0..MAX_HELD {
        hold(key, &format!("m-{i}"), After::Queue, input("x")).unwrap();
    }
    assert!(hold(key, "m-over", After::Queue, input("x")).is_err());
    while let Some(taken) = take_held(key) {
        settle_held(key, &taken.id);
    }
}

/// R8b review: failed sends awaiting the window's acknowledgement don't
/// count against the cap on sends waiting for a turn.
#[test]
fn failed_held_sends_do_not_count_against_the_cap() {
    let key = "s-20261115-cap";
    let mut failed = held_item("f-0", "failed");
    failed.input.failed = Some("Gone.".into());
    super::hold(key, failed).unwrap();
    for i in 0..MAX_HELD {
        super::hold(key, held_item(&format!("w-{i}"), "waiting")).unwrap();
    }
    assert!(super::hold(key, held_item("w-over", "over")).is_err());
    assert_eq!(ack_failed(key, "f-0"), Ok(true));
    while let Some(taken) = take_held(key) {
        settle_held(key, &taken.id);
    }
}

fn held_item(id: &str, prompt: &str) -> Held {
    Held {
        id: id.into(),
        after: After::Queue,
        at: "2026-11-13T01:30:00Z".into(),
        input: HeldInput {
            prompt: prompt.into(),
            ..Default::default()
        },
    }
}

/// The ids of `key`'s sends as a restart would load them.
fn saved_ids(key: &str) -> Vec<String> {
    saved_view()
        .get(key)
        .map(|queue| queue.iter().map(|item| item.id.clone()).collect())
        .unwrap_or_default()
}

/// 2026-11-13 (a): a taken send stays saved until its turn is queued, so
/// a quit between the take and the run queue's commit still has it, first.
#[test]
fn a_taken_held_send_survives_a_quit_before_it_is_queued() {
    let key = "s-st-held-crash";
    hold(key, held_item("c-1", "first")).unwrap();
    hold(key, held_item("c-2", "second")).unwrap();
    let before = saved_ids(key);
    let taken = take_held(key).unwrap();
    assert_eq!(taken.id, "c-1");
    // Hollow quits here: what it would load is what it had.
    assert_eq!(saved_ids(key), before);
    assert_eq!(before, vec!["c-1", "c-2"]);
    // A later save (another hold) writes it too, still first.
    hold(key, held_item("c-3", "third")).unwrap();
    assert_eq!(saved_ids(key), vec!["c-1", "c-2", "c-3"]);
    // Counted until settled; a drain that stopped part way gets it again.
    assert_eq!(held_count(key), 3);
    assert_eq!(take_held(key).unwrap().id, "c-1");
    assert!(settle_held(key, "c-1"));
    assert!(settle_held(key, &take_held(key).unwrap().id));
    assert!(settle_held(key, &take_held(key).unwrap().id));
    assert!(saved_ids(key).is_empty());
}

/// 2026-11-13 (b): the normal path removes the send exactly once, and only
/// once its turn is queued; settling again or another id changes nothing.
#[test]
fn a_queued_held_send_is_removed_exactly_once() {
    let key = "s-st-held-once";
    hold(key, held_item("o-1", "first")).unwrap();
    hold(key, held_item("o-2", "second")).unwrap();
    let taken = take_held(key).unwrap();
    assert!(!settle_held(key, "o-2"), "not the taken one");
    assert!(settle_held(key, &taken.id));
    assert!(!settle_held(key, &taken.id), "once");
    assert_eq!(saved_ids(key), vec!["o-2"]);
    assert_eq!(held_count(key), 1);
    assert_eq!(take_held(key).unwrap().id, "o-2");
    assert!(settle_held(key, "o-2"));
    assert_eq!(held_count(key), 0);
    assert!(!held_conversations().contains(&key.to_string()));
    // Its turn's row is found by this key after a restart.
    assert_eq!(queue_key("o-1"), "held:o-1");
}

/// 2026-11-13 (c): a refused send put back is held once, at the front, with
/// what the drain changed on it.
#[test]
fn a_refused_held_send_put_back_is_held_once() {
    let key = "s-st-held-back";
    hold(key, held_item("b-1", "first")).unwrap();
    hold(key, held_item("b-2", "second")).unwrap();
    let mut taken = take_held(key).unwrap();
    taken.input.attempts += 1;
    put_back(key, taken);
    assert_eq!(saved_ids(key), vec!["b-1", "b-2"]);
    assert_eq!(held_count(key), 2);
    assert!(!settle_held(key, "b-1"), "nothing taken now");
    let again = take_held(key).unwrap();
    assert_eq!((again.id.as_str(), again.input.attempts), ("b-1", 1));
    assert_eq!(saved_ids(key), vec!["b-1", "b-2"]);
    assert!(settle_held(key, "b-1"));
    assert!(settle_held(key, &take_held(key).unwrap().id));
    assert!(saved_ids(key).is_empty());
}

#[test]
fn the_send_choice_parses_strictly() {
    assert_eq!(After::parse(None), Ok(None));
    assert_eq!(After::parse(Some("queue")), Ok(Some(After::Queue)));
    assert_eq!(After::parse(Some("stop")), Ok(Some(After::Stop)));
    assert!(After::parse(Some("steer")).is_err());
}

/// Steering PR 3: a Codex steer goes down the pipe as a steer, not a line:
/// only the attempt's `codex_app_server::AppSession` knows the running
/// turn's id its `turn/steer` must name. The attempt reports its outcome
/// ([`mark`]) and closes the input when the turn is over ([`close_input`]);
/// a steer after that is refused as ending.
#[test]
fn a_codex_steer_goes_to_the_attempts_app_session() {
    let key = "s-st-cx";
    let _live = open(
        key,
        "run-x",
        (AgentKind::Codex, SteerSupport::Native),
        1,
        PathBuf::from("/tmp/vault"),
        Map::new(),
    );
    let (tx, mut rx) = unbounded_channel();
    assert_eq!(attach(key, "run-x", tx), Some(1));
    steer_pub(key, "Use staging.", "id-x");
    assert_eq!(
        rx.try_recv().unwrap(),
        PipeMsg::Steer {
            id: "id-x".into(),
            text: "Use staging.".into()
        }
    );
    let moved = mark(key, "run-x", "id-x", SteerState::Delivered);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].state, SteerState::Delivered);
    assert!(mark(key, "run-x", "id-x", SteerState::NotDelivered).is_empty());
    assert!(mark(key, "run-other", "id-x", SteerState::NotDelivered).is_empty());

    // An undelivered steer goes again to the next attempt's session.
    steer_pub(key, "And skip e2e.", "id-y");
    let _ = rx.try_recv();
    let (tx2, mut rx2) = unbounded_channel();
    assert_eq!(attach(key, "run-x", tx2), Some(2));
    assert_eq!(
        rx2.try_recv().unwrap(),
        PipeMsg::Steer {
            id: "id-y".into(),
            text: "And skip e2e.".into()
        }
    );
    close_input(key, "run-x");
    assert_eq!(rx2.try_recv(), Ok(PipeMsg::Close));
    assert_eq!(
        steer(key, "Late?", "id-z", now()).unwrap_err(),
        STEER_ENDING
    );
}

// AC1 (2026-11-17): Steer now asks the agent's `steer` capability first and
// holds a true flag to the Codex version floor second.

#[test]
fn steer_support_follows_the_steer_capability_then_the_version() {
    for agent in AgentKind::all() {
        let flag = crate::agent::capabilities(agent).steer;
        // Whatever the version, a false flag never steers.
        for version in [None, Some("0.163.0"), Some("9.9.9")] {
            if !flag {
                assert_eq!(
                    support_for(agent, version),
                    SteerSupport::None,
                    "{agent:?} {version:?}"
                );
            }
        }
        // A true flag steers, except Codex below the floor.
        if flag && agent != AgentKind::Codex {
            assert_eq!(support_for(agent, None), SteerSupport::Native, "{agent:?}");
        }
    }
    assert!(crate::agent::capabilities(AgentKind::Codex).steer);
    assert_eq!(
        support_for(AgentKind::Codex, Some("0.162.9")),
        SteerSupport::None
    );
    assert_eq!(
        support_for(AgentKind::Codex, Some("0.163.0")),
        SteerSupport::Native
    );
}

#[test]
fn a_steerless_agents_note_is_its_capability_reason() {
    for agent in [AgentKind::Grok, AgentKind::Antigravity] {
        let reason = crate::agent::unsupported_reason(agent, crate::agent::Feature::Steer).unwrap();
        assert_eq!(
            support_info(agent, Some("0.163.0")).note,
            reason,
            "{agent:?}"
        );
        assert!(reason.contains(agent.label()), "{reason}");
    }
    // Codex has the flag but not the version: its own note, not the reason.
    assert!(
        crate::agent::unsupported_reason(AgentKind::Codex, crate::agent::Feature::Steer).is_none()
    );
    assert!(support_info(AgentKind::Codex, None)
        .note
        .contains("0.163.0"));
}

#[test]
fn a_send_for_an_agent_without_queue_is_refused_with_its_reason() {
    use crate::agent::Flags;
    let with_queue = Flags {
        steer: true,
        interrupt: true,
        fork: false,
        rollback: false,
        queue: true,
        pending_survives_restart: true,
    };
    let without = Flags {
        queue: false,
        ..with_queue
    };
    // A test row with queue false: refused with a sentence naming the agent.
    let reason = queue_refusal_in(without, "Newbie").expect("refused");
    assert!(
        reason.contains("Newbie") && reason.contains("hold a message"),
        "{reason}"
    );
    assert!(
        !reason.contains('\u{2014}') && !reason.contains('\u{2013}'),
        "{reason}"
    );
    assert_eq!(queue_refusal_in(with_queue, "Newbie"), None);
    // The refusal is a pure function of the agent: it stores and stops
    // nothing, so a caller that asks it first (send_or_hold before Stop and
    // send's cancel, hold_agent_send, hold) leaves no side effect behind.
    // No agent in the table lacks queue today, and an unresolved one is
    // left to the caller's own resolve.
    for agent in AgentKind::all() {
        assert_eq!(queue_refusal(Some(agent)), None, "{agent:?}");
    }
    assert_eq!(queue_refusal(None), None);
}

#[test]
fn the_steer_gates_each_carry_their_own_reason() {
    // Capability gate: the capability's reason, at any version.
    let grok = steer_gate_reason(AgentKind::Grok, Some("0.163.0")).unwrap();
    assert!(grok.contains("Grok") && !grok.contains("0.163.0"), "{grok}");
    // Version gate: only the agent whose transport has a floor, with its label.
    let codex = steer_gate_reason(AgentKind::Codex, Some("0.162.0")).unwrap();
    assert!(
        codex.contains("Codex") && codex.contains("0.163.0"),
        "{codex}"
    );
    assert_eq!(steer_gate_reason(AgentKind::Codex, Some("0.163.0")), None);
    assert_eq!(steer_gate_reason(AgentKind::Claude, None), None);
}
