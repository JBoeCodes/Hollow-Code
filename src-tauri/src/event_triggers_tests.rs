//! F2 event triggers: off by default, deduplicated across restarts, capped
//! per hour, never fired for older events, team AI only, and the lead wake
//! reaches the bound project's lead and leaves an open card open.

use super::*;
use crate::run_queue::{self, RowStatus, RunSource};
use std::path::Path;

fn at(text: &str) -> DateTime<Utc> {
    parse(text).unwrap()
}

fn issue(id: &str, created: &str, ws: Option<&str>, team: &str) -> TriggerEvent {
    TriggerEvent {
        kind: EventKind::TrackerIssue.token().into(),
        id: id.into(),
        workspace_id: ws.map(str::to_string),
        at: created.into(),
        title: format!("Issue {id}"),
        key: Some(format!("AI-{}", id.trim_start_matches('i'))),
        url: Some(format!("https://tracker.example/AI-{id}")),
        note: Some(format!("tasks/{id}.md")),
        team_id: Some(team.into()),
    }
}

/// Everything is bound (an automation would run for each event).
fn all_bound(_: &TriggerEvent) -> Vec<String> {
    vec!["watcher".into()]
}

fn on_for(state: &mut State, ws: &str, now: DateTime<Utc>) {
    state
        .set_workspace(EventKind::TrackerIssue, ws, true, now)
        .unwrap();
}

#[test]
fn every_trigger_is_off_by_default_and_fires_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = load(dir.path());
    assert_eq!(state, State::default(), "nothing saved yet");
    for kind in EventKind::ALL {
        assert!(!state.is_on(kind), "{kind:?}");
        assert_eq!(state.cap(kind), DEFAULT_CAP);
    }
    let mut state = state;
    let now = at("2026-11-10T12:00:00Z");
    let events = [issue("i1", "2026-11-10T11:59:00Z", Some("ws"), TEAM)];
    assert!(decide(
        &mut state,
        EventKind::TrackerIssue,
        &events,
        now,
        &all_bound
    )
    .is_empty());
    assert!(
        state.seen.is_empty(),
        "an event while off is not even recorded"
    );
    // The kind on but the project off: still nothing.
    state.set_kind(EventKind::TrackerIssue, true, now - Duration::hours(1));
    assert!(decide(
        &mut state,
        EventKind::TrackerIssue,
        &events,
        now,
        &all_bound
    )
    .is_empty());
}

#[test]
fn an_event_fires_once_even_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let on = at("2026-11-10T09:00:00Z");
    update(v, |s| {
        on_for(s, "ws", on);
        Ok(())
    })
    .unwrap();
    let events = [issue("i1", "2026-11-10T10:00:00Z", Some("ws"), TEAM)];
    let now = at("2026-11-10T10:01:00Z");
    let fired = update(v, |s| {
        Ok(decide(s, EventKind::TrackerIssue, &events, now, &all_bound))
    })
    .unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].1, vec!["watcher".to_string()]);
    // A restart reads the saved seen-set: the same event never fires again.
    let mut reloaded = load(v);
    assert!(reloaded.was_seen(EventKind::TrackerIssue, "i1"));
    let again = decide(
        &mut reloaded,
        EventKind::TrackerIssue,
        &events,
        now + Duration::minutes(5),
        &all_bound,
    );
    assert!(again.is_empty());
    // The seen-set is bounded.
    let mut many = State::default();
    on_for(&mut many, "ws", on);
    many.set_cap(EventKind::TrackerIssue, MAX_CAP);
    let burst: Vec<TriggerEvent> = (0..SEEN_MAX + 20)
        .map(|n| issue(&format!("i{n}"), "2026-11-10T10:00:00Z", Some("ws"), TEAM))
        .collect();
    decide(&mut many, EventKind::TrackerIssue, &burst, now, &|_| {
        Vec::new()
    });
    assert_eq!(many.seen[EventKind::TrackerIssue.token()].len(), SEEN_MAX);
}

#[test]
fn the_hourly_cap_drops_the_rest_and_frees_up_an_hour_later() {
    let on = at("2026-11-10T09:00:00Z");
    let now = at("2026-11-10T10:00:00Z");
    let mut state = State::default();
    on_for(&mut state, "ws", on);
    let burst: Vec<TriggerEvent> = (0..8)
        .map(|n| issue(&format!("i{n}"), "2026-11-10T09:30:00Z", Some("ws"), TEAM))
        .collect();
    let fired = decide(&mut state, EventKind::TrackerIssue, &burst, now, &all_bound);
    assert_eq!(fired.len(), DEFAULT_CAP as usize);
    let capped: Vec<&Seen> = state.seen[EventKind::TrackerIssue.token()]
        .iter()
        .filter(|s| s.outcome == "capped")
        .collect();
    assert_eq!(capped.len(), 2, "recorded, never queued for later");
    assert_eq!(state.fired_last_hour(EventKind::TrackerIssue, now), 6);
    // An unbound event never counts against the cap.
    let lone = [issue("lone", "2026-11-10T09:40:00Z", Some("ws"), TEAM)];
    assert!(
        decide(&mut state, EventKind::TrackerIssue, &lone, now, &|_| {
            Vec::new()
        })
        .is_empty()
    );
    assert_eq!(
        state.seen[EventKind::TrackerIssue.token()]
            .last()
            .unwrap()
            .outcome,
        "unbound"
    );
    // An hour on, the cap is free again; the capped ones stay dropped.
    let later = now + Duration::minutes(61);
    let next = [issue("i9", "2026-11-10T10:30:00Z", Some("ws"), TEAM)];
    let mut all = burst.clone();
    all.extend(next.clone());
    let fired = decide(&mut state, EventKind::TrackerIssue, &all, later, &all_bound);
    assert_eq!(
        fired.iter().map(|f| f.0.id.as_str()).collect::<Vec<_>>(),
        vec!["i9"]
    );
    // A lower cap applies at once.
    state.set_cap(EventKind::TrackerIssue, 1);
    let two = [
        issue("j1", "2026-11-10T10:40:00Z", Some("ws"), TEAM),
        issue("j2", "2026-11-10T10:41:00Z", Some("ws"), TEAM),
    ];
    assert!(decide(&mut state, EventKind::TrackerIssue, &two, later, &all_bound).is_empty());
}

#[test]
fn events_from_before_a_trigger_was_turned_on_never_fire() {
    let on = at("2026-11-10T09:00:00Z");
    let now = at("2026-11-10T10:00:00Z");
    let mut state = State::default();
    state.set_kind(EventKind::TrackerIssue, true, on - Duration::days(1));
    // The project was turned on later than the kind: its time counts.
    on_for(&mut state, "ws", on);
    let events = [
        issue("old", "2026-11-10T08:59:59Z", Some("ws"), TEAM),
        issue("new", "2026-11-10T09:00:01Z", Some("ws"), TEAM),
    ];
    let fired = decide(
        &mut state,
        EventKind::TrackerIssue,
        &events,
        now,
        &all_bound,
    );
    assert_eq!(
        fired.iter().map(|f| f.0.id.as_str()).collect::<Vec<_>>(),
        vec!["new"]
    );
    // Off and on again: since moves, so an event between never fires.
    state.set_kind(EventKind::TrackerIssue, false, now);
    let between = [issue("between", "2026-11-10T10:05:00Z", Some("ws"), TEAM)];
    state.set_kind(EventKind::TrackerIssue, true, now + Duration::minutes(10));
    assert!(decide(
        &mut state,
        EventKind::TrackerIssue,
        &between,
        now + Duration::minutes(11),
        &all_bound
    )
    .is_empty());

    // Recordings carry a day only: the first pass after turning on records
    // every listed one as seen, then only new ids fire.
    let mut rec = State::default();
    rec.set_kind(EventKind::RecordingEnded, true, on);
    let rows = vec![crate::recordings::RecordingRow {
        id: "rec-001".into(),
        name: "Standup".into(),
        date: Some("2026-11-10".into()),
        duration: Some("12:00".into()),
    }];
    let listed = recording_events(&rows);
    // A pass with no clean Plaud read: nothing fires and nothing primes.
    assert!(decide(
        &mut rec,
        EventKind::RecordingEnded,
        &listed,
        now,
        &all_bound
    )
    .is_empty());
    assert!(!rec.kind(EventKind::RecordingEnded).primed);
    assert!(rec.seen.is_empty());
    // A clean read primes, with only the recordings' own ids.
    let mut read = listed.clone();
    read.push(issue(
        "not-a-recording",
        "2026-11-10T09:30:00Z",
        Some("ws"),
        TEAM,
    ));
    prime(&mut rec, &read, now);
    assert!(rec.kind(EventKind::RecordingEnded).primed);
    assert_eq!(
        rec.seen[EventKind::RecordingEnded.token()]
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec!["recording:rec-001"]
    );
    assert!(decide(
        &mut rec,
        EventKind::RecordingEnded,
        &listed,
        now,
        &all_bound
    )
    .is_empty());
    let mut more = rows.clone();
    more.push(crate::recordings::RecordingRow {
        id: "rec-002".into(),
        name: "Design review".into(),
        date: Some("2026-11-10".into()),
        duration: None,
    });
    more.push(crate::recordings::RecordingRow {
        id: "rec-000".into(),
        name: "Yesterday".into(),
        date: Some("2026-11-09".into()),
        duration: None,
    });
    let fired = decide(
        &mut rec,
        EventKind::RecordingEnded,
        &recording_events(&more),
        now,
        &all_bound,
    );
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].0.id, "recording:rec-002");
    // A meeting's end is exact; one still running or long over is no event.
    let view = serde_json::json!({"events": [
        {"id": "m1", "subject": "Planning", "end": "2026-11-10T09:30:00Z"},
        {"id": "m2", "subject": "Later", "end": "2026-11-10T11:00:00Z"},
        {"id": "m3", "subject": "Before on", "end": "2026-11-10T08:30:00Z"},
        {"id": "m4", "subject": "Cancelled", "end": "2026-11-10T09:45:00Z", "cancelled": true},
    ]});
    let meetings = meeting_events(&view, now);
    assert_eq!(meetings.len(), 2, "{meetings:?}");
    let fired = decide(
        &mut rec,
        EventKind::RecordingEnded,
        &meetings,
        now,
        &all_bound,
    );
    assert_eq!(fired.len(), 1);
    assert!(fired[0].0.id.starts_with("meeting:m1@"));
}

#[test]
fn an_issue_outside_team_ai_never_fires() {
    let on = at("2026-11-10T09:00:00Z");
    let now = at("2026-11-10T10:00:00Z");
    let mut state = State::default();
    on_for(&mut state, "ws", on);
    let events = [
        issue("other", "2026-11-10T09:30:00Z", Some("ws"), "team-other"),
        issue("unbound-ws", "2026-11-10T09:30:00Z", None, TEAM),
    ];
    assert!(decide(
        &mut state,
        EventKind::TrackerIssue,
        &events,
        now,
        &all_bound
    )
    .is_empty());
    assert!(!state.was_seen(EventKind::TrackerIssue, "other"));
    // The ledger mapping: an issue's project names its workspace.
    let mut ledger_issue: crate::tracker::TrackerIssue = serde_json::from_value(serde_json::json!({
        "id": "i-77", "key": "ENG-77", "url": "https://tracker.example/ENG-77", "title": "Fix the gate",
        "state": {"id": "s1", "name": "Todo", "kind": "unstarted"}, "teamId": TEAM,
        "updatedAt": "2026-11-10T09:30:00Z", "createdAt": "2026-11-10T09:30:00Z", "projectId": "p1"
    }))
    .unwrap();
    let tracked = vec![crate::tracker::status::Tracked {
        workspace_id: "ws".into(),
        title: "WS".into(),
        project_id: "p1".into(),
    }];
    let mapped = issue_events(
        &[(ledger_issue.clone(), "tasks/ENG-77.md".into())],
        &tracked,
    );
    assert_eq!(mapped[0].workspace_id.as_deref(), Some("ws"));
    assert_eq!(mapped[0].key.as_deref(), Some("ENG-77"));
    assert_eq!(
        decide(
            &mut state,
            EventKind::TrackerIssue,
            &mapped,
            now,
            &all_bound
        )
        .len(),
        1
    );
    ledger_issue.id = "i-78".into();
    ledger_issue.team_id = "team-other".into();
    let other = issue_events(&[(ledger_issue, String::new())], &tracked);
    assert!(decide(&mut state, EventKind::TrackerIssue, &other, now, &all_bound).is_empty());
}

#[test]
fn merged_prs_map_to_each_bound_project_and_automations_bind_by_token() {
    let json = r#"[{"number": 12, "title": "Ship it", "url": "https://github.com/acme/app/pull/12", "mergedAt": "2026-11-10T09:30:00Z"}]"#;
    let events = pr_events(json, &["ws-a".to_string()]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].key.as_deref(), Some("acme/app#12"));
    assert_eq!(events[0].workspace_id.as_deref(), Some("ws-a"));
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let folder = v.join(crate::vault_workflows::WORKFLOWS_DIR);
    std::fs::create_dir_all(&folder).unwrap();
    for (slug, triggers) in [
        ("for-a", "pr-merged:ws-a"),
        ("for-b", "pr-merged:ws-b"),
        ("any", "pr-merged"),
        ("issues", "tracker-issue:ws-a"),
    ] {
        std::fs::write(
            folder.join(format!("{slug}.md")),
            format!("---\ntitle: {slug}\nschedule: manual\ntriggers: {triggers}\n---\nBODY\n"),
        )
        .unwrap();
    }
    let flows = crate::vault_workflows::list(v);
    assert_eq!(flows.len(), 4);
    // A trigger row per bound automation, keyed by the event: a repeat is a
    // duplicate, never a second run.
    let mut state = State::default();
    let fired = vec![(
        events[0].clone(),
        vec!["for-a".to_string(), "any".to_string()],
    )];
    let now = at("2026-11-10T10:00:00Z");
    let rows = dispatch(v, &mut state, &fired, &flows, now);
    assert_eq!(rows.len(), 2);
    let again = dispatch(v, &mut state, &fired, &flows, now);
    assert_eq!(again, rows, "the same rows");
    let row = run_queue::get(v, rows[0]).unwrap().unwrap();
    assert_eq!(row.source, RunSource::Trigger);
    assert!(row.reason.contains("acme/app#12"), "{}", row.reason);
    let tokens = run_queue::fired_tokens(&row.payload);
    assert_eq!(tokens[0].event_ids, vec![events[0].id.clone()]);
    let mut bound: Vec<String> = workflow_bindings(&flows, &events[0])
        .into_iter()
        .map(|b| b.slug)
        .collect();
    bound.sort();
    assert_eq!(bound, vec!["any", "for-a"]);
}

fn config_for(v: &Path) -> crate::config::Config {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = v.to_path_buf();
    config.tracker.provider = Some("linear".into());
    config.tracker.team_id = Some(TEAM.into());
    config
}

/// Two tracked projects; the lead of the one whose project the issue names
/// is woken, with a short unattended turn quoting it. The project's open
/// question card holds that turn and stays open.
#[test]
fn the_lead_wake_targets_the_bound_projects_lead_and_a_card_stays_open() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let v = dir.path();
    let a = crate::workspaces::create(v, "Alpha", "Build Alpha").unwrap();
    let b = crate::workspaces::create(v, "Beta", "Build Beta").unwrap();
    for (ws, project) in [(&a, "p-alpha"), (&b, "p-beta")] {
        std::fs::write(
            v.join("workspaces").join(&ws.id).join("PROJECT.md"),
            format!(
                "---\ntitle: {}\ntracker: linear\ntracker_project: {project}\n---\n",
                ws.title
            ),
        )
        .unwrap();
    }
    let config = config_for(v);
    let now = Utc::now();
    // Off by default; an untracked id is refused.
    let status = set(
        &config,
        &SetTrigger {
            kind: "tracker-issue".into(),
            workspace_id: Some(b.id.clone()),
            lead_wake: Some(true),
            ..SetTrigger::default()
        },
        now - Duration::minutes(10),
    )
    .unwrap();
    assert_eq!(status.lead_wakes, vec![b.id.clone()]);
    assert!(status.kinds[0].on, "the lead wake turns the kind on");
    assert!(set(
        &config,
        &SetTrigger {
            kind: "tracker-issue".into(),
            workspace_id: Some("nope".into()),
            lead_wake: Some(true),
            ..SetTrigger::default()
        },
        now,
    )
    .is_err());

    // The lead conversation exists with an open card.
    let lead = crate::chat_chain::lead_current(v, &b.id).unwrap();
    let ord =
        crate::session_store::start_turn(v, &lead, "claude", &b.cwd, "run-lead", "ask").unwrap();
    // The lead runs in bypass: its woken turn keeps it (2026-11-12).
    crate::session_store::record_turn_permission(v, &lead, ord, "bypassPermissions").unwrap();
    let arguments = Ok(serde_json::json!({
        "mode": "confirmation",
        "supersedeOnUserMessage": true,
        "confirmation": { "prompt": "Go?" }
    }));
    let card = crate::interactions::record(
        v,
        &crate::interactions::NewInteraction {
            session_key: &lead,
            turn_ord: ord,
            run_id: "run-lead",
            agent: crate::agent::AgentKind::Claude,
            native_session_id: None,
            tool_use_id: "toolu-lead",
            source: crate::interactions::InteractionSource::Mcp,
            arguments: &arguments,
            cwd: Path::new(&b.cwd),
            vault_root: v,
        },
    )
    .unwrap()
    .interaction;
    crate::session_store::finish_turn(v, &lead, ord, "success").ok();

    let tracked = crate::tracker::status::tracked_workspaces(v, "linear");
    let mut ledger_issue: crate::tracker::TrackerIssue = serde_json::from_value(serde_json::json!({
        "id": "i-501", "key": "ENG-501", "url": "https://tracker.example/ENG-501", "title": "Beta needs a login page",
        "state": {"id": "s1", "name": "Todo", "kind": "unstarted"}, "teamId": TEAM,
        "updatedAt": now.to_rfc3339(), "createdAt": now.to_rfc3339(), "projectId": "p-beta"
    }))
    .unwrap();
    let mut issues = vec![(ledger_issue.clone(), "tasks/ENG-501.md".to_string())];
    ledger_issue.id = "i-502".into();
    ledger_issue.project_id = Some("p-alpha".into());
    issues.push((ledger_issue, String::new()));
    let events = issue_events(&issues, &tracked);

    let queued = update(v, |state| {
        let snapshot = state.clone();
        let bound = |e: &TriggerEvent| bindings(&snapshot, &[], e);
        let fired = decide(state, EventKind::TrackerIssue, &events, now, &bound);
        assert_eq!(fired.len(), 1, "Alpha is off: only Beta's issue fires");
        assert_eq!(fired[0].1, vec![format!("lead:{}", b.id)]);
        assert!(
            dispatch(v, state, &fired, &[], now).is_empty(),
            "no automation row"
        );
        assert_eq!(state.pending.len(), 1);
        Ok(deliver_pending(v, state, now))
    })
    .unwrap();
    assert_eq!(queued.len(), 1, "one lead turn");
    assert!(load(v).pending.is_empty());
    let row = run_queue::get(v, queued[0]).unwrap().unwrap();
    assert_eq!(row.source, RunSource::Chat);
    assert_eq!(row.source_id.as_deref(), Some(lead.as_str()));
    assert_ne!(
        row.source_id,
        crate::chat_chain::lead_current(v, &a.id),
        "never the other project's lead"
    );
    assert_eq!(row.payload["hostAsker"], serde_json::json!(ASKED_BY_EVENT));
    assert_eq!(row.payload["unattended"], serde_json::json!(true));
    assert_eq!(row.payload["permissionMode"], "bypassPermissions");
    let prompt = row.payload["prompt"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    for part in [
        "ENG-501",
        "Beta needs a login page",
        "https://tracker.example/ENG-501",
        "tasks/ENG-501.md",
    ] {
        assert!(prompt.contains(part), "{part} in {prompt}");
    }
    assert!(!prompt.contains("ENG-502"));

    // The card stays open, and holds the woken turn behind it.
    assert!(!run_queue::payload_closes_cards(&row.payload));
    let pending = crate::interactions::list_pending(v, &lead).unwrap();
    assert!(
        pending.iter().any(|c| c.id == card.id),
        "the card stays open"
    );
    assert!(crate::interactions::open_hold_until(v, &lead, now).is_some());
    let live = run_queue::live_rows(v).unwrap();
    let facts = crate::run_pump::QueueFacts::read(v, &live);
    let woken = live.iter().find(|r| r.id == queued[0]).unwrap();
    assert_eq!(woken.status, RowStatus::Queued);
    assert_eq!(
        crate::run_pump::item_with_reason(v, woken, Some(1), &facts, now)
            .waiting_reason
            .as_deref(),
        Some(crate::run_pump::CARD_HOLD_WAIT)
    );

    // A second issue while that turn is still queued waits for the lead,
    // never a second turn beside it.
    let second = [TriggerEvent {
        id: "i-503".into(),
        key: Some("ENG-503".into()),
        ..events.iter().find(|e| e.id == "i-501").unwrap().clone()
    }];
    let again = update(v, |state| {
        let snapshot = state.clone();
        let bound = |e: &TriggerEvent| bindings(&snapshot, &[], e);
        let fired = decide(state, EventKind::TrackerIssue, &second, now, &bound);
        dispatch(v, state, &fired, &[], now);
        Ok(deliver_pending(v, state, now))
    })
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(load(v).pending.len(), 1, "kept for the next pass");

    // Turning the lead wake off drops its pending wake and the project.
    let status = set(
        &config,
        &SetTrigger {
            kind: "tracker-issue".into(),
            workspace_id: Some(b.id.clone()),
            lead_wake: Some(false),
            ..SetTrigger::default()
        },
        now,
    )
    .unwrap();
    assert!(status.lead_wakes.is_empty());
    assert_eq!(status.pending_wakes, 0);
    assert!(status.kinds[0].workspaces.is_empty());
}

/// Review 1: an evicted id is never decided again, so a full seen-set can't
/// cascade into re-firing every old event, and a relisted one stays quiet.
#[test]
fn eviction_never_refires_old_events() {
    let on = at("2026-11-10T00:00:00Z");
    let now = at("2026-11-10T23:00:00Z");
    let mut state = State::default();
    on_for(&mut state, "ws", on);
    state.set_cap(EventKind::TrackerIssue, MAX_CAP);
    let stamp = |n: usize| (on + Duration::seconds(n as i64 + 1)).to_rfc3339();
    let all: Vec<TriggerEvent> = (0..=SEEN_MAX)
        .map(|n| issue(&format!("i{n}"), &stamp(n), Some("ws"), TEAM))
        .collect();
    decide(&mut state, EventKind::TrackerIssue, &all, now, &|_| {
        Vec::new()
    });
    assert!(!state.was_seen(EventKind::TrackerIssue, "i0"), "evicted");
    assert_eq!(
        state
            .kind(EventKind::TrackerIssue)
            .evicted_through
            .as_deref()
            .and_then(parse),
        parse(&stamp(0))
    );
    // The next ledger pass lists them all again, now bound: no cascade.
    let before = state.seen[EventKind::TrackerIssue.token()].clone();
    for _ in 0..3 {
        assert!(decide(&mut state, EventKind::TrackerIssue, &all, now, &all_bound).is_empty());
    }
    assert_eq!(state.seen[EventKind::TrackerIssue.token()], before);
    // The high-water mark survives a save and reload.
    let dir = tempfile::tempdir().unwrap();
    update(dir.path(), |s| {
        *s = state.clone();
        Ok(())
    })
    .unwrap();
    let mut reloaded = load(dir.path());
    assert!(decide(
        &mut reloaded,
        EventKind::TrackerIssue,
        &all[..1],
        now,
        &all_bound
    )
    .is_empty());

    // E, then 500 newer unbound events evict it; E relisted never fires.
    let mut rec = State::default();
    rec.set_kind(EventKind::RecordingEnded, true, on);
    let meeting = |n: usize| TriggerEvent {
        kind: EventKind::RecordingEnded.token().into(),
        id: format!("meeting:m{n}@x"),
        workspace_id: None,
        at: stamp(n),
        title: format!("Meeting {n}"),
        key: None,
        url: None,
        note: None,
        team_id: None,
    };
    let e = meeting(0);
    assert_eq!(
        decide(
            &mut rec,
            EventKind::RecordingEnded,
            std::slice::from_ref(&e),
            now,
            &all_bound
        )
        .len(),
        1
    );
    let newer: Vec<TriggerEvent> = (1..=SEEN_MAX).map(meeting).collect();
    decide(&mut rec, EventKind::RecordingEnded, &newer, now, &|_| {
        Vec::new()
    });
    assert!(!rec.was_seen(EventKind::RecordingEnded, &e.id));
    assert!(decide(
        &mut rec,
        EventKind::RecordingEnded,
        &[e],
        now + Duration::hours(2),
        &all_bound
    )
    .is_empty());
}

/// Review 3 and 4: the lead wake has its own switch and time. Turning the
/// project's trigger off leaves the lead wake; a lead wakes only for issues
/// created after its own switch; a pending wake whose switch went off drops.
#[test]
fn the_lead_wake_is_its_own_switch_with_its_own_time() {
    let project_on = at("2026-11-10T09:00:00Z");
    let lead_on = at("2026-11-10T10:00:00Z");
    let now = at("2026-11-10T11:00:00Z");
    let mut state = State::default();
    on_for(&mut state, "ws", project_on);
    state.set_lead_wake("ws", true, lead_on);
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let folder = v.join(crate::vault_workflows::WORKFLOWS_DIR);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("triage.md"),
        "---\ntitle: triage\nschedule: manual\ntriggers: tracker-issue\n---\nBODY\n",
    )
    .unwrap();
    let flows = crate::vault_workflows::list(v);
    let early = issue("early", "2026-11-10T09:30:00Z", Some("ws"), TEAM);
    let late = issue("late", "2026-11-10T10:30:00Z", Some("ws"), TEAM);
    assert_eq!(bindings(&state, &flows, &early), vec!["triage".to_string()]);
    assert_eq!(
        bindings(&state, &flows, &late),
        vec!["triage".to_string(), "lead:ws".to_string()]
    );
    // The editor's Turn off: the project's trigger only.
    state
        .set_workspace(EventKind::TrackerIssue, "ws", false, now)
        .unwrap();
    assert!(state.lead_wakes.contains_key("ws"), "the lead still wakes");
    assert_eq!(bindings(&state, &flows, &late), vec!["lead:ws".to_string()]);
    let snapshot = state.clone();
    let fired = decide(
        &mut state,
        EventKind::TrackerIssue,
        std::slice::from_ref(&late),
        now,
        &|e| bindings(&snapshot, &flows, e),
    );
    assert_eq!(fired.len(), 1, "the lead alone still takes it");
    // A wake pending when its switch goes off is dropped, not sent.
    dispatch(v, &mut state, &fired, &flows, now);
    assert_eq!(state.pending.len(), 1);
    state.lead_wakes.remove("ws");
    assert!(deliver_pending(v, &mut state, now).is_empty());
    assert!(state.pending.is_empty());
    let seen = state.seen[EventKind::TrackerIssue.token()].last().unwrap();
    assert!(seen.outcome.contains("turned off"), "{}", seen.outcome);
}

/// Review 5: passing failures keep a wake, permanent ones drop it, each
/// wake expires by its own age, and a PR bound to two projects counts once.
#[test]
fn wakes_retry_passing_failures_and_a_shared_pr_counts_once() {
    assert!(matches!(
        WakeError::from_accept(crate::chat::CHAT_ALREADY_QUEUED.to_string()),
        WakeError::Retry(_)
    ));
    assert!(matches!(
        WakeError::from_accept("session store: database is locked".into()),
        WakeError::Retry(_)
    ));
    assert!(matches!(
        WakeError::from_accept("UNSAFE_WORKING_DIRECTORY: /".into()),
        WakeError::Never(_)
    ));
    assert!(matches!(
        WakeError::from_accept(
            "This lead conversation belongs to another project. Nothing was queued.".into()
        ),
        WakeError::Never(_)
    ));

    // Each wake by its own age: the old one expires, the young one doesn't.
    let now = at("2026-11-10T20:00:00Z");
    let dir = tempfile::tempdir().unwrap();
    let mut state = State::default();
    state.set_lead_wake("ws", true, now - Duration::hours(10));
    for (id, age) in [("old", PENDING_HOURS + 1), ("young", 1)] {
        let e = issue(id, "2026-11-10T11:00:00Z", Some("ws"), TEAM);
        state.remember(
            EventKind::TrackerIssue,
            Seen {
                id: id.into(),
                at: now.to_rfc3339(),
                outcome: "fired".into(),
                title: String::new(),
                workspace_id: Some("ws".into()),
                started: vec!["lead:ws".into()],
                event_at: Some(e.at.clone()),
            },
        );
        state.pending.push(PendingWake {
            workspace_id: "ws".into(),
            event: e,
            since: (now - Duration::hours(age)).to_rfc3339(),
        });
    }
    deliver_pending(dir.path(), &mut state, now);
    let outcome = |id: &str| {
        state.seen[EventKind::TrackerIssue.token()]
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .outcome
            .clone()
    };
    assert!(outcome("old").contains("hours"), "{}", outcome("old"));
    assert!(!outcome("young").contains("hours"), "{}", outcome("young"));

    // One merged PR in a repo two projects share: two events, one cap slot.
    let mut pr = State::default();
    let on = at("2026-11-10T09:00:00Z");
    for ws in ["ws-a", "ws-b"] {
        pr.set_workspace(EventKind::PrMerged, ws, true, on).unwrap();
    }
    pr.set_cap(EventKind::PrMerged, 1);
    let json = r#"[{"number": 7, "title": "One", "url": "https://github.com/acme/app/pull/7", "mergedAt": "2026-11-10T09:30:00Z"},
                  {"number": 8, "title": "Two", "url": "https://github.com/acme/app/pull/8", "mergedAt": "2026-11-10T09:40:00Z"}]"#;
    let events = pr_events(json, &["ws-a".to_string(), "ws-b".to_string()]);
    assert_eq!(events.len(), 4);
    let fired = decide(&mut pr, EventKind::PrMerged, &events, now, &all_bound);
    let ids: Vec<&str> = fired.iter().map(|f| f.0.id.as_str()).collect();
    assert_eq!(ids, vec!["acme/app#7@ws-a", "acme/app#7@ws-b"]);
    assert_eq!(pr.fired_last_hour(EventKind::PrMerged, now), 1);
}

/// Delta review: the high-water mark never runs ahead of the eviction, an
/// evicted entry with no event time still advances it, and turning a kind or
/// a lead wake off drops its pending wakes at once.
#[test]
fn the_mark_is_clamped_and_always_advances_and_off_drops_pending() {
    let now = at("2026-11-10T12:00:00Z");
    let seen = |id: &str, event_at: Option<&str>| Seen {
        id: id.into(),
        at: "2026-11-10T08:00:00Z".into(),
        outcome: "unbound".into(),
        title: String::new(),
        workspace_id: None,
        started: Vec::new(),
        event_at: event_at.map(str::to_string),
    };
    let fill = |state: &mut State, first: Seen| {
        state
            .seen
            .insert(EventKind::TrackerIssue.token().into(), vec![first]);
        for n in 1..SEEN_MAX {
            state
                .seen
                .get_mut(EventKind::TrackerIssue.token())
                .unwrap()
                .push(seen(&format!("x{n}"), Some("2026-11-10T07:00:00Z")));
        }
        state.remember(
            EventKind::TrackerIssue,
            Seen {
                at: now.to_rfc3339(),
                ..seen("last", Some("2026-11-10T07:00:00Z"))
            },
        );
    };
    let mark = |state: &State| {
        state
            .kind(EventKind::TrackerIssue)
            .evicted_through
            .as_deref()
            .and_then(parse)
    };
    // An entry dated in the future evicts at most up to now.
    let mut future = State::default();
    fill(&mut future, seen("future", Some("2026-12-01T00:00:00Z")));
    assert_eq!(mark(&future), Some(now));
    on_for(&mut future, "ws", at("2026-11-10T00:00:00Z"));
    let real = issue("real", "2026-11-10T12:30:00Z", Some("ws"), TEAM);
    assert_eq!(
        decide(
            &mut future,
            EventKind::TrackerIssue,
            &[real],
            now,
            &all_bound
        )
        .len(),
        1
    );
    // No event time: its decision time advances the mark.
    let mut timeless = State::default();
    fill(&mut timeless, seen("timeless", None));
    assert_eq!(mark(&timeless), Some(at("2026-11-10T08:00:00Z")));

    // Off drops pending wakes at once; on again delivers nothing old.
    let mut state = State::default();
    state.set_lead_wake("ws", true, at("2026-11-10T09:00:00Z"));
    let wake = PendingWake {
        workspace_id: "ws".into(),
        event: issue("i1", "2026-11-10T10:00:00Z", Some("ws"), TEAM),
        since: now.to_rfc3339(),
    };
    state.pending.push(wake.clone());
    state.set_kind(EventKind::TrackerIssue, false, now);
    assert!(state.pending.is_empty(), "the kind off drops them");
    state.set_kind(EventKind::TrackerIssue, true, now);
    state.pending.push(wake);
    state.set_lead_wake("ws", false, now);
    assert!(state.pending.is_empty(), "the lead wake off drops them");
}
