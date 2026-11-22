//! v1.2 Live Activities (`docs/phone-api/live-activity.md`): the token
//! registry, the step labels and payloads against
//! `docs/phone-api-fixtures/live-activity/`, the routes, and the pushes the
//! pusher sends to a fake APNs.

use super::super::live_activity::{self, Activities, Phase, State};
use super::super::push::KeySource as _;
use super::*;
use std::collections::HashSet;

const KEY: &str = "s-20261107-0000aaaa";

fn hex64(n: u32) -> String {
    format!("{n:0>64x}")
}

fn live_with(tools: &[(&str, &str)], text: &str) -> wire::Live {
    wire::Live {
        key: KEY.into(),
        ord: 1,
        gen: 1,
        started_at: "2026-11-07T10:00:00Z".into(),
        text: text.into(),
        truncated: false,
        tools: tools
            .iter()
            .enumerate()
            .map(|(i, (name, status))| wire::Tool {
                id: format!("call_{i}"),
                name: name.to_string(),
                status: status.to_string(),
                detail: None,
            })
            .collect(),
    }
}

// ── The step and the payloads ────────────────────────────────────────────

#[test]
fn the_step_table_is_the_fixtures_and_no_label_names_a_file_or_a_command() {
    let rows = fixture("live-activity/step-labels.json");
    let rows = rows.as_array().unwrap();
    assert!(rows.len() > 20);
    for row in rows {
        let tool = row["tool"].as_str().unwrap();
        let step = row["step"].as_str().unwrap();
        assert_eq!(live_activity::step_for_tool(tool), step, "{tool}");
        assert!(step.chars().count() <= live_activity::STEP_MAX, "{step}");
        assert!(!step.contains(['/', '$', '`', '\\']), "{step}");
    }
    // The case of a tool's name never matters.
    assert_eq!(live_activity::step_for_tool("EDIT"), "Editing files");
    assert_eq!(live_activity::step_for_tool("MCP__x__y"), "Using a tool");
}

#[test]
fn a_running_turns_step_is_its_newest_running_tool_else_the_answer_else_working_else_starting() {
    let step =
        |tools: &[(&str, &str)], text: &str| live_activity::current_step(&live_with(tools, text));
    assert_eq!(step(&[], ""), "Starting");
    assert_eq!(step(&[("Read", "done")], ""), "Working");
    assert_eq!(step(&[("Edit", "error")], ""), "Working");
    assert_eq!(step(&[("Read", "done")], "Here is"), "Writing the answer");
    // The newest call that is still running, not the newest call.
    assert_eq!(
        step(&[("Read", "running"), ("Bash", "done")], ""),
        "Reading files"
    );
    assert_eq!(
        step(&[("Read", "running"), ("Edit", "running")], "text"),
        "Editing files"
    );
    // A running tool beats the text before it.
    assert_eq!(step(&[("Grep", "running")], "text"), "Searching files");
}

#[test]
fn the_pushes_are_the_fixtures_and_an_end_carries_no_step() {
    let running = State {
        phase: Phase::Running,
        step: Some("Editing files".into()),
    };
    assert_eq!(
        live_activity::payload(&running, 1794027000),
        fixture("live-activity/push-update.json")
    );
    assert_eq!(
        live_activity::payload(&State::waiting(), 1794027120),
        fixture("live-activity/push-waiting.json")
    );
    assert_eq!(
        live_activity::payload(&State::ended(Phase::Finished), 1794027300),
        fixture("live-activity/push-end.json")
    );
    for (phase, word) in [(Phase::Failed, "failed"), (Phase::Stopped, "stopped")] {
        let body = live_activity::payload(&State::ended(phase), 100);
        assert_eq!(body["aps"]["event"], "end");
        assert_eq!(body["aps"]["content-state"], json!({"phase": word}));
        assert_eq!(body["aps"]["dismissal-date"], 400);
        assert!(body["aps"].get("stale-date").is_none());
    }
    // A step is never longer than 80 characters.
    let long = State {
        phase: Phase::Running,
        step: Some("x".repeat(200)),
    };
    let step = live_activity::payload(&long, 1)["aps"]["content-state"]["step"]
        .as_str()
        .unwrap()
        .chars()
        .count();
    assert_eq!(step, live_activity::STEP_MAX);
}

// ── The registry ─────────────────────────────────────────────────────────

#[test]
fn the_registry_keeps_one_token_per_phone_and_chat_and_eight_chats_per_phone() {
    let a = Activities::default();
    a.register("p1", "s-1", &hex64(1), "production", 100);
    a.register("p1", "s-1", &hex64(2), "production", 101);
    assert_eq!(
        a.len(),
        1,
        "a new token for the same phone and chat replaces"
    );
    assert_eq!(a.for_chat("s-1", 102)[0].token, hex64(2));
    // A token names one activity: another phone registering it takes it.
    a.register("p2", "s-9", &hex64(2), "development", 103);
    assert_eq!(a.len(), 1);
    assert!(!a.has_chat("s-1") && a.has_chat("s-9"));
    // The ninth chat of a phone drops its oldest; other phones are untouched.
    for n in 0..9u32 {
        a.register(
            "p3",
            &format!("s-c{n}"),
            &hex64(100 + n),
            "production",
            200 + i64::from(n),
        );
    }
    let kept: Vec<u32> = (0..9).filter(|n| a.has_chat(&format!("s-c{n}"))).collect();
    assert_eq!(kept, (1..9).collect::<Vec<u32>>());
    assert!(a.has_chat("s-9"));
    // Forget one chat, then all of a phone's.
    assert_eq!(a.forget("p3", Some("s-c3")), 1);
    assert!(!a.has_chat("s-c3") && a.has_chat("s-c4"));
    assert_eq!(a.forget("p3", None), 7);
    assert_eq!(a.len(), 1);
    // A token Apple refuses goes.
    assert_eq!(a.drop_token(&hex64(2)), 1);
    assert_eq!(a.len(), 0);
}

#[test]
fn a_token_older_than_twelve_hours_and_a_phone_no_longer_paired_go() {
    let a = Activities::default();
    a.register("p1", "s-1", &hex64(1), "production", 1_000);
    a.register(
        "p2",
        "s-1",
        &hex64(2),
        "production",
        1_000 + live_activity::LIFE - 5,
    );
    let now = 1_000 + live_activity::LIFE;
    let kept = a.for_chat("s-1", now);
    assert_eq!(kept.len(), 1, "the first one is exactly twelve hours old");
    assert_eq!(kept[0].device_id, "p2");
    assert_eq!(a.len(), 1, "an expired one is dropped, not just skipped");
    a.retain_devices(&HashSet::from(["p9".to_string()]));
    assert_eq!(a.len(), 0);
}

// ── The routes ───────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_activity_routes_keep_a_token_per_phone_and_chat_and_delete_drops_it() {
    let rig = rig().await;
    let token = rig.token();
    let vault = rig.vault.to_string_lossy().into_owned();
    let asked = fixture("live-activity/register-request.json");
    let key = asked["sessionKey"].as_str().unwrap().to_string();
    seed_chat(
        &rig.vault,
        &key,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "acceptEdits",
    );
    let other_key = "s-20261107-0000aaab";
    seed_chat(
        &rig.vault,
        other_key,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "acceptEdits",
    );
    let now = Utc::now().timestamp();
    let kept = |key: &str| rig.ctx.activities.for_chat(key, now);

    // The pairing token is needed, as for every route.
    let resp = rig
        .call("POST", "/v1/live-activity", None, Some(asked.clone()))
        .await;
    assert_eq!(resp.status, 401);
    assert_eq!(rig.ctx.activities.len(), 0);

    let resp = rig
        .call(
            "POST",
            "/v1/live-activity",
            Some(&token),
            Some(asked.clone()),
        )
        .await;
    assert_eq!(resp.status, 204);
    assert!(resp.body.is_empty());
    assert_eq!(resp.header("content-length").unwrap_or("0"), "0");
    let first = kept(&key);
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].token,
        asked["token"].as_str().unwrap().to_ascii_lowercase()
    );
    assert_eq!(first[0].environment, "production");

    // Upper case is the same token; a new one for the same chat replaces.
    let upper = "AB".repeat(32);
    let resp = rig
        .call(
            "POST",
            "/v1/live-activity",
            Some(&token),
            Some(json!({"sessionKey": key, "token": upper, "environment": "development"})),
        )
        .await;
    assert_eq!(resp.status, 204);
    let replaced = kept(&key);
    assert_eq!(
        (replaced.len(), replaced[0].token.as_str()),
        (1, "ab".repeat(32).as_str())
    );
    assert_eq!(replaced[0].environment, "development");

    // A chat Hollow doesn't list is ignored, still 204 (a 404 means no v1.2).
    let resp = rig
        .call(
            "POST",
            "/v1/live-activity",
            Some(&token),
            Some(json!({"sessionKey": "s-20261107-ffffffff", "token": hex64(7), "environment": "production"})),
        )
        .await;
    assert_eq!(resp.status, 204);
    assert!(!rig.ctx.activities.has_chat("s-20261107-ffffffff"));

    // A bad field is a 400 and changes nothing.
    for body in [
        json!({"token": hex64(7), "environment": "production"}),
        json!({"sessionKey": "../etc", "token": hex64(7), "environment": "production"}),
        json!({"sessionKey": key, "token": "xyz", "environment": "production"}),
        json!({"sessionKey": key, "token": hex64(7)}),
        json!({"sessionKey": key, "token": hex64(7), "environment": "staging"}),
    ] {
        let resp = rig
            .call(
                "POST",
                "/v1/live-activity",
                Some(&token),
                Some(body.clone()),
            )
            .await;
        assert_eq!(resp.status, 400, "{body}");
        assert_eq!(resp.json()["error"]["code"], "bad_request");
    }
    assert_eq!(kept(&key)[0].token, "ab".repeat(32));

    // DELETE with a session drops that chat's token only; without, all of them.
    rig.call(
        "POST",
        "/v1/live-activity",
        Some(&token),
        Some(json!({"sessionKey": other_key, "token": hex64(8), "environment": "production"})),
    )
    .await;
    assert_eq!(rig.ctx.activities.len(), 2);
    let resp = rig
        .call(
            "DELETE",
            &format!("/v1/live-activity?session={key}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(resp.status, 204);
    assert!(kept(&key).is_empty() && kept(other_key).len() == 1);
    // Again: still 204.
    assert_eq!(
        rig.call(
            "DELETE",
            &format!("/v1/live-activity?session={key}"),
            Some(&token),
            None
        )
        .await
        .status,
        204
    );
    let resp = rig
        .call("DELETE", "/v1/live-activity", Some(&token), None)
        .await;
    assert_eq!(resp.status, 204);
    assert_eq!(rig.ctx.activities.len(), 0);

    // Unpair (DELETE /v1/push) takes the phone's tokens too, and only its own.
    let (_other_phone, other_token) = rig.store.pair("Other", None, Utc::now()).unwrap();
    for (bearer, n) in [(&token, 20u32), (&other_token, 21)] {
        let body = json!({"sessionKey": key, "token": hex64(n), "environment": "production"});
        assert_eq!(
            rig.call("POST", "/v1/live-activity", Some(bearer), Some(body))
                .await
                .status,
            204
        );
    }
    assert_eq!(rig.ctx.activities.len(), 2, "two phones, one token each");
    let resp = rig.call("DELETE", "/v1/push", Some(&token), None).await;
    assert_eq!(resp.status, 204);
    let left = rig.ctx.activities.for_chat(&key, Utc::now().timestamp());
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].token, hex64(21), "the other phone's token stays");
}

// ── The pushes ───────────────────────────────────────────────────────────

/// Fold `event` into the live buffer and show it to the pusher, as the live
/// worker does.
fn fold(rig: &PushRig, event: Value) {
    rig.live.ingest(&event);
    rig.pusher.observe(&event);
}

fn tool_use(id: &str, name: &str, command: &str) -> Value {
    json!({"type": "toolUse", "conversationKey": KEY, "id": id, "name": name, "input": {"command": command}})
}

fn tool_result(id: &str) -> Value {
    json!({"type": "toolResult", "conversationKey": KEY, "id": id, "isError": false})
}

fn run_started() -> Value {
    json!({"type": "runStarted", "conversationKey": KEY, "sessionKey": KEY, "attended": true})
}

fn done() -> Value {
    json!({"type": "done", "conversationKey": KEY, "sessionKey": KEY})
}

/// A rig with chat [`KEY`] seeded, one paired phone with no notifications on,
/// and a Live Activity token registered for that chat: (rig, the phone's
/// device id, the activity's token).
async fn live_rig() -> (PushRig, String, String) {
    let rig = push_rig().await;
    let vault = rig.vault.to_string_lossy().into_owned();
    seed_chat(
        &rig.vault,
        KEY,
        "claude",
        &vault,
        &[("Go", "Done")],
        "acceptEdits",
    );
    let (phone, _) = rig.store.pair("Phone", None, Utc::now()).unwrap();
    let activity = "ab".repeat(32);
    let now = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&phone.device_id, KEY, &activity, push::PRODUCTION, now);
    (rig, phone.device_id, activity)
}

fn content_state(request: &ApnsRequest) -> Value {
    request.body["aps"]["content-state"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registered_chats_step_waiting_and_end_go_to_apple_as_live_activity_pushes() {
    let (mut rig, _phone, activity) = live_rig().await;
    let now = rig.now.load(Ordering::SeqCst);

    // A step: the newest running tool, by its name.
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "never shown"));
    assert_eq!(rig.run_jobs().await, 1);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, format!("/3/device/{activity}"));
    assert_eq!(seen[0].headers["apns-topic"], push::LIVE_ACTIVITY_TOPIC);
    assert_eq!(seen[0].headers["apns-push-type"], "liveactivity");
    assert_eq!(seen[0].headers["apns-priority"], "5");
    assert!(seen[0].headers["authorization"].starts_with("bearer "));
    assert_eq!(rig.iat_of(&seen[0]), now);
    assert_eq!(
        seen[0].body,
        json!({"aps": {"timestamp": now, "event": "update", "stale-date": now + 600, "relevance-score": 100,
                       "content-state": {"phase": "running", "step": "Editing files"}}})
    );

    // Past the gap, the next step goes; the same step again does not.
    rig.now.fetch_add(20, Ordering::SeqCst);
    fold(&rig, tool_result("call_1"));
    fold(&rig, tool_use("call_2", "Bash", "echo SECRET=hunter2"));
    assert_eq!(rig.run_jobs().await, 1);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        content_state(&seen[1]),
        json!({"phase": "running", "step": "Running a command"})
    );
    assert_eq!(seen[1].headers["apns-priority"], "5");
    fold(&rig, tool_use("call_3", "Bash", "ls"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 2, "the same step is not sent twice");

    // A card opened in the chat: waiting for the person, at once and urgent.
    let card = seed_card(
        &rig.vault,
        KEY,
        1,
        "toolu-q1",
        &card_arguments("Who reads it?"),
    );
    fold(
        &rig,
        json!({"type": "interaction", "interaction": card, "sessionKey": KEY, "conversationKey": KEY}),
    );
    assert_eq!(
        rig.run_jobs().await,
        2,
        "the question's own job and the activity's"
    );
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].headers["apns-priority"], "10");
    assert_eq!(seen[2].headers["apns-push-type"], "liveactivity");
    assert_eq!(
        content_state(&seen[2]),
        json!({"phase": "waiting", "step": "Waiting for your answer"})
    );

    // The turn ends: the activity ends, urgent, with no step, and its token goes.
    rig.now.fetch_add(5, Ordering::SeqCst);
    let ended = rig.now.load(Ordering::SeqCst);
    fold(&rig, done());
    assert_eq!(
        rig.run_jobs().await,
        2,
        "the finished turn's job and the activity's"
    );
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[3].headers["apns-priority"], "10");
    assert_eq!(
        seen[3].body,
        json!({"aps": {"timestamp": ended, "event": "end", "dismissal-date": ended + 300,
                       "content-state": {"phase": "finished"}}})
    );
    assert!(!rig.pusher.activities().has_chat(KEY));

    // Nothing a tool did, ran or said reached Apple.
    for request in &seen {
        let text = request.body.to_string();
        assert!(
            !text.contains("SECRET") && !text.contains("hunter2") && !text.contains("never shown"),
            "{text}"
        );
    }
    // After the end, more events make no jobs: nobody holds a token.
    fold(&rig, tool_use("call_9", "Read", "x"));
    assert_eq!(rig.run_jobs().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_steps_pushes_once_per_gap_and_the_newest_step_wins() {
    let (mut rig, _phone, _activity) = live_rig().await;
    rig.pusher.set_live_gap(2);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1);

    // Inside the gap three changes make one job, which sends nothing yet.
    fold(&rig, tool_result("call_1"));
    fold(&rig, tool_use("call_2", "Bash", "b"));
    fold(&rig, tool_use("call_3", "Grep", "c"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1, "inside the gap nothing goes");
    // More events while it waits make no job of their own.
    fold(&rig, tool_use("call_4", "WebSearch", "d"));
    assert_eq!(rig.run_jobs().await, 0);

    // It comes back when the gap is over, and the newest step is read then.
    let job = tokio::time::timeout(Duration::from_secs(8), rig.jobs.recv())
        .await
        .expect("the step came back after the gap")
        .expect("the job channel stays open");
    assert_eq!(job, push::Job::LiveStep { key: KEY.into() });
    rig.now.fetch_add(2, Ordering::SeqCst);
    rig.pusher.run(job).await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        content_state(&seen[1]),
        json!({"phase": "running", "step": "Searching the web"})
    );
    assert_eq!(
        rig.run_jobs().await,
        0,
        "one job came back, not one per event"
    );
}

/// A new turn of chat [`KEY`] that the store holds as ended with `status`
/// (`None`: started and never finished, as a running one reads).
fn store_turn(rig: &PushRig, status: Option<&str>) {
    let vault = rig.vault.to_string_lossy().into_owned();
    let ord = session_store::start_turn(&rig.vault, KEY, "claude", &vault, "run-live-end", "Go on")
        .unwrap();
    if let Some(status) = status {
        session_store::finish_turn(&rig.vault, KEY, ord, status).unwrap();
    }
}

/// A turn that emits `events` (type names, each for [`KEY`]) and then ends,
/// with the store holding it as `stored`: what its activity's end says.
async fn how_it_ends(events: &[&str], stored: Option<&str>) -> Value {
    let (mut rig, _phone, _activity) = live_rig().await;
    fold(&rig, run_started());
    for event in events {
        fold(&rig, json!({"type": event, "conversationKey": KEY}));
    }
    store_turn(&rig, stored);
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2, "{events:?} {stored:?}");
    let seen = rig.apns.seen();
    let end = seen.last().expect("an end push");
    assert_eq!(end.body["aps"]["event"], "end", "{events:?} {stored:?}");
    content_state(end)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activity_ends_as_the_stored_turn_ended_and_the_events_only_hint() {
    let phase = |name: &str| json!({ "phase": name });
    // Stored and hinted agree.
    assert_eq!(
        how_it_ends(&["cancelled"], Some("cancelled")).await,
        phase("stopped")
    );
    assert_eq!(
        how_it_ends(&["error"], Some("error")).await,
        phase("failed")
    );
    assert_eq!(how_it_ends(&[], Some("success")).await, phase("finished"));
    // Review M2: a refusal or an error envelope ends the turn as an error
    // with no `error` event at all, and the lock screen must not say Finished.
    assert_eq!(how_it_ends(&[], Some("error")).await, phase("failed"));
    // A stop with no `cancelled` event, and a turn the app's restart cut.
    assert_eq!(how_it_ends(&[], Some("cancelled")).await, phase("stopped"));
    assert_eq!(
        how_it_ends(&[], Some("interrupted")).await,
        phase("stopped")
    );
    // An `error` event in a turn that went on to succeed (a retried call, a
    // failed tool the agent got past) is not a failed turn.
    assert_eq!(
        how_it_ends(&["error"], Some("success")).await,
        phase("finished")
    );
    assert_eq!(
        how_it_ends(&["error", "cancelled"], Some("success")).await,
        phase("finished")
    );
    // When the store can't say (the turn still reads as running), the events
    // decide: a stop after an error is a stop, an error alone is a failure.
    assert_eq!(
        how_it_ends(&["error", "cancelled"], None).await,
        phase("stopped")
    );
    assert_eq!(how_it_ends(&["error"], None).await, phase("failed"));
    assert_eq!(how_it_ends(&[], None).await, phase("finished"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_turn_does_not_inherit_the_last_ones_hint() {
    let (mut rig, _phone, _activity) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, json!({"type": "error", "conversationKey": KEY}));
    fold(&rig, run_started());
    // The store can't say (still running), so only the hint could.
    store_turn(&rig, None);
    fold(&rig, done());
    rig.run_jobs().await;
    assert_eq!(
        content_state(rig.apns.seen().last().unwrap()),
        json!({"phase": "finished"})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_chat_with_a_token_makes_jobs_and_no_key_sends_nothing_and_keeps_the_token() {
    let (mut rig, _phone, _activity) = live_rig().await;
    // Another chat's events: no token, no job.
    let other = json!({"type": "toolUse", "conversationKey": "s-20261107-0000aaab", "id": "c", "name": "Edit", "input": {}});
    rig.pusher.observe(&other);
    rig.pusher
        .observe(&json!({"type": "done", "conversationKey": "s-20261107-0000aaab"}));
    assert_eq!(rig.run_jobs().await, 0);

    // No key imported: the job runs, nothing goes, nothing is wrong.
    rig.keys.delete().unwrap();
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert!(rig.apns.seen().is_empty());
    assert_eq!(rig.pusher.status().last_error, None);
    assert!(
        rig.pusher.activities().has_chat(KEY),
        "the token waits for a key"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_apple_refuses_is_dropped_and_a_revoked_phones_is_never_used() {
    // Review L4: a 410 is the routine answer for an activity that already
    // ended (the app ended it first, the person swiped it away): the token
    // goes and Settings does not show a red error for it.
    let (mut rig, _phone, activity) = live_rig().await;
    rig.apns
        .answer(&activity, 410, r#"{"reason":"Unregistered"}"#);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1);
    assert!(
        !rig.pusher.activities().has_chat(KEY),
        "Apple's 410 drops the token"
    );
    assert_eq!(rig.pusher.status().last_error, None, "a 410 is quiet");

    // A token Apple calls bad is dropped too, and that one is a real error.
    let (mut rig, phone, activity) = live_rig().await;
    rig.apns
        .answer(&activity, 400, r#"{"reason":"BadDeviceToken"}"#);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert!(!rig.pusher.activities().has_chat(KEY));
    let said = rig
        .pusher
        .status()
        .last_error
        .expect("the refusal is kept for Settings");
    assert!(
        said.contains("Live Activity push") && !said.contains(&activity),
        "{said}"
    );

    // A phone revoked after it registered: no push, and its token is forgotten.
    let now = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&phone, KEY, &activity, push::PRODUCTION, now);
    rig.store.revoke(&phone).unwrap();
    rig.now.fetch_add(60, Ordering::SeqCst);
    fold(&rig, tool_use("call_2", "Bash", "b"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        rig.apns.seen().len(),
        1,
        "nothing went to the revoked phone"
    );
    assert!(!rig.pusher.activities().has_chat(KEY));
}

/// The next job the pusher made, waiting for it up to `seconds` (a refresh or
/// a step sleeping out its gap).
async fn next_job(rig: &mut PushRig, seconds: u64) -> push::Job {
    tokio::time::timeout(Duration::from_secs(seconds), rig.jobs.recv())
        .await
        .expect("a job came back in time")
        .expect("the job channel stays open")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_step_is_told_again_before_it_goes_stale_and_the_end_stops_that() {
    // Review M1: `stale-date` is ten minutes out and nothing else sends while
    // one step simply runs long, so Hollow tells the activity again.
    let (mut rig, _phone, _activity) = live_rig().await;
    rig.pusher.set_live_refresh(1);
    let first = rig.now.load(Ordering::SeqCst);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Bash", "sleep 900"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1);

    // Nothing changes. The refresh comes back after its period and sends the
    // same state with a newer stale-date, at the update's priority.
    let job = next_job(&mut rig, 5).await;
    assert_eq!(job, push::Job::LiveRefresh { key: KEY.into() });
    rig.now.fetch_add(2, Ordering::SeqCst);
    rig.pusher.run(job).await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2, "told again");
    assert_eq!(content_state(&seen[1]), content_state(&seen[0]));
    assert_eq!(seen[1].body["aps"]["event"], "update");
    assert_eq!(seen[1].body["aps"]["stale-date"], first + 2 + 600);
    assert_eq!(seen[1].headers["apns-priority"], "5");

    // It keeps coming while the turn lives. The period is long from here on
    // (so the clock moves past it for this one), and no timer fires again
    // during this test.
    rig.pusher.set_live_refresh(600);
    let job = next_job(&mut rig, 5).await;
    rig.now.fetch_add(601, Ordering::SeqCst);
    rig.pusher.run(job).await;
    assert_eq!(rig.apns.seen().len(), 3);

    // A refresh that fires after another push went out waits out what is
    // left of its period instead of sending twice.
    fold(&rig, tool_result("call_1"));
    fold(&rig, tool_use("call_2", "Grep", "x"));
    rig.now.fetch_add(20, Ordering::SeqCst);
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 4, "the new step went");
    rig.pusher
        .run(push::Job::LiveRefresh { key: KEY.into() })
        .await;
    assert_eq!(
        rig.apns.seen().len(),
        4,
        "a push just now is not told again at once"
    );

    // The end stops it: a refresh that fires after finds nobody to tell.
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    let after_end = rig.apns.seen().len();
    assert_eq!(
        content_state(rig.apns.seen().last().unwrap()),
        json!({"phase": "finished"})
    );
    rig.now.fetch_add(1000, Ordering::SeqCst);
    rig.pusher
        .run(push::Job::LiveRefresh { key: KEY.into() })
        .await;
    assert_eq!(rig.apns.seen().len(), after_end, "nothing after the end");
    assert_eq!(rig.run_jobs().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_activity_is_told_again_while_its_question_is_pending() {
    let (mut rig, _phone, _activity) = live_rig().await;
    rig.pusher.set_live_refresh(1);
    fold(&rig, run_started());
    let card = seed_card(
        &rig.vault,
        KEY,
        1,
        "toolu-q1",
        &card_arguments("Who reads it?"),
    );
    fold(
        &rig,
        json!({"type": "interaction", "interaction": card, "sessionKey": KEY, "conversationKey": KEY}),
    );
    rig.run_jobs().await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(content_state(&seen[0])["phase"], "waiting");
    let job = next_job(&mut rig, 5).await;
    rig.now.fetch_add(2, Ordering::SeqCst);
    rig.pusher.run(job).await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        content_state(&seen[1]),
        content_state(&seen[0]),
        "still waiting"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_past_the_live_buffers_tool_cap_still_follows_the_newest_tool() {
    // Review M3: the live buffer stops holding tool calls at its cap, which
    // froze the step of a long turn.
    let (mut rig, _phone, _activity) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, tool_use("call_0", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        content_state(&rig.apns.seen()[0]),
        json!({"phase": "running", "step": "Editing files"})
    );
    fold(&rig, tool_result("call_0"));
    rig.now.fetch_add(20, Ordering::SeqCst);
    for i in 1..=(super::super::live::TOOLS_CAP + 25) {
        fold(&rig, tool_use(&format!("call_{i}"), "Read", "r"));
        fold(&rig, tool_result(&format!("call_{i}")));
    }
    assert_eq!(
        rig.live.snapshot(KEY).expect("running").tools.len(),
        super::super::live::TOOLS_CAP,
        "the buffer is at its cap"
    );
    fold(&rig, tool_use("call_late", "Bash", "echo late"));
    assert_eq!(rig.run_jobs().await, 1);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(
        content_state(&seen[1]),
        json!({"phase": "running", "step": "Running a command"}),
        "the newest tool, though the buffer never held it"
    );
    // When it ends nothing runs but a tool was seen: Working.
    fold(&rig, tool_result("call_late"));
    rig.now.fetch_add(20, Ordering::SeqCst);
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        content_state(rig.apns.seen().last().unwrap()),
        json!({"phase": "running", "step": "Working"})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watch_that_starts_partway_through_a_turn_takes_what_the_buffer_holds() {
    // The token registers after the turn began: the tool that was already
    // running is the step, not "Starting".
    let mut rig = push_rig().await;
    let vault = rig.vault.to_string_lossy().into_owned();
    seed_chat(
        &rig.vault,
        KEY,
        "claude",
        &vault,
        &[("Go", "Done")],
        "acceptEdits",
    );
    let (phone, _) = rig.store.pair("Phone", None, Utc::now()).unwrap();
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "WebFetch", "https://example.test"));
    let now = rig.now.load(Ordering::SeqCst);
    rig.pusher.activities().register(
        &phone.device_id,
        KEY,
        &"cd".repeat(32),
        push::PRODUCTION,
        now,
    );
    fold(&rig, tool_use("call_2", "Grep", "x"));
    fold(&rig, tool_result("call_2"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        content_state(&rig.apns.seen()[0]),
        json!({"phase": "running", "step": "Searching the web"}),
        "the fetch from before the token is still running"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_question_is_not_overwritten_by_a_step_until_it_is_decided() {
    // Review L7: the turn ends right after a card, but its text may stream a
    // moment longer; that must not read "Writing the answer" over the card.
    let (mut rig, _phone, _activity) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    rig.now.fetch_add(20, Ordering::SeqCst);
    fold(&rig, tool_result("call_1"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 2, "editing, then working");
    let card = seed_card(
        &rig.vault,
        KEY,
        1,
        "toolu-q1",
        &card_arguments("Who reads it?"),
    );
    fold(
        &rig,
        json!({"type": "interaction", "interaction": card, "sessionKey": KEY, "conversationKey": KEY}),
    );
    rig.run_jobs().await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(content_state(&seen[2])["phase"], "waiting");
    rig.now.fetch_add(20, Ordering::SeqCst);
    fold(
        &rig,
        json!({"type": "assistantDelta", "conversationKey": KEY, "text": "I asked you."}),
    );
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        rig.apns.seen().len(),
        3,
        "the waiting stays while the card is pending"
    );

    // Decided (the person answered or dismissed it): steps flow again.
    crate::interactions::dismiss(&rig.vault, &card.id).unwrap();
    rig.now.fetch_add(20, Ordering::SeqCst);
    fold(
        &rig,
        json!({"type": "assistantDelta", "conversationKey": KEY, "text": " More."}),
    );
    assert_eq!(rig.run_jobs().await, 1);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(
        content_state(&seen[3]),
        json!({"phase": "running", "step": "Writing the answer"})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_turn_starts_clean_and_a_clock_set_back_does_not_hold_the_steps() {
    // Review L8: an entry that survived a token drop without an `end` would
    // hide an equal first push of the next turn.
    let (mut rig, phone, activity) = live_rig().await;
    rig.pusher.set_live_gap(1);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1);
    // Its token goes without an end (the app ended the activity itself), and
    // the next turn registers a new one for the same chat.
    rig.pusher.activities().forget_chat(KEY);
    let now = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&phone, KEY, &activity, push::PRODUCTION, now);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_2", "Edit", "b"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(
        rig.apns.seen().len(),
        2,
        "the next turn's first step goes though it reads the same"
    );

    // A wall clock set back an hour: the step waits at most the gap.
    rig.now.fetch_sub(3600, Ordering::SeqCst);
    fold(&rig, tool_result("call_2"));
    fold(&rig, tool_use("call_3", "Bash", "c"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 2, "inside the gap nothing goes yet");
    let job = next_job(&mut rig, 5).await;
    assert_eq!(job, push::Job::LiveStep { key: KEY.into() });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_end_apple_did_not_take_is_sent_again_and_a_new_turn_cancels_the_resend() {
    // Review (grok, medium): an end that failed used to drop the token, so a
    // closed app's lock screen kept a running turn that had ended.
    let (mut rig, _phone, activity) = live_rig().await;
    rig.pusher.set_live_end_wait(1);
    let t0 = rig.now.load(Ordering::SeqCst);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    assert_eq!(rig.apns.seen().len(), 1);

    // Apple answers 503 to the end, then 429 to its first re-send, then takes
    // the second.
    rig.apns
        .answer_once(&activity, 503, r#"{"reason":"ServiceUnavailable"}"#);
    rig.apns
        .answer_once(&activity, 429, r#"{"reason":"TooManyRequests"}"#);
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    assert_eq!(rig.apns.seen().len(), 2, "the end went once and failed");
    assert!(
        rig.pusher.activities().has_chat(KEY),
        "its token stays for the re-send"
    );

    let job = next_job(&mut rig, 5).await;
    assert!(
        matches!(&job, push::Job::LiveEnd { key, phase: Phase::Finished, tries: 1, .. } if key == KEY),
        "{job:?}"
    );
    rig.pusher.run(job).await;
    assert_eq!(rig.apns.seen().len(), 3);
    assert!(
        rig.pusher.activities().has_chat(KEY),
        "a 429 is tried again too"
    );

    let job = next_job(&mut rig, 5).await;
    assert!(matches!(job, push::Job::LiveEnd { tries: 2, .. }));
    rig.pusher.run(job).await;
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(
        content_state(seen.last().unwrap()),
        json!({"phase": "finished"})
    );
    assert!(
        !rig.pusher.activities().has_chat(KEY),
        "Apple took it: the token goes"
    );
    // The same state is pushed again, so each push is a second newer than the
    // one before it (ActivityKit drops one that isn't), the clock standing
    // still for the three.
    let stamps: Vec<i64> = seen
        .iter()
        .map(|r| r.body["aps"]["timestamp"].as_i64().unwrap())
        .collect();
    assert_eq!(stamps, vec![t0, t0 + 1, t0 + 2, t0 + 3]);
    assert!(
        rig.jobs.try_recv().is_err(),
        "no third re-send once it went"
    );

    // Out of tries: the token goes with the last failed end, quietly.
    let (mut rig, _phone, activity) = live_rig().await;
    rig.pusher.set_live_end_wait(1);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    rig.apns.answer(&activity, 503, "{}");
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    for tries in 1..=push::LIVE_END_TRIES {
        let job = next_job(&mut rig, 8).await;
        assert!(matches!(job, push::Job::LiveEnd { tries: t, .. } if t == tries));
        rig.pusher.run(job).await;
    }
    assert_eq!(rig.apns.seen().len(), 2 + usize::from(push::LIVE_END_TRIES));
    assert!(
        !rig.pusher.activities().has_chat(KEY),
        "after the last try the token is dropped"
    );
    assert!(rig.jobs.try_recv().is_err(), "and nothing is sent after");

    // The chat's next turn begins before the re-send fires and the app
    // registers that turn's activity (for the same phone and chat, which
    // replaces the old registration): the re-send never tells the new one.
    let (mut rig, phone, old) = live_rig().await;
    rig.pusher.set_live_end_wait(1);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    rig.apns.answer_once(&old, 503, "{}");
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    assert_eq!(rig.apns.seen().len(), 2);
    fold(&rig, run_started());
    let next = "cd".repeat(32);
    let at = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&phone, KEY, &next, push::PRODUCTION, at);
    let job = next_job(&mut rig, 5).await;
    assert!(matches!(job, push::Job::LiveEnd { tries: 1, .. }));
    rig.pusher.run(job).await;
    assert_eq!(
        rig.apns.seen().len(),
        2,
        "the re-send found only the next turn's token (the old one was replaced) and did not tell it"
    );
    assert!(
        rig.pusher.activities().has_chat(KEY),
        "the next turn's token stays registered"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_end_is_for_the_activities_registered_when_the_turn_ended() {
    // Review (grok delta, medium): the end job can wait behind the turn's
    // notification push to Apple while the open app ends the old activity and
    // registers the next turn's; that one must not be told the previous turn
    // is over, nor have its token dropped.
    let (mut rig, phone, _old) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    let sent_before = rig.apns.seen().len();
    fold(&rig, done()); // the jobs are made now and run below, as if late
    rig.pusher.activities().forget_chat(KEY);
    let next = "cd".repeat(32);
    let at = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&phone, KEY, &next, push::PRODUCTION, at);
    assert_eq!(rig.run_jobs().await, 2);
    assert_eq!(
        rig.apns.seen().len(),
        sent_before,
        "the end did not go to the next turn's activity"
    );
    assert!(
        rig.pusher.activities().has_chat(KEY),
        "and its token was not dropped"
    );

    // An old activity whose token is still registered, next to a later
    // registration from ANOTHER phone, gets its end and nobody else does.
    let (mut rig, _phone, old) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    fold(&rig, done());
    let (other, _) = rig.store.pair("Other phone", None, Utc::now()).unwrap();
    let new_token = "ef".repeat(32);
    let at = rig.now.load(Ordering::SeqCst);
    rig.pusher
        .activities()
        .register(&other.device_id, KEY, &new_token, push::PRODUCTION, at);
    assert_eq!(rig.run_jobs().await, 2);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2, "one end, to the old activity only");
    assert!(seen[1].path.ends_with(&old), "{}", seen[1].path);
    let left = rig.pusher.activities().for_chat(KEY, at);
    assert_eq!(left.len(), 1, "only the later registration is left");
    assert_eq!(left[0].token, new_token);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_pushes_in_one_second_go_a_second_apart_and_a_gone_token_is_not_retried() {
    // Review (grok, low): `aps.timestamp` is whole seconds and ActivityKit
    // drops a push that isn't newer than the last one it applied.
    let (mut rig, _phone, _activity) = live_rig().await;
    let t0 = rig.now.load(Ordering::SeqCst);
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    let seen = rig.apns.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body["aps"]["timestamp"], t0);
    assert_eq!(
        seen[1].body["aps"]["timestamp"],
        t0 + 1,
        "the end in the same second goes a second later"
    );

    // An end Apple answers 410 (the activity is gone) is not sent again, and
    // its token goes.
    let (mut rig, _phone, activity) = live_rig().await;
    fold(&rig, run_started());
    fold(&rig, tool_use("call_1", "Edit", "a"));
    assert_eq!(rig.run_jobs().await, 1);
    rig.apns
        .answer_once(&activity, 410, r#"{"reason":"Unregistered"}"#);
    fold(&rig, done());
    assert_eq!(rig.run_jobs().await, 2);
    assert_eq!(rig.apns.seen().len(), 2, "the end went once");
    assert!(!rig.pusher.activities().has_chat(KEY));
    assert!(rig.jobs.try_recv().is_err(), "a gone token is not retried");
    assert_eq!(rig.pusher.status().last_error, None, "a 410 is quiet");
}
