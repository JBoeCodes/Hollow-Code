//! The `run_pump` unit tests (split out of `run_pump.rs` by B11, 2026-10-16,
//! no behaviour change). Attached from `run_pump.rs` as `#[path] mod tests`,
//! so `super::` is still `crate::run_pump`.

use super::*;
use crate::automated_history::{begin_run, finish_run, lease_holder, HistoryRun};
use crate::run_queue::NewRow;
use crate::vault_workflows::{seed_workflow, seed_workflow_with, MISSED_WHILE_CLOSED_NOTE};
use std::cell::{Cell, RefCell};

// Workflow slugs are process-global (the running registry), so every
// test seeds titles no other test uses.

fn temp_vault() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}

fn due_cron(vault: &Path, title: &str, due_at: DateTime<Utc>) -> Workflow {
    seed_workflow_with(vault, title, "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(due_at.to_rfc3339());
    })
}

/// Scheduler v2 regression: a window automation whose occurrence was
/// missed while Hollow was closed runs once on wake (run-once-on-wake is
/// the window default) instead of being skipped past its grace; a cron
/// automation keeps its grace, and `/skip` restores it.
#[test]
fn a_missed_window_occurrence_runs_on_wake_but_a_cron_keeps_its_grace() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let due = (now - chrono::Duration::days(3)).to_rfc3339();
    let window = seed_workflow_with(v, "Window catch-up F1", "window:friday-pm", "BODY", |fm| {
        fm.next_run_at = Some(due.clone());
    });
    assert!(catches_up(&window));
    let cron = due_cron(v, "Cron grace F1", now - chrono::Duration::days(3));
    assert_eq!(
        past_grace(&cron, &due, now),
        Some(vault_workflows::DEFAULT_MISFIRE_GRACE_MIN)
    );
    let skip = seed_workflow_with(v, "Window skip F1", "window:friday-pm/skip", "BODY", |fm| {
        fm.next_run_at = Some(due.clone());
    });
    assert!(!catches_up(&skip));
    assert!(!catches_up(&cron));
    // At claim: a runnable missed window occurrence runs late; a held one
    // (invalid or refused) keeps its grace and is skipped, never deferred
    // into a run at the wrong hour (review finding 1).
    let id = queue_cron(v, &window, now);
    let queued = row(v, id);
    let facts = |precheck| ClaimFacts {
        workflow: WorkflowFact::Present(window.clone()),
        precheck: Some(precheck),
        live_overdue_ids: None,
        live_unblocked_ids: None,
        heartbeat_enabled: true,
    };
    assert!(matches!(
        decide_cron(&queued, &facts(Ok(())), now),
        Decision::Run { .. }
    ));
    assert!(matches!(
        decide_cron(
            &queued,
            &facts(Err(PrecheckFailure::Invalid("empty prompt".into()))),
            now
        ),
        Decision::Verdict(ClaimVerdict::Skip { .. })
    ));
    // After the late run, the next occurrence is in the future: the three
    // missed Fridays collapse into the one run.
    let next = vault_workflows::next_after_run(&window, now).unwrap();
    assert!(next > now);
}

/// Moving an approved automation onto a window is the owned schedule
/// write: it still passes the permission check, and an unknown window
/// changes nothing.
#[test]
fn set_automation_window_keeps_the_approval_and_refuses_unknown_windows() {
    let dir = temp_vault();
    let v = dir.path();
    let w = seed_workflow(v, "Window move F1", "0 0 7 * * * *", "BODY");
    let moved =
        crate::schedule_windows::apply_window(v, &w.slug, "friday-pm", Some("run-all")).unwrap();
    assert_eq!(moved.schedule, "window:friday-pm/run-all");
    assert_eq!(moved.catch_up.as_deref(), Some("run-all"));
    assert!(moved.next_run_at.is_some());
    assert!(crate::workflow_permissions::verify(v, &w.slug).is_ok());
    let before = workflow_bytes(v, &w.slug);
    assert!(
        crate::schedule_windows::apply_window(v, &w.slug, "someday", None)
            .unwrap_err()
            .contains("Unknown window")
    );
    assert!(
        crate::schedule_windows::apply_window(v, &w.slug, "nightly", Some("often"))
            .unwrap_err()
            .contains("Nothing changed")
    );
    assert_eq!(workflow_bytes(v, &w.slug), before);
    let manual = crate::schedule_windows::apply_window(v, &w.slug, "none", None).unwrap();
    assert_eq!(manual.schedule, "manual");
    assert!(crate::workflow_permissions::verify(v, &w.slug).is_ok());
}

fn workflow_bytes(vault: &Path, slug: &str) -> Vec<u8> {
    std::fs::read(vault.join(format!("workflows/{slug}.md"))).unwrap()
}

fn run_log(vault: &Path, slug: &str) -> Vec<RunLogEntry> {
    vault_workflows::read_full(vault, slug).unwrap().2
}

fn next_run(vault: &Path, slug: &str) -> Option<String> {
    vault_workflows::read_full(vault, slug)
        .unwrap()
        .0
        .next_run_at
}

fn touch_note(vault: &Path, rel: &str) {
    let path = vault.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "changed").unwrap();
}

fn tamper(vault: &Path, slug: &str, from: &str, to: &str) {
    let path = vault.join(format!("workflows/{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains(from), "tamper anchor {from:?} missing");
    std::fs::write(path, content.replacen(from, to, 1)).unwrap();
}

fn pending_permission_reviews(vault: &Path) -> Vec<crate::review_queue::ReviewItem> {
    crate::review_queue::list(vault, Some(crate::workflow_permissions::REVIEW_KIND))
        .unwrap()
        .into_iter()
        .filter(|item| item.status == "pending")
        .collect()
}

fn changed_paths(run: &HistoryRun) -> Vec<String> {
    run.changes.iter().map(|c| c.vault_path.clone()).collect()
}

fn chat_holder() -> LeaseHolder {
    LeaseHolder::Run {
        id: "run-chat".into(),
        source: Some("chat".into()),
        source_id: None,
        started_at: None,
    }
}

fn stub_beat(vault: &Path) -> Result<PreparedBeat, String> {
    Ok(heartbeat::prepared_beat_for_test(vault))
}

fn deps() -> StepDeps<'static> {
    StepDeps {
        precheck: &workflows::precheck,
        prepare_beat: &stub_beat,
        heartbeat_cfg: HeartbeatConfig {
            work_hours_start: 0,
            work_hours_end: 24,
            min_interval_min: 30,
            daily_budget: 10,
        },
        heartbeat_enabled: true,
        local_hour: 12,
        today: "2026-09-25".into(),
        bookkeeping_lease: &automated_history::scheduler_write_lease,
        before_bookkeeping: &no_hook,
        before_admit: &no_hook,
        after_claim: &no_hook,
        fail_mark_running: false,
        budget_fallbacks: &no_fallbacks,
        cfo_hold: &no_cfo_hold,
    }
}

/// COO rebuild L7: no CFO hold by default (the machine's live readings
/// never decide a test).
fn no_cfo_hold(_: &Path, _: &str) -> Option<String> {
    None
}

/// No agent to fall back to (the default in tests: the machine's
/// installed CLIs never decide a test).
fn no_fallbacks(_: &Workflow) -> Vec<AgentKind> {
    Vec::new()
}

fn run_step(
    vault: &Path,
    now: DateTime<Utc>,
    gap: Option<DateTime<Utc>>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    let mut pending = Vec::new();
    let outcome = step(vault, now, gap, &mut pending, deps);
    assert!(pending.is_empty(), "no terminal write failed: {pending:?}");
    outcome
}

fn admitted(outcome: PumpStep) -> Dispatch {
    match outcome {
        PumpStep::Admitted(dispatch) => dispatch,
        PumpStep::Idle => panic!("expected an admission, got Idle"),
        PumpStep::Sleep(at) => panic!("expected an admission, got Sleep({at})"),
        PumpStep::Busy { holder, .. } => panic!("expected an admission, got Busy({holder:?})"),
        PumpStep::Handled { row_id, event, .. } => {
            panic!("expected an admission, got Handled({row_id}, {event:?})")
        }
        PumpStep::Error(error) => panic!("expected an admission, got Error({error})"),
    }
}

fn handled(outcome: PumpStep) -> (i64, Option<&'static str>, Option<SkipNotice>) {
    match outcome {
        PumpStep::Handled {
            row_id,
            event,
            notice,
            ..
        } => (row_id, event, notice),
        PumpStep::Admitted(dispatch) => {
            panic!("expected Handled, got Admitted({})", dispatch.row().id)
        }
        PumpStep::Idle => panic!("expected Handled, got Idle"),
        PumpStep::Sleep(at) => panic!("expected Handled, got Sleep({at})"),
        PumpStep::Busy { holder, .. } => panic!("expected Handled, got Busy({holder:?})"),
        PumpStep::Error(error) => panic!("expected Handled, got Error({error})"),
    }
}

fn dispatch_slug(dispatch: &Dispatch) -> Option<&str> {
    match dispatch {
        Dispatch::Workflow { run, .. } => Some(run.slug()),
        Dispatch::Heartbeat { .. } | Dispatch::Chat { .. } => None,
    }
}

/// Finalize an admitted run without executing it, and end its row.
fn finish_dispatch(vault: &Path, dispatch: Dispatch, status: &str) -> HistoryRun {
    match dispatch {
        Dispatch::Workflow { row, run } => {
            let history = run.finish_for_test(status);
            run_queue::finish(
                vault,
                row.id,
                RowStatus::from_run_status(status),
                None,
                Utc::now(),
            )
            .unwrap();
            history
        }
        Dispatch::Heartbeat { row, lease, .. } => {
            let history = lease.finish(status).unwrap();
            run_queue::finish(
                vault,
                row.id,
                RowStatus::from_run_status(status),
                None,
                Utc::now(),
            )
            .unwrap();
            history
        }
        // The row ends before the gate is released, as in `execute`.
        Dispatch::Chat { row, turn, guard } => {
            let history = turn.finish_for_test(status);
            run_queue::finish(
                vault,
                row.id,
                RowStatus::from_run_status(status),
                None,
                Utc::now(),
            )
            .unwrap();
            drop(guard);
            history
        }
    }
}

fn queue_cron(vault: &Path, workflow: &Workflow, now: DateTime<Utc>) -> i64 {
    let due = workflow.next_run_at.clone().expect("a due occurrence");
    run_queue::enqueue(
        vault,
        NewRow::new(
            RunSource::Cron,
            Some(&workflow.slug),
            &workflow.title,
            "scheduled",
        )
        .with_payload(run_queue::cron_payload(&due))
        .with_key(format!("cron:{}:{due}", workflow.slug)),
        now,
    )
    .unwrap()
    .id()
}

fn queue_manual(vault: &Path, workflow: &Workflow, now: DateTime<Utc>) -> i64 {
    run_queue::enqueue(
        vault,
        NewRow::new(
            RunSource::Manual,
            Some(&workflow.slug),
            &workflow.title,
            "run now",
        ),
        now,
    )
    .unwrap()
    .id()
}

fn queue_trigger(
    vault: &Path,
    workflow: &Workflow,
    fired: Vec<FiredToken>,
    requested: DateTime<Utc>,
) -> i64 {
    run_queue::enqueue(
        vault,
        NewRow::new(
            RunSource::Trigger,
            Some(&workflow.slug),
            &workflow.title,
            "fired",
        )
        .with_payload(run_queue::trigger_payload(&fired)),
        requested,
    )
    .unwrap()
    .id()
}

fn row(vault: &Path, id: i64) -> QueueRow {
    run_queue::get(vault, id).unwrap().expect("the row exists")
}

fn fn_body(source: &'static str, needle: &str) -> &'static str {
    let start = source.find(needle).unwrap_or_else(|| panic!("{needle}"));
    let end = start + source[start..].find("\n}\n").expect("end of fn");
    &source[start..end]
}

#[test]
fn the_pump_runs_one_row_at_a_time() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let a = due_cron(v, "Pump One A", now - Duration::minutes(2));
    let b = due_cron(v, "Pump One B", now - Duration::minutes(1));
    queue_cron(v, &a, now);
    queue_cron(v, &b, now);
    let b_before = workflow_bytes(v, &b.slug);
    let deps = deps();

    let first = admitted(run_step(v, now, None, &deps));
    assert_eq!(dispatch_slug(&first), Some(a.slug.as_str()));
    match run_step(v, now, None, &deps) {
        PumpStep::Busy { holder, .. } => assert!(holder.is_workflow(&a.slug), "{holder:?}"),
        _ => panic!("the second row waits for the first run"),
    }
    assert_eq!(
        workflow_bytes(v, &b.slug),
        b_before,
        "nothing is written to b meanwhile"
    );
    finish_dispatch(v, first, "success");

    let gap = Some(now + Duration::seconds(MIN_AUTOMATION_GAP_SECS));
    match run_step(v, now, gap, &deps) {
        PumpStep::Sleep(at) => assert_eq!(Some(at), gap),
        _ => panic!("a cron row waits out the gap"),
    }
    let second = admitted(run_step(v, now + Duration::seconds(31), gap, &deps));
    assert_eq!(dispatch_slug(&second), Some(b.slug.as_str()));
    finish_dispatch(v, second, "success");
    assert!(matches!(run_step(v, now, None, &deps), PumpStep::Idle));

    // The loop opens the gap only once a vault-lane run has ended (its
    // lane task joined; merged plan slice 3), and nowhere else.
    // `the_loop_starts_workspace_turns_while_a_vault_lane_run_is_live`
    // drives the loop itself.
    let source = include_str!("run_pump.rs");
    let body = fn_body(source, concat!("pub(crate) async fn ", "drain<"));
    let gapped = crate::source_pins::count(body, concat!("own_gap = ", "Some("));
    let lane = crate::source_pins::count(body, concat!("own_gap = Some(", "lane_ended("));
    assert!(lane > 0 && lane == gapped, "{lane} of {gapped}");
    let ended = fn_body(source, concat!("async fn ", "lane_ended<"));
    let gap = concat!(
        "Utc::now() + Duration::seconds(",
        "MIN_AUTOMATION_GAP_SECS)"
    );
    assert!(crate::source_pins::contains(ended, gap));
}

#[test]
fn manual_and_trigger_rows_are_not_skipped_by_cron_rules() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let t = seed_workflow_with(v, "Pump NotCron T", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    touch_note(v, "notes/x.md");
    workflows::enqueue_due(v, now);
    let deps = deps();
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(
        dispatch_slug(&run),
        Some(t.slug.as_str()),
        "a manual+triggers workflow runs"
    );
    finish_dispatch(v, run, "success");
    assert!(run_log(v, &t.slug).is_empty(), "no skip row");

    let m = due_cron(v, "Pump NotCron M", now - Duration::hours(5));
    let before = next_run(v, &m.slug);
    queue_manual(v, &m, now);
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(
        dispatch_slug(&run),
        Some(m.slug.as_str()),
        "Run now ignores grace"
    );
    finish_dispatch(v, run, "success");
    assert!(run_log(v, &m.slug).is_empty(), "no skip row");
    assert_eq!(
        next_run(v, &m.slug),
        before,
        "the schedule was not consulted"
    );
}

#[test]
fn cron_rows_are_checked_for_schedule_occurrence_and_grace_at_claim() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();

    // Switched to manual: silent skip.
    let manual = due_cron(v, "Pump Cron Manual", now - Duration::minutes(1));
    let id = queue_cron(v, &manual, now);
    vault_workflows::set_schedule(v, &manual.slug, "manual").unwrap();
    let (_, _, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!(notice, None);
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    assert!(run_log(v, &manual.slug).is_empty());

    // The occurrence was consumed: no row.
    let consumed = due_cron(v, "Pump Cron Consumed", now - Duration::minutes(1));
    let id = queue_cron(v, &consumed, now);
    vault_workflows::finalize_run(
        v,
        &consumed.slug,
        "success",
        None,
        1,
        Some(now + Duration::days(1)),
        RunLogEntry {
            at: Utc::now().to_rfc3339(),
            status: "success".into(),
            duration_sec: 1,
            note: "done".into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        },
    )
    .unwrap();
    let (_, _, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!(notice, None);
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    assert_eq!(
        run_log(v, &consumed.slug).len(),
        1,
        "only the run's own row"
    );

    // Past grace with no wait: the plain misfire, advanced, and announced.
    let stale = due_cron(v, "Pump Cron Stale", now - Duration::hours(3));
    queue_cron(v, &stale, now);
    let (_, _, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!(
        notice,
        Some(SkipNotice {
            title: stale.title.clone(),
            kind: NoticeKind::Missed,
            notify: NotifyPolicy::Always,
        })
    );
    assert_eq!(
        run_log(v, &stale.slug).pop().unwrap().note,
        MISSED_WHILE_CLOSED_NOTE
    );
    assert_ne!(next_run(v, &stale.slug), stale.next_run_at, "advanced");

    // An unparseable schedule is skipped without a write.
    let broken = seed_workflow_with(v, "Pump Cron Broken", "0 7 * * *", "BODY", |fm| {
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
    });
    let bytes = workflow_bytes(v, &broken.slug);
    let id = queue_cron(v, &broken, now);
    handled(run_step(v, now, None, &deps));
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    assert_eq!(workflow_bytes(v, &broken.slug), bytes, "byte-identical");
}

#[test]
fn an_absurd_grace_never_goes_stale_at_claim() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let g = seed_workflow_with(v, "Pump Forever Grace", "0 0 7 * * * *", "BODY", |fm| {
        fm.misfire_grace_min = Some(200_000_000_000_000_000);
        fm.next_run_at = Some((now - Duration::days(30)).to_rfc3339());
    });
    queue_cron(v, &g, now);
    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch_slug(&run), Some(g.slug.as_str()));
    finish_dispatch(v, run, "success");
}

#[test]
fn a_refused_or_unavailable_workflow_holds_its_rows_within_their_window_and_refuses_manual_rows() {
    let now = Utc::now();
    let refuse = |_: &Path, _: &str| -> Result<Runnable, PrecheckFailure> {
        Err(PrecheckFailure::Refused {
            reason: "Automation paused for review: x".into(),
            note: "permission stamp changed".into(),
        })
    };
    let unavailable = |_: &Path, _: &str| -> Result<Runnable, PrecheckFailure> {
        Err(PrecheckFailure::Refused {
            reason: "Automation permission records unavailable".into(),
            note: vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON.into(),
        })
    };
    type Check<'a> = &'a dyn Fn(&Path, &str) -> Result<Runnable, PrecheckFailure>;
    let cases: [(Check<'_>, &str); 2] = [
        (&refuse, "permission stamp changed"),
        (
            &unavailable,
            vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON,
        ),
    ];
    for (check, note) in cases {
        let held_deps = StepDeps {
            precheck: check,
            ..deps()
        };

        // A cron within grace is held: nothing written, no wait.
        let dir = temp_vault();
        let v = dir.path();
        let c = due_cron(v, "Pump Held Cron", now - Duration::minutes(1));
        let before = workflow_bytes(v, &c.slug);
        let id = queue_cron(v, &c, now);
        let (_, event, notice) = handled(run_step(v, now, None, &held_deps));
        assert_eq!((event, notice), (Some("held"), None));
        let held = row(v, id);
        assert_eq!(held.status, RowStatus::Queued);
        assert_eq!(held.held_reason.as_deref(), Some(note));
        assert!(held.wait.is_none(), "a hold is not a wait");
        assert_eq!(workflow_bytes(v, &c.slug), before);
        assert_eq!(lease_holder(v), None, "the lease is never taken");

        // Once the check passes, the held cron runs, with no deferred row.
        let run = admitted(run_step(v, now + Duration::minutes(2), None, &deps()));
        assert_eq!(dispatch_slug(&run), Some(c.slug.as_str()));
        assert!(
            run_log(v, &c.slug).is_empty(),
            "a hold never claims it waited"
        );
        finish_dispatch(v, run, "success");

        // Past grace it is skipped with its note, advanced, unannounced.
        let dir = temp_vault();
        let v = dir.path();
        let s = due_cron(v, "Pump Held Stale", now - Duration::minutes(1));
        let id = queue_cron(v, &s, now);
        handled(run_step(v, now, None, &held_deps));
        let (_, _, notice) = handled(run_step(v, now + Duration::hours(3), None, &held_deps));
        assert_eq!(notice, None, "a refusal is never announced as missed");
        assert_eq!(row(v, id).status, RowStatus::Skipped);
        assert_eq!(
            run_log(v, &s.slug).pop().unwrap().note,
            format!("not run: {note}")
        );
        assert_ne!(next_run(v, &s.slug), s.next_run_at);

        // A trigger row is held and stays queued.
        let dir = temp_vault();
        let v = dir.path();
        let t = seed_workflow_with(v, "Pump Held Trigger", "manual", "BODY", |fm| {
            fm.triggers = vec!["file-changed:notes/*.md".into()];
        });
        touch_note(v, "notes/x.md");
        workflows::enqueue_due(v, now);
        let (id, event, _) = handled(run_step(v, now, None, &held_deps));
        assert_eq!(event, Some("held"));
        assert_eq!(row(v, id).status, RowStatus::Queued);
        assert_eq!(row(v, id).source_id.as_deref(), Some(t.slug.as_str()));

        // Run now is skipped and its waiter gets the refusal; a retry row
        // is skipped silently.
        let dir = temp_vault();
        let v = dir.path();
        let m = seed_workflow(v, "Pump Held Manual", "manual", "BODY");
        let (enqueued, mut receiver) = run_queue::enqueue_and_wait(
            v,
            NewRow::new(RunSource::Manual, Some(&m.slug), &m.title, "run now"),
            now,
        )
        .unwrap();
        handled(run_step(v, now, None, &held_deps));
        assert_eq!(row(v, enqueued.id()).status, RowStatus::Skipped);
        match receiver.try_recv() {
            Ok(RowOutcome::Finished { status, error }) => {
                assert_eq!(status, RowStatus::Skipped);
                assert!(error.unwrap_or_default().starts_with("Automation"));
            }
            other => panic!("the waiter hears the refusal: {other:?}"),
        }
        let retry = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Retry, Some(&m.slug), &m.title, "retry"),
            now,
        )
        .unwrap()
        .id();
        handled(run_step(v, now, None, &held_deps));
        assert_eq!(row(v, retry).status, RowStatus::Skipped);
        assert!(run_log(v, &m.slug).is_empty());
        assert!(automated_history::list_runs(v, 10).unwrap().is_empty());
    }
}

#[test]
fn cheap_claim_checks_run_before_the_permission_check() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let calls = Cell::new(0);
    let counting = |vault: &Path, slug: &str| {
        calls.set(calls.get() + 1);
        workflows::precheck(vault, slug)
    };
    let deps = StepDeps {
        precheck: &counting,
        ..deps()
    };

    let cancelled = due_cron(v, "Pump Cheap Cancelled", now - Duration::minutes(1));
    let cancelled_id = queue_cron(v, &cancelled, now);
    assert_eq!(
        run_queue::cancel(v, cancelled_id, now).unwrap(),
        run_queue::CancelOutcome::SkipRequested
    );
    let moved = due_cron(v, "Pump Cheap Moved", now - Duration::minutes(1));
    run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Cron,
            Some(&moved.slug),
            &moved.title,
            "scheduled",
        )
        .with_payload(run_queue::cron_payload("2026-01-01T07:00:00+00:00")),
        now,
    )
    .unwrap();
    let missing_id = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Manual,
            Some("pump-cheap-no-such"),
            "Gone",
            "run now",
        ),
        now,
    )
    .unwrap()
    .id();
    let meeting = seed_workflow_with(v, "Pump Cheap Meeting", "manual", "BODY", |fm| {
        fm.triggers = vec!["meeting-in:30m".into()];
    });
    let lapsed_id = queue_trigger(
        v,
        &meeting,
        vec![FiredToken {
            token: "meeting-in:30m".into(),
            ..FiredToken::default()
        }],
        now,
    );
    for _ in 0..4 {
        handled(run_step(v, now, None, &deps));
    }
    assert_eq!(
        calls.get(),
        0,
        "every cheap decision skipped the permission check"
    );
    assert_eq!(row(v, cancelled_id).status, RowStatus::Cancelled);
    assert_eq!(
        run_log(v, &cancelled.slug).pop().unwrap().note,
        "not run: skipped from the run queue",
        "a skipped occurrence is recorded and advanced"
    );
    assert_ne!(next_run(v, &cancelled.slug), cancelled.next_run_at);
    assert_eq!(row(v, missing_id).status, RowStatus::Failed);
    assert_eq!(row(v, lapsed_id).status, RowStatus::Skipped);
    assert!(run_log(v, &moved.slug).is_empty());

    let due = due_cron(v, "Pump Cheap Due", now - Duration::minutes(1));
    queue_cron(v, &due, now);
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(
        calls.get(),
        1,
        "exactly one permission check for the due row"
    );
    assert_eq!(dispatch_slug(&run), Some(due.slug.as_str()));
    finish_dispatch(v, run, "success");
}

#[test]
fn lease_contention_requeues_without_an_attempt_and_names_the_holder_once() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let c = due_cron(v, "Pump Race C", now - Duration::minutes(1));
    let id = queue_cron(v, &c, now);
    let raced = RefCell::new(None);
    let hook = |vault: &Path| {
        *raced.borrow_mut() = Some(begin_run(vault, "chat", None).unwrap());
    };
    let racing = StepDeps {
        before_admit: &hook,
        ..deps()
    };

    let (_, event, _) = handled(run_step(v, now, None, &racing));
    assert_eq!(
        event,
        Some("waiting"),
        "the lost race starts the wait episode"
    );
    let waiting = row(v, id);
    assert_eq!(waiting.status, RowStatus::Queued);
    assert_eq!(waiting.attempt, 0, "a lost race is not an attempt");
    assert_eq!(
        waiting.wait.as_ref().map(|w| w.holder.as_str()),
        Some("a chat turn")
    );
    match run_step(v, now, None, &deps()) {
        PumpStep::Busy {
            episodes_started, ..
        } => assert!(episodes_started.is_empty(), "the holder is named once"),
        _ => panic!("the chat still holds the vault"),
    }
    finish_run(v, raced.borrow_mut().take().unwrap(), "success").unwrap();

    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(row(v, id).attempt, 1, "counted once the lease was held");
    assert_eq!(row(v, id).status, RowStatus::Running);
    finish_dispatch(v, run, "success");
}

#[test]
fn a_skip_decided_while_a_chat_holds_the_lease_leaves_that_chats_receipt_unchanged() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let c = due_cron(v, "Pump Chat Skip C", now - Duration::minutes(1));
    let id = queue_cron(v, &c, now);
    let chat = begin_run(v, "chat", None).unwrap();
    // The scheduler's ticks touch the waiting row while the chat runs.
    let holder = lease_holder(v).unwrap();
    run_queue::touch_waiting(v, &holder, now).unwrap();
    let later = now + Duration::hours(3);
    run_queue::touch_waiting(v, &holder, later).unwrap();
    let before = workflow_bytes(v, &c.slug);

    assert!(matches!(
        run_step(v, later, None, &deps()),
        PumpStep::Busy { .. }
    ));
    assert_eq!(
        workflow_bytes(v, &c.slug),
        before,
        "no row inside the chat's lease"
    );
    let chat_run = finish_run(v, chat, "success").unwrap();
    assert!(
        !changed_paths(&chat_run)
            .iter()
            .any(|p| p.starts_with("workflows/")),
        "{:?}",
        changed_paths(&chat_run)
    );
    assert!(chat_run.flags.is_empty(), "{:?}", chat_run.flags);

    // Another chat takes the lease right before the bookkeeping write.
    let raced = RefCell::new(None);
    let hook = |vault: &Path| {
        *raced.borrow_mut() = Some(begin_run(vault, "chat", None).unwrap());
    };
    let racing = StepDeps {
        before_bookkeeping: &hook,
        ..deps()
    };
    handled(run_step(v, later, None, &racing));
    assert_eq!(
        row(v, id).status,
        RowStatus::Queued,
        "re-queued with nothing written"
    );
    assert_eq!(workflow_bytes(v, &c.slug), before);
    let second = finish_run(v, raced.borrow_mut().take().unwrap(), "success").unwrap();
    assert!(!changed_paths(&second)
        .iter()
        .any(|p| p.starts_with("workflows/")));

    let (_, _, notice) = handled(run_step(v, later, None, &deps()));
    assert_eq!(notice.map(|n| n.kind), Some(NoticeKind::Busy));
    let newest = run_log(v, &c.slug).pop().unwrap();
    assert!(newest.note.contains("a chat turn"), "{}", newest.note);
    assert!(newest.note.contains("grace"), "{}", newest.note);
    assert_ne!(newest.note, MISSED_WHILE_CLOSED_NOTE);
}

#[test]
fn wake_from_sleep_skips_past_grace_crons_and_runs_the_rest_one_gap_apart() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let stale: Vec<Workflow> = ["Pump Wake S1", "Pump Wake S2", "Pump Wake S3"]
        .iter()
        .map(|title| due_cron(v, title, now - Duration::hours(4)))
        .collect();
    let fresh: Vec<Workflow> = ["Pump Wake F1", "Pump Wake F2"]
        .iter()
        .map(|title| due_cron(v, title, now - Duration::minutes(1)))
        .collect();
    for w in stale.iter().chain(&fresh) {
        queue_cron(v, w, now);
    }
    let deps = deps();
    let mut notices = Vec::new();
    let run = loop {
        match run_step(v, now, None, &deps) {
            PumpStep::Handled { notice, .. } => notices.extend(notice),
            PumpStep::Admitted(dispatch) => break dispatch,
            _ => panic!("every row is decided or admitted"),
        }
    };
    let mut titles: Vec<String> = notices
        .iter()
        .filter(|n| n.kind == NoticeKind::Missed)
        .map(|n| n.title.clone())
        .collect();
    titles.sort();
    let mut want: Vec<String> = stale.iter().map(|w| w.title.clone()).collect();
    want.sort();
    assert_eq!(titles, want, "one aggregated Missed notice's titles");
    assert_eq!(
        dispatch_slug(&run),
        Some(fresh[0].slug.as_str()),
        "exactly one fresh cron"
    );
    finish_dispatch(v, run, "success");
    let gap = Some(now + Duration::seconds(MIN_AUTOMATION_GAP_SECS));
    assert!(
        matches!(run_step(v, now, gap, &deps), PumpStep::Sleep(_)),
        "the next waits"
    );
    let runs = automated_history::list_runs(v, 20).unwrap();
    assert_eq!(runs.len(), 1, "no past-grace workflow ran");
}

#[test]
fn trigger_rows_lapse_at_claim_when_their_fire_is_no_longer_true() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();

    let meeting = seed_workflow_with(v, "Pump Lapse Meeting", "manual", "BODY", |fm| {
        fm.triggers = vec!["meeting-in:30m".into()];
    });
    let id = queue_trigger(
        v,
        &meeting,
        vec![FiredToken {
            token: "meeting-in:30m".into(),
            ..FiredToken::default()
        }],
        now,
    );
    handled(run_step(v, now, None, &deps));
    assert_eq!(
        row(v, id).status,
        RowStatus::Skipped,
        "a retired meeting-in fire lapses"
    );
    assert!(
        run_log(v, &meeting.slug).is_empty(),
        "it never waited: silent"
    );

    // Task fires carry `vault:tasks/…` URLs (what `task_trigger_ids`
    // returns), so the fixtures are real task files. A bare path could
    // never be in the live set, which made the old lapse check vacuous.
    let put_task = |rel: &str, body: &str| {
        std::fs::create_dir_all(v.join("tasks")).unwrap();
        std::fs::write(v.join(rel), body).unwrap();
    };
    put_task(
        "tasks/finished-since.md",
        "---\ntitle: Finished since\nstatus: Done\ndue: 2020-01-01\n---\n",
    );
    put_task(
        "tasks/still-late.md",
        "---\ntitle: Still late\nstatus: Not started\ndue: 2020-01-01\n---\n",
    );
    let overdue_fire = |id: &str| {
        vec![FiredToken {
            token: "task-overdue".into(),
            overdue_ids: vec![id.to_string()],
            ..FiredToken::default()
        }]
    };

    let overdue = seed_workflow_with(v, "Pump Lapse Overdue", "manual", "BODY", |fm| {
        fm.triggers = vec!["task-overdue".into()];
    });
    let id = queue_trigger(
        v,
        &overdue,
        overdue_fire("vault:tasks/finished-since.md"),
        now,
    );
    handled(run_step(v, now, None, &deps));
    assert_eq!(
        row(v, id).status,
        RowStatus::Skipped,
        "the task is no longer overdue"
    );
    assert!(
        run_log(v, &overdue.slug).is_empty(),
        "it never waited: silent"
    );

    // The discriminating case: a task that is still overdue runs.
    let late = seed_workflow_with(v, "Pump Lapse Late", "manual", "BODY", |fm| {
        fm.triggers = vec!["task-overdue".into()];
    });
    queue_trigger(v, &late, overdue_fire("vault:tasks/still-late.md"), now);
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(
        dispatch_slug(&run),
        Some(late.slug.as_str()),
        "still overdue: admitted"
    );
    finish_dispatch(v, run, "success");

    // task-unblocked (Paperclip PR 11b): a re-blocked task's fire lapses
    // silently; a task that is still unblocked runs.
    put_task(
        "tasks/blocker-open.md",
        "---\ntitle: Blocker open\nstatus: In progress\n---\n",
    );
    put_task(
        "tasks/blocker-done.md",
        "---\ntitle: Blocker done\nstatus: Done\n---\n",
    );
    put_task(
        "tasks/reblocked.md",
        "---\ntitle: Reblocked\nstatus: Not started\nblocked_by: \"vault:tasks/blocker-done.md, vault:tasks/blocker-open.md\"\n---\n",
    );
    put_task(
        "tasks/free.md",
        "---\ntitle: Free\nstatus: Waiting\nblocked_by: \"vault:tasks/blocker-done.md\"\n---\n",
    );
    let unblocked_fire = |id: &str| {
        vec![FiredToken {
            token: "task-unblocked".into(),
            unblocked_ids: vec![id.to_string()],
            ..FiredToken::default()
        }]
    };
    let reblocked = seed_workflow_with(v, "Pump Lapse Reblocked", "manual", "BODY", |fm| {
        fm.triggers = vec!["task-unblocked".into()];
    });
    let id = queue_trigger(
        v,
        &reblocked,
        unblocked_fire("vault:tasks/reblocked.md"),
        now,
    );
    handled(run_step(v, now, None, &deps));
    assert_eq!(
        row(v, id).status,
        RowStatus::Skipped,
        "the task is blocked again"
    );
    assert!(
        run_log(v, &reblocked.slug).is_empty(),
        "it never waited: silent"
    );

    let free = seed_workflow_with(v, "Pump Lapse Unblocked", "manual", "BODY", |fm| {
        fm.triggers = vec!["task-unblocked".into()];
    });
    queue_trigger(v, &free, unblocked_fire("vault:tasks/free.md"), now);
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(
        dispatch_slug(&run),
        Some(free.slug.as_str()),
        "still unblocked: admitted"
    );
    finish_dispatch(v, run, "success");

    let old = seed_workflow_with(v, "Pump Lapse Old", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    let file_fire = || {
        vec![FiredToken {
            token: "file-changed:notes/*.md".into(),
            max_mtime: Some(now - Duration::days(8)),
            ..FiredToken::default()
        }]
    };
    let id = queue_trigger(v, &old, file_fire(), now - Duration::days(8));
    handled(run_step(v, now, None, &deps));
    assert_eq!(row(v, id).status, RowStatus::Skipped, "older than the TTL");

    // A file-changed fire that survived a restart still runs.
    let survivor = seed_workflow_with(v, "Pump Lapse Survivor", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    queue_trigger(v, &survivor, file_fire(), now - Duration::hours(2));
    let run = admitted(run_step(v, now, None, &deps));
    assert_eq!(dispatch_slug(&run), Some(survivor.slug.as_str()));
    finish_dispatch(v, run, "success");
}

#[test]
fn startup_interrupts_running_work_and_reruns_an_interrupted_cron_once_within_grace() {
    let dir = temp_vault();
    let v = dir.path();
    let before_boot = Utc::now() - Duration::minutes(5);
    let boot_at = Utc::now() - Duration::minutes(1);
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Boot Manual", "manual", "BODY");
    let manual_id = queue_manual(v, &m, before_boot);
    run_queue::claim(v, manual_id, before_boot)
        .unwrap()
        .unwrap();
    run_queue::mark_running(v, manual_id, "run-old", before_boot).unwrap();
    let c = due_cron(v, "Pump Boot Cron", now - Duration::minutes(10));
    let d = due_cron(v, "Pump Boot Stale", now - Duration::hours(3));
    workflows::enqueue_due(v, before_boot);
    for w in [&c, &d] {
        let id = run_queue::live_rows(v)
            .unwrap()
            .into_iter()
            .find(|row| {
                row.source == RunSource::Cron && row.source_id.as_deref() == Some(w.slug.as_str())
            })
            .unwrap()
            .id;
        run_queue::claim(v, id, before_boot).unwrap().unwrap();
        run_queue::mark_running(v, id, &format!("run-old-{}", w.slug), before_boot).unwrap();
        vault_workflows::mark_status(v, &w.slug, "running", None, None, None).unwrap();
    }

    let interrupted = run_queue::recover_at_startup(v, boot_at, now, &HashSet::new()).unwrap();
    assert_eq!(interrupted.len(), 3);
    assert_eq!(row(v, manual_id).status, RowStatus::Interrupted);
    let healed = vault_workflows::heal_orphan_running(v, &[]);
    assert_eq!(healed.len(), 2);
    assert_eq!(
        next_run(v, &c.slug),
        c.next_run_at,
        "the heal leaves the occurrence due"
    );

    workflows::enqueue_due(v, now);
    let deps = deps();
    let mut ran = Vec::new();
    let mut notices = Vec::new();
    loop {
        match run_step(v, now, None, &deps) {
            PumpStep::Admitted(dispatch) => {
                ran.push(dispatch_slug(&dispatch).unwrap().to_string());
                let slug = ran.last().unwrap().clone();
                // What finalize writes: the next occurrence.
                vault_workflows::finalize_run(
                    v,
                    &slug,
                    "success",
                    None,
                    1,
                    Some(now + Duration::days(1)),
                    RunLogEntry {
                        at: Utc::now().to_rfc3339(),
                        status: "success".into(),
                        duration_sec: 1,
                        note: "done".into(),
                        run_id: None,
                        error_tail: None,
                        exit_kind: None,
                    },
                )
                .unwrap();
                finish_dispatch(v, dispatch, "success");
            }
            PumpStep::Handled { notice, .. } => notices.extend(notice),
            PumpStep::Idle => break,
            _ => panic!("nothing else is queued"),
        }
    }
    assert_eq!(
        ran,
        vec![c.slug.clone()],
        "the interrupted cron re-runs once; Run now never"
    );
    assert_eq!(
        run_log(v, &d.slug).pop().unwrap().note,
        MISSED_WHILE_CLOSED_NOTE,
        "past grace the same occurrence gets the misfire label"
    );
    workflows::enqueue_due(v, now);
    assert!(
        matches!(run_step(v, now, None, &deps), PumpStep::Idle),
        "exactly once"
    );
}

#[test]
fn a_waited_row_writes_its_deferred_row_inside_its_own_run() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let a = due_cron(v, "Pump Waited A", now - Duration::minutes(2));
    let b = due_cron(v, "Pump Waited B", now - Duration::minutes(1));
    queue_cron(v, &a, now);
    queue_cron(v, &b, now);
    let deps = deps();
    let first = admitted(run_step(v, now, None, &deps));
    match run_step(v, now, None, &deps) {
        PumpStep::Busy {
            episodes_started, ..
        } => assert_eq!(episodes_started.len(), 1),
        _ => panic!("b waits on a"),
    }
    finish_dispatch(v, first, "success");

    let second = admitted(run_step(v, now + Duration::seconds(31), None, &deps));
    assert_eq!(dispatch_slug(&second), Some(b.slug.as_str()));
    let newest = run_log(v, &b.slug).pop().expect("b's deferred row");
    assert_eq!(newest.status, vault_workflows::DEFERRED_STATUS);
    assert!(
        newest.note.contains(&format!("workflow `{}`", a.slug)),
        "{}",
        newest.note
    );
    let history = finish_dispatch(v, second, "success");
    assert!(
        changed_paths(&history).contains(&format!("workflows/{}.md", b.slug)),
        "the deferred row is written inside b's own run"
    );
}

#[test]
fn a_legacy_unknown_manual_run_keeps_its_existing_occurrence_consumption() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let t = seed_workflow_with(v, "Pump Absorb T", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    touch_note(v, "notes/x.md");
    let chat = begin_run(v, "chat", None).unwrap();
    workflows::enqueue_due(v, now);
    let queued: Vec<QueueRow> = run_queue::live_rows(v).unwrap();
    assert_eq!(queued.len(), 2, "a cron row and a trigger row");
    // Model rows written by a pre-origin binary: the queue and trigger state
    // exist, but no provenance records do. No migration promotes them.
    crate::session_store::with_write(v, "legacy queue fixture", |tx| {
        tx.execute_batch("DELETE FROM work_origin_occurrences; DELETE FROM work_origin_queue; DELETE FROM work_origin_requests;")
            .map_err(|e| e.to_string())
    }).unwrap();
    // A change after the enqueue, still pending when Run now is admitted.
    touch_note(v, "notes/y.md");
    let newer = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(v.join("notes/y.md"))
        .unwrap()
        .set_modified(newer)
        .unwrap();
    finish_run(v, chat, "success").unwrap();

    let manual_id = queue_manual(v, &t, now);
    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(run.row().id, manual_id, "Run now is claimed first");
    for sibling in &queued {
        let absorbed = row(v, sibling.id);
        assert_eq!(absorbed.status, RowStatus::Coalesced);
        assert_eq!(absorbed.coalesced_into, Some(manual_id));
    }
    let manual = row(v, manual_id);
    assert!(manual.reason.contains("scheduled"), "{}", manual.reason);
    assert!(manual.reason.contains("file changed"), "{}", manual.reason);
    let Dispatch::Workflow {
        run: workflow_run, ..
    } = &run
    else {
        panic!("workflow")
    };
    workflow_run
        .record_completion_for_test(v, "success")
        .unwrap();
    finish_dispatch(v, run, "success");

    let after = workflows::enqueue_due(v, now);
    assert!(after.changed.is_empty(), "one change starts one run");
}

#[test]
fn direct_workflow_keeps_independent_trigger_watermarks_and_cron_until_scheduler_runs() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let due = (now - Duration::minutes(1)).to_rfc3339();
    let workflow = seed_workflow_with(v, "Origin Consumption", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(due.clone());
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    touch_note(v, "notes/first.md");
    workflows::enqueue_due(v, now);
    let scheduled = run_queue::live_rows(v).unwrap();
    assert_eq!(scheduled.len(), 2);
    let before = crate::trigger_scanner::load_state(v);
    touch_note(v, "notes/later.md");
    std::fs::File::options()
        .write(true)
        .open(v.join("notes/later.md"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let direct = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Manual,
            Some(&workflow.slug),
            &workflow.title,
            "run now",
        )
        .with_origin(crate::work_origin::Request::person_workflow()),
        now,
    )
    .unwrap()
    .id();
    let dispatch = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch.row().id, direct);
    assert_eq!(
        crate::trigger_scanner::load_state(v),
        before,
        "direct admission cannot consume a pending fire"
    );
    for sibling in &scheduled {
        assert_eq!(row(v, sibling.id).status, RowStatus::Queued);
    }
    let Dispatch::Workflow { run, .. } = &dispatch else {
        panic!("workflow")
    };
    run.record_completion_for_test(v, "success").unwrap();
    assert_eq!(next_run(v, &workflow.slug).as_deref(), Some(due.as_str()));
    finish_dispatch(v, dispatch, "success");
    workflows::enqueue_due(v, now + Duration::minutes(1));
    assert_ne!(
        crate::trigger_scanner::load_state(v),
        before,
        "only the scheduler's durable enqueue advances the pending event"
    );
    // The eligible trigger runs next and can absorb the compatible Cron. Its
    // successful completion advances the exact occurrence it actually owns.
    let dispatch = admitted(run_step(v, now + Duration::minutes(1), None, &deps()));
    assert!(scheduled.iter().any(|q| q.id == dispatch.row().id));
    let Dispatch::Workflow { run, .. } = &dispatch else {
        panic!("workflow")
    };
    run.record_completion_for_test(v, "success").unwrap();
    assert_ne!(next_run(v, &workflow.slug).as_deref(), Some(due.as_str()));
    finish_dispatch(v, dispatch, "success");
}

#[test]
fn direct_admission_with_failed_mark_running_leaves_unevaluated_trigger_intact() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let workflow = seed_workflow_with(v, "Origin Unmarked", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    touch_note(v, "notes/pending.md");
    let before = crate::trigger_scanner::load_state(v);
    run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Manual,
            Some(&workflow.slug),
            &workflow.title,
            "run now",
        )
        .with_origin(crate::work_origin::Request::person_workflow()),
        now,
    )
    .unwrap();
    let failed = StepDeps {
        fail_mark_running: true,
        ..deps()
    };
    assert!(matches!(
        run_step(v, now, None, &failed),
        PumpStep::Handled { .. }
    ));
    assert_eq!(crate::trigger_scanner::load_state(v), before);
    workflows::enqueue_due(v, now + Duration::seconds(1));
    assert!(run_queue::live_rows(v)
        .unwrap()
        .iter()
        .any(|r| r.source == RunSource::Trigger));
}

#[test]
fn absorbed_cron_completion_heals_failed_workflow_write_after_child_retention() {
    for status in ["success", "error", "cancelled"] {
        let dir = tempfile::Builder::new()
            .prefix("absorbed-cron-")
            .tempdir()
            .unwrap();
        let v = dir.path();
        let now = Utc::now();
        let old = now - Duration::days(40);
        let due = (old - Duration::minutes(1)).to_rfc3339();
        let workflow =
            seed_workflow_with(v, "Absorbed Completion", "0 0 7 * * * *", "BODY", |fm| {
                fm.next_run_at = Some(due.clone());
                fm.triggers = vec!["file-changed:notes/*.md".into()];
            });
        // Use the scheduler's real configured fire and payload. A source tag
        // alone does not make a trigger eligible: its facts must still hold.
        touch_note(v, "notes/input.md");
        std::fs::File::options()
            .write(true)
            .open(v.join("notes/input.md"))
            .unwrap()
            .set_modified(std::time::SystemTime::from(old - Duration::seconds(30)))
            .unwrap();
        workflows::enqueue_due(v, old);
        assert!(
            run_queue::live_rows(v)
                .unwrap()
                .iter()
                .any(|row| row.source == RunSource::Trigger),
            "the real scheduler must retain the configured trigger fire"
        );
        let cron = run_queue::live_rows(v)
            .unwrap()
            .into_iter()
            .find(|row| row.source == RunSource::Cron)
            .unwrap();
        let dispatch = admitted(run_step(v, old, None, &deps()));
        assert_eq!(dispatch.row().source, RunSource::Trigger);
        let receiver = dispatch.row().id;
        assert_eq!(row(v, cron.id).coalesced_into, Some(receiver));
        // Exercise ordinary retention while the receiving run still owns its
        // live slot. Completion must not depend on the child queue surviving.
        let sweep = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("sweep"), "sweep", "sweep"),
            now,
        )
        .unwrap()
        .id();
        run_queue::finish(v, sweep, RowStatus::Cancelled, None, now).unwrap();
        assert!(run_queue::get(v, cron.id).unwrap().is_none());
        assert!(!run_queue::occurrence_completed(v, &workflow.slug, &due).unwrap());
        let Dispatch::Workflow { run, .. } = &dispatch else {
            panic!("workflow");
        };
        // A real filesystem failure at the shared completion writer, without
        // changing the retained workflow content or its permission stamp.
        let path = v.join("workflows").join(format!("{}.md", workflow.slug));
        let parked = v.join("parked-workflow.md");
        std::fs::rename(&path, &parked).unwrap();
        let result = run.record_completion_for_test(v, status);
        std::fs::rename(&parked, &path).unwrap();
        assert!(
            result.is_err(),
            "missing workflow must fail the completion write"
        );
        assert_eq!(next_run(v, &workflow.slug).as_deref(), Some(due.as_str()));
        finish_dispatch(v, dispatch, status);
        assert!(run_queue::occurrence_completed(v, &workflow.slug, &due).unwrap());
        assert_eq!(row(v, receiver).status, RowStatus::from_run_status(status));
        workflows::enqueue_due(v, now);
        assert_ne!(next_run(v, &workflow.slug).as_deref(), Some(due.as_str()));
        assert!(
            !run_queue::live_rows(v)
                .unwrap()
                .iter()
                .any(|row| row.source == RunSource::Cron),
            "the completed absorbed occurrence heals without another execution"
        );
    }
}

#[test]
fn scheduler_completion_does_not_advance_a_newer_saved_occurrence() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let due = (now - Duration::minutes(1)).to_rfc3339();
    let workflow = seed_workflow_with(
        v,
        "Origin Exact Occurrence",
        "0 0 7 * * * *",
        "BODY",
        |fm| {
            fm.next_run_at = Some(due.clone());
        },
    );
    workflows::enqueue_due(v, now);
    let dispatch = admitted(run_step(v, now, None, &deps()));
    let newer = now + Duration::days(3);
    vault_workflows::finalize_run(
        v,
        &workflow.slug,
        "success",
        None,
        0,
        Some(newer),
        RunLogEntry {
            at: now.to_rfc3339(),
            status: "success".into(),
            duration_sec: 0,
            note: "synthetic newer occurrence".into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        },
    )
    .unwrap();
    let saved = next_run(v, &workflow.slug);
    let Dispatch::Workflow { run, .. } = &dispatch else {
        panic!("workflow")
    };
    run.record_completion_for_test(v, "success").unwrap();
    assert_eq!(
        next_run(v, &workflow.slug),
        saved,
        "the saved value is compared inside the final workflow write"
    );
    finish_dispatch(v, dispatch, "success");
}

#[test]
fn unknown_manual_completion_preserves_a_separately_proven_cron_occurrence() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let due = (now - Duration::minutes(1)).to_rfc3339();
    let workflow = seed_workflow_with(
        v,
        "Unknown Versus Known Cron",
        "0 0 7 * * * *",
        "BODY",
        |fm| {
            fm.next_run_at = Some(due.clone());
        },
    );
    workflows::enqueue_due(v, now);
    let cron = run_queue::live_rows(v).unwrap()[0].id;
    let manual = queue_manual(v, &workflow, now);
    let dispatch = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch.row().id, manual);
    assert_eq!(row(v, cron).status, RowStatus::Queued);
    let Dispatch::Workflow { run, .. } = &dispatch else {
        panic!("workflow")
    };
    run.record_completion_for_test(v, "success").unwrap();
    assert_eq!(next_run(v, &workflow.slug).as_deref(), Some(due.as_str()));
    finish_dispatch(v, dispatch, "success");
    let dispatch = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch.row().id, cron);
    finish_dispatch(v, dispatch, "success");
}

/// `cancel` on a claimed heartbeat row reports StopRequested; the beat
/// used to run anyway because only workflow admission re-read the flag.
#[test]
fn a_cancel_that_lands_while_a_beat_is_prepared_stops_it_before_it_runs() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let beat = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "first beat")
            .with_key("hb:2026-09-25:1"),
        now,
    )
    .unwrap()
    .id();
    let cancelling = |vault: &Path| -> Result<PreparedBeat, String> {
        let claimed = run_queue::live_rows(vault)
            .unwrap()
            .into_iter()
            .find(|r| r.status == RowStatus::Claimed)
            .expect("the beat's row is claimed");
        assert_eq!(
            run_queue::cancel(vault, claimed.id, Utc::now()).unwrap(),
            run_queue::CancelOutcome::StopRequested
        );
        stub_beat(vault)
    };
    let cancelled_mid_prepare = StepDeps {
        prepare_beat: &cancelling,
        ..deps()
    };
    let (id, event, _) = handled(run_step(v, now, None, &cancelled_mid_prepare));
    assert_eq!((id, event), (beat, Some("cancelled")));
    assert_eq!(row(v, beat).status, RowStatus::Cancelled);
    assert_eq!(lease_holder(v), None, "the lease is released");
    let runs = automated_history::list_runs(v, 10).unwrap();
    let run = runs
        .iter()
        .find(|r| r.source == "heartbeat")
        .expect("the beat's run");
    assert_eq!(run.status, "cancelled");
}

/// A trigger row claimed ahead of a past-grace cron row for the same
/// workflow used to absorb it as `coalesced`, so the stale occurrence lost
/// its misfire row and notification (PR 2 kept both).
#[test]
fn a_past_grace_cron_beside_a_trigger_keeps_its_misfire_row_before_the_trigger_runs() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let t = seed_workflow_with(
        v,
        "Pump Stale Beside Trigger",
        "0 0 7 * * * *",
        "BODY",
        |fm| {
            fm.next_run_at = Some((now - Duration::hours(3)).to_rfc3339());
            fm.triggers = vec!["file-changed:notes/*.md".into()];
        },
    );
    touch_note(v, "notes/x.md");
    workflows::enqueue_due(v, now);
    let queued = run_queue::live_rows(v).unwrap();
    let cron = queued
        .iter()
        .find(|r| r.source == RunSource::Cron)
        .expect("a cron row")
        .id;
    let trigger = queued
        .iter()
        .find(|r| r.source == RunSource::Trigger)
        .expect("a trigger row")
        .id;

    let (handled_id, event, notice) = handled(run_step(v, now, None, &deps()));
    assert_eq!(
        (handled_id, event),
        (cron, Some("finished")),
        "the stale occurrence first"
    );
    assert_eq!(notice.map(|n| n.kind), Some(NoticeKind::Missed));
    assert_eq!(row(v, cron).status, RowStatus::Skipped);
    let log = run_log(v, &t.slug);
    assert_eq!(log[0].status, "skipped");
    assert_eq!(log[0].note, MISSED_WHILE_CLOSED_NOTE);
    assert_eq!(
        row(v, trigger).status,
        RowStatus::Queued,
        "the trigger row was put back"
    );

    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(run.row().id, trigger, "then the trigger fire runs");
    assert_eq!(
        row(v, cron).status,
        RowStatus::Skipped,
        "not re-labelled coalesced"
    );
    finish_dispatch(v, run, "success");
}

#[test]
fn an_invalid_cron_is_retried_within_grace_and_skipped_with_its_reason_past_grace() {
    invalid_cron_retried_then_skipped(Utc::now());
}

/// The 10-19 gate flake: the same case 30 s before 07:00 local.
#[test]
fn an_invalid_cron_case_holds_just_before_seven_local() {
    use chrono::TimeZone;
    let yesterday = Local::now().date_naive() - Duration::days(1);
    let at = Local
        .from_local_datetime(&yesterday.and_hms_opt(6, 59, 30).unwrap())
        .earliest()
        .unwrap();
    invalid_cron_retried_then_skipped(at.with_timezone(&Utc));
}

fn invalid_cron_retried_then_skipped(now: DateTime<Utc>) {
    let dir = temp_vault();
    let v = dir.path();
    let deps = deps();
    // A daily cron about 12 hours from now. A fixed `0 0 7` made the stale
    // workflow's next run the coming 07:00, which `now + 1 min` below reached
    // whenever the test ran between 06:59 and 07:00 local (overnight 10-19).
    let cron = format!(
        "0 0 {} * * * *",
        (now.with_timezone(&Local).hour() + 12) % 24
    );
    let fresh = seed_workflow_with(v, "Pump Empty Fresh", &cron, "", |fm| {
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
    });
    let before = workflow_bytes(v, &fresh.slug);
    let id = queue_cron(v, &fresh, now);
    let (_, event, _) = handled(run_step(v, now, None, &deps));
    assert_eq!(event, Some("requeued"));
    assert_eq!(row(v, id).attempt, 0);
    assert!(row(v, id).not_before.is_some());
    assert_eq!(workflow_bytes(v, &fresh.slug), before);

    let stale = seed_workflow_with(v, "Pump Empty Stale", &cron, "", |fm| {
        fm.next_run_at = Some((now - Duration::hours(3)).to_rfc3339());
    });
    queue_cron(v, &stale, now);
    let (_, _, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!(notice, None, "Hollow was open and nothing waited");
    assert_eq!(
        run_log(v, &stale.slug).pop().unwrap().note,
        "not run: prompt is empty; add a prompt body before running"
    );
    assert_ne!(next_run(v, &stale.slug), stale.next_run_at);

    let trigger = seed_workflow_with(v, "Pump Empty Trigger", "manual", "", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    touch_note(v, "notes/x.md");
    workflows::enqueue_due(v, now);
    let (id, _, _) = handled(run_step(v, now, None, &deps));
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    assert!(run_log(v, &trigger.slug).is_empty());
    let again = workflows::enqueue_due(v, now + Duration::minutes(1));
    assert!(again.changed.is_empty(), "the spent fire does not re-fire");
}

/// Paperclip PR 19: an unsafe working folder is the precheck's
/// configuration error: a queued Run now fails with it, a cron row is
/// retried silently within grace and skipped past it with the
/// `not run: working folder …` row, and nothing ever takes the lease.
#[cfg(unix)]
#[test]
fn a_run_now_with_an_unsafe_cwd_fails_and_a_cron_row_is_skipped_past_grace() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();

    let manual = seed_workflow_with(v, "Pump Unsafe Manual", "manual", "BODY", |fm| {
        fm.cwd = Some("/tmp".into());
    });
    let id = queue_manual(v, &manual, now);
    let (handled_id, event, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!((handled_id, event, notice), (id, Some("finished"), None));
    let failed = row(v, id);
    assert_eq!(failed.status, RowStatus::Failed);
    let error = failed.error.unwrap_or_default();
    assert!(error.starts_with("working folder "), "{error}");
    assert!(error.contains("system or shared folder"), "{error}");
    assert_eq!(lease_holder(v), None, "the lease is never taken");

    let fresh = seed_workflow_with(v, "Pump Unsafe Fresh", "0 0 7 * * * *", "BODY", |fm| {
        fm.cwd = Some("/".into());
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
    });
    let before = workflow_bytes(v, &fresh.slug);
    let id = queue_cron(v, &fresh, now);
    let (_, event, _) = handled(run_step(v, now, None, &deps));
    assert_eq!(event, Some("requeued"), "retried silently within grace");
    assert_eq!(row(v, id).attempt, 0);
    assert_eq!(workflow_bytes(v, &fresh.slug), before);

    let stale = seed_workflow_with(v, "Pump Unsafe Stale", "0 0 7 * * * *", "BODY", |fm| {
        fm.cwd = Some("/tmp".into());
        fm.next_run_at = Some((now - Duration::hours(3)).to_rfc3339());
    });
    queue_cron(v, &stale, now);
    let (_, _, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!(notice, None, "Hollow was open and nothing waited");
    let note = run_log(v, &stale.slug).pop().unwrap().note;
    assert!(note.starts_with("not run: working folder "), "{note}");
    assert_ne!(
        next_run(v, &stale.slug),
        stale.next_run_at,
        "the occurrence was consumed"
    );
    assert_eq!(lease_holder(v), None);
}

#[test]
fn heartbeat_rows_are_gated_at_claim_and_charge_the_budget_only_after_the_lease() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    // Heartbeat state lives outside the vault since Paperclip PR 8a.
    let state = heartbeat::state_path(v);
    let beat = |requested: DateTime<Utc>, key: &str| {
        run_queue::enqueue(
            v,
            NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "first beat").with_key(key),
            requested,
        )
        .unwrap()
        .id()
    };
    let night = StepDeps {
        local_hour: 3,
        heartbeat_cfg: HeartbeatConfig {
            work_hours_start: 7,
            work_hours_end: 19,
            min_interval_min: 30,
            daily_budget: 10,
        },
        ..deps()
    };
    let id = beat(now, "hb:2026-09-25:1");
    handled(run_step(v, now, None, &night));
    assert_eq!(row(v, id).status, RowStatus::Skipped, "outside work hours");
    assert!(!state.exists(), "nothing charged");

    let id = beat(now, "hb:2026-09-25:1");
    let chat = begin_run(v, "chat", None).unwrap();
    assert!(matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Busy { .. }
    ));
    assert!(!state.exists(), "a held lease charges nothing");
    finish_run(v, chat, "success").unwrap();

    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(run.row().id, id);
    let charged: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(charged["beats_today"], 1, "charged once the lease was held");
    assert!(
        !v.join(heartbeat::LEGACY_STATE_FILE).exists(),
        "never charged in the vault"
    );
    finish_dispatch(v, run, "success");

    let id = beat(now - Duration::minutes(61), "hb:2026-09-25:2");
    handled(run_step(v, now, None, &deps()));
    assert_eq!(row(v, id).status, RowStatus::Skipped, "stale");
    let id = beat(now, "hb:2026-09-25:3");
    let off = StepDeps {
        heartbeat_enabled: false,
        ..deps()
    };
    handled(run_step(v, now, None, &off));
    assert_eq!(row(v, id).status, RowStatus::Skipped, "switched off");
}

#[test]
fn run_now_delivers_the_executed_result_with_its_receipt_to_its_waiter() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Result M", "manual", "BODY");
    let (enqueued, mut receiver) = run_queue::enqueue_and_wait(
        v,
        NewRow::new(RunSource::Manual, Some(&m.slug), &m.title, "run now"),
        now,
    )
    .unwrap();
    let Dispatch::Workflow { row: claimed, run } = admitted(run_step(v, now, None, &deps())) else {
        panic!("a workflow dispatch");
    };
    let run_id = run.run_id().to_string();
    run.finish_for_test("success");
    let receipt = crate::evidence::EvidenceReceipt {
        id: "receipt-1".into(),
        run_id: run_id.clone(),
        source: "workflow".into(),
        source_id: Some(m.slug.clone()),
        started_at: now.to_rfc3339(),
        finished_at: now.to_rfc3339(),
        status: "success".into(),
        verification_state: "noChanges".into(),
        files: Vec::new(),
        checks: Vec::new(),
        failures: Vec::new(),
        history_run_id: run_id.clone(),
        ..Default::default()
    };
    let result = WorkflowRunResult {
        slug: m.slug.clone(),
        status: "success".into(),
        duration_sec: 3,
        output: "the brief".into(),
        error: None,
        receipt: Some(receipt),
        evidence_error: None,
    };
    let mut pending = Vec::new();
    let status = complete_workflow(v, &claimed, Ok(result), now, &mut pending);
    assert_eq!(status, RowStatus::Done);
    match receiver.try_recv() {
        Ok(RowOutcome::Workflow(Ok(delivered))) => {
            assert_eq!(delivered.output, "the brief");
            assert_eq!(delivered.duration_sec, 3);
            assert_eq!(
                delivered.receipt.map(|r| r.id),
                Some("receipt-1".to_string())
            );
        }
        other => panic!("the waiter gets the executed result: {other:?}"),
    }
    let done = row(v, enqueued.id());
    assert_eq!(done.status, RowStatus::Done);
    assert_eq!(done.history_run_id.as_deref(), Some(run_id.as_str()));

    let body = fn_body(
        include_str!("run_pump.rs"),
        concat!("pub(crate) async fn ", "execute("),
    );
    let executed = body
        .find(concat!("workflows::", "execute("))
        .expect("runs the workflow");
    let completed = body
        .find(concat!("complete_", "workflow("))
        .expect("completes the row");
    assert!(
        executed < completed,
        "the executed result is what reaches the waiter"
    );
}

/// Paperclip PR 15c: the receipt's timings reach the queue row through a
/// real pump admission. The mirror only lands while the row is still
/// `running` and only when the collector carries the id `mark_running`
/// recorded (`run.run_id()` for a workflow, `lease.id()` for a beat), so
/// this drives both surfaces through `step`, writes the receipt the way
/// their finalize does (after `finish_run`, before the pump ends the
/// row), and checks the row then holds both its own wait and the run's
/// evidence timing. A refactor that ended the row first, or recorded a
/// different id, fails here instead of silently dropping the mirror.
#[test]
fn pumped_runs_mirror_their_receipt_timings_into_their_row() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let requested = now - Duration::seconds(2);

    let m = seed_workflow(v, "Pump Timings M", "manual", "BODY");
    let id = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Manual, Some(&m.slug), &m.title, "run now"),
        requested,
    )
    .unwrap()
    .id();
    let Dispatch::Workflow { row: claimed, run } = admitted(run_step(v, now, None, &deps())) else {
        panic!("a workflow dispatch");
    };
    assert_eq!(claimed.id, id);
    let run_id = run.run_id().to_string();
    run.finish_for_test("success");
    let receipt = crate::evidence::EvidenceCollector::new("workflow", Some(&m.slug), &run_id)
        .finish_mirrored(v, "success", Vec::new())
        .unwrap();
    let timings = receipt.phase_timings.expect("receipt timings");
    assert_eq!(
        timings.queue_wait_ms,
        Some(2000),
        "the row's wait is on the receipt"
    );
    let mirrored = row(v, id).phase_timings.expect("row timings");
    assert_eq!(mirrored["queueWaitMs"].as_u64(), Some(2000));
    assert!(
        mirrored["evidenceMs"].is_u64(),
        "the receipt's timings reached the row: {mirrored}"
    );
    run_queue::finish(v, id, RowStatus::Done, None, now).unwrap();
    let done = row(v, id);
    assert_eq!(done.status, RowStatus::Done);
    assert!(
        done.phase_timings.unwrap()["evidenceMs"].is_u64(),
        "ending the row keeps them"
    );

    let id = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "first beat")
            .with_key("hb:2026-09-26:1"),
        requested,
    )
    .unwrap()
    .id();
    let Dispatch::Heartbeat {
        row: claimed,
        lease,
        ..
    } = admitted(run_step(v, now, None, &deps()))
    else {
        panic!("a heartbeat dispatch");
    };
    assert_eq!(claimed.id, id);
    let lease_id = lease.id().to_string();
    lease.finish("success").unwrap();
    crate::evidence::EvidenceCollector::new("heartbeat", None, &lease_id)
        .finish_mirrored(v, "success", Vec::new())
        .unwrap();
    let mirrored = row(v, id).phase_timings.expect("row timings");
    assert_eq!(mirrored["queueWaitMs"].as_u64(), Some(2000));
    assert!(
        mirrored["evidenceMs"].is_u64(),
        "a beat mirrors too: {mirrored}"
    );
    run_queue::finish(v, id, RowStatus::Done, None, now).unwrap();
}

#[test]
fn a_stale_or_unknown_lease_fails_interactive_rows_with_the_restart_hint() {
    let stale = LeaseHolder::Stale { id: "run-x".into() };
    assert!(refuse_interactive(Some(&stale))
        .unwrap()
        .contains("restart Hollow"));
    assert!(refuse_interactive(Some(&LeaseHolder::Unknown))
        .unwrap()
        .contains("restart Hollow"));
    assert_eq!(refuse_interactive(Some(&chat_holder())), None);
    assert_eq!(refuse_interactive(None), None);

    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Stale Manual", "manual", "BODY");
    let c = due_cron(v, "Pump Stale Cron", now - Duration::minutes(1));
    let (enqueued, mut receiver) = run_queue::enqueue_and_wait(
        v,
        NewRow::new(RunSource::Manual, Some(&m.slug), &m.title, "run now"),
        now,
    )
    .unwrap();
    let cron_id = queue_cron(v, &c, now);
    std::fs::create_dir_all(v.join(".hollow-history")).unwrap();
    std::fs::write(
        v.join(".hollow-history/active-run.nosync"),
        "not-a-run-owner\n",
    )
    .unwrap();

    match run_step(v, now, None, &deps()) {
        PumpStep::Busy { failed, .. } => assert_eq!(failed, vec![enqueued.id()]),
        _ => panic!("an unknown lease blocks the queue"),
    }
    match receiver.try_recv() {
        Ok(RowOutcome::Finished { status, error }) => {
            assert_eq!(status, RowStatus::Failed);
            assert!(error.unwrap_or_default().contains("restart Hollow"));
        }
        other => panic!("Run now fails with the restart hint: {other:?}"),
    }
    assert_eq!(
        row(v, cron_id).status,
        RowStatus::Queued,
        "automation rows keep waiting"
    );
}

#[test]
fn automations_wait_for_the_follow_up_grace_after_a_chat_turn() {
    let t = Utc::now();
    assert_eq!(
        gap_open_at(None, Some(t)),
        Some(t + Duration::seconds(CHAT_FOLLOWUP_GRACE_SECS))
    );
    let own = t + Duration::seconds(200);
    assert_eq!(
        gap_open_at(Some(own), Some(t)),
        Some(own),
        "the later of the two"
    );
    note_chat_finished(t);
    assert!(current_gap(None) >= Some(t + Duration::seconds(CHAT_FOLLOWUP_GRACE_SECS)));

    let dir = temp_vault();
    let v = dir.path();
    let c = due_cron(v, "Pump Grace Cron", t - Duration::minutes(1));
    let m = seed_workflow(v, "Pump Grace Manual", "manual", "BODY");
    queue_cron(v, &c, t);
    let gap = gap_open_at(None, Some(t));
    match run_step(v, t, gap, &deps()) {
        PumpStep::Sleep(at) => assert_eq!(Some(at), gap),
        _ => panic!("a cron head waits out the grace"),
    }
    queue_manual(v, &m, t);
    let run = admitted(run_step(v, t, gap, &deps()));
    assert_eq!(
        dispatch_slug(&run),
        Some(m.slug.as_str()),
        "Run now is exempt"
    );
    finish_dispatch(v, run, "success");

    let chat = include_str!("chat.rs");
    let finish = fn_body(chat, concat!("fn ", "finish_observed_run("));
    // PR 7b: the turn's `RunLease` is finished here (the public
    // `finish_run` underneath).
    let released = finish.find(concat!("lease.", "finish(status)")).unwrap();
    let noted = finish
        .find(concat!("note_chat", "_finished("))
        .expect("chat notes its finish");
    // Releasing the lease wakes the pump, which reads the gap at once: a
    // note written only after the release loses that race.
    assert!(noted < released, "noted before the lease is released");
}

#[test]
fn control_plane_failures_back_off_instead_of_spinning() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let s = due_cron(v, "Pump Failing Book", now - Duration::hours(3));
    let id = queue_cron(v, &s, now);
    let failing = |_: &Path| -> Result<BackgroundWriteLease, BeginRunError> {
        Err(BeginRunError::Failed("disk full".into()))
    };
    let broken = StepDeps {
        bookkeeping_lease: &failing,
        ..deps()
    };
    let (_, event, _) = handled(run_step(v, now, None, &broken));
    assert_eq!(event, Some("requeued"));
    let deferred = row(v, id);
    assert_eq!(deferred.status, RowStatus::Queued);
    assert_eq!(deferred.error.as_deref(), Some("disk full"));
    let not_before = parse_stamp(deferred.not_before.as_deref().unwrap()).unwrap();
    assert!(not_before >= now + Duration::seconds(RETRY_DEFER_SECS - 1));
    match run_step(v, now, None, &deps()) {
        PumpStep::Sleep(at) => assert_eq!(at, not_before),
        _ => panic!("the next step sleeps instead of spinning"),
    }

    // A terminal write that failed earlier is retried at the next step.
    let m = seed_workflow(v, "Pump Failing Finish", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    run_queue::claim(v, manual, now).unwrap().unwrap();
    let mut pending = vec![PendingFinish {
        id: manual,
        status: RowStatus::Done,
        error: None,
    }];
    let _ = step(v, now, None, &mut pending, &deps());
    assert!(pending.is_empty(), "the retried write landed");
    assert_eq!(row(v, manual).status, RowStatus::Done);
}

/// A claim whose follow-up `defer` or `requeue_waiting` failed (the
/// database's busy timeout) used to leave the row `claimed` for the whole
/// session: nothing re-claims a claimed row, so its Run now waiter hung and
/// its workflow's cron and trigger rows were never enqueued again.
#[test]
fn a_row_left_claimed_by_a_failed_write_is_queued_again_at_the_next_step() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Stray Claim", "manual", "BODY");
    let id = queue_manual(v, &m, now);
    // The claim committed; the write that should have followed it did not.
    run_queue::claim(v, id, now).unwrap().expect("claimed");
    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(
        run.row().id,
        id,
        "the stray row was queued again and admitted"
    );
    finish_dispatch(v, run, "success");
    assert_eq!(row(v, id).status, RowStatus::Done);

    // A claimed row with a pending terminal write has already run: it is
    // finished by the retry, never released to run a second time.
    let second = queue_manual(v, &m, now);
    run_queue::claim(v, second, now).unwrap().expect("claimed");
    assert_eq!(
        run_queue::release_stray_claims(v, &[second]).unwrap(),
        Vec::<i64>::new(),
        "kept rows stay claimed"
    );
    assert_eq!(row(v, second).status, RowStatus::Claimed);

    // A cancel that landed on the stuck claim is honoured once it is
    // claimed again (a cron row, so the skip is recorded at claim).
    let c = due_cron(v, "Pump Stray Cancel", now - Duration::minutes(1));
    let cron = queue_cron(v, &c, now);
    run_queue::finish(v, second, RowStatus::Done, None, now).unwrap();
    run_queue::claim(v, cron, now).unwrap().expect("claimed");
    assert_eq!(
        run_queue::cancel(v, cron, now).unwrap(),
        run_queue::CancelOutcome::StopRequested
    );
    let (handled_id, event, _) = handled(run_step(v, now, None, &deps()));
    assert_eq!((handled_id, event), (cron, Some("cancelled")));
    assert_eq!(row(v, cron).status, RowStatus::Cancelled);
}

/// Merged plan slice 3 (A·R4): a `mark_running` that fails (a busy
/// database) leaves the row non-executable this step. Its run is
/// finalized unexecuted, the lease is free, and the row is queued again
/// a minute later, when it runs once.
#[test]
fn a_failed_mark_running_runs_nothing_and_the_row_runs_once_later() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Unmarked M", "manual", "BODY");
    let id = queue_manual(v, &m, now);
    let flaky = StepDeps {
        fail_mark_running: true,
        ..deps()
    };
    let (handled_id, event, _) = handled(run_step(v, now, None, &flaky));
    assert_eq!((handled_id, event), (id, Some("requeued")), "not executed");
    let deferred = row(v, id);
    assert_eq!(deferred.status, RowStatus::Queued);
    assert_eq!(deferred.attempt, 0, "no attempt was counted");
    assert!(deferred.not_before.is_some(), "retried later, not at once");
    let note = deferred.error.unwrap_or_default();
    assert!(note.contains("could not mark it running"), "{note}");
    assert_eq!(
        lease_holder(v),
        None,
        "the unexecuted run released the lease"
    );
    assert!(!crate::workflows::is_running(&m.slug));
    let runs = automated_history::list_runs(v, 10).unwrap();
    let workflow_run = runs
        .iter()
        .find(|run| run.source == "workflow")
        .expect("the admitted run");
    assert_eq!(workflow_run.status, "cancelled", "finalized unexecuted");

    let later = now + Duration::seconds(RETRY_DEFER_SECS + 1);
    let run = admitted(run_step(v, later, None, &deps()));
    assert_eq!(run.row().id, id, "the same row runs on a later step");
    assert_eq!(row(v, id).status, RowStatus::Running);
    assert_eq!(row(v, id).attempt, 1);
    finish_dispatch(v, run, "success");
    assert_eq!(row(v, id).status, RowStatus::Done);
}

#[test]
fn a_heartbeat_whose_mark_running_fails_never_starts() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let beat = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "unmarked beat")
            .with_key("hb:2026-09-25:unmarked"),
        now,
    )
    .unwrap()
    .id();
    let flaky = StepDeps {
        fail_mark_running: true,
        ..deps()
    };
    let (handled_id, event, _) = handled(run_step(v, now, None, &flaky));
    assert_eq!(
        (handled_id, event),
        (beat, Some("requeued")),
        "not executed"
    );
    assert_eq!(row(v, beat).status, RowStatus::Queued);
    assert_eq!(lease_holder(v), None);
    let runs = automated_history::list_runs(v, 10).unwrap();
    let heartbeat_run = runs
        .iter()
        .find(|run| run.source == "heartbeat")
        .expect("the beat's run");
    assert_eq!(heartbeat_run.status, "cancelled");
}

#[test]
fn a_dropped_dispatch_is_finalized() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let beat = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "first beat")
            .with_key("hb:2026-09-25:1"),
        now,
    )
    .unwrap()
    .id();
    let dispatch = admitted(run_step(v, now + Duration::minutes(1), None, &deps()));
    assert_eq!(dispatch.row().id, beat);
    drop(dispatch);
    assert_eq!(
        lease_holder(v),
        None,
        "a dropped dispatch releases the lease"
    );
    let runs = automated_history::list_runs(v, 10).unwrap();
    let heartbeat_run = runs
        .iter()
        .find(|run| run.source == "heartbeat")
        .expect("the heartbeat's run");
    assert_eq!(heartbeat_run.status, "cancelled");
}

/// The contract's single-pump rule, pinned in source (needles split so
/// this test's own text never matches them).
#[test]
fn only_the_pump_calls_try_begin_run_for_automations() {
    let whole = include_str!("workflows.rs");
    let workflows_rs = &whole[..whole
        .find("#[cfg(test)]\n#[path = \"workflows_tests.rs\"]\nmod tests;")
        .expect("the test module")];
    let owned = concat!("try_begin", "_owned(");
    assert_eq!(workflows_rs.matches(owned).count(), 1);
    let admit = fn_body(workflows_rs, concat!("pub(crate) fn ", "admit_runnable("));
    assert!(
        admit.contains(owned),
        "the one lease call is in admit_runnable"
    );
    assert!(!workflows_rs.contains(concat!("try_begin", "_run(")));
    assert!(workflows_rs.contains(concat!("#[cfg(test)]\npub(crate) fn ", "admit(")));

    let admit_call = concat!("admit_", "runnable(");
    let gate_call = concat!("gate_scheduled", "_beat(");
    for (name, source) in [
        // chat_queue.rs is chat.rs's queued-turn half (B14): scan both.
        (
            "chat.rs + chat_queue.rs",
            concat!(include_str!("chat.rs"), include_str!("chat_queue.rs")),
        ),
        ("commands.rs", include_str!("commands.rs")),
        ("lib.rs", include_str!("lib.rs")),
        ("core_boot.rs", include_str!("core_boot.rs")),
        ("serve.rs", include_str!("serve.rs")),
        ("ingest.rs", include_str!("ingest.rs")),
        ("heartbeat.rs", include_str!("heartbeat.rs")),
    ] {
        assert!(!source.contains(admit_call), "{name} admits a workflow");
        if name != "heartbeat.rs" {
            assert!(!source.contains(gate_call), "{name} admits a beat");
        }
    }
    assert!(!workflows_rs.contains(gate_call));
    assert!(include_str!("run_pump.rs").contains(admit_call));
    let heartbeat_rs = include_str!("heartbeat.rs");
    for body in [
        fn_body(heartbeat_rs, concat!("pub fn ", "start_timer(")),
        fn_body(heartbeat_rs, concat!("pub async fn ", "run_now(")),
        fn_body(workflows_rs, concat!("pub fn ", "start_scheduler(")),
    ] {
        assert!(!body.contains(gate_call));
        for forbidden in [
            concat!("try_", "begin"),
            concat!("adm", "it_"),
            concat!("skip_scheduled", "_run("),
            concat!("append_run", "_log("),
        ] {
            assert!(!body.contains(forbidden), "{forbidden} in a timer");
        }
    }
    assert!(!include_str!("lib.rs").contains(concat!("begin_automated", "_history_run")));
    assert!(!include_str!("commands.rs").contains(concat!("begin_automated", "_history_run")));
}

#[test]
fn queue_ownership_is_exclusive_and_a_non_owner_never_recovers() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.pump.lock");
    let first = take_ownership(&path).expect("the first process owns it");
    assert!(take_ownership(&path).is_err(), "a second owner is refused");
    // A child another test is spawning right now holds a copy of the
    // descriptor until it execs; this copy plays that child, so the
    // release must not wait for it (merged plan wave 2 lock flake).
    let child_copy = first.descriptor_copy();
    drop(first);
    let again = take_ownership(&path).expect("free once the owner exits");
    drop(again);
    drop(child_copy);

    assert!(accepting_in(QueueState::NotStarted)
        .unwrap_err()
        .contains("restart Hollow"));
    assert!(accepting_in(QueueState::OwnedElsewhere)
        .unwrap_err()
        .contains("Another Hollow process"));
    assert!(accepting_in(QueueState::Recovering).is_ok());
    assert!(accepting_in(QueueState::Owned).is_ok());
    assert!(QueueState::Recovering.owned() && QueueState::Owned.owned());
    assert!(!QueueState::OwnedElsewhere.owned() && !QueueState::NotStarted.owned());

    // Only an owner spawns the loop, and the loop recovers before its
    // first claim.
    let source = include_str!("run_pump.rs");
    let start = fn_body(source, concat!("pub fn ", "start("));
    assert!(start.contains(concat!("QueueState::", "Recovering")));
    let pump = fn_body(source, concat!("async fn ", "pump_loop("));
    let recovered = pump.find(concat!("recover_", "queue(")).expect("recovers");
    let stepped =
        crate::source_pins::find(pump, concat!("drain(Arc::new(", "AppPump")).expect("drains");
    assert!(recovered < stepped);
    // Paperclip PR 7b moved recovery into `recover_queue`: it recovers
    // the rows first, and retiring the legacy file is best effort, so a
    // failure there never sends recovery round its retry loop. Merged
    // plan slice 3 (B·F10) moved the chat-turn close into the rows'
    // own transaction (`run_queue::interrupt_chat_run`, with the usage
    // rows), so it is retried with them and never runs on its own here.
    let recover = fn_body(source, concat!("pub(crate) fn ", "recover_queue("));
    let rows = recover
        .find(concat!("recover_at", "_startup("))
        .expect("recovers rows");
    let retired = recover
        .find(concat!("retire_legacy", "_deferrals("))
        .expect("retires");
    assert!(rows < retired);
    let line = &recover[retired..retired + recover[retired..].find('\n').unwrap()];
    assert!(
        !line.contains(")?"),
        "best-effort recovery step is not `?`-propagated"
    );
    assert!(!crate::source_pins::contains(
        recover,
        concat!("interrupt_turns", "_for_runs(")
    ));
    let queue = include_str!("run_queue.rs");
    let at_startup = fn_body(queue, concat!("pub(crate) fn ", "recover_at_startup("));
    assert!(
        crate::source_pins::contains(at_startup, concat!("interrupt_chat", "_run(tx, row, now)?")),
        "the chat run closes in the recovery transaction"
    );

    // A second process never finalizes the owner's live run: the owner
    // lock is taken before history reconciliation, which it gates (core
    // startup, `core_boot`, for the GUI and `hollow serve` alike).
    let lib = include_str!("core_boot.rs");
    let claimed = lib
        .find(concat!("run_pump::claim", "_ownership("))
        .expect("claims");
    // Paperclip PR 18: the startup reconcile is the sparing form, which
    // leaves a run another live process holds the lease lock for alone.
    let reconciled = lib
        .find(concat!(
            "automated_history::reconcile",
            "_interrupted_sparing("
        ))
        .expect("reconciles");
    assert!(
        claimed < reconciled,
        "ownership is claimed before history reconciliation"
    );
    assert!(lib[claimed..reconciled].contains(concat!("QueueState::", "OwnedElsewhere")));

    // Only the owner may end rows (their waiters live in its memory).
    let commands = include_str!("commands.rs");
    let cancel_workflow = fn_body(commands, concat!("pub async fn ", "cancel_workflow("));
    assert!(cancel_workflow.contains(concat!("run_pump::", "accepting()?")));
    let api = include_str!("commands/run_queue_api.rs");
    let cancel_queued = fn_body(api, concat!("pub async fn ", "cancel_queued_run("));
    assert!(cancel_queued.contains(concat!("run_pump::", "accepting()?")));
}

/// Contract PR 7 with the real permission guard: a tampered workflow's
/// cron, trigger, Run now and retry rows are all refused at claim, one
/// review exists, the lease is never taken, and after approval the held
/// work runs once.
#[test]
fn a_tampered_workflows_rows_are_refused_at_claim_from_every_source() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow_with(v, "Pump Guard W", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    tamper(
        v,
        &w.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    touch_note(v, "notes/x.md");
    workflows::enqueue_due(v, now);
    assert!(
        crate::trigger_scanner::load_state(v).contains_key(&w.slug),
        "the fire's watermark is committed at enqueue, before any approval"
    );
    let (manual, mut receiver) = run_queue::enqueue_and_wait(
        v,
        NewRow::new(RunSource::Manual, Some(&w.slug), &w.title, "run now"),
        now,
    )
    .unwrap();
    let retry = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Retry, Some(&w.slug), &w.title, "retry"),
        now,
    )
    .unwrap()
    .id();
    let deps = deps();
    for _ in 0..4 {
        handled(run_step(v, now, None, &deps));
    }
    match receiver.try_recv() {
        Ok(RowOutcome::Finished { error, .. }) => assert!(error
            .unwrap_or_default()
            .starts_with(crate::workflow_permissions::REFUSAL_PREFIX)),
        other => panic!("Run now hears the refusal: {other:?}"),
    }
    assert_eq!(row(v, manual.id()).status, RowStatus::Skipped);
    assert_eq!(row(v, retry).status, RowStatus::Skipped);
    let held: Vec<QueueRow> = run_queue::live_rows(v).unwrap();
    assert_eq!(held.len(), 2, "the cron and trigger rows are held");
    assert!(held
        .iter()
        .all(|r| r.held_reason.as_deref() == Some(vault_workflows::PERMISSION_REFUSED_REASON)));
    assert_eq!(pending_permission_reviews(v).len(), 1, "exactly one review");
    assert!(
        automated_history::list_runs(v, 10).unwrap().is_empty(),
        "no lease taken"
    );
    let refused = workflows::enqueue_run_now(
        v,
        &w.slug,
        now,
        workflows::precheck,
        None,
        crate::work_origin::Request::default(),
    )
    .expect_err("Run now refuses up front");
    assert!(refused.starts_with(crate::workflow_permissions::REFUSAL_PREFIX));

    let reviews = pending_permission_reviews(v);
    crate::workflow_permissions::approve(v, &reviews[0], None, Some(&reviews[0].fingerprint))
        .unwrap();
    let run = admitted(run_step(v, now + Duration::minutes(2), None, &deps));
    assert_eq!(dispatch_slug(&run), Some(w.slug.as_str()));
    let others: Vec<QueueRow> = run_queue::live_rows(v)
        .unwrap()
        .into_iter()
        .filter(|r| r.id != run.row().id)
        .collect();
    assert!(
        others.is_empty(),
        "the other held row was absorbed into the run"
    );
    finish_dispatch(v, run, "success");
}

/// Seed `budgets` (JSON defs) into the vault's test store and one ledger
/// row counting `tokens` for `surface`/`scope` (Paperclip PR 8a).
fn budget_over(v: &Path, budgets: Value, rows: &[(&str, Option<&str>, i64)]) {
    let file: budgets::BudgetsFile = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "budgets": budgets,
    }))
    .unwrap();
    budgets::Store::for_app(v).unwrap().save(&file).unwrap();
    for (i, (surface, scope, tokens)) in rows.iter().enumerate() {
        let mut row = crate::session_store::tests::usage_row_fixture(
            &format!("budget-row-{i}"),
            &crate::session_store::db_time(Utc::now()),
            "success",
        );
        row.surface = surface.to_string();
        row.scope = scope.map(str::to_string);
        row.input_tokens = Some(*tokens);
        row.output_tokens = Some(0);
        row.cost_status = "unpriced".into();
        crate::session_store::upsert_usage(v, &row).unwrap();
    }
}

fn pause_cards(v: &Path) -> usize {
    crate::review_queue::list(v, Some(budgets::REVIEW_KIND))
        .unwrap()
        .len()
}

/// Paperclip PR 8 plan test 40: a budget-paused workflow is refused at
/// claim, LAST (after PR 5's verify); each cron occurrence is consumed,
/// with one run-log row per pause episode, and never a misfire note;
/// Run now refuses up front; a tampered workflow gets PR 5's refusal and
/// no budget bookkeeping at all.
#[test]
fn a_budget_paused_workflow_is_refused_at_claim_and_consumes_each_occurrence() {
    // The second occurrence runs up to ~90 s later and must count the
    // same local day's usage (overnight D2).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    // Every minute, so the next occurrence stays inside today's window.
    let w = seed_workflow_with(v, "Pump Budget W", "0 * * * * * *", "BODY", |fm| {
        fm.agent = Some("claude".into());
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
    });
    let t = seed_workflow_with(v, "Pump Budget Tampered", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    budget_over(
        v,
        serde_json::json!([
            { "id": "bw", "scope": format!("workflow:{}", w.slug), "metric": "tokens", "window": "day", "limit": 1000 },
            { "id": "bt", "scope": format!("workflow:{}", t.slug), "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[
            ("workflow", Some(w.slug.as_str()), 1200),
            ("workflow", Some(t.slug.as_str()), 1200),
        ],
    );
    let first_due = w.next_run_at.clone().unwrap();
    let deps = deps();

    let id = queue_cron(v, &w, now);
    let (row_id, event, notice) = handled(run_step(v, now, None, &deps));
    assert_eq!((row_id, event), (id, Some("finished")));
    assert!(
        notice.is_none(),
        "a budget skip joins no missed/busy notice"
    );
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert!(ended
        .error
        .unwrap()
        .starts_with(budgets::BUDGET_PAUSED_PREFIX));
    let log = run_log(v, &w.slug);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert!(
        log[0]
            .note
            .starts_with(vault_workflows::BUDGET_SKIP_NOTE_PREFIX),
        "{}",
        log[0].note
    );
    assert!(log.iter().all(|e| e.note != MISSED_WHILE_CLOSED_NOTE));
    let next = next_run(v, &w.slug).expect("advanced");
    assert_ne!(next, first_due, "the occurrence was consumed");
    assert_eq!(pause_cards(v), 1);
    assert!(
        automated_history::list_runs(v, 10).unwrap().is_empty(),
        "no run was admitted"
    );

    // The next occurrence, once due: consumed as well, no second row.
    let due_next = parse_stamp(&next).unwrap();
    let later = due_next + Duration::seconds(30);
    let w_now = vault_workflows::read_full(v, &w.slug).unwrap().0;
    let second = queue_cron(v, &w_now, later);
    let (second_id, _, _) = handled(run_step(v, later, None, &deps));
    assert_eq!(second_id, second);
    assert_eq!(row(v, second).status, RowStatus::Skipped);
    assert_eq!(run_log(v, &w.slug).len(), 1, "one row per pause episode");
    assert_ne!(next_run(v, &w.slug), Some(next));
    assert_eq!(pause_cards(v), 1, "the same pause episode");

    // Run now refuses at once, read-only.
    let refused = workflows::enqueue_run_now(
        v,
        &w.slug,
        later,
        workflows::precheck,
        None,
        crate::work_origin::Request::default(),
    )
    .expect_err("Run now refuses up front");
    assert!(
        refused.starts_with(budgets::BUDGET_PAUSED_PREFIX),
        "{refused}"
    );

    // A tampered workflow: PR 5's refusal wins, and no budget card opens.
    tamper(
        v,
        &t.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    let (manual, mut receiver) = run_queue::enqueue_and_wait(
        v,
        NewRow::new(RunSource::Manual, Some(&t.slug), &t.title, "run now"),
        later,
    )
    .unwrap();
    let (manual_id, _, _) = handled(run_step(v, later, None, &deps));
    assert_eq!(manual_id, manual.id());
    match receiver.try_recv() {
        Ok(RowOutcome::Finished { error, .. }) => assert!(error
            .unwrap_or_default()
            .starts_with(crate::workflow_permissions::REFUSAL_PREFIX)),
        other => panic!("Run now hears the permission refusal: {other:?}"),
    }
    assert_eq!(
        pause_cards(v),
        1,
        "a refused workflow gets no budget bookkeeping"
    );
}

/// Paperclip PR 8b: the claim's budget reaches the admitted run. A
/// workflow under a token budget (not yet over) is claimed with the
/// budget's check and its claim-time baseline, and the admission carries
/// that budget to `execute`, which arms the run's guard from it.
#[test]
fn a_claimed_workflow_run_carries_its_token_checks_to_the_admission() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow_with(v, "Pump Budget Guard", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    budget_over(
        v,
        serde_json::json!([
            { "id": "bg", "scope": format!("workflow:{}", w.slug), "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("workflow", Some(w.slug.as_str()), 300)],
    );
    let deps = deps();

    // The claim itself.
    let id = queue_manual(v, &w, now);
    let row = run_queue::claim(v, id, now).unwrap().expect("claimed");
    match claim_verdict(v, &row, now, &deps) {
        ClaimVerdict::RunWorkflow {
            budget: Some(budget),
            ..
        } => {
            assert_eq!(budget.checks.len(), 1);
            assert_eq!(budget.checks[0].baseline, 300.0);
            assert!(
                budget.checks[0].enforced,
                "a workflow is an enforced intent"
            );
            assert!(budget.usage_stream());
        }
        _ => panic!("expected a workflow run carrying its budget"),
    }
    run_queue::finish(v, id, RowStatus::Cancelled, None, now).unwrap();

    // Through the pump: the admitted run carries the same budget.
    queue_manual(v, &w, now);
    match admitted(run_step(v, now, None, &deps)) {
        Dispatch::Workflow { row, run } => {
            let budget = run
                .budget()
                .expect("the admission carries the claim's budget");
            assert_eq!(budget.checks.len(), 1);
            assert_eq!(budget.checks[0].baseline, 300.0);
            finish_dispatch(v, Dispatch::Workflow { row, run }, "success");
        }
        _ => panic!("expected a workflow dispatch"),
    }
}

/// Merged plan slice 3, decision 6: a workflow whose own agent is at its
/// `agent:` ceiling runs on another agent with room instead of stopping,
/// and the admitted budget names the switch (`execute` puts it on the
/// receipt). With nobody to take over, the block stands and Run now's
/// failure says so.
#[test]
fn a_workflow_at_its_agent_ceiling_moves_to_an_agent_with_room() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow_with(v, "Pump Budget Fallback", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    budget_over(
        v,
        serde_json::json!([
            { "id": "ac", "scope": "agent:claude", "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("chat", Some("s-1"), 1200)],
    );
    fn claude_then_codex(_: &Workflow) -> Vec<AgentKind> {
        vec![AgentKind::Claude, AgentKind::Codex]
    }
    let with_codex = StepDeps {
        budget_fallbacks: &claude_then_codex,
        ..deps()
    };

    let id = queue_manual(v, &w, now);
    let row = run_queue::claim(v, id, now).unwrap().expect("claimed");
    match claim_verdict(v, &row, now, &with_codex) {
        ClaimVerdict::RunWorkflow {
            budget: Some(budget),
            ..
        } => {
            let fallback = budget.fallback.clone().expect("the run moved");
            assert_eq!(
                (fallback.from, fallback.to),
                (AgentKind::Claude, AgentKind::Codex)
            );
            assert!(
                fallback.reason.starts_with(budgets::BUDGET_PAUSED_PREFIX),
                "{}",
                fallback.reason
            );
            assert_eq!(budget.intent.agent, Some(AgentKind::Codex));
            let switch = fallback.receipt_switch();
            assert_eq!(
                (
                    switch.kind.as_str(),
                    switch.from.as_str(),
                    switch.to.as_str()
                ),
                ("agent", "Claude", "Codex")
            );
        }
        _ => panic!("expected the workflow to run on Codex"),
    }
    run_queue::finish(v, id, RowStatus::Cancelled, None, now).unwrap();
    assert_eq!(pause_cards(v), 1, "Claude's own ceiling is still paused");

    // Nobody with room: the stop is the block, and it says why.
    let id = queue_manual(v, &w, now);
    let row = run_queue::claim(v, id, now).unwrap().expect("claimed");
    match claim_verdict(v, &row, now, &deps()) {
        ClaimVerdict::Fail { error } => {
            assert!(error.starts_with(budgets::BUDGET_PAUSED_PREFIX), "{error}");
            assert!(error.ends_with(budgets::NO_ROOM_NOTE), "{error}");
        }
        _ => panic!("expected Run now to fail at the ceiling"),
    }
    run_queue::finish(v, id, RowStatus::Cancelled, None, now).unwrap();
}

/// Paperclip PR 8 plan test 41 (the Claude half; the stub beat is Claude):
/// an `agent:` budget applies to a pumped heartbeat, which is skipped
/// before it is charged or admitted.
#[test]
fn an_agent_budget_applies_to_a_pumped_heartbeat() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    budget_over(
        v,
        serde_json::json!([
            { "id": "ac", "scope": "agent:claude", "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("chat", Some("s-1"), 1200)],
    );
    let id = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "test")
            .with_key("hb:budget:1".to_string()),
        now,
    )
    .unwrap()
    .id();
    let (row_id, event, _) = handled(run_step(v, now, None, &deps()));
    assert_eq!((row_id, event), (id, Some("finished")));
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert!(ended
        .error
        .unwrap()
        .starts_with(budgets::BUDGET_PAUSED_PREFIX));
    assert!(
        automated_history::list_runs(v, 10).unwrap().is_empty(),
        "never admitted"
    );
    assert_eq!(pause_cards(v), 1);
}

/// Plan test 41, the Codex half: an `agent:claude` budget over its limit
/// leaves a heartbeat whose default agent resolves to Codex alone.
#[test]
fn an_agent_budget_leaves_a_heartbeat_on_another_agent_alone() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    budget_over(
        v,
        serde_json::json!([
            { "id": "ac", "scope": "agent:claude", "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("chat", Some("s-1"), 1200)],
    );
    fn codex_beat(vault: &Path) -> Result<PreparedBeat, String> {
        Ok(heartbeat::prepared_beat_for_test_as(
            vault,
            crate::agent::AgentKind::Codex,
        ))
    }
    let deps = StepDeps {
        prepare_beat: &codex_beat,
        ..deps()
    };
    let id = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "test")
            .with_key("hb:budget:codex".to_string()),
        now,
    )
    .unwrap()
    .id();
    match admitted(run_step(v, now, None, &deps)) {
        Dispatch::Heartbeat { row, .. } => assert_eq!(row.id, id),
        _ => panic!("the Codex heartbeat was admitted"),
    }
    assert_eq!(
        pause_cards(v),
        0,
        "a budget the run does not count toward opens nothing"
    );
}

/// Plan test 40's other claim arms: a budget-blocked Run now row fails
/// with the notice (its waiter hears it), a trigger row is skipped with
/// no run-log row, and a GLOBAL pause logs one row for EACH blocked
/// workflow.
#[test]
fn budget_blocked_manual_trigger_and_global_rows_end_as_designed() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let seed = |title: &str| {
        seed_workflow_with(v, title, "0 * * * * * *", "BODY", |fm| {
            fm.agent = Some("claude".into());
            fm.triggers = vec!["file-changed:notes/*.md".into()];
            fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
        })
    };
    let a = seed("Pump Budget Global A");
    let b = seed("Pump Budget Global B");
    budget_over(
        v,
        serde_json::json!([
            { "id": "g", "scope": "global", "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("chat", Some("s-1"), 1200)],
    );
    let deps = deps();

    // Run now that reached the queue before the budget tripped (the
    // up-front check is bypassed): the claim fails it with the notice.
    let (manual, mut receiver) = run_queue::enqueue_and_wait(
        v,
        NewRow::new(RunSource::Manual, Some(&a.slug), &a.title, "run now"),
        now,
    )
    .unwrap();
    let (manual_id, _, _) = handled(run_step(v, now, None, &deps));
    assert_eq!(manual_id, manual.id());
    match receiver.try_recv() {
        Ok(RowOutcome::Finished { status, error }) => {
            assert_eq!(status, RowStatus::Failed);
            assert!(error
                .unwrap_or_default()
                .starts_with(budgets::BUDGET_PAUSED_PREFIX));
        }
        other => panic!("Run now hears the budget notice: {other:?}"),
    }
    assert!(
        run_log(v, &a.slug).is_empty(),
        "a Run now refusal writes no run-log row"
    );

    // A trigger row: skipped, no run-log row, the fire spent.
    let trigger = queue_trigger(
        v,
        &a,
        vec![FiredToken {
            token: "file-changed:notes/*.md".into(),
            ..FiredToken::default()
        }],
        now,
    );
    let (trigger_id, _, _) = handled(run_step(v, now, None, &deps));
    assert_eq!(trigger_id, trigger);
    let ended = row(v, trigger);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert!(ended
        .error
        .unwrap_or_default()
        .starts_with(budgets::BUDGET_PAUSED_PREFIX));
    assert!(
        run_log(v, &a.slug).is_empty(),
        "a trigger skip writes no run-log row"
    );

    // One global pause, two workflows: one budget row each.
    for w in [&a, &b] {
        let id = queue_cron(v, w, now);
        let (claimed, _, _) = handled(run_step(v, now, None, &deps));
        assert_eq!(claimed, id);
        assert_eq!(row(v, id).status, RowStatus::Skipped);
        let log = run_log(v, &w.slug);
        assert_eq!(log.len(), 1, "{log:#?}");
        assert!(
            log[0]
                .note
                .starts_with(vault_workflows::BUDGET_SKIP_NOTE_PREFIX),
            "{}",
            log[0].note
        );
    }
    assert_eq!(pause_cards(v), 1, "one pause episode for the global budget");
    assert!(
        automated_history::list_runs(v, 10).unwrap().is_empty(),
        "nothing was admitted"
    );
}

/// A block because budget settings could not be READ is not a pause: the
/// cron row is held (its occurrence kept) within grace, and past it is
/// skipped with a "could not be read" row, never "over its limit" or a
/// misfire note.
#[test]
fn an_unreadable_budget_store_holds_a_cron_row_instead_of_consuming_it() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow_with(v, "Pump Budget Unreadable", "0 * * * * * *", "BODY", |fm| {
        fm.agent = Some("claude".into());
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
    });
    let store = budgets::Store::for_app(v).unwrap();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(store.path(), b"{ not json").unwrap();
    let due = w.next_run_at.clone().unwrap();
    let deps = deps();

    let id = queue_cron(v, &w, now);
    handled(run_step(v, now, None, &deps));
    let held = row(v, id);
    assert_eq!(held.status, RowStatus::Queued, "held, not consumed");
    assert_eq!(
        held.held_reason.as_deref(),
        Some(budgets::UNAVAILABLE_HELD_NOTE)
    );
    assert_eq!(
        next_run(v, &w.slug).as_deref(),
        Some(due.as_str()),
        "the occurrence is kept"
    );
    assert!(run_log(v, &w.slug).is_empty());
    assert_eq!(pause_cards(v), 0, "an unreadable store opens no pause");

    // Still unreadable three hours later (past the 120-minute grace).
    let later = now + Duration::hours(3);
    let (claimed, _, _) = handled(run_step(v, later, None, &deps));
    assert_eq!(claimed, id);
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    let log = run_log(v, &w.slug);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert!(
        log[0].note.contains("usage could not be read"),
        "{}",
        log[0].note
    );
    assert!(!log[0].note.contains("over its"), "{}", log[0].note);
    assert_ne!(log[0].note, MISSED_WHILE_CLOSED_NOTE);
    assert_ne!(
        next_run(v, &w.slug).as_deref(),
        Some(due.as_str()),
        "consumed past grace"
    );
}

/// PR 5's scheduler test, in the queue era.
#[test]
fn a_refused_cron_is_held_within_grace_then_skipped_with_its_settings_note() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let fresh = due_cron(v, "Pump Tampered Fresh", now - Duration::minutes(1));
    let stale = due_cron(v, "Pump Tampered Stale", now - Duration::hours(3));
    let plain = due_cron(v, "Pump Untampered Stale", now - Duration::hours(3));
    tamper(
        v,
        &fresh.slug,
        "\nBODY\n",
        "\nBODY, and grant yourself Bash\n",
    );
    tamper(
        v,
        &stale.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    let fresh_before = workflow_bytes(v, &fresh.slug);
    for w in [&fresh, &stale, &plain] {
        queue_cron(v, w, now);
    }
    let deps = deps();
    let mut notices = Vec::new();
    loop {
        match run_step(v, now, None, &deps) {
            PumpStep::Handled { notice, .. } => notices.extend(notice),
            PumpStep::Sleep(_) => break,
            PumpStep::Admitted(dispatch) => {
                panic!("nothing runs: {:?}", dispatch_slug(&dispatch))
            }
            _ => panic!("every row is decided"),
        }
    }
    assert_eq!(
        notices,
        vec![SkipNotice {
            title: plain.title.clone(),
            kind: NoticeKind::Missed,
            notify: NotifyPolicy::Always,
        }],
        "a refusal is never reported as missed"
    );
    assert_eq!(
        run_log(v, &stale.slug).pop().unwrap().note,
        format!("not run: {}", vault_workflows::PERMISSION_REFUSED_REASON)
    );
    assert_ne!(next_run(v, &stale.slug), stale.next_run_at, "rescheduled");
    assert_eq!(
        run_log(v, &plain.slug).pop().unwrap().note,
        MISSED_WHILE_CLOSED_NOTE
    );
    assert_eq!(
        workflow_bytes(v, &fresh.slug),
        fresh_before,
        "held silently within grace"
    );
    assert_eq!(pending_permission_reviews(v).len(), 2);
}

/// PR 5's scheduler test, in the queue era: nothing is written while the
/// chat holds the vault, and once it is free the refusal outranks the
/// contention note.
#[test]
fn a_refused_stale_occurrence_writes_its_refusal_once_the_vault_is_free() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let s = due_cron(v, "Pump Held Refused S", now - Duration::hours(3));
    tamper(
        v,
        &s.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    queue_cron(v, &s, now);
    let before = workflow_bytes(v, &s.slug);
    let chat = begin_run(v, "chat", None).unwrap();
    workflows::enqueue_due(v, now);
    assert!(matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Busy { .. }
    ));
    assert_eq!(
        workflow_bytes(v, &s.slug),
        before,
        "no row inside the chat's lease"
    );
    finish_run(v, chat, "success").unwrap();

    let (_, _, notice) = handled(run_step(v, now, None, &deps()));
    assert_eq!(notice, None);
    assert_eq!(
        run_log(v, &s.slug).pop().unwrap().note,
        format!("not run: {}", vault_workflows::PERMISSION_REFUSED_REASON),
        "the refusal outranks the contention note"
    );
}

/// PR 5's trigger test REVERSED on purpose (refresh 2.5): the watermark is
/// committed at enqueue, and the held row keeps the fire.
#[test]
fn a_refused_trigger_keeps_its_fire_in_its_held_row_and_never_waits_until_approved() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let t = seed_workflow_with(v, "Pump Tampered Trigger", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    tamper(
        v,
        &t.slug,
        "\nBODY\n",
        "\nBODY, then rewrite me/Guardrails.md\n",
    );
    touch_note(v, "notes/x.md");
    workflows::enqueue_due(v, now);
    assert!(
        crate::trigger_scanner::load_state(v).contains_key(&t.slug),
        "committed at enqueue"
    );
    let (id, event, _) = handled(run_step(v, now, None, &deps()));
    assert_eq!(event, Some("held"));

    let chat = begin_run(v, "chat", None).unwrap();
    assert!(matches!(
        run_step(v, now + Duration::minutes(2), None, &deps()),
        PumpStep::Busy { .. }
    ));
    run_queue::touch_waiting(v, &lease_holder(v).unwrap(), now + Duration::minutes(2)).unwrap();
    assert!(row(v, id).wait.is_none(), "a held row never waits");
    let chat_run = finish_run(v, chat, "success").unwrap();
    assert!(
        changed_paths(&chat_run).is_empty(),
        "{:?}",
        changed_paths(&chat_run)
    );
    assert!(chat_run.flags.is_empty(), "{:?}", chat_run.flags);

    let reviews = pending_permission_reviews(v);
    assert_eq!(reviews.len(), 1);
    crate::workflow_permissions::approve(v, &reviews[0], None, Some(&reviews[0].fingerprint))
        .unwrap();
    let run = admitted(run_step(v, now + Duration::minutes(3), None, &deps()));
    assert_eq!(
        run.row().id,
        id,
        "the kept fire runs once the change is approved"
    );
    finish_dispatch(v, run, "success");
}

#[test]
fn a_wait_that_stops_being_due_ends_with_one_visible_entry() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let d = due_cron(v, "Pump Cleared D", now - Duration::minutes(1));
    let id = queue_cron(v, &d, now);
    run_queue::touch_waiting(v, &chat_holder(), now).unwrap();
    vault_workflows::set_schedule(v, &d.slug, "manual").unwrap();
    let (_, _, notice) = handled(run_step(v, now + Duration::minutes(1), None, &deps()));
    assert_eq!(notice.map(|n| n.kind), Some(NoticeKind::Busy));
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    let log = run_log(v, &d.slug);
    assert_eq!(log.len(), 1);
    assert!(log[0].note.contains("no longer due"), "{}", log[0].note);
    assert!(
        matches!(run_step(v, now, None, &deps()), PumpStep::Idle),
        "it ends exactly once"
    );
}

#[test]
fn a_waiting_row_whose_workflow_ran_meanwhile_gets_no_false_skip_row() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let d = due_cron(v, "Pump Ran D", now - Duration::minutes(1));
    let id = queue_cron(v, &d, now);
    run_queue::touch_waiting(v, &chat_holder(), now).unwrap();
    vault_workflows::finalize_run(
        v,
        &d.slug,
        "success",
        None,
        1,
        Some(now + Duration::days(1)),
        RunLogEntry {
            at: (now + Duration::seconds(30)).to_rfc3339(),
            status: "success".into(),
            duration_sec: 1,
            note: "done".into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        },
    )
    .unwrap();
    let (_, _, notice) = handled(run_step(v, now + Duration::minutes(1), None, &deps()));
    assert_eq!(notice, None);
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    let log = run_log(v, &d.slug);
    assert!(!log
        .iter()
        .any(|e| e.status == vault_workflows::SKIPPED_STATUS));
}

#[test]
fn an_abandoned_wait_is_reported_as_missed_while_closed() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let e = due_cron(
        v,
        "Pump Quiet E",
        now - Duration::hours(3) - Duration::minutes(1),
    );
    queue_cron(v, &e, now - Duration::hours(3));
    run_queue::touch_waiting(v, &chat_holder(), now - Duration::hours(3)).unwrap();
    let (_, _, notice) = handled(run_step(v, now, None, &deps()));
    assert_eq!(notice.map(|n| n.kind), Some(NoticeKind::Missed));
    let newest = run_log(v, &e.slug).pop().unwrap();
    assert!(
        newest.note.starts_with(MISSED_WHILE_CLOSED_NOTE),
        "{}",
        newest.note
    );
    assert!(newest.note.contains("a chat turn"), "{}", newest.note);
    assert_ne!(newest.note, MISSED_WHILE_CLOSED_NOTE);
}

// ── Paperclip PR 9 (9a-2) ───────────────────────────────────────────

fn admitted_context(dispatch: &Dispatch) -> RunContext {
    match dispatch {
        Dispatch::Workflow { run, .. } => run.context().clone(),
        Dispatch::Heartbeat { .. } | Dispatch::Chat { .. } => {
            panic!("expected a workflow admission")
        }
    }
}

/// Every admission carries its claimed row's context: the source, the
/// reason (a trigger's fire reason reaches `{{trigger_reason}}`, nothing
/// else does), the attempt and the row id.
#[test]
fn scheduled_admissions_carry_their_run_context() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();

    let cron = due_cron(v, "Pump Ctx Cron", now - Duration::minutes(1));
    let cron_id = queue_cron(v, &cron, now);
    let run = admitted(run_step(v, now, None, &deps));
    let ctx = admitted_context(&run);
    assert_eq!(
        (ctx.source, ctx.reason.as_str(), ctx.attempt, ctx.queue_id),
        (RunSource::Cron, "scheduled", 0, Some(cron_id))
    );
    assert_eq!(ctx.trigger_reason(), None);
    finish_dispatch(v, run, "success");

    let watcher = seed_workflow_with(v, "Pump Ctx Trigger", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    let fired = vec![FiredToken {
        token: "file-changed:notes/*.md".into(),
        ..FiredToken::default()
    }];
    let trigger_id = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Trigger,
            Some(&watcher.slug),
            &watcher.title,
            "file changed: notes/a.md",
        )
        .with_payload(run_queue::trigger_payload(&fired)),
        now,
    )
    .unwrap()
    .id();
    let run = admitted(run_step(v, now, None, &deps));
    let ctx = admitted_context(&run);
    assert_eq!(
        (ctx.source, ctx.queue_id),
        (RunSource::Trigger, Some(trigger_id))
    );
    assert_eq!(
        ctx.trigger_reason().as_deref(),
        Some("file changed: notes/a.md")
    );
    finish_dispatch(v, run, "success");

    let manual = seed_workflow(v, "Pump Ctx Manual", "manual", "BODY");
    let manual_id = queue_manual(v, &manual, now);
    let run = admitted(run_step(v, now, None, &deps));
    let ctx = admitted_context(&run);
    assert_eq!(
        (ctx.source, ctx.reason.as_str(), ctx.queue_id),
        (RunSource::Manual, "run now", Some(manual_id))
    );
    assert_eq!(ctx.trigger_reason(), None);
    finish_dispatch(v, run, "success");
}

/// A row that absorbs a queued trigger fire at admission consumes that
/// fire, so its `{{trigger_reason}}` reads the fire's reason even though
/// the claimed row is Run now (a trigger row outranks a cron row, so a
/// cron row never claims ahead of a waiting fire; Run now does).
#[test]
fn an_admission_that_absorbs_a_trigger_fire_reads_its_reason() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();
    let w = seed_workflow_with(v, "Pump Ctx Absorb", "manual", "BODY", |fm| {
        fm.triggers = vec!["file-changed:notes/*.md".into()];
    });
    let fired = vec![FiredToken {
        token: "file-changed:notes/*.md".into(),
        ..FiredToken::default()
    }];
    let trigger_id = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Trigger,
            Some(&w.slug),
            &w.title,
            "file changed: notes/b.md",
        )
        .with_payload(run_queue::trigger_payload(&fired)),
        now,
    )
    .unwrap()
    .id();
    let manual_id = queue_manual(v, &w, now);
    let run = admitted(run_step(v, now, None, &deps));
    let ctx = admitted_context(&run);
    assert_eq!(
        (ctx.source, ctx.queue_id),
        (RunSource::Manual, Some(manual_id))
    );
    assert_eq!(
        ctx.trigger_reason().as_deref(),
        Some("file changed: notes/b.md")
    );
    assert_eq!(
        row(v, trigger_id).status,
        RowStatus::Coalesced,
        "the fire was consumed"
    );
    finish_dispatch(v, run, "success");
}

/// `notify: failures` or `never` keeps a workflow's title out of the
/// aggregated skip banners; its skip row is still written.
#[test]
fn a_notify_never_workflow_is_left_out_of_skip_banners_but_still_logged() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let deps = deps();
    let stale_at = now - Duration::hours(3);
    let quiet = seed_workflow_with(v, "Pump Notify Never", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(stale_at.to_rfc3339());
        fm.notify = Some("never".into());
    });
    let picky = seed_workflow_with(v, "Pump Notify Failures", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(stale_at.to_rfc3339());
        fm.notify = Some("failures".into());
    });
    let loud = due_cron(v, "Pump Notify Always", stale_at);
    let mut notices = Vec::new();
    for w in [&quiet, &picky, &loud] {
        queue_cron(v, w, now);
        let (_, _, notice) = handled(run_step(v, now, None, &deps));
        notices.extend(notice);
    }
    assert_eq!(
        notices.len(),
        3,
        "every skip still yields its notice: {notices:?}"
    );
    assert_eq!(
        announced_titles(&notices, NoticeKind::Missed),
        vec![loud.title.clone()],
        "only the default policy joins the banner"
    );
    assert!(announced_titles(&notices, NoticeKind::Busy).is_empty());
    for w in [&quiet, &picky, &loud] {
        assert_eq!(
            run_log(v, &w.slug).pop().unwrap().note,
            MISSED_WHILE_CLOSED_NOTE
        );
        assert_ne!(next_run(v, &w.slug), w.next_run_at, "advanced");
    }
}

/// The Busy banner obeys `notify` too, on both paths that build it: a
/// wait held past grace (`decide_cron`) and a wait that stopped being due
/// (`wait_ended`). The skip rows are still written.
#[test]
fn a_notify_never_workflow_is_left_out_of_the_busy_banner() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let later = now + Duration::hours(3);
    let deps = deps();
    let due_at = now - Duration::minutes(1);
    let quiet = seed_workflow_with(v, "Pump Busy Never", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(due_at.to_rfc3339());
        fm.notify = Some("never".into());
    });
    let loud = due_cron(v, "Pump Busy Always", due_at);
    for w in [&quiet, &loud] {
        queue_cron(v, w, now);
    }
    // Held past grace: the rows waited behind a chat turn for 3 hours.
    run_queue::touch_waiting(v, &chat_holder(), now).unwrap();
    run_queue::touch_waiting(v, &chat_holder(), later).unwrap();
    let mut notices = Vec::new();
    for _ in 0..2 {
        let (_, _, notice) = handled(run_step(v, later, None, &deps));
        notices.extend(notice);
    }
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(
        notices.iter().all(|n| n.kind == NoticeKind::Busy),
        "{notices:?}"
    );
    assert_eq!(
        announced_titles(&notices, NoticeKind::Busy),
        vec![loud.title.clone()]
    );
    for w in [&quiet, &loud] {
        let note = run_log(v, &w.slug).pop().unwrap().note;
        assert!(note.contains("grace"), "{note}");
    }

    // A wait that stopped being due (`wait_ended`).
    let quiet2 = seed_workflow_with(v, "Pump Cleared Never", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some(due_at.to_rfc3339());
        fm.notify = Some("never".into());
    });
    let loud2 = due_cron(v, "Pump Cleared Always", due_at);
    for w in [&quiet2, &loud2] {
        queue_cron(v, w, now);
    }
    run_queue::touch_waiting(v, &chat_holder(), now).unwrap();
    for w in [&quiet2, &loud2] {
        vault_workflows::set_schedule(v, &w.slug, "manual").unwrap();
    }
    let mut notices = Vec::new();
    for _ in 0..2 {
        let (_, _, notice) = handled(run_step(v, now + Duration::minutes(1), None, &deps));
        notices.extend(notice);
    }
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(
        notices.iter().all(|n| n.kind == NoticeKind::Busy),
        "{notices:?}"
    );
    assert_eq!(
        announced_titles(&notices, NoticeKind::Busy),
        vec![loud2.title.clone()]
    );
    for w in [&quiet2, &loud2] {
        let note = run_log(v, &w.slug).pop().unwrap().note;
        assert!(note.contains("no longer due"), "{note}");
    }
}

// ── Paperclip PR 7b: chat turns ─────────────────────────────────────────

/// Chat tests register the keyed stop slot and the live record, both
/// process-global: hold chat's lock, then the live record's.
fn chat_locks() -> (
    std::sync::MutexGuard<'static, ()>,
    std::sync::MutexGuard<'static, ()>,
) {
    let turns = chat::TURN_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let live = crate::agent_sessions::LIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::agent_sessions::clear();
    (turns, live)
}

/// A vault root the protected walk and history snapshots can see (a
/// dot-prefixed `.tmpXXXX` root is pruned).
fn chat_vault() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
}

fn chat_request(vault: &Path) -> chat::ChatSendRequest {
    chat::ChatSendRequest {
        prompt: "Hello pump".into(),
        session_id: None,
        session_key: None,
        cwd: vault.to_path_buf(),
        model: None,
        permission_mode: None,
        agent: crate::agent::AgentKind::Claude,
        client_turn_id: Some("ct-pump".into()),
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    }
}

fn queue_chat(vault: &Path, requested: DateTime<Utc>) -> chat::AcceptedTurn {
    chat::accept_turn(vault, chat_request(vault), requested).unwrap()
}

const CARD_KEY: &str = "s-20261108-e1card01";

fn record_open_card(vault: &Path, ord: i64) -> crate::types::ChatInteraction {
    let arguments = Ok(serde_json::json!({
        "mode": "confirmation",
        "supersedeOnUserMessage": true,
        "confirmation": { "prompt": "Go?" }
    }));
    let run = "run-card";
    let tool = "toolu-card";
    let new = crate::interactions::NewInteraction {
        session_key: CARD_KEY,
        turn_ord: ord,
        run_id: run,
        agent: crate::agent::AgentKind::Claude,
        native_session_id: None,
        tool_use_id: tool,
        source: crate::interactions::InteractionSource::Mcp,
        arguments: &arguments,
        cwd: vault,
        vault_root: vault,
    };
    crate::interactions::record(vault, &new)
        .unwrap()
        .interaction
}

/// An open card holds a wake. The person's own message still queues beside
/// it, and that message is what the pump admits. The wake stays queued, and
/// the person's later turn is what closes the card.
#[test]
fn e1_card_holds_a_wake_and_the_persons_message_closes_it() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let cwd = v.to_string_lossy().into_owned();
    let ord =
        crate::session_store::start_turn(v, CARD_KEY, "claude", &cwd, "run-card", "ask").unwrap();
    let card = record_open_card(v, ord);
    let now = Utc::now();
    let hold = crate::interactions::card_hold_until(v, CARD_KEY, now).unwrap();
    let mut wake = chat_request(v);
    wake.session_key = Some(CARD_KEY.into());
    wake.client_turn_id = Some("ct-wake".into());
    wake.host_asker = Some(crate::main_agent::ASKED_BY_WAKE);
    let queued = chat::accept_turn(v, wake, now).unwrap();
    match run_step(v, now, None, &deps()) {
        PumpStep::Sleep(at) => assert_eq!(at, hold, "the wake waits out the card"),
        PumpStep::Idle => panic!("the wake should stay queued, not idle"),
        PumpStep::Admitted(dispatch) => panic!("admitted {}", dispatch.row().id),
        PumpStep::Busy { .. } => panic!("busy"),
        PumpStep::Handled { row_id, .. } => panic!("handled {row_id}"),
        PumpStep::Error(error) => panic!("{error}"),
    }
    assert_eq!(
        run_queue::get(v, queued.queue_id).unwrap().unwrap().status,
        RowStatus::Queued
    );
    let live = run_queue::live_rows(v).unwrap();
    let facts = crate::run_pump::QueueFacts::read(v, &live);
    let wake_row = live.iter().find(|r| r.id == queued.queue_id).unwrap();
    assert_eq!(
        crate::run_pump::item_with_reason(v, wake_row, Some(1), &facts, now)
            .waiting_reason
            .as_deref(),
        Some(crate::run_pump::CARD_HOLD_WAIT),
        "the queue list names the card hold so the composer does not adopt it"
    );
    let mut person = chat_request(v);
    person.session_key = Some(CARD_KEY.into());
    person.client_turn_id = Some("ct-person".into());
    person.prompt = "Here is my answer".into();
    person.typed = true;
    let sent = chat::accept_turn(v, person, now).unwrap();
    assert_ne!(sent.queue_id, queued.queue_id);
    let admitted = admitted(run_step(v, now, None, &deps()));
    assert_eq!(
        admitted.row().id,
        sent.queue_id,
        "the person's turn, not the wake"
    );
    assert_eq!(
        run_queue::get(v, queued.queue_id).unwrap().unwrap().status,
        RowStatus::Queued
    );
    finish_dispatch(v, admitted, "success");
    let gone = crate::interactions::supersede_for_message(v, CARD_KEY, ord + 1).unwrap();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, card.id);
    assert!(crate::interactions::list_pending(v, CARD_KEY)
        .unwrap()
        .is_empty());
}

/// A lease wait or a permission hold on the queued wake does not hide the
/// card. The composer matches `an open question card` and leaves Send
/// enabled. The person's own queued row keeps no card name.
#[test]
fn e1_card_lease_wait_still_names_the_card() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let cwd = v.to_string_lossy().into_owned();
    let ord =
        crate::session_store::start_turn(v, CARD_KEY, "claude", &cwd, "run-card", "ask").unwrap();
    record_open_card(v, ord);
    let now = Utc::now();
    let mut wake = chat_request(v);
    wake.session_key = Some(CARD_KEY.into());
    wake.client_turn_id = Some("ct-wake-lease".into());
    wake.host_asker = Some(crate::main_agent::ASKED_BY_WAKE);
    let queued = chat::accept_turn(v, wake, now).unwrap();
    let stamp = crate::session_store::db_time(now);
    crate::session_store::with_write(v, "test", |tx| {
        tx.execute(
            "UPDATE run_queue SET waiting_since = ?2, last_waited_at = ?2, \
             waiting_holder = 'a chat', wait_count = 1 WHERE id = ?1",
            rusqlite::params![queued.queue_id, stamp],
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let named = |id: i64| {
        let live = run_queue::live_rows(v).unwrap();
        let facts = crate::run_pump::QueueFacts::read(v, &live);
        let row = live.iter().find(|r| r.id == id).unwrap();
        let reason = crate::run_pump::item_with_reason(v, row, Some(1), &facts, now).waiting_reason;
        (row.wait.is_some(), row.held_reason.clone(), reason)
    };
    let (wait, _held, reason) = named(queued.queue_id);
    assert!(wait, "the lease wait is on the row");
    assert_eq!(reason.as_deref(), Some(crate::run_pump::CARD_HOLD_WAIT));
    crate::session_store::with_write(v, "test", |tx| {
        tx.execute(
            "UPDATE run_queue SET held_reason = 'held for review' WHERE id = ?1",
            rusqlite::params![queued.queue_id],
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let (wait, held, reason) = named(queued.queue_id);
    assert_eq!(held.as_deref(), Some("held for review"));
    assert!(wait);
    assert_eq!(reason.as_deref(), Some(crate::run_pump::CARD_HOLD_WAIT));
    let mut person = chat_request(v);
    person.session_key = Some(CARD_KEY.into());
    person.client_turn_id = Some("ct-person-lease".into());
    person.prompt = "Here is my answer".into();
    person.typed = true;
    let sent = chat::accept_turn(v, person, now).unwrap();
    crate::session_store::with_write(v, "test", |tx| {
        tx.execute(
            "UPDATE run_queue SET waiting_since = ?2, last_waited_at = ?2, \
             waiting_holder = 'a chat', wait_count = 1 WHERE id = ?1",
            rusqlite::params![sent.queue_id, stamp],
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let (wait, _held, reason) = named(sent.queue_id);
    assert!(wait);
    assert_eq!(
        reason, None,
        "a person's row keeps its lease wait, not the card name"
    );
}

/// Past 30 minutes the card no longer holds the queue. The wake may start,
/// and starting it does not close the card (only the person's turn does).
#[test]
fn e1_card_older_than_thirty_minutes_does_not_hold() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let cwd = v.to_string_lossy().into_owned();
    let ord =
        crate::session_store::start_turn(v, CARD_KEY, "claude", &cwd, "run-old", "ask").unwrap();
    let card = record_open_card(v, ord);
    let now = Utc::now();
    let aged = now - Duration::minutes(31);
    crate::session_store::with_write(v, "test", |tx| {
        tx.execute(
            "UPDATE interactions SET created_at = ?2 WHERE id = ?1",
            rusqlite::params![card.id, crate::session_store::db_time(aged)],
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(crate::interactions::card_hold_until(v, CARD_KEY, now), None);
    let mut wake = chat_request(v);
    wake.session_key = Some(CARD_KEY.into());
    wake.client_turn_id = Some("ct-wake-old".into());
    wake.host_asker = Some(crate::main_agent::ASKED_BY_WAKE);
    let queued = chat::accept_turn(v, wake, now).unwrap();
    match run_step(v, now, None, &deps()) {
        PumpStep::Admitted(dispatch) => {
            assert_eq!(dispatch.row().id, queued.queue_id);
            assert_eq!(
                crate::interactions::list_pending(v, CARD_KEY)
                    .unwrap()
                    .len(),
                1,
                "admitting the wake leaves the card open"
            );
            finish_dispatch(v, dispatch, "success");
        }
        PumpStep::Sleep(at) => panic!("slept until {at} with no card hold"),
        PumpStep::Idle => panic!("idle"),
        PumpStep::Busy { .. } => panic!("busy"),
        PumpStep::Handled { row_id, .. } => panic!("handled {row_id}"),
        PumpStep::Error(error) => panic!("{error}"),
    }
}

/// Send resolves the card before the answer is queued. The answer still
/// queues beside the wake, which stays queued.
#[test]
fn e1_card_answer_queues_after_the_card_is_resolved() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let cwd = v.to_string_lossy().into_owned();
    let ord =
        crate::session_store::start_turn(v, CARD_KEY, "claude", &cwd, "run-card", "ask").unwrap();
    let card = record_open_card(v, ord);
    let now = Utc::now();
    let mut wake = chat_request(v);
    wake.session_key = Some(CARD_KEY.into());
    wake.client_turn_id = Some("ct-wake-resolved".into());
    wake.host_asker = Some(crate::main_agent::ASKED_BY_WAKE);
    let queued = chat::accept_turn(v, wake, now).unwrap();
    crate::session_store::with_write(v, "test", |tx| {
        tx.execute(
            "UPDATE interactions SET status = 'answered', resolved_at = ?2 WHERE id = ?1",
            rusqlite::params![card.id, crate::session_store::db_time(now)],
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(crate::interactions::open_hold_until(v, CARD_KEY, now), None);
    let mut person = chat_request(v);
    person.session_key = Some(CARD_KEY.into());
    person.client_turn_id = Some("ct-answered".into());
    person.prompt = "Here is my answer".into();
    person.typed = true;
    let sent = chat::accept_turn(v, person, now).unwrap();
    assert_ne!(sent.queue_id, queued.queue_id);
    assert_eq!(
        run_queue::get(v, queued.queue_id).unwrap().unwrap().status,
        RowStatus::Queued
    );
}

/// A card on the conversation this one continued from still holds a wake
/// here, and the person's send here closes that card.
#[test]
fn e1_card_on_an_earlier_chat_holds_the_current_one() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let cwd = v.to_string_lossy().into_owned();
    let newer = "s-20261108-e1card02";
    let ord =
        crate::session_store::start_turn(v, CARD_KEY, "claude", &cwd, "run-card", "ask").unwrap();
    let card = record_open_card(v, ord);
    crate::session_store::start_turn(v, newer, "claude", &cwd, "run-next", "continue").unwrap();
    crate::app_sessions::record_handover(v, CARD_KEY, newer, 1).unwrap();
    let now = Utc::now();
    let hold = crate::interactions::open_hold_until(v, newer, now).unwrap();
    assert_eq!(
        hold,
        crate::interactions::card_hold_until(v, CARD_KEY, now).unwrap()
    );
    let mut wake = chat_request(v);
    wake.session_key = Some(newer.into());
    wake.client_turn_id = Some("ct-wake-chain".into());
    wake.host_asker = Some(crate::main_agent::ASKED_BY_WAKE);
    let queued = chat::accept_turn(v, wake, now).unwrap();
    match run_step(v, now, None, &deps()) {
        PumpStep::Sleep(at) => assert_eq!(at, hold),
        PumpStep::Idle => panic!("idle"),
        PumpStep::Admitted(dispatch) => panic!("admitted {}", dispatch.row().id),
        PumpStep::Busy { .. } => panic!("busy"),
        PumpStep::Handled { row_id, .. } => panic!("handled {row_id}"),
        PumpStep::Error(error) => panic!("{error}"),
    }
    assert_eq!(
        run_queue::get(v, queued.queue_id).unwrap().unwrap().status,
        RowStatus::Queued
    );
    let gone = crate::interactions::supersede_for_message(v, newer, 1).unwrap();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, card.id);
    assert!(crate::interactions::list_pending(v, CARD_KEY)
        .unwrap()
        .is_empty());
}

/// One conversation from the person's side (2026-11-07): a send for a
/// conversation that handed over never queues there, including the race
/// where it arrives while the old turn still runs its handover (on 11-07
/// "Install" ran in both chats); it goes to the successor, once that is
/// free, with the note in both.
#[test]
fn a_send_for_a_handed_over_conversation_goes_to_its_successor_even_mid_handover() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let cwd = v.to_string_lossy().into_owned();
    let (old, new) = ("s-20261107-mcc-old", "s-20261107-mcc-new");
    crate::session_store::start_turn(v, old, "claude", &cwd, "R-mcc-old", "start").unwrap();
    let mut first = chat_request(v);
    first.session_key = Some(old.into());
    let running = chat::accept_turn(v, first, now).unwrap();
    // Mid-handover: the successor is queued and recorded; the old turn runs on.
    crate::session_store::start_turn(v, new, "claude", &cwd, "R-mcc-new", "continue").unwrap();
    crate::app_sessions::record_handover(v, old, new, 1).unwrap();
    let mut heir_turn = chat_request(v);
    heir_turn.session_key = Some(new.into());
    heir_turn.client_turn_id = Some("ct-heir".into());
    let heir = chat::accept_turn(v, heir_turn, now).unwrap();
    // "Install" typed in the old tab: never queued in the old conversation.
    let mut late = chat_request(v);
    late.session_key = Some(old.into());
    late.prompt = "Install".into();
    late.client_turn_id = Some("ct-late".into());
    assert_eq!(
        chat::accept_turn(v, late.clone(), now).unwrap_err(),
        chat::CHAT_ALREADY_QUEUED,
        "the successor is busy (the person's send is held for it)"
    );
    run_queue::finish(v, running.queue_id, RowStatus::Done, None, now).unwrap();
    assert_eq!(
        chat::accept_turn(v, late.clone(), now).unwrap_err(),
        chat::CHAT_ALREADY_QUEUED,
        "the old turn ended: still the successor's"
    );
    run_queue::finish(v, heir.queue_id, RowStatus::Done, None, now).unwrap();
    let sent = chat::accept_turn(v, late, now).unwrap();
    assert_eq!(sent.session_key, new);
    assert_eq!(
        sent.forwarded.as_ref().and_then(|r| r.from.as_deref()),
        Some(old)
    );
    let row = run_queue::get(v, sent.queue_id).unwrap().unwrap();
    assert_eq!(row.source_id.as_deref(), Some(new));
    assert!(row.payload["prompt"]
        .as_str()
        .unwrap()
        .starts_with("<hollow-forwarded>"));
    assert_eq!(
        row.detail.as_deref(),
        Some("Install"),
        "the person's own words"
    );
    let notes = crate::chat_chain::forwards_for(v, old);
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].to.as_str(), notes[0].preview.as_str()),
        (new, "Install")
    );
    assert_eq!(
        crate::chat_chain::forwards_for(v, new),
        notes,
        "both show it"
    );
    let wire = sent.into_wire();
    assert_eq!(wire.forwarded_from.as_deref(), Some(old));
}

/// Lane Q (2026-11-13): a send typed in a conversation that continued
/// elsewhere carries the old composer's level, which is only the old
/// conversation's own. Forwarded, it is dropped so the successor's own level
/// applies when the turn runs; a level that differs (a phone send's pick) is
/// kept.
#[test]
fn a_forwarded_send_runs_at_the_successors_effort_not_the_old_composers() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let cwd = v.to_string_lossy().into_owned();
    let (old, new) = ("s-20261113-qeff-old", "s-20261113-qeff-new");
    crate::session_store::start_turn(v, old, "claude", &cwd, "R-qeff-old", "start").unwrap();
    crate::session_store::set_chat_effort(v, old, Some("max")).unwrap();
    crate::session_store::start_turn(v, new, "claude", &cwd, "R-qeff-new", "continue").unwrap();
    crate::app_sessions::record_handover(v, old, new, 1).unwrap();
    crate::session_store::set_chat_effort(v, new, Some("low")).unwrap();
    let mut late = chat_request(v);
    late.session_key = Some(old.into());
    late.prompt = "Install".into();
    let sent = chat::accept_turn_with_effort(v, late.clone(), now, Some("max".into())).unwrap();
    assert_eq!(sent.session_key, new);
    let row = run_queue::get(v, sent.queue_id).unwrap().unwrap();
    assert!(row.payload.get("effort").is_none(), "no named level");
    assert_eq!(
        chat::resolve_requested_effort(v, new, None).as_deref(),
        Some("low"),
        "the successor's own level"
    );
    run_queue::finish(v, sent.queue_id, RowStatus::Done, None, now).unwrap();
    // A level that isn't the old conversation's own is the sender's pick.
    let sent = chat::accept_turn_with_effort(v, late, now, Some("high".into())).unwrap();
    let row = run_queue::get(v, sent.queue_id).unwrap().unwrap();
    assert_eq!(row.payload["effort"], "high");
}

/// A successor whose first turn is still queued has no conversation row
/// yet; a send for the chat that handed over still never queues there
/// (review of #1220).
#[test]
fn a_send_for_a_chat_whose_successor_is_still_queued_never_queues_in_it() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let cwd = v.to_string_lossy().into_owned();
    let (old, new) = ("s-20261107-mcq-old", "s-20261107-mcq-new");
    crate::session_store::start_turn(v, old, "claude", &cwd, "R-mcq-old", "start").unwrap();
    let mut heir_turn = chat_request(v);
    heir_turn.session_key = Some(new.into());
    heir_turn.client_turn_id = Some("ct-mcq-heir".into());
    let heir = chat::accept_turn(v, heir_turn, now).unwrap();
    crate::app_sessions::record_handover(v, old, new, 1).unwrap();
    assert!(crate::session_store::session_by_key(v, new)
        .unwrap()
        .is_none());
    let mut late = chat_request(v);
    late.session_key = Some(old.into());
    late.client_turn_id = Some("ct-mcq-late".into());
    assert_eq!(
        chat::accept_turn(v, late.clone(), now).unwrap_err(),
        chat::CHAT_ALREADY_QUEUED,
        "its successor's queued turn holds it"
    );
    run_queue::finish(v, heir.queue_id, RowStatus::Done, None, now).unwrap();
    let sent = chat::accept_turn(v, late, now).unwrap();
    assert_eq!(sent.session_key, new);
}

/// A new Main agent conversation started while the current one works joins
/// it instead of running beside it; a handover's successor (depth 1) still
/// starts its own.
#[test]
fn a_new_main_thread_joins_the_working_one_but_a_handover_starts_its_own() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::workspaces::ensure_main_agent(v).unwrap();
    let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
    let main = "s-20261107-mcc-main";
    crate::session_store::start_turn(v, main, "claude", &cwd, "R-mcc-main", "hi").unwrap();
    let mut first = chat_request(v);
    first.session_key = Some(main.into());
    first.cwd = PathBuf::from(&cwd);
    let running = chat::accept_turn(v, first, now).unwrap();
    let mut fresh = chat_request(v);
    fresh.cwd = PathBuf::from(&cwd);
    fresh.client_turn_id = Some("ct-fresh".into());
    assert_eq!(
        chat::accept_turn(v, fresh.clone(), now).unwrap_err(),
        chat::CHAT_ALREADY_QUEUED,
        "joined the working thread, which is busy"
    );
    let mut heir = fresh.clone();
    heir.app_depth = 1;
    heir.client_turn_id = Some("ct-main-heir".into());
    let started = chat::accept_turn(v, heir, now).unwrap();
    assert_ne!(started.session_key, main);
    assert!(started.forwarded.is_none());
    run_queue::finish(v, running.queue_id, RowStatus::Done, None, now).unwrap();
    // Idle: a new conversation is the person's reset, not a join.
    let mut reset = fresh;
    reset.client_turn_id = Some("ct-reset".into());
    let opened = chat::accept_turn(v, reset, now).unwrap();
    assert_ne!(opened.session_key, main);
}

fn handled_chat(outcome: PumpStep) -> (i64, Option<&'static str>, Vec<Value>) {
    match outcome {
        PumpStep::Handled {
            row_id,
            event,
            chat,
            ..
        } => (row_id, event, chat),
        PumpStep::Admitted(dispatch) => {
            panic!("expected Handled, got Admitted({})", dispatch.row().id)
        }
        PumpStep::Busy { holder, .. } => panic!("expected Handled, got Busy({holder:?})"),
        PumpStep::Idle => panic!("expected Handled, got Idle"),
        PumpStep::Sleep(at) => panic!("expected Handled, got Sleep({at})"),
        PumpStep::Error(error) => panic!("expected Handled, got Error({error})"),
    }
}

#[test]
fn a_queued_chat_turn_waits_for_a_running_automation_without_preempting_it() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Pump Chat Wait Manual", "manual", "BODY");
    let c = due_cron(v, "Pump Chat Wait Cron", now - Duration::minutes(1));
    queue_manual(v, &m, now);
    let workflow = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch_slug(&workflow), Some(m.slug.as_str()));

    let accepted = queue_chat(v, now);
    assert!(
        accepted
            .waiting_for
            .as_deref()
            .is_some_and(|holder| holder.contains("workflow")),
        "the reply names the holder: {:?}",
        accepted.waiting_for
    );
    queue_cron(v, &c, now);
    match run_step(v, now, None, &deps()) {
        PumpStep::Busy {
            holder,
            failed,
            chat,
            ..
        } => {
            assert!(holder.is_workflow(&m.slug), "{holder:?}");
            assert!(
                failed.is_empty() && chat.is_empty(),
                "a live holder is waited for"
            );
        }
        _ => panic!("the chat turn waits for the running automation"),
    }
    assert!(
        crate::workflows::is_running(&m.slug),
        "nothing stopped the automation"
    );
    let waiting = row(v, accepted.queue_id);
    assert_eq!(waiting.status, RowStatus::Queued);
    assert!(waiting.wait.is_some(), "its wait names the holder");
    finish_dispatch(v, workflow, "success");

    // First once the lease is free, ahead of the cron row, gap or not.
    let gap = Some(now + Duration::seconds(MIN_AUTOMATION_GAP_SECS));
    let turn = admitted(run_step(v, now, gap, &deps()));
    match &turn {
        Dispatch::Chat {
            row: admitted_row, ..
        } => assert_eq!(admitted_row.id, accepted.queue_id),
        _ => panic!("the chat turn is admitted first"),
    }
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Running);
    assert_eq!(row(v, accepted.queue_id).attempt, 1);
    assert!(matches!(
        lease_holder(v),
        Some(LeaseHolder::Run { source: Some(ref source), .. }) if source == "chat"
    ));
    let mut same = chat_request(v);
    same.session_key = Some(accepted.session_key.clone());
    assert_eq!(
        chat::accept_turn(v, same, now).unwrap_err(),
        chat::CHAT_ALREADY_QUEUED,
        "one turn at a time per conversation"
    );
    finish_dispatch(v, turn, "success");
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Done);
    assert!(lease_holder(v).is_none());
}

/// Build programme PR B3: a build workspace's chat turn is admitted on its
/// own folder while an automation holds the vault, and each run records
/// only its own writes.
#[test]
fn a_build_workspace_turn_runs_beside_a_running_automation() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let workspace = crate::workspaces::create(v, "Pump build lane", "Ship it").unwrap();
    let tree_dir = tempfile::Builder::new()
        .prefix("worktree-")
        .tempdir()
        .unwrap();
    crate::build_workspaces::bind_for_test(v, &workspace.id, tree_dir.path());
    let m = seed_workflow(v, "Pump Build Lane Manual", "manual", "BODY");
    queue_manual(v, &m, now);
    let workflow = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch_slug(&workflow), Some(m.slug.as_str()));

    let mut request = chat_request(v);
    request.cwd = tree_dir.path().to_path_buf();
    let accepted = chat::accept_turn(v, request, now).unwrap();
    assert_eq!(accepted.waiting_for, None, "a build turn waits for nobody");
    let turn = admitted(run_step(v, now, None, &deps()));
    assert!(turn.is_scoped(), "admitted on its own folder");
    assert_eq!(turn.row().id, accepted.queue_id);
    assert!(
        lease_holder(v).is_some_and(|holder| holder.is_workflow(&m.slug)),
        "the automation keeps the vault"
    );

    let brief = v.join(&workspace.path).join("WORKSPACE.md");
    std::fs::write(&brief, "brief, updated by the build").unwrap();
    touch_note(v, "notes/routine.md");
    let chat_run = finish_dispatch(v, turn, "success");
    assert_eq!(chat_run.scope.as_deref(), Some(workspace.path.as_str()));
    assert_eq!(
        changed_paths(&chat_run),
        [format!("{}/WORKSPACE.md", workspace.path)]
    );
    let routine = finish_dispatch(v, workflow, "success");
    assert!(changed_paths(&routine).contains(&"notes/routine.md".to_string()));
    assert!(
        !changed_paths(&routine)
            .iter()
            .any(|p| p.starts_with(&workspace.path)),
        "the build's folder is not the routine's change: {:?}",
        changed_paths(&routine)
    );
}

/// 2026-09-28: a conversation in an ordinary workspace (its cwd is the
/// workspace's own vault folder) is scoped too, so it runs beside a
/// running automation and beside another workspace's conversation, while
/// a turn at the vault root still takes the whole vault.
#[test]
fn plain_workspace_turns_run_in_parallel_and_the_vault_root_waits() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let a = crate::workspaces::create(v, "Plain A", "Do it").unwrap();
    let b = crate::workspaces::create(v, "Plain B", "Do it too").unwrap();
    let m = seed_workflow(v, "Plain Lane Manual", "manual", "BODY");
    queue_manual(v, &m, now);
    let workflow = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch_slug(&workflow), Some(m.slug.as_str()));

    let send = |cwd: PathBuf| {
        let mut request = chat_request(v);
        request.cwd = cwd;
        chat::accept_turn(v, request, now).unwrap()
    };
    let sent_a = send(v.join(&a.path));
    let sent_b = send(v.join(&b.path));
    assert_eq!(
        sent_a.waiting_for, None,
        "a workspace turn waits for nobody"
    );
    let turn_a = admitted(run_step(v, now, None, &deps()));
    let turn_b = admitted(run_step(v, now, None, &deps()));
    assert!(turn_a.is_scoped() && turn_b.is_scoped());
    assert_eq!(
        [turn_a.row().id, turn_b.row().id],
        [sent_a.queue_id, sent_b.queue_id]
    );
    assert!(
        lease_holder(v).is_some_and(|holder| holder.is_workflow(&m.slug)),
        "the automation keeps the vault"
    );

    // A turn at the vault root is a full-vault turn and waits.
    let root = send(v.to_path_buf());
    assert!(
        !matches!(run_step(v, now, None, &deps()), PumpStep::Admitted(_)),
        "the root turn waits"
    );
    assert_eq!(row(v, root.queue_id).status, RowStatus::Queued);

    std::fs::write(v.join(&a.path).join("PLAN.md"), "plan, updated by A").unwrap();
    let run_a = finish_dispatch(v, turn_a, "success");
    assert_eq!(run_a.scope.as_deref(), Some(a.path.as_str()));
    assert_eq!(changed_paths(&run_a), [format!("{}/PLAN.md", a.path)]);
    finish_dispatch(v, turn_b, "success");
    let routine = finish_dispatch(v, workflow, "success");
    assert!(!changed_paths(&routine)
        .iter()
        .any(|p| p.starts_with("workspaces/")));
    let root_turn = admitted(run_step(v, now, None, &deps()));
    assert!(!root_turn.is_scoped(), "the vault root holds the vault");
    finish_dispatch(v, root_turn, "success");
}

/// 2026-10-07: several conversations in ONE ordinary workspace run at
/// once, Quick chats included, beside an automation holding the vault.
/// Before, a second turn in the same workspace waited for the first (a
/// chat waited 82 minutes behind a long turn in its workspace). The
/// app-wide limit still holds (lowered here to 4: the automation is one of
/// them since 2026-10-20), and a turn waiting for it isn't said to be
/// waiting for the automation: a workspace's turn never waits for the
/// vault, so its row names no vault holder.
#[test]
fn several_conversations_in_one_workspace_run_at_once() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 4,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let a = crate::workspaces::create(v, "Busy workspace", "Do it").unwrap();
    let quick = crate::workspaces::ensure_quick_chats(v).unwrap();
    let m = seed_workflow(v, "Same Lane Manual", "manual", "BODY");
    queue_manual(v, &m, now);
    let workflow = admitted(run_step(v, now, None, &deps()));

    let send = |cwd: PathBuf| {
        let mut request = chat_request(v);
        request.cwd = cwd;
        chat::accept_turn(v, request, now).unwrap()
    };
    let first = send(v.join(&a.path));
    let second = send(v.join(&a.path));
    let quick_one = send(quick.clone());
    assert_ne!(first.session_key, second.session_key);
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        turns.iter().map(|t| t.row().id).collect::<Vec<_>>(),
        [first.queue_id, second.queue_id, quick_one.queue_id],
        "both of the workspace's conversations and a quick chat start"
    );
    assert!(turns.iter().all(Dispatch::is_scoped));
    assert_eq!(automated_history::scoped_run_count(v), 3);

    // A fourth waits for the limit, not for the automation.
    let quick_two = send(quick.clone());
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    let holder = lease_holder(v).expect("the automation holds the vault");
    run_queue::start_waiting_episodes(v, &holder, now).unwrap();
    run_queue::touch_waiting(v, &holder, now).unwrap();
    let waiting = row(v, quick_two.queue_id);
    assert_eq!(waiting.status, RowStatus::Queued);
    assert!(
        waiting.wait.is_none(),
        "a workspace's turn doesn't wait for the vault holder: {:?}",
        waiting.wait
    );
    assert_eq!(
        reason_for(v, quick_two.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 4 chats at once (Settings → Chat → Chats at once)")
    );

    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, quick_two.queue_id);
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    finish_dispatch(v, workflow, "success");
    assert_eq!(automated_history::scoped_run_count(v), 0);
}

// ── One budget, two limits and the memory gate (2026-10-20) ────────────────

/// What a queued row's badge says it waits for, read like the app reads it
/// (`list_run_queue`, `chat_send`).
fn reason_for(vault: &Path, id: i64) -> Option<String> {
    let live = run_queue::live_rows(vault).unwrap();
    let facts = QueueFacts::read(vault, &live);
    let row = live
        .iter()
        .find(|r| r.id == id)
        .expect("a live row")
        .clone();
    waiting_reason(vault, &row, &facts, Utc::now())
}

fn send_in(vault: &Path, cwd: PathBuf, now: DateTime<Utc>) -> chat::AcceptedTurn {
    let mut request = chat_request(vault);
    request.cwd = cwd;
    chat::accept_turn(vault, request, now).unwrap()
}

fn ids_of(turns: &[Dispatch]) -> Vec<i64> {
    turns.iter().map(|t| t.row().id).collect()
}

/// 2026-10-20: with the default limits, five conversations in one
/// workspace's folder run at once and a sixth waits for them, saying so,
/// while another workspace's turn still starts beside them.
#[test]
fn a_sixth_turn_in_one_workspace_waits_while_five_run() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let a = crate::workspaces::create(v, "Five at once", "Do it").unwrap();
    let b = crate::workspaces::create(v, "Next door", "Do it").unwrap();
    let sent: Vec<_> = (0..6).map(|_| send_in(v, v.join(&a.path), now)).collect();
    let turns: Vec<Dispatch> = (0..5)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        ids_of(&turns),
        sent[..5].iter().map(|s| s.queue_id).collect::<Vec<_>>()
    );
    let other = send_in(v, v.join(&b.path), now);
    let beside = admitted(run_step(v, now, None, &deps()));
    assert_eq!(beside.row().id, other.queue_id, "another workspace starts");
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    assert_eq!(row(v, sent[5].queue_id).status, RowStatus::Queued);
    assert_eq!(
        reason_for(v, sent[5].queue_id).as_deref(),
        Some("a free chat slot: this workspace is at its limit of 5 chats at once (Settings → Chat → Chats at once)")
    );

    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let sixth = admitted(run_step(v, now, None, &deps()));
    assert_eq!(sixth.row().id, sent[5].queue_id);
    for turn in turns.chain([beside, sixth]) {
        finish_dispatch(v, turn, "success");
    }
}

/// 2026-10-20: ten turns run at once across workspaces; an eleventh waits
/// for the app-wide limit, and so does a vault-lane row (an automation is
/// an agent turn too).
#[test]
fn an_eleventh_turn_app_wide_waits_while_ten_run() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let mut sent = Vec::new();
    for i in 0..5 {
        let w = crate::workspaces::create(v, &format!("Ten wide {i}"), "Do it").unwrap();
        sent.push(send_in(v, v.join(&w.path), now));
        sent.push(send_in(v, v.join(&w.path), now));
    }
    let turns: Vec<Dispatch> = (0..10)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        ids_of(&turns),
        sent.iter().map(|s| s.queue_id).collect::<Vec<_>>()
    );
    let last = crate::workspaces::create(v, "Ten wide last", "Do it").unwrap();
    let eleventh = send_in(v, v.join(&last.path), now);
    let m = seed_workflow(v, "Ten Wide Manual", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    assert_eq!(row(v, eleventh.queue_id).status, RowStatus::Queued);
    assert_eq!(row(v, manual).status, RowStatus::Queued);
    assert_eq!(
        reason_for(v, manual).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)"),
        "the vault lane waits for the same limit"
    );
    assert_eq!(
        reason_for(v, eleventh.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)")
    );

    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    assert_eq!(
        reason_for(v, eleventh.queue_id),
        Some(format!("{} to start first", m.title)),
        "a Run now gets the free slot (fairness)"
    );
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, manual);
    assert_eq!(
        reason_for(v, eleventh.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)"),
        "ten again"
    );
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, eleventh.queue_id);
    finish_dispatch(v, next, "success");
}

/// 2026-10-20: the vault lane's run (here an automation) is one of the
/// ten, so nine workspace turns fit beside it and a tenth waits until it
/// ends.
#[test]
fn a_vault_lane_run_counts_toward_the_ten() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let m = seed_workflow(v, "Lane Counts Manual", "manual", "BODY");
    queue_manual(v, &m, now);
    let workflow = admitted(run_step(v, now, None, &deps()));
    assert_eq!(dispatch_slug(&workflow), Some(m.slug.as_str()));
    let mut sent = Vec::new();
    for i in 0..2 {
        let w = crate::workspaces::create(v, &format!("Beside lane {i}"), "Do it").unwrap();
        for _ in 0..5 {
            sent.push(send_in(v, v.join(&w.path), now));
        }
    }
    let turns: Vec<Dispatch> = (0..9)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert!(turns.iter().all(Dispatch::is_scoped));
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    assert_eq!(row(v, sent[9].queue_id).status, RowStatus::Queued);
    assert_eq!(
        reason_for(v, sent[9].queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)")
    );
    finish_dispatch(v, workflow, "success");
    let tenth = admitted(run_step(v, now, None, &deps()));
    assert_eq!(tenth.row().id, sent[9].queue_id);
    for turn in turns.into_iter().chain([tenth]) {
        finish_dispatch(v, turn, "success");
    }
}

/// 2026-10-20: a Main agent swarm's part is one of the chats running at
/// once (before, parts ran in slots of their own beside the person's
/// three), and the swarm's own setting still caps how many parts run.
#[test]
fn a_swarm_part_counts_toward_the_limit_and_keeps_the_swarms_own() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 3,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let a = crate::workspaces::create(v, "Mine", "Do it").unwrap();
    let b = crate::workspaces::create(v, "Swarm's", "Do it").unwrap();
    let c = crate::workspaces::create(v, "Also mine", "Do it").unwrap();
    let mine = [
        send_in(v, v.join(&a.path), now),
        send_in(v, v.join(&a.path), now),
    ];
    let part = send_in(v, v.join(&b.path), now);
    crate::main_agent::swarm_parts::add(v, &part.session_key);
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        ids_of(&turns),
        [mine[0].queue_id, mine[1].queue_id, part.queue_id]
    );
    let fourth = send_in(v, v.join(&c.path), now);
    assert!(
        !matches!(run_step(v, now, None, &deps()), PumpStep::Admitted(_)),
        "the part took one of the three"
    );
    assert_eq!(
        reason_for(v, fourth.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 3 chats at once (Settings → Chat → Chats at once)")
    );
    for turn in turns {
        finish_dispatch(v, turn, "success");
    }
    let fourth_turn = admitted(run_step(v, now, None, &deps()));
    finish_dispatch(v, fourth_turn, "success");

    // The swarm's own limit, under the app-wide ten: one part per workspace
    // so no workspace limit applies.
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits::default());
    let parts_at_once = crate::main_agent::settings().swarm_concurrency as usize;
    assert!(parts_at_once < 10, "room for a part and a chat beside them");
    let mut parts = Vec::new();
    for i in 0..=parts_at_once {
        let w = crate::workspaces::create(v, &format!("Part {i}"), "Do it").unwrap();
        let sent = send_in(v, v.join(&w.path), now);
        crate::main_agent::swarm_parts::add(v, &sent.session_key);
        parts.push(sent);
    }
    let running: Vec<Dispatch> = (0..parts_at_once)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    let mine = send_in(v, v.join(&a.path), now);
    let beside = admitted(run_step(v, now, None, &deps()));
    assert_eq!(beside.row().id, mine.queue_id, "a chat still starts");
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    let waiting = parts.last().unwrap().queue_id;
    assert_eq!(row(v, waiting).status, RowStatus::Queued);
    assert_eq!(
        reason_for(v, waiting),
        Some(format!("a free swarm slot: the swarm is at its limit of {parts_at_once} parts at once (Settings → Background work → Swarm parts at once)"))
    );
    for turn in running.into_iter().chain([beside]) {
        finish_dispatch(v, turn, "success");
    }
    let last = admitted(run_step(v, now, None, &deps()));
    assert_eq!(last.row().id, waiting);
    finish_dispatch(v, last, "success");
}

/// SE3: three bootstrap turns start under pressure, then admission retries
/// each second and records the level. Recovery immediately releases the hold.
#[test]
fn memory_pressure_allows_three_turns_then_rechecks_and_records_the_hold() {
    use crate::run_permit::{set_test_pressure, PressureSample};
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = crate::workspaces::create(v, "Under pressure", "Do it").unwrap();
    let chat = send_in(v, v.join(&w.path), now);
    let peers = [
        send_in(v, v.join(&w.path), now),
        send_in(v, v.join(&w.path), now),
    ];
    let m = seed_workflow(v, "Pressure Manual", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    let level = |level| PressureSample {
        vm_pressure_level: Some(level),
        ..PressureSample::default()
    };
    set_test_pressure(level(2));
    let turns: Vec<_> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        ids_of(&turns),
        [chat.queue_id, peers[0].queue_id, peers[1].queue_id]
    );
    let held = run_step(v, now, None, &deps());
    assert!(
        matches!(held, PumpStep::Sleep(at) if at == now + Duration::seconds(1)),
        "the fourth turn rechecks within one second"
    );
    assert_eq!(row(v, chat.queue_id).status, RowStatus::Running);
    assert_eq!(row(v, manual).status, RowStatus::Queued);
    assert_eq!(reason_for(v, manual).as_deref(), Some("memory"));
    let timings = row(v, manual).phase_timings.unwrap();
    assert_eq!(timings["memoryPressureLevel"], 2);
    assert!(timings["memoryHeldSince"].is_string());
    set_test_pressure(level(4));
    run_step(v, now + Duration::seconds(1), None, &deps());
    let later = row(v, manual).phase_timings.unwrap();
    assert_eq!(later["memoryPressureLevel"], 4);
    assert_eq!(later["memoryHeldSince"], timings["memoryHeldSince"]);

    set_test_pressure(level(1));
    assert_eq!(reason_for(v, manual), None);
    let fourth = admitted(run_step(v, now, None, &deps()));
    assert_eq!(fourth.row().id, manual);
    // A turn already running is untouched when pressure rises.
    set_test_pressure(level(4));
    assert_eq!(row(v, chat.queue_id).status, RowStatus::Running);
    for turn in turns.into_iter().chain([fourth]) {
        finish_dispatch(v, turn, "success");
    }
}

/// 2026-10-20: lowered Settings (3 at once, 2 in one workspace) hold at
/// their own numbers, and the badge quotes them.
#[test]
fn lowered_chat_limits_are_honoured() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 3,
        chats_per_workspace: 2,
        ..Default::default()
    });
    let a = crate::workspaces::create(v, "Lowered A", "Do it").unwrap();
    let b = crate::workspaces::create(v, "Lowered B", "Do it").unwrap();
    let in_a: Vec<_> = (0..3).map(|_| send_in(v, v.join(&a.path), now)).collect();
    let in_b: Vec<_> = (0..2).map(|_| send_in(v, v.join(&b.path), now)).collect();
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    assert_eq!(
        ids_of(&turns),
        [in_a[0].queue_id, in_a[1].queue_id, in_b[0].queue_id],
        "A's third waits for A; B's first goes past it"
    );
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    assert_eq!(
        reason_for(v, in_a[2].queue_id).as_deref(),
        Some("a free chat slot: this workspace is at its limit of 2 chats at once (Settings → Chat → Chats at once)")
    );
    assert_eq!(
        reason_for(v, in_b[1].queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 3 chats at once (Settings → Chat → Chats at once)")
    );
    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, in_a[2].queue_id);
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    let last = admitted(run_step(v, now, None, &deps()));
    assert_eq!(last.row().id, in_b[1].queue_id);
    finish_dispatch(v, last, "success");
}

#[test]
fn memory_headroom_reserves_live_turns_and_recovers_without_a_count_change() {
    let _locks = chat_locks();
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_HEADROOM.with(|cell| cell.set(None));
        }
    }
    let _reset = Reset;
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = crate::workspaces::create(v, "Headroom fixture", "Do it").unwrap();
    let first = send_in(v, v.join(&w.path), now);
    let second = send_in(v, v.join(&w.path), now);
    // Enough for the floor and one conservative turn, not two pending allocations.
    TEST_HEADROOM.with(|cell| cell.set(Some(MEMORY_FLOOR_BYTES + 250_000_000)));
    let running = admitted(run_step(v, now, None, &deps()));
    assert_eq!(running.row().id, first.queue_id);
    assert!(
        matches!(run_step(v, now, None, &deps()), PumpStep::Sleep(at)
        if at == now + Duration::seconds(1))
    );
    assert_eq!(
        reason_for(v, second.queue_id).as_deref(),
        Some("memory headroom")
    );
    TEST_HEADROOM.with(|cell| cell.set(Some(MEMORY_FLOOR_BYTES + 500_000_000)));
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, second.queue_id);
    for turn in [running, next] {
        finish_dispatch(v, turn, "success");
    }

    let third = send_in(v, v.join(&w.path), now);
    let mut candidate = row(v, third.queue_id);
    candidate.payload["agent"] = serde_json::json!("codex");
    let mut facts = QueueFacts::read(v, &run_queue::live_rows(v).unwrap());
    facts.available_memory = Some(MEMORY_FLOOR_BYTES + 180_000_000);
    assert!(facts.headroom_hold(&candidate).is_none());
    candidate.payload["agent"] = serde_json::json!("claude");
    assert_eq!(facts.headroom_hold(&candidate), Some(Hold::Headroom));
    facts.available_memory = None;
    assert!(
        facts.headroom_hold(&candidate).is_none(),
        "unknown sample falls back to saved limits"
    );
    facts.live_turns = facts.limits.chats_at_once as usize;
    facts.available_memory = Some(10_000_000_000);
    assert!(
        facts.hold(v, &candidate, false).is_none(),
        "headroom replaces the saved overall count"
    );
    facts.available_memory = None;
    assert!(matches!(
        facts.hold(v, &candidate, false),
        Some(Hold::AppLimit(_))
    ));
    facts.available_memory = Some(MEMORY_FLOOR_BYTES + 250_000_000);
    facts.lane_first = Some("waiting automation".into());
    assert!(
        matches!(facts.hold(v, &candidate, false), Some(Hold::LaneFirst(_))),
        "the last memory reservation belongs to the waiting vault lane"
    );
}

#[test]
fn memory_headroom_checks_the_candidate_selected_after_a_lease_fallback() {
    let _locks = chat_locks();
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_HEADROOM.with(|cell| cell.set(None));
        }
    }
    let _reset = Reset;
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = crate::workspaces::create(v, "Fallback headroom", "Do it").unwrap();
    let lease = automated_history::try_begin_owned(v, "ingest", Some("fixture")).unwrap();
    let mut request = chat_request(v);
    request.agent = crate::agent::AgentKind::Codex;
    let root = chat::accept_turn(v, request, now).unwrap();
    let scoped = send_in(v, v.join(&w.path), now);
    TEST_HEADROOM.with(|cell| cell.set(Some(MEMORY_FLOOR_BYTES + 200_000_000)));
    assert!(matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Sleep(_)
    ));
    assert_eq!(row(v, root.queue_id).status, RowStatus::Queued);
    assert_eq!(row(v, scoped.queue_id).status, RowStatus::Queued);
    assert_eq!(
        reason_for(v, scoped.queue_id).as_deref(),
        Some("memory headroom")
    );
    lease.finish("success").unwrap();
}

/// 2026-10-20 review: a Run now waiting for the app-wide limit gets the
/// next free slot, though chat rows go first in claim order: otherwise a
/// steady backlog of workspace chats would hold it back for good.
#[test]
fn a_run_now_gets_the_next_free_slot_before_waiting_workspace_chats() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 3,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let w = crate::workspaces::create(v, "Busy backlog", "Do it").unwrap();
    for _ in 0..3 {
        send_in(v, v.join(&w.path), now);
    }
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    let m = seed_workflow(v, "Fair Lane Manual", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    let backlog = send_in(v, v.join(&w.path), now);
    assert!(!matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Admitted(_)
    ));
    assert_eq!(
        reason_for(v, manual).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 3 chats at once (Settings → Chat → Chats at once)")
    );
    assert_eq!(
        reason_for(v, backlog.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 3 chats at once (Settings → Chat → Chats at once)"),
        "no slot is free yet"
    );

    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    assert_eq!(
        reason_for(v, backlog.queue_id),
        Some(format!("{} to start first", m.title))
    );
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, manual, "the automation, not the chat");
    assert_eq!(
        reason_for(v, backlog.queue_id).as_deref(),
        Some("a free chat slot: Hollow is at its limit of 3 chats at once (Settings → Chat → Chats at once)")
    );
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    let last = admitted(run_step(v, now, None, &deps()));
    assert_eq!(last.row().id, backlog.queue_id);
    finish_dispatch(v, last, "success");
}

/// 2026-10-20 review: the fairness is bounded the other way too. A
/// vault-root chat (not a Run now) yields to workspace chats ahead of it in
/// line for a minute, then gets the next free slot.
#[test]
fn a_vault_lane_row_waiting_a_minute_gets_the_next_free_slot() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 2,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let w = crate::workspaces::create(v, "Fair minute", "Do it").unwrap();
    for _ in 0..2 {
        send_in(v, v.join(&w.path), now);
    }
    let turns: Vec<Dispatch> = (0..2)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    let first = send_in(v, v.join(&w.path), now);
    let second = send_in(v, v.join(&w.path), now);
    let root = send_in(v, v.to_path_buf(), now);
    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(
        next.row().id,
        first.queue_id,
        "within the minute, line order"
    );

    let later = now + Duration::seconds(LANE_FAIRNESS_SECS + 1);
    finish_dispatch(v, turns.next().unwrap(), "success");
    let lane = admitted(run_step(v, later, None, &deps()));
    assert_eq!(lane.row().id, root.queue_id, "after it, the vault root's");
    assert!(!lane.is_scoped());
    finish_dispatch(v, next, "success");
    finish_dispatch(v, lane, "success");
    let last = admitted(run_step(v, later, None, &deps()));
    assert_eq!(last.row().id, second.queue_id);
    finish_dispatch(v, last, "success");
}

/// 2026-10-20 delta review: a lease wait episode is history, not a lease
/// held now. A Run now that once waited for Notion sync still gets the next
/// free slot once the lease is free.
#[test]
fn a_run_now_that_once_waited_for_a_lease_still_gets_the_next_free_slot() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 3,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let w = crate::workspaces::create(v, "Once waited", "Do it").unwrap();
    for _ in 0..3 {
        send_in(v, v.join(&w.path), now);
    }
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    let m = seed_workflow(v, "Once Waited Manual", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    run_queue::start_waiting_episodes(v, &LeaseHolder::TrackerSync, now).unwrap();
    assert!(row(v, manual).wait.is_some(), "an episode on record");
    let backlog = send_in(v, v.join(&w.path), now);
    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, manual, "the automation, not the chat");
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    let last = admitted(run_step(v, now, None, &deps()));
    assert_eq!(last.row().id, backlog.queue_id);
    finish_dispatch(v, last, "success");
}

/// 2026-10-20 delta review: while another writer holds the vault lease the
/// waiting automation can't start anyway, so the free slot goes to a
/// workspace chat at once instead of standing empty.
#[test]
fn a_held_lease_keeps_fairness_from_leaving_a_slot_empty() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 3,
        chats_per_workspace: 5,
        ..Default::default()
    });
    let w = crate::workspaces::create(v, "Lease held", "Do it").unwrap();
    for _ in 0..3 {
        send_in(v, v.join(&w.path), now);
    }
    let turns: Vec<Dispatch> = (0..3)
        .map(|_| admitted(run_step(v, now, None, &deps())))
        .collect();
    let m = seed_workflow(v, "Lease Held Manual", "manual", "BODY");
    let manual = queue_manual(v, &m, now);
    let backlog = send_in(v, v.join(&w.path), now);
    let ingest = automated_history::try_begin_owned(v, "ingest", Some("page")).unwrap();
    let mut turns = turns.into_iter();
    finish_dispatch(v, turns.next().unwrap(), "success");
    let next = admitted(run_step(v, now, None, &deps()));
    assert_eq!(next.row().id, backlog.queue_id, "the chat, at once");
    assert_eq!(row(v, manual).status, RowStatus::Queued);
    ingest.finish("success").unwrap();
    for turn in turns.chain([next]) {
        finish_dispatch(v, turn, "success");
    }
    let last = admitted(run_step(v, now, None, &deps()));
    assert_eq!(last.row().id, manual);
    finish_dispatch(v, last, "success");
}

/// 2026-10-20 delta review: a gap-bound row is ready only once the
/// automation gap opens, so its minute of waiting counts from the later of
/// its request, its retry time and the gap opening.
#[test]
fn direct_workflow_fairness_uses_shared_order_without_taking_extra_chat_capacity() {
    let _locks = chat_locks();
    for limit in [1, 2] {
        let dir = chat_vault();
        let v = dir.path();
        let now = Utc::now();
        crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
            chats_at_once: limit,
            chats_per_workspace: 5,
            ..Default::default()
        });
        let older = seed_workflow(v, "Order Older Unknown", "manual", "BODY");
        let older_id = queue_manual(v, &older, now);
        let direct = seed_workflow(v, "Order Direct Request", "manual", "BODY");
        let direct_id = run_queue::enqueue(
            v,
            NewRow::new(
                RunSource::Manual,
                Some(&direct.slug),
                &direct.title,
                "run now",
            )
            .with_origin(crate::work_origin::Request::person_workflow()),
            now,
        )
        .unwrap()
        .id();
        let workspace = crate::workspaces::create(v, "Order conversation", "Do it").unwrap();
        let person = typed_chat_in(v, v.join(&workspace.path), now);
        assert_eq!(
            lane_first(v, &run_queue::live_rows(v).unwrap(), now, None),
            Some(direct.title.clone())
        );
        let first = admitted(run_step(v, now, None, &deps()));
        if limit == 1 {
            assert_eq!(
                first.row().id,
                direct_id,
                "existing last-slot lane fairness names the direct request"
            );
            assert_eq!(row(v, person.queue_id).status, RowStatus::Queued);
            assert_eq!(row(v, older_id).status, RowStatus::Queued);
        } else {
            assert_eq!(
                first.row().id,
                person.queue_id,
                "ordinary chat keeps its earlier band when fairness does not claim the last slot"
            );
            assert_eq!(row(v, direct_id).status, RowStatus::Queued);
        }
        finish_dispatch(v, first, "success");
        // The test leaves no live turn or lease. Remaining requests are simply
        // withdrawn rather than starting more work to prove an ordering fact.
        for id in [older_id, direct_id, person.queue_id] {
            run_queue::cancel(v, id, now).unwrap();
        }
    }
}

#[test]
fn fairness_counts_a_gap_bound_rows_wait_from_when_the_gap_opened() {
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    run_queue::enqueue(
        v,
        NewRow::new(RunSource::Trigger, Some("watch"), "Watcher", "fired"),
        now - Duration::seconds(300),
    )
    .unwrap();
    let live = run_queue::live_rows(v).unwrap();
    assert!(live[0].priority >= run_queue::GAP_PRIORITY);
    assert_eq!(
        lane_first(v, &live, now, Some(now - Duration::seconds(10))),
        None,
        "ready for 10 s only"
    );
    assert_eq!(
        lane_first(
            v,
            &live,
            now,
            Some(now - Duration::seconds(LANE_FAIRNESS_SECS + 1))
        )
        .as_deref(),
        Some("Watcher")
    );
    assert_eq!(lane_first(v, &live, now, None).as_deref(), Some("Watcher"));
}

/// Merged plan slice 3 (A·R4): a scoped turn whose `mark_running` hits a
/// busy database is not started. Its run ends cancelled, the gate is
/// released, and the row is queued again for a later step, so it can
/// never be admitted beside a copy of itself.
#[test]
fn a_scoped_turn_whose_mark_running_fails_is_not_started() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let workspace = crate::workspaces::create(v, "Unmarked turn", "Do it").unwrap();
    let mut request = chat_request(v);
    request.cwd = v.join(&workspace.path);
    let sent = chat::accept_turn(v, request, now).unwrap();
    let flaky = StepDeps {
        fail_mark_running: true,
        ..deps()
    };
    let (row_id, event, events) = handled_chat(run_step(v, now, None, &flaky));
    assert_eq!((row_id, event), (sent.queue_id, Some("requeued")));
    assert!(events.is_empty(), "the turn neither started nor ended");
    assert_eq!(row(v, sent.queue_id).status, RowStatus::Queued);
    assert!(chat::active_chat_rows(v).is_empty(), "the gate is released");
    assert_eq!(automated_history::scoped_run_count(v), 0);
    let runs = automated_history::list_runs(v, 10).unwrap();
    let chat_run = runs
        .iter()
        .find(|run| run.source == "chat")
        .expect("the turn's run");
    assert_eq!(chat_run.status, "cancelled", "ended unstarted");
    assert!(
        !matches!(run_step(v, now, None, &deps()), PumpStep::Admitted(_)),
        "not again this minute"
    );

    let later = now + Duration::seconds(RETRY_DEFER_SECS + 1);
    let turn = admitted(run_step(v, later, None, &deps()));
    assert!(turn.is_scoped());
    assert_eq!(turn.row().id, sent.queue_id, "it runs once, later");
    assert_eq!(row(v, sent.queue_id).attempt, 1);
    finish_dispatch(v, turn, "success");
    assert_eq!(row(v, sent.queue_id).status, RowStatus::Done);
}

/// A·R4: a chat row whose turn is active is never released as a stray
/// claim, so it is never claimed and admitted a second time.
#[test]
fn an_active_chat_claim_is_never_queued_again() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let workspace = crate::workspaces::create(v, "Active claim", "Do it").unwrap();
    let mut request = chat_request(v);
    request.cwd = v.join(&workspace.path);
    let sent = chat::accept_turn(v, request, now).unwrap();
    // Claimed with its turn live (the gate holds it), as when a
    // follow-up write after the claim failed.
    run_queue::claim(v, sent.queue_id, now)
        .unwrap()
        .expect("claimed");
    let live = ChatRowGuard::hold(v, sent.queue_id).expect("a free id is held");
    assert!(
        !matches!(run_step(v, now, None, &deps()), PumpStep::Admitted(_)),
        "an active row is never admitted twice"
    );
    assert_eq!(
        row(v, sent.queue_id).status,
        RowStatus::Claimed,
        "an active claim is not queued again"
    );
    drop(live);
    let turn = admitted(run_step(v, now, None, &deps()));
    assert_eq!(
        turn.row().id,
        sent.queue_id,
        "released once it is not active"
    );
    finish_dispatch(v, turn, "success");
}

/// A [`PumpHost`] that drives the production loop ([`drain`]): the real
/// step with the test deps and no automation gap, and an executor that
/// records each dispatch it starts, holds every vault-lane run until the
/// test releases it (then finishes it, or panics when told to), and
/// finishes a scoped turn at once.
struct LoopProbe {
    vault: PathBuf,
    started: Mutex<Vec<i64>>,
    release: Arc<tokio::sync::Semaphore>,
    panic_lane: AtomicBool,
    panic_scoped: AtomicBool,
    hold_scoped: AtomicBool,
    scoped_release: Arc<tokio::sync::Semaphore>,
    scoped_entered: Arc<tokio::sync::Notify>,
}

impl LoopProbe {
    fn new(vault: &Path) -> Arc<Self> {
        Arc::new(LoopProbe {
            vault: vault.to_path_buf(),
            started: Mutex::new(Vec::new()),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
            panic_lane: AtomicBool::new(false),
            panic_scoped: AtomicBool::new(false),
            hold_scoped: AtomicBool::new(false),
            scoped_release: Arc::new(tokio::sync::Semaphore::new(0)),
            scoped_entered: Arc::new(tokio::sync::Notify::new()),
        })
    }

    fn started(&self) -> Vec<i64> {
        self.started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl PumpHost for LoopProbe {
    fn vault(&self) -> &Path {
        &self.vault
    }

    fn step(
        &self,
        _gap: Option<DateTime<Utc>>,
        pending: &mut Vec<PendingFinish>,
        lane: Option<i64>,
    ) -> PumpStep {
        step_beside(&self.vault, Utc::now(), None, pending, &deps(), lane)
    }

    fn execute(&self, dispatch: Dispatch) -> RunFuture {
        self.started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(dispatch.row().id);
        let lane = !dispatch.is_scoped();
        let blow_up = if lane {
            self.panic_lane.swap(false, Ordering::SeqCst)
        } else {
            self.panic_scoped.swap(false, Ordering::SeqCst)
        };
        let hold_scoped = !lane && self.hold_scoped.swap(false, Ordering::SeqCst);
        let (vault, release, scoped_release, scoped_entered) = (
            self.vault.clone(),
            Arc::clone(&self.release),
            Arc::clone(&self.scoped_release),
            Arc::clone(&self.scoped_entered),
        );
        Box::pin(async move {
            if lane {
                let permit = release.acquire().await.expect("gate open");
                permit.forget();
                if blow_up {
                    panic!("a vault-lane run panicked (test)");
                }
            } else {
                if hold_scoped {
                    scoped_entered.notify_one();
                    let permit = scoped_release.acquire().await.expect("scoped gate open");
                    permit.forget();
                }
                if blow_up {
                    panic!("a scoped chat turn panicked (test)");
                }
            }
            tauri::async_runtime::spawn_blocking(move || {
                finish_dispatch(&vault, dispatch, "success");
            })
            .await
            .expect("the run finishes");
            Vec::new()
        })
    }

    fn emit_row(&self, _kind: &str, _id: i64) {}

    fn emit_chat(&self, _events: Vec<Value>) {}

    fn flush_notices(&self, notices: &mut Vec<SkipNotice>) {
        notices.clear();
    }

    fn gap(&self, _own: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        None
    }
}

/// Poll (nudging the loop) until `done`, or fail naming `what`.
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        run_queue::wake();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Merged plan slice 3 (A-R1, B-F09), through the production loop: while
/// a vault-lane run is live, a workspace's chat turn starts and a second
/// vault-lane row does not; the second runs once the first ends; and a
/// stop waits for the live vault-lane run before the loop returns.
#[test]
fn the_loop_starts_workspace_turns_while_a_vault_lane_run_is_live() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let workspace = crate::workspaces::create(v, "Loop Lane Space", "Do it").unwrap();
    let first = seed_workflow(v, "Loop Lane First", "manual", "BODY");
    let second = seed_workflow(v, "Loop Lane Second", "manual", "BODY");
    let first_row = queue_manual(v, &first, now);
    let second_row = queue_manual(v, &second, now);
    let probe = LoopProbe::new(v);
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));

    wait_until("the first run", || probe.started() == [first_row]);
    assert!(
        lease_holder(v).is_some_and(|holder| holder.is_workflow(&first.slug)),
        "the first run holds the vault"
    );

    let mut request = chat_request(v);
    request.cwd = v.join(&workspace.path);
    let turn = chat::accept_turn(v, request, Utc::now()).unwrap();
    wait_until("the workspace turn to start beside the run", || {
        probe.started().contains(&turn.queue_id)
    });
    wait_until("the workspace turn to finish", || {
        row(v, turn.queue_id).status == RowStatus::Done
    });
    // More steps while the first run is still live: the second row stays
    // queued, with no wait episode behind the loop's own lane.
    for _ in 0..10 {
        run_queue::wake();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(probe.started(), [first_row, turn.queue_id]);
    let waiting = row(v, second_row);
    assert_eq!(waiting.status, RowStatus::Queued);
    assert!(waiting.wait.is_none(), "{:?}", waiting.wait);
    assert_eq!(row(v, first_row).status, RowStatus::Running);

    probe.release.add_permits(1);
    wait_until("the second run to start once the first ends", || {
        probe.started() == [first_row, turn.queue_id, second_row]
    });
    assert_eq!(row(v, first_row).status, RowStatus::Done);
    assert!(
        lease_holder(v).is_some_and(|holder| holder.is_workflow(&second.slug)),
        "the second run holds the vault"
    );

    stop.request();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !pump.inner().is_finished(),
        "a stop waits for the live vault-lane run"
    );
    assert_eq!(row(v, second_row).status, RowStatus::Running);
    probe.release.add_permits(1);
    tauri::async_runtime::block_on(pump).expect("the loop ends cleanly");
    assert_eq!(row(v, second_row).status, RowStatus::Done);
    assert!(lease_holder(v).is_none());
}

/// Merged plan slice 3: a vault-lane run whose task panics is observed by
/// the loop. Its row fails instead of staying `running`, the lane and the
/// lease are free, and the next vault-lane row runs.
#[test]
fn a_panicked_vault_lane_run_fails_its_row_and_frees_the_lane() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let first = seed_workflow(v, "Loop Panic First", "manual", "BODY");
    let second = seed_workflow(v, "Loop Panic Second", "manual", "BODY");
    let first_row = queue_manual(v, &first, now);
    let second_row = queue_manual(v, &second, now);
    let probe = LoopProbe::new(v);
    probe.panic_lane.store(true, Ordering::SeqCst);
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));

    wait_until("the first run", || probe.started() == [first_row]);
    probe.release.add_permits(1);
    wait_until("the next row to start after the panic", || {
        probe.started() == [first_row, second_row]
    });
    let failed = row(v, first_row);
    assert_eq!(failed.status, RowStatus::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("stopped unexpectedly")),
        "{:?}",
        failed.error
    );
    assert!(
        lease_holder(v).is_some_and(|holder| holder.is_workflow(&second.slug)),
        "the panicked run released the vault"
    );

    stop.request();
    probe.release.add_permits(1);
    tauri::async_runtime::block_on(pump).expect("the loop ends cleanly");
    assert_eq!(row(v, second_row).status, RowStatus::Done);
    assert!(lease_holder(v).is_none());
}

/// Supervised scoped chat turns: a panicked scoped turn is observed by the
/// loop's JoinSet, marked `failed` via `end_row` so neither its row nor its
/// workspace slot leaks, and shutdown drains live scoped turns before exit.
#[test]
fn a_panicked_scoped_turn_fails_its_row_and_frees_capacity_and_stop_drains_live_scoped_turns() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    crate::chat_limits::set_test_limits(crate::chat_limits::ChatLimits {
        chats_at_once: 4,
        chats_per_workspace: 1,
        ..Default::default()
    });
    let workspace = crate::workspaces::create(v, "Scoped Panic Space", "Do it").unwrap();
    let probe = LoopProbe::new(v);
    probe.panic_scoped.store(true, Ordering::SeqCst);
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));

    let first = send_in(v, v.join(&workspace.path), Utc::now());
    wait_until("the panicked scoped turn to be marked failed", || {
        row(v, first.queue_id).status == RowStatus::Failed
    });
    let failed = row(v, first.queue_id);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("stopped unexpectedly")),
        "{:?}",
        failed.error
    );
    assert_eq!(automated_history::scoped_run_count(v), 0);
    assert!(chat::active_chat_rows(v).is_empty());

    // Capacity in the same workspace is free: a second scoped turn starts,
    // and stop waits until it finishes.
    probe.hold_scoped.store(true, Ordering::SeqCst);
    let second = send_in(v, v.join(&workspace.path), Utc::now());
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            probe.scoped_entered.notified(),
        )
        .await
        .expect("second scoped turn started");
    });
    assert_eq!(row(v, second.queue_id).status, RowStatus::Running);
    stop.request();
    assert!(
        !pump.inner().is_finished(),
        "stop drains live scoped turns before returning"
    );
    probe.scoped_release.add_permits(1);
    tauri::async_runtime::block_on(pump).expect("the loop ends cleanly");
    assert_eq!(row(v, second.queue_id).status, RowStatus::Done);
    assert_eq!(automated_history::scoped_run_count(v), 0);
}

/// A [`PumpHost`] whose every step finds the vault busy (as behind Notion
/// sync) and records when it ran. It has its own enqueue and lease-release
/// notifies, so no other test's enqueue or release wakes its loop.
struct BusyProbe {
    vault: PathBuf,
    idle: bool,
    stepped: Mutex<Vec<std::time::Instant>>,
    queue: tokio::sync::Notify,
    lease: tokio::sync::Notify,
}

impl BusyProbe {
    fn new() -> Arc<Self> {
        Arc::new(BusyProbe {
            vault: std::env::temp_dir(),
            idle: false,
            stepped: Mutex::new(Vec::new()),
            queue: tokio::sync::Notify::new(),
            lease: tokio::sync::Notify::new(),
        })
    }

    fn steps(&self) -> Vec<std::time::Instant> {
        self.stepped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The step times once there are at least `count`, or after 20 s
    /// (never a panic, so a test stops its loop before it asserts).
    fn wait_for_steps(&self, count: usize) -> Vec<std::time::Instant> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let steps = self.steps();
            if steps.len() >= count || std::time::Instant::now() >= deadline {
                return steps;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

impl PumpHost for BusyProbe {
    fn vault(&self) -> &Path {
        &self.vault
    }

    fn step(
        &self,
        _gap: Option<DateTime<Utc>>,
        _pending: &mut Vec<PendingFinish>,
        _lane: Option<i64>,
    ) -> PumpStep {
        self.stepped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(std::time::Instant::now());
        if self.idle {
            return PumpStep::Idle;
        }
        PumpStep::Busy {
            holder: LeaseHolder::TrackerSync,
            episodes_started: Vec::new(),
            failed: Vec::new(),
            chat: Vec::new(),
        }
    }

    fn execute(&self, _dispatch: Dispatch) -> RunFuture {
        Box::pin(std::future::ready(Vec::<PendingFinish>::new()))
    }

    fn emit_row(&self, _kind: &str, _id: i64) {}

    fn emit_chat(&self, _events: Vec<Value>) {}

    fn flush_notices(&self, notices: &mut Vec<SkipNotice>) {
        notices.clear();
    }

    fn gap(&self, _own: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        None
    }

    fn queue_woken(&self) -> &tokio::sync::Notify {
        &self.queue
    }

    fn lease_released(&self) -> &tokio::sync::Notify {
        &self.lease
    }
}

#[test]
fn a_queued_row_rechecks_an_idle_pump_without_any_wake() {
    let dir = temp_vault();
    let id = run_queue::enqueue(
        dir.path(),
        NewRow::new(
            RunSource::Manual,
            Some("idle-fixture"),
            "Idle fixture",
            "test",
        ),
        Utc::now(),
    )
    .unwrap()
    .id();
    let probe = Arc::new(BusyProbe {
        vault: dir.path().into(),
        idle: true,
        stepped: Mutex::new(Vec::new()),
        queue: tokio::sync::Notify::new(),
        lease: tokio::sync::Notify::new(),
    });
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));
    // The private Notify deliberately never receives the enqueue or end wake.
    let steps = probe.wait_for_steps(2);
    stop.request();
    tauri::async_runtime::block_on(pump).unwrap();
    assert!(
        steps.len() >= 2,
        "a queued row must not sleep for the 60 second idle poll"
    );
    let elapsed = steps[1].duration_since(steps[0]);
    assert!(elapsed >= QUEUED_POLL, "the recovery poll must not spin");
    assert!(
        elapsed < IDLE_POLL,
        "the missed wake recovers before the idle poll"
    );
    run_queue::finish(dir.path(), id, RowStatus::Done, None, Utc::now()).unwrap();
    assert!(!run_queue::has_queued_rows(dir.path()).unwrap());
}

/// S14: a holder that releases and retakes the lease every ~10 ms (Notion
/// sync, page by page; a queued chat logged 379 waits in 3.6 s behind it)
/// steps the busy pump at most once per [`LEASE_WAKE_MIN`], and still
/// sooner than [`BUSY_POLL`].
#[test]
fn lease_releases_step_a_busy_pump_at_most_once_per_lease_wake_min() {
    let probe = BusyProbe::new();
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));
    probe.wait_for_steps(1);

    // Churn until the pump has stepped three times, so the lower bound holds
    // however starved this test's threads are (R13, 2026-11-15: a fixed 1 s
    // window counted steps and could see fewer than three on a loaded gate);
    // never less than 1 s, so the upper bound still judges real churn.
    let began = std::time::Instant::now();
    let deadline = began + std::time::Duration::from_secs(30);
    while began.elapsed() < std::time::Duration::from_secs(1)
        || (probe.steps().len() < 3 && std::time::Instant::now() < deadline)
    {
        // As `notify_lease_released` does.
        probe.lease.notify_one();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let churned = began.elapsed();
    let steps = probe.steps();
    stop.request();
    tauri::async_runtime::block_on(pump).expect("the loop ends cleanly");

    // Undebounced, every release stepped it: ~100 steps per second here.
    // Debounced, one step per LEASE_WAKE_MIN, plus the first and one of slack.
    let most = (churned.as_millis() / LEASE_WAKE_MIN.as_millis()) as usize + 2;
    assert!(
        steps.len() <= most,
        "{} steps in {churned:?} of lease churn (at most {most})",
        steps.len()
    );
    assert!(
        steps.len() >= 3,
        "lease releases still step it before BUSY_POLL: {} steps",
        steps.len()
    );
    for pair in steps.windows(2) {
        let gap = pair[1].duration_since(pair[0]);
        assert!(gap >= LEASE_WAKE_MIN, "two steps {gap:?} apart");
    }
}

/// S14: a lease release just after a busy step waits out the debounce,
/// and during the next debounce an enqueue (a new or cancelled row) steps
/// the busy pump at once rather than after [`BUSY_POLL`].
#[test]
fn an_enqueue_during_the_lease_debounce_steps_a_busy_pump_at_once() {
    let probe = BusyProbe::new();
    let stop = Arc::new(PumpStop::default());
    let pump = tauri::async_runtime::spawn(drain(Arc::clone(&probe), Arc::clone(&stop)));
    probe.wait_for_steps(1);
    // As `notify_lease_released` does: the second step waits for the
    // debounce, and consumes the release.
    probe.lease.notify_one();
    probe.wait_for_steps(2);
    let woke = std::time::Instant::now();
    // As `run_queue::wake` does. With no release pending, a missed enqueue
    // would leave the pump waiting until BUSY_POLL.
    probe.queue.notify_one();
    let steps = probe.wait_for_steps(3);
    stop.request();
    tauri::async_runtime::block_on(pump).expect("the loop ends cleanly");

    assert!(steps.len() >= 3, "{} steps", steps.len());
    let held = steps[1].duration_since(steps[0]);
    assert!(
        held >= LEASE_WAKE_MIN,
        "a release stepped it {held:?} after a step"
    );
    let late = woke.duration_since(steps[1]);
    if late >= LEASE_WAKE_MIN {
        // A loaded run sent the enqueue after the debounce, where a queue
        // wake always acted: nothing here tests the debounce.
        eprintln!("skipped: the enqueue came {late:?} after the step");
        return;
    }
    let delay = steps[2].saturating_duration_since(woke);
    assert!(
        delay < std::time::Duration::from_secs(2),
        "the enqueue stepped it {delay:?} later"
    );
}

/// Build programme PR B4: two build workspaces' turns run at once, and a
/// second conversation in a busy workspace waits for it (without
/// blocking anything else) until its turn ends.
#[test]
fn build_workspace_turns_run_in_parallel_one_per_workspace() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let mut trees = Vec::new();
    let mut spaces = Vec::new();
    for name in ["Parallel A", "Parallel B"] {
        let workspace = crate::workspaces::create(v, name, "Ship it").unwrap();
        let tree = tempfile::Builder::new()
            .prefix("worktree-")
            .tempdir()
            .unwrap();
        crate::build_workspaces::bind_for_test(v, &workspace.id, tree.path());
        spaces.push(workspace);
        trees.push(tree);
    }
    let send = |tree: &Path| {
        let mut request = chat_request(v);
        request.cwd = tree.to_path_buf();
        chat::accept_turn(v, request, now).unwrap()
    };
    let a = send(trees[0].path());
    let b = send(trees[1].path());
    let turn_a = admitted(run_step(v, now, None, &deps()));
    let turn_b = admitted(run_step(v, now, None, &deps()));
    assert!(turn_a.is_scoped() && turn_b.is_scoped());
    assert_eq!([turn_a.row().id, turn_b.row().id], [a.queue_id, b.queue_id]);
    assert_eq!(automated_history::scoped_run_count(v), 2);

    // Another conversation in workspace A waits for A's turn.
    let a2 = send(trees[0].path());
    assert_ne!(a2.session_key, a.session_key);
    assert!(
        matches!(run_step(v, now, None, &deps()), PumpStep::Idle),
        "nothing else can start"
    );
    assert_eq!(row(v, a2.queue_id).status, RowStatus::Queued);
    finish_dispatch(v, turn_a, "success");
    let turn_a2 = admitted(run_step(v, now, None, &deps()));
    assert_eq!(turn_a2.row().id, a2.queue_id);
    finish_dispatch(v, turn_a2, "success");
    finish_dispatch(v, turn_b, "success");
    assert_eq!(automated_history::scoped_run_count(v), 0);
}

/// Refresh note 4.1: chat admission goes through `try_begin_owned` →
/// `try_begin_run`, so the turn is tracked; an admitted turn that is
/// dropped unexecuted still finalizes through the public `finish_run`,
/// with its flags and ONE notification, and releases everything.
#[test]
fn a_pump_admitted_chat_turn_is_tracked_and_a_dropped_turn_still_flags() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    std::fs::create_dir_all(v.join("me")).unwrap();
    std::fs::write(v.join("me/Guardrails.md"), "# Guardrails\n\n- Ask first.\n").unwrap();
    let accepted = queue_chat(v, now);
    let (row_id, turn, guard) = match admitted(run_step(v, now, None, &deps())) {
        Dispatch::Chat { row, turn, guard } => (row.id, turn, guard),
        _ => panic!("the chat turn is admitted"),
    };
    assert_eq!(row_id, accepted.queue_id);
    assert!(
        crate::run_guard::is_tracking(v),
        "the pump did not lose PR 5's tracking"
    );
    let run_id = turn.run_id().to_string();
    std::fs::write(
        v.join("me/Guardrails.md"),
        "# Guardrails\n\n- Anything goes.\n",
    )
    .unwrap();
    drop(turn);

    let run = crate::automated_history::get_run(v, &run_id).unwrap();
    assert_eq!(run.status, "cancelled");
    assert!(
        run.flags
            .iter()
            .any(|flag| flag.kind == "operatorProfile" && flag.path == "me/Guardrails.md"),
        "{:?}",
        run.flags
    );
    let sent = crate::run_guard::sent_for_test(v);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].0.starts_with("chat"), "{sent:?}");
    assert!(!crate::run_guard::is_tracking(v));
    assert!(lease_holder(v).is_none());
    assert!(
        crate::agent_sessions::live().is_none(),
        "no live record is left behind"
    );
    run_queue::finish(v, row_id, RowStatus::Cancelled, None, now).unwrap();
    drop(guard);
}

/// A cancel that lands while the pump holds the claimed row (Edit or
/// Discard in that window) ends the turn before it starts: the lease is
/// released, no `runStarted` or turn row exists, and the step returns the
/// `cancelled{queued}` and `done` events.
#[test]
fn a_cancel_while_claimed_ends_the_turn_before_it_starts() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    let key = accepted.session_key.clone();
    let cancel_in_window = |vault: &Path| {
        let withdrawn = chat::withdraw_queued(vault, &key, Utc::now()).unwrap();
        assert!(
            matches!(withdrawn, chat::Withdrawn::StopRequested { .. }),
            "{withdrawn:?}"
        );
    };
    let racing = StepDeps {
        before_admit: &cancel_in_window,
        ..deps()
    };
    let (row_id, event, events) = handled_chat(run_step(v, now, None, &racing));
    assert_eq!(row_id, accepted.queue_id);
    assert_eq!(event, Some("cancelled"));
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["type"], "cancelled");
    assert_eq!(events[0]["queued"], true);
    assert_eq!(events[0]["clientTurnId"], "ct-pump");
    assert_eq!(events[1]["type"], "done");
    let ended = row(v, accepted.queue_id);
    assert_eq!(ended.status, RowStatus::Cancelled);
    assert_eq!(ended.detail, None);
    assert!(lease_holder(v).is_none(), "the lease is released");
    assert!(crate::agent_sessions::live().is_none());
    let history =
        crate::automated_history::get_run(v, ended.history_run_id.as_deref().unwrap()).unwrap();
    assert_eq!(history.status, "cancelled");
    assert!(
        crate::session_store::read_conversation(v, &key).is_err()
            || crate::session_store::read_conversation(v, &key)
                .unwrap()
                .turns
                .is_empty(),
        "a turn that never started has no turn row"
    );
    // The gate was released with the step: the next send is accepted.
    assert!(chat::accept_turn(v, chat_request(v), now).is_ok());
}

/// The `cancelled{queued}` and `done` pair a turn ended before it started
/// sends to the webview.
fn assert_ended_unstarted(events: &[Value]) {
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["type"], "cancelled");
    assert_eq!(events[0]["queued"], true);
    assert_eq!(events[1]["type"], "done");
}

/// C-S3 (11-14, "Stop always stops"): a Stop that lands while the chat row
/// is claimed, when the turn then loses admission to another run, ends the
/// turn as cancelled. It used to go back to `queued` with the flag set and
/// wait behind that run, so the chat stayed queued and busy.
#[test]
fn a_cancel_while_claimed_ends_the_turn_when_admission_is_then_lost() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    let key = accepted.session_key.clone();
    let blocker = RefCell::new(None);
    let cancel_then_lose = |vault: &Path| {
        let withdrawn = chat::withdraw_queued(vault, &key, Utc::now()).unwrap();
        assert!(
            matches!(withdrawn, chat::Withdrawn::StopRequested { .. }),
            "{withdrawn:?}"
        );
        *blocker.borrow_mut() = Some(begin_run(vault, "workflow", Some("blocker")).unwrap());
    };
    let racing = StepDeps {
        before_admit: &cancel_then_lose,
        ..deps()
    };
    let (row_id, event, events) = handled_chat(run_step(v, now, None, &racing));
    assert_eq!(row_id, accepted.queue_id);
    assert_eq!(event, Some("cancelled"), "not requeued behind the blocker");
    assert_ended_unstarted(&events);
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Cancelled);
    finish_run(v, blocker.borrow_mut().take().unwrap(), "success").unwrap();
}

/// C-S3: a chat row that went back to `queued` carrying a Stop (a lost
/// admission race before this fix, a failed write, a stray-claim release)
/// ends at the next step even while another run holds the vault: the
/// advisory lease check, the holds and its not-before never keep it waiting.
#[test]
fn a_stopped_chat_row_back_in_the_queue_ends_without_waiting_for_the_holder() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    let id = accepted.queue_id;
    let blocker = begin_run(v, "workflow", Some("blocker")).unwrap();
    let holder = lease_holder(v).expect("the blocker holds the vault");
    run_queue::claim(v, id, now).unwrap().expect("claimed");
    assert_eq!(
        run_queue::cancel(v, id, now).unwrap(),
        run_queue::CancelOutcome::StopRequested
    );
    run_queue::requeue_waiting(v, id, &holder, now).unwrap();
    run_queue::defer(v, id, now + Duration::minutes(5), None, false, None).unwrap();
    let queued = row(v, id);
    assert_eq!(queued.status, RowStatus::Queued);
    assert!(queued.cancel_requested);

    let (row_id, event, events) = handled_chat(run_step(v, now, None, &deps()));
    assert_eq!(row_id, id);
    assert_eq!(event, Some("cancelled"));
    assert_ended_unstarted(&events);
    assert_eq!(row(v, id).status, RowStatus::Cancelled);
    finish_run(v, blocker, "success").unwrap();
}

/// Refresh note 4.4: a live run of this process whose agent forged its own
/// manifest (the lease then reads as stale) is still a live holder, so a
/// chat send queues instead of failing with the restart hint, and the
/// queued turn is admitted once that run finishes.
#[test]
fn a_live_tracked_run_with_a_forged_manifest_does_not_fail_a_chat_send() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let handle = automated_history::try_begin_run(v, "workflow", Some("forged")).unwrap();
    let manifest = v.join(format!(".hollow-history/runs/{}.json", handle.id));
    let mut json: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    json["run"]["finishedAt"] = Value::String("2026-09-25T00:00:00Z".into());
    json["run"]["status"] = Value::String("success".into());
    std::fs::write(&manifest, serde_json::to_string(&json).unwrap()).unwrap();
    assert!(matches!(lease_holder(v), Some(LeaseHolder::Stale { .. })));
    assert!(
        refuse_interactive(lease_holder(v).as_ref()).is_some(),
        "it reads as stale"
    );
    assert!(
        refuse_interactive_in(v, lease_holder(v).as_ref()).is_none(),
        "but it is live here"
    );

    let accepted = queue_chat(v, now);
    match run_step(v, now, None, &deps()) {
        PumpStep::Busy { failed, chat, .. } => {
            assert!(
                failed.is_empty() && chat.is_empty(),
                "the chat turn keeps waiting"
            );
        }
        _ => panic!("the forged run still holds the vault"),
    }
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Queued);
    // Finishing a run whose manifest was forged reports an error but still
    // releases the lease (PR 5).
    let _ = automated_history::finish_run(v, handle, "success");
    assert!(lease_holder(v).is_none());
    let turn = admitted(run_step(v, now, None, &deps()));
    assert!(matches!(turn, Dispatch::Chat { .. }));
    finish_dispatch(v, turn, "success");
}

/// The chat-row gate is taken BEFORE the claim: a send that lands between
/// the claim and the verdict is refused as a second live turn and never
/// repairs the claimed row to `interrupted`, and the step still admits it.
#[test]
fn a_send_racing_the_claim_of_a_chat_row_is_refused_and_never_repairs_it() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    let id = accepted.queue_id;
    let raced = std::cell::Cell::new(false);
    let send_in_window = |vault: &Path| {
        assert_eq!(row(vault, id).status, RowStatus::Claimed);
        let mut same = chat_request(vault);
        same.session_key = Some(accepted.session_key.clone());
        let refused = chat::accept_turn(vault, same, Utc::now()).unwrap_err();
        assert_eq!(refused, chat::CHAT_ALREADY_QUEUED);
        assert_eq!(
            row(vault, id).status,
            RowStatus::Claimed,
            "never repaired to interrupted"
        );
        raced.set(true);
    };
    let racing = StepDeps {
        after_claim: &send_in_window,
        ..deps()
    };
    let turn = admitted(run_step(v, now, None, &racing));
    assert!(
        raced.get(),
        "the hook ran between the claim and the verdict"
    );
    assert!(matches!(turn, Dispatch::Chat { .. }));
    assert_eq!(turn.row().id, id);
    finish_dispatch(v, turn, "success");
}

/// Through the pump, a queued turn whose working folder vanished while it
/// waited ends `failed` with a recovered `done` that carries no session
/// key (the webview must not adopt the dead conversation's key back), and
/// nothing takes the lease.
#[test]
fn a_queued_chat_turn_whose_folder_vanished_ends_recovered_through_the_pump() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let project = v.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let mut request = chat_request(v);
    request.cwd = project.clone();
    let accepted = chat::accept_turn(v, request, now).unwrap();
    std::fs::remove_dir(&project).unwrap();
    let (row_id, _event, events) = handled_chat(run_step(v, now, None, &deps()));
    assert_eq!(row_id, accepted.queue_id);
    assert_eq!(row(v, row_id).status, RowStatus::Failed);
    assert!(lease_holder(v).is_none(), "nothing took the lease");
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["type"], "error");
    assert!(events[0]["message"]
        .as_str()
        .unwrap_or("")
        .starts_with(chat::CHAT_CONTEXT_RECOVERED_ERROR));
    assert_eq!(events[1]["type"], "done");
    assert_eq!(events[1]["recovered"], true);
    assert!(events[1].get("sessionKey").is_none(), "{:?}", events[1]);
    assert!(events[1].get("sessionId").is_none(), "{:?}", events[1]);
}

/// A healthy lease reads `Unknown` between its exclusive create and the
/// owner's write, and `Stale` between a finishing run's finalized manifest
/// and its release. Both windows run under `HISTORY_LOCK`; a chat send, or
/// a pump step with a queued chat turn, landing in one must wait for the
/// lease, never fail with the restart hint (review of PR 7b).
#[test]
fn a_lease_being_written_or_released_never_fails_a_chat_send() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let lease = v.join(".hollow-history/active-run.nosync");
    std::fs::create_dir_all(v.join(".hollow-history")).unwrap();
    // The writer parks the lease under `HISTORY_LOCK` and rewrites it only
    // once the call under test has looked: its lock-free read saw the
    // transient state and it is about to re-read under the lock (the hook
    // fires on this thread inside `dead_lease_holder`). No sleep, so a
    // loaded machine can't let the rewrite land before the call looks
    // (OF09). The bound is only a no-hang guard: the writer gives up and
    // releases the lock if the call never re-reads, and the join fails.
    // Not covered here any more: that the re-read takes `HISTORY_LOCK` (the
    // old 300 ms window also exercised that by timing); the writer rewrites
    // as soon as the hook fires.
    let park = |initial: Option<String>, then: Option<String>| {
        let lease = lease.clone();
        let (parked, wait) = std::sync::mpsc::channel();
        let (looked, seen) = std::sync::mpsc::channel::<()>();
        let writer = std::thread::spawn(move || {
            let _guard = automated_history::hold_history_lock_for_test();
            match &initial {
                Some(text) => std::fs::write(&lease, text).unwrap(),
                None => {
                    let _ = std::fs::remove_file(&lease);
                }
            }
            parked.send(()).unwrap();
            seen.recv_timeout(std::time::Duration::from_secs(60))
                .expect("the call re-read the lease under HISTORY_LOCK");
            match then {
                Some(text) => std::fs::write(&lease, text).unwrap(),
                None => std::fs::remove_file(&lease).unwrap(),
            }
        });
        wait.recv().unwrap();
        automated_history::set_before_dead_lease_read_hook(move || {
            let _ = looked.send(());
        });
        writer
    };
    let looked_under_the_lock = || {
        assert!(
            !automated_history::disarm_before_dead_lease_read_hook(),
            "the call re-read the lease under HISTORY_LOCK"
        );
    };

    // Window 1: created empty, the owner (a Notion sync) not yet written.
    let writer = park(
        Some(String::new()),
        Some("tracker-sync-run-parked\n".into()),
    );
    assert_eq!(
        lease_holder(v),
        Some(LeaseHolder::Unknown),
        "it reads as unknown"
    );
    let accepted =
        chat::accept_turn(v, chat_request(v), now).expect("queued behind the live lease");
    looked_under_the_lock();
    writer.join().unwrap();
    assert_eq!(lease_holder(v), Some(LeaseHolder::TrackerSync));
    std::fs::remove_file(&lease).unwrap();

    // Window 2: a finished run's manifest is final, its lease not yet gone.
    let handle = automated_history::try_begin_run(v, "workflow", Some("finishing")).unwrap();
    let id = handle.id.clone();
    automated_history::finish_run(v, handle, "success").unwrap();
    let writer = park(Some(format!("{id}\n")), None);
    assert!(
        matches!(lease_holder(v), Some(LeaseHolder::Stale { .. })),
        "it reads as stale"
    );
    match run_step(v, now, None, &deps()) {
        PumpStep::Busy { failed, chat, .. } => {
            assert!(
                failed.is_empty() && chat.is_empty(),
                "the queued turn keeps waiting"
            );
        }
        _ => panic!("the finishing run still held the vault when the step looked"),
    }
    looked_under_the_lock();
    writer.join().unwrap();
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Queued);
    assert!(lease_holder(v).is_none());
    let turn = admitted(run_step(v, now, None, &deps()));
    assert!(matches!(turn, Dispatch::Chat { .. }));
    finish_dispatch(v, turn, "success");
}

/// A genuinely stale lease (no run of this process behind it) fails a
/// queued chat turn at once with the restart hint, as it does Run now,
/// and the step returns the turn's error and `done`.
#[test]
fn a_stale_lease_fails_a_queued_chat_turn_with_its_events() {
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    std::fs::create_dir_all(v.join(".hollow-history")).unwrap();
    std::fs::write(
        v.join(".hollow-history/active-run.nosync"),
        "not-a-run-owner\n",
    )
    .unwrap();
    match run_step(v, now, None, &deps()) {
        PumpStep::Busy { failed, chat, .. } => {
            assert_eq!(failed, vec![accepted.queue_id]);
            assert_eq!(chat.len(), 2, "{chat:?}");
            assert_eq!(chat[0]["type"], "error");
            assert!(chat[0]["message"]
                .as_str()
                .unwrap_or("")
                .contains("restart Hollow"));
            assert_eq!(chat[0]["clientTurnId"], "ct-pump");
            assert_eq!(chat[1]["type"], "done");
        }
        _ => panic!("an unknown lease blocks the queue"),
    }
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Failed);
    // And a new send is refused up front with the same hint.
    let refused = chat::accept_turn(v, chat_request(v), now).unwrap_err();
    assert!(refused.contains("restart Hollow"), "{refused}");
}

/// A stale lease fails a queued chat turn once even when its terminal write
/// fails: the row stays `queued` with a pending finish, and the next steps
/// behind the same lease skip it instead of sending its error and `done`
/// again and owing a second finish (11-13).
#[test]
fn a_failed_turn_whose_finish_is_owed_is_not_failed_again_behind_the_lease() {
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let accepted = queue_chat(v, now);
    std::fs::create_dir_all(v.join(".hollow-history")).unwrap();
    std::fs::write(
        v.join(".hollow-history/active-run.nosync"),
        "not-a-run-owner\n",
    )
    .unwrap();
    crate::session_store::with_write(v, "refuse failed finish fixture", |tx| {
        tx.execute_batch(
            "CREATE TRIGGER refuse_failed_finish BEFORE UPDATE ON run_queue \
             WHEN NEW.status = 'failed' \
             BEGIN SELECT RAISE(ABORT, 'synthetic finish refusal'); END;",
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let mut pending = Vec::new();
    match step(v, now, None, &mut pending, &deps()) {
        PumpStep::Busy { failed, chat, .. } => {
            assert_eq!(failed, vec![accepted.queue_id]);
            assert_eq!(chat.len(), 2, "{chat:?}");
        }
        _ => panic!("an unknown lease blocks the queue"),
    }
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Queued);
    // An automation row keeps the queue non-empty, so the next steps reach
    // the lease check (and, once it is gone, admission).
    let c = due_cron(v, "Pump Owed Finish Cron", now - Duration::minutes(1));
    let cron_id = queue_cron(v, &c, now);
    // The write still fails: the next step owes the same one finish and
    // ends the turn in no chat again.
    match step(v, now, None, &mut pending, &deps()) {
        PumpStep::Busy { failed, chat, .. } => {
            assert!(failed.is_empty(), "{failed:?}");
            assert!(chat.is_empty(), "{chat:?}");
        }
        _ => panic!("an unknown lease blocks the queue"),
    }
    assert_eq!(pending.len(), 1, "one owed finish per row: {pending:?}");
    assert_eq!(pending[0].id, accepted.queue_id);
    // The lease goes away while the write still fails: the ended row is not
    // claimed and run again.
    std::fs::remove_file(v.join(".hollow-history/active-run.nosync")).unwrap();
    assert!(lease_holder(v).is_none());
    let run = admitted(step(v, now, None, &mut pending, &deps()));
    assert_eq!(run.row().id, cron_id, "the automation, not the ended turn");
    finish_dispatch(v, run, "success");
    assert!(matches!(
        step(v, now, None, &mut pending, &deps()),
        PumpStep::Idle
    ));
    assert_eq!(row(v, accepted.queue_id).status, RowStatus::Queued);
    assert_eq!(pending.len(), 1, "{pending:?}");
    // The store recovers: the retry lands once and nothing is owed.
    crate::session_store::with_write(v, "drop refusal fixture", |tx| {
        tx.execute_batch("DROP TRIGGER refuse_failed_finish")
            .map_err(|e| e.to_string())
    })
    .unwrap();
    assert!(matches!(
        step(v, now, None, &mut pending, &deps()),
        PumpStep::Idle
    ));
    assert!(pending.is_empty(), "{pending:?}");
    let ended = row(v, accepted.queue_id);
    assert_eq!(ended.status, RowStatus::Failed);
    assert!(
        ended
            .error
            .as_deref()
            .unwrap_or("")
            .contains("restart Hollow"),
        "the original outcome: {:?}",
        ended.error
    );
    assert!(ended.claimed_at.is_none(), "never claimed: {ended:?}");
    assert!(ended.history_run_id.is_none(), "never ran: {ended:?}");
}

#[test]
fn startup_marks_interrupted_chat_turns_and_never_runs_a_prior_boot_queued_turn() {
    let dir = chat_vault();
    let v = dir.path();
    let boot = Utc::now();
    let before = boot - Duration::hours(1);
    // A turn the previous process was running, with its turn row.
    let running = queue_chat(v, before);
    run_queue::claim(v, running.queue_id, before)
        .unwrap()
        .unwrap();
    run_queue::mark_running(v, running.queue_id, "run-prior-chat", before).unwrap();
    crate::session_store::start_turn(
        v,
        &running.session_key,
        "claude",
        &v.to_string_lossy(),
        "run-prior-chat",
        "Hello pump",
    )
    .unwrap();
    // And a turn it had queued but never started.
    let mut queued = NewRow::new(RunSource::Chat, Some("s-20260925-0000pr10"), "Chat", "chat")
        .with_payload(serde_json::json!({ "clientTurnId": "ct-prior" }));
    queued.detail = Some("Send this later".into());
    let queued_id = run_queue::enqueue(v, queued, before).unwrap().id();

    let (interrupted, _) = recover_queue(v, boot, boot, &HashSet::new()).unwrap();
    let ids: Vec<i64> = interrupted.iter().map(|row| row.id).collect();
    assert!(
        ids.contains(&running.queue_id) && ids.contains(&queued_id),
        "{ids:?}"
    );
    let was_running = row(v, running.queue_id);
    assert_eq!(was_running.status, RowStatus::Interrupted);
    assert_eq!(
        was_running.error.as_deref(),
        Some(run_queue::INTERRUPTED_RUNNING)
    );
    let conversation = crate::session_store::read_conversation(v, &running.session_key).unwrap();
    assert!(
        conversation
            .turns
            .iter()
            .filter(|turn| turn.run_id.as_deref() == Some("run-prior-chat"))
            .all(|turn| turn.status.as_deref() == Some("interrupted")),
        "the mid-run turn reads interrupted"
    );
    let was_queued = row(v, queued_id);
    assert_eq!(was_queued.status, RowStatus::Interrupted);
    assert_eq!(
        was_queued.error.as_deref(),
        Some(run_queue::INTERRUPTED_QUEUED)
    );
    assert_eq!(
        was_queued.detail.as_deref(),
        Some("Send this later"),
        "kept for the composer"
    );
    assert!(
        matches!(run_step(v, boot, None, &deps()), PumpStep::Idle),
        "never claimed"
    );
}

/// Merged plan slice 3 (B·F10): crash recovery finalizes a chat turn the
/// previous process left running in one place, so its queue row, its
/// transcript turn and its usage rows (its own and a delegate's) all end
/// `interrupted` together. Before, the turn was closed here, best effort,
/// and the usage row only by a separate startup reconcile.
#[test]
fn recovery_closes_a_crashed_chat_turns_transcript_and_usage_together() {
    let dir = chat_vault();
    let v = dir.path();
    let boot = Utc::now();
    let before = boot - Duration::hours(1);
    let running = queue_chat(v, before);
    run_queue::claim(v, running.queue_id, before)
        .unwrap()
        .unwrap();
    run_queue::mark_running(v, running.queue_id, "run-crashed-chat", before).unwrap();
    crate::session_store::start_turn(
        v,
        &running.session_key,
        "claude",
        &v.to_string_lossy(),
        "run-crashed-chat",
        "Hello pump",
    )
    .unwrap();
    let at = crate::session_store::db_time(before);
    for (key, surface) in [("u-crashed", "chat"), ("u-crashed-delegate", "delegate")] {
        let mut usage = crate::session_store::tests::usage_row_fixture(key, &at, "running");
        usage.run_id = Some("run-crashed-chat".into());
        usage.surface = surface.into();
        crate::session_store::upsert_usage(v, &usage).unwrap();
    }

    recover_queue(v, boot, boot, &HashSet::new()).unwrap();

    assert_eq!(row(v, running.queue_id).status, RowStatus::Interrupted);
    let conversation = crate::session_store::read_conversation(v, &running.session_key).unwrap();
    let turn = conversation
        .turns
        .iter()
        .find(|turn| turn.run_id.as_deref() == Some("run-crashed-chat"))
        .expect("the crashed turn");
    assert_eq!(turn.status.as_deref(), Some("interrupted"));
    let since = crate::session_store::db_time(before - Duration::minutes(1));
    let usage = crate::session_store::usage_rows_since(v, &since).unwrap();
    assert_eq!(usage.len(), 2, "{usage:?}");
    assert!(
        usage.iter().all(|row| row.outcome == "interrupted"),
        "the usage rows close with the turn: {usage:?}"
    );
}

// --- Paperclip PR 9b: idle skips and retry rows ------------------------

fn queued_retry(
    v: &Path,
    w: &Workflow,
    started: DateTime<Utc>,
    finished: DateTime<Utc>,
) -> (crate::workflow_retry::RetryPlan, i64) {
    let plan = crate::workflow_retry::retry_plan(&crate::workflow_retry::RetryInput {
        slug: &w.slug,
        title: &w.title,
        history_run_id: "run-failed",
        source: RunSource::Cron,
        attempt: 0,
        queue_id: Some(7),
        priority: RunSource::Cron.default_priority(),
        class: crate::workflow_retry::FailureClass::Transient(
            crate::workflow_retry::TransientKind::Network,
        ),
        history_finished: true,
        history_flagged: false,
        outside_effects_unsafe: false,
        started_at: started,
        finished_at: finished,
        error: "API Error: 503 service unavailable",
        trigger_reason: None,
        prior_last_run_at: None,
    })
    .expect("a transient cron failure retries");
    let id = workflows::enqueue_retry(v, &w.slug, &w.title, &plan)
        .unwrap()
        .id();
    (plan, id)
}

#[test]
fn a_transient_failure_enqueues_one_delayed_retry_and_its_claim_is_validated() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow(v, "Pump Retry R", "manual", "BODY");
    let started = now - Duration::minutes(2);
    vault_workflows::mark_started(v, &w.slug, started).unwrap();
    let (plan, id) = queued_retry(v, &w, started, now);

    let queued = row(v, id);
    assert_eq!(queued.source, RunSource::Retry);
    assert_eq!(queued.retry_of, Some(7));
    assert_eq!(queued.priority, RunSource::Cron.default_priority());
    // Stored at millisecond precision (`db_time`), so compare the stamp.
    assert_eq!(
        queued.not_before.as_deref(),
        Some(crate::session_store::db_time(plan.not_before).as_str())
    );
    assert!(
        parse_stamp(queued.not_before.as_deref().unwrap()).unwrap()
            > now + Duration::seconds(60) - Duration::milliseconds(1)
    );
    // The same failed run adds nothing.
    assert_eq!(
        workflows::enqueue_retry(v, &w.slug, &w.title, &plan)
            .unwrap()
            .id(),
        id
    );
    assert_eq!(run_queue::live_rows(v).unwrap().len(), 1);

    // Not claimable before its not_before.
    assert!(matches!(
        run_step(v, now, None, &deps()),
        PumpStep::Sleep(_)
    ));

    // Due: it runs as a retry, and the run it becomes never retries again.
    let due = plan.not_before + Duration::seconds(1);
    let run = admitted(run_step(v, due, None, &deps()));
    assert_eq!(run.row().id, id);
    match &run {
        Dispatch::Workflow { run: admitted, .. } => {
            assert_eq!(admitted.context().source, RunSource::Retry);
        }
        _ => panic!("a workflow dispatch"),
    }
    finish_dispatch(v, run, "error");
    assert!(
        crate::workflow_retry::retry_plan(&crate::workflow_retry::RetryInput {
            slug: &w.slug,
            title: &w.title,
            history_run_id: "run-retry",
            source: RunSource::Retry,
            attempt: 0,
            queue_id: Some(id),
            priority: 2,
            class: crate::workflow_retry::FailureClass::Transient(
                crate::workflow_retry::TransientKind::Network,
            ),
            history_finished: true,
            history_flagged: false,
            outside_effects_unsafe: false,
            started_at: due,
            finished_at: due,
            error: "API Error: 503",
            trigger_reason: None,
            prior_last_run_at: None,
        })
        .is_none()
    );

    // Ran since: the workflow started again after the failure.
    let w2 = seed_workflow(v, "Pump Retry Ran Since", "manual", "BODY");
    vault_workflows::mark_started(v, &w2.slug, started).unwrap();
    let (plan2, id2) = queued_retry(v, &w2, started, now);
    vault_workflows::mark_started(v, &w2.slug, now).unwrap();
    let before = run_log(v, &w2.slug);
    let (row_id, _, _) = handled(run_step(v, plan2.not_before, None, &deps()));
    assert_eq!(row_id, id2);
    let ended = row(v, id2);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert_eq!(
        ended.error.as_deref(),
        Some("retry skipped: the automation ran again")
    );
    assert_eq!(run_log(v, &w2.slug), before, "no run-log row");

    // Stale: claimed more than 30 minutes after its not_before.
    let w3 = seed_workflow(v, "Pump Retry Stale", "manual", "BODY");
    vault_workflows::mark_started(v, &w3.slug, started).unwrap();
    let (plan3, id3) = queued_retry(v, &w3, started, now);
    handled(run_step(
        v,
        plan3.not_before + Duration::minutes(31),
        None,
        &deps(),
    ));
    let ended = row(v, id3);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert!(ended.error.unwrap().contains("30 minutes"));

    // A usage budget holds the automation: skipped, no run.
    let w4 = seed_workflow_with(v, "Pump Retry Budget", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    vault_workflows::mark_started(v, &w4.slug, started).unwrap();
    budget_over(
        v,
        serde_json::json!([
            { "id": "rb", "scope": format!("workflow:{}", w4.slug), "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("workflow", Some(w4.slug.as_str()), 1200)],
    );
    let (plan4, id4) = queued_retry(v, &w4, started, now);
    handled(run_step(v, plan4.not_before, None, &deps()));
    let ended = row(v, id4);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert_eq!(
        ended.error.as_deref(),
        Some("retry skipped: a usage budget paused it")
    );
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        1,
        "only the first retry ran"
    );

    // Missing: the workflow was deleted.
    let w5 = seed_workflow(v, "Pump Retry Gone", "manual", "BODY");
    vault_workflows::mark_started(v, &w5.slug, started).unwrap();
    let (plan5, id5) = queued_retry(v, &w5, started, now);
    std::fs::remove_file(v.join(&w5.path)).unwrap();
    handled(run_step(v, plan5.not_before, None, &deps()));
    assert_eq!(
        row(v, id5).error.as_deref(),
        Some("retry skipped: the automation no longer exists")
    );
}

/// Paperclip PR 9b (refresh C2): a retry that no longer matters ends in
/// phase 1, before PR 5's permission check, so it opens no review; a due
/// retry of a tampered automation is skipped as refused; an automation
/// that cannot run as written is reported as such, not as missing; and
/// every such end (a user cancel included) records the deferred failure.
#[test]
fn a_retry_claim_opens_no_review_unless_due_and_reports_every_skip() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let started = now - Duration::minutes(2);
    let streak_count =
        |slug: &str| crate::workflow_hygiene::failure_streak(&run_log(v, slug)).map(|s| s.count);
    let failed_row = |slug: &str| {
        vault_workflows::finalize_run(
            v,
            slug,
            "error",
            Some("API Error: 503"),
            1,
            None,
            RunLogEntry {
                at: now.to_rfc3339(),
                status: "error".into(),
                duration_sec: 1,
                note: "API Error: 503".into(),
                run_id: Some("run-failed".into()),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
    };

    // Stale, tampered: skipped as stale, no review.
    let stale = seed_workflow(v, "Retry Tampered Stale", "manual", "BODY");
    vault_workflows::mark_started(v, &stale.slug, started).unwrap();
    let (plan, id) = queued_retry(v, &stale, started, now);
    tamper(
        v,
        &stale.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    handled(run_step(
        v,
        plan.not_before + Duration::minutes(31),
        None,
        &deps(),
    ));
    assert!(row(v, id).error.unwrap().contains("30 minutes"));
    assert!(
        pending_permission_reviews(v).is_empty(),
        "a stale retry opens no review"
    );

    // Ran since, tampered: ends silently, no review.
    let ran = seed_workflow(v, "Retry Tampered Ran", "manual", "BODY");
    vault_workflows::mark_started(v, &ran.slug, started).unwrap();
    let (plan, id) = queued_retry(v, &ran, started, now);
    vault_workflows::mark_started(v, &ran.slug, now).unwrap();
    tamper(
        v,
        &ran.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    handled(run_step(v, plan.not_before, None, &deps()));
    assert_eq!(
        row(v, id).error.as_deref(),
        Some("retry skipped: the automation ran again")
    );
    assert!(
        pending_permission_reviews(v).is_empty(),
        "a ran-since retry opens no review"
    );

    // Due, tampered: PR 5 refuses it; the retry is skipped as refused and
    // the deferred failure counts.
    let due = seed_workflow(v, "Retry Tampered Due", "manual", "BODY");
    vault_workflows::mark_started(v, &due.slug, started).unwrap();
    failed_row(&due.slug);
    let (plan, id) = queued_retry(v, &due, started, now);
    tamper(
        v,
        &due.slug,
        "permission_mode: acceptEdits",
        "permission_mode: bypassPermissions",
    );
    handled(run_step(v, plan.not_before, None, &deps()));
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert_eq!(
        ended.error.as_deref(),
        Some("retry skipped: its settings changed and await approval")
    );
    assert_eq!(
        pending_permission_reviews(v).len(),
        1,
        "the due claim's check opened it"
    );
    assert_eq!(streak_count(&due.slug), Some(1));
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        0,
        "never ran"
    );

    // Cannot run as written (pure claim rule): Invalid, not Missing.
    let row_now = row(v, id);
    let facts = ClaimFacts {
        workflow: WorkflowFact::Unreadable,
        precheck: Some(Err(PrecheckFailure::Invalid("empty prompt".into()))),
        live_overdue_ids: None,
        live_unblocked_ids: None,
        heartbeat_enabled: true,
    };
    let queued_like = QueueRow {
        status: RowStatus::Queued,
        error: None,
        ..row_now
    };
    match decide_claim(&queued_like, &facts, plan.not_before) {
        Decision::Verdict(ClaimVerdict::EndRetry { reason, .. }) => {
            assert_eq!(reason, RetrySkipReason::Invalid);
            assert_eq!(
                reason.row_error(),
                "retry skipped: the automation cannot run as written"
            );
        }
        _ => panic!("an invalid workflow ends the retry"),
    }

    // A user cancel of the queued retry records the deferred failure.
    let cancelled = seed_workflow(v, "Retry Cancelled", "manual", "BODY");
    vault_workflows::mark_started(v, &cancelled.slug, started).unwrap();
    failed_row(&cancelled.slug);
    let (_, id) = queued_retry(v, &cancelled, started, now);
    let acted = workflows::cancel_queued(v, &cancelled.slug).unwrap();
    assert_eq!(acted.len(), 1);
    assert_eq!(row(v, id).status, RowStatus::Cancelled);
    assert_eq!(streak_count(&cancelled.slug), Some(1));
    let before = crate::review_queue::list(v, Some(crate::workflow_hygiene::STREAK_KIND))
        .unwrap()
        .len();
    // Two more cancelled retries after failures open the streak: each
    // cancel recorded its outcome.
    for n in 0..2 {
        failed_row(&cancelled.slug);
        let (_, _) = queued_retry(v, &cancelled, started + Duration::seconds(n + 1), now);
        workflows::cancel_queued(v, &cancelled.slug).unwrap();
    }
    assert_eq!(
        crate::review_queue::list(v, Some(crate::workflow_hygiene::STREAK_KIND))
            .unwrap()
            .len(),
        before + 1,
        "the third cancelled retry's deferred failure opened the streak"
    );
}

/// Paperclip PR 9b: a retry row Hollow stopped while it was claimed never
/// reached `finalize`; startup recovery reports its deferred failure.
#[test]
fn startup_recovery_reports_an_interrupted_retrys_deferred_failure() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let started = now - Duration::minutes(2);
    let w = seed_workflow(v, "Retry Interrupted", "manual", "BODY");
    vault_workflows::mark_started(v, &w.slug, started).unwrap();
    for n in 0..3 {
        vault_workflows::finalize_run(
            v,
            &w.slug,
            "error",
            Some("API Error: 503"),
            1,
            None,
            RunLogEntry {
                at: now.to_rfc3339(),
                status: "error".into(),
                duration_sec: 1,
                note: "API Error: 503".into(),
                run_id: Some(format!("run-{n}")),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
    }
    let (plan, id) = queued_retry(v, &w, started, now);
    assert!(
        run_queue::claim(v, id, plan.not_before).unwrap().is_some(),
        "claimed by the old process"
    );
    let (interrupted, _) =
        recover_queue(v, plan.not_before, plan.not_before, &HashSet::new()).unwrap();
    assert!(interrupted.iter().any(|r| r.id == id));
    assert_eq!(row(v, id).status, RowStatus::Interrupted);
    assert_eq!(
        crate::review_queue::list(v, Some(crate::workflow_hygiene::STREAK_KIND))
            .unwrap()
            .len(),
        1,
        "the deferred third failure opened the streak"
    );
}

#[test]
fn a_quiet_skip_waits_for_the_bookkeeping_lease_and_never_creates_a_history_run() {
    // Usage is written now and the claim runs later (up to a minute for a retry): both must
    // count the same local day's budget window, so start clear of local midnight (11-13 flake).
    crate::test_clock::clear_of_midnight(Duration::minutes(3));
    use crate::workflow_hygiene::tests::{
        current, previous_run_due, quiet_vault, workspace_workflow,
    };
    let dir = quiet_vault();
    let v = dir.path();
    let (workspace, w) = workspace_workflow(v, "Pump Quiet Q");
    let plan = format!("{}/PLAN.md", workspace.path);
    let due_at = Utc::now() - Duration::minutes(1);
    previous_run_due(v, &w.slug, &[(&plan, "new plan")], &[], Some(due_at));
    // After the previous run: its recorded start must not lie ahead.
    let now = Utc::now();
    let w = current(v, &w.slug);
    let due = w.next_run_at.clone().expect("due");
    // An exceeded budget for this workflow: the quiet gate runs before
    // the budget preflight, so the skip opens no pause.
    budget_over(
        v,
        serde_json::json!([
            { "id": "bq", "scope": format!("workflow:{}", w.slug), "metric": "tokens", "window": "day", "limit": 1000 },
        ]),
        &[("workflow", Some(w.slug.as_str()), 1200)],
    );
    let id = queue_cron(v, &w, now);
    let runs_before = automated_history::list_runs(v, usize::MAX).unwrap().len();
    let before = workflow_bytes(v, &w.slug);

    // A chat takes the lease right before the skip's bookkeeping.
    let raced = RefCell::new(None);
    let hook = |vault: &Path| {
        *raced.borrow_mut() = Some(begin_run(vault, "chat", None).unwrap());
    };
    let racing = StepDeps {
        before_bookkeeping: &hook,
        ..deps()
    };
    handled(run_step(v, now, None, &racing));
    assert_eq!(
        row(v, id).status,
        RowStatus::Queued,
        "re-queued, not skipped"
    );
    assert_eq!(
        workflow_bytes(v, &w.slug),
        before,
        "nothing written inside the chat's lease"
    );
    let chat_run = finish_run(v, raced.borrow_mut().take().unwrap(), "success").unwrap();
    assert!(!changed_paths(&chat_run)
        .iter()
        .any(|p| p.starts_with("workflows/")));

    // Free again: the skip is written under the bookkeeping lease.
    let (row_id, event, notice) = handled(run_step(v, now, None, &deps()));
    assert_eq!((row_id, event), (id, Some("finished")));
    assert!(notice.is_none(), "a quiet skip joins no missed/busy notice");
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert_eq!(
        ended.error.as_deref(),
        Some(vault_workflows::QUIET_SKIP_NOTE)
    );
    let newest = run_log(v, &w.slug).pop().unwrap();
    assert_eq!(newest.status, vault_workflows::SKIPPED_STATUS);
    assert_eq!(newest.note, vault_workflows::QUIET_SKIP_NOTE);
    assert_ne!(
        next_run(v, &w.slug),
        Some(due),
        "the occurrence was consumed"
    );
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        runs_before + 1,
        "only the chat's run: a quiet skip never begins a history run"
    );
    assert_eq!(pause_cards(v), 0, "a quiet skip opens no budget pause");
    assert!(
        run_log(v, &w.slug)
            .iter()
            .all(|e| !e.note.starts_with(vault_workflows::BUDGET_SKIP_NOTE_PREFIX)),
        "and writes no budget skip row"
    );

    // Control: once something changed, the next occurrence reaches the
    // budget preflight, which pauses it (so the budget above was live).
    std::thread::sleep(std::time::Duration::from_millis(20));
    touch_note(v, &format!("{}/SOURCES.md", workspace.path));
    let raw = std::fs::read_to_string(v.join(&w.path)).unwrap();
    let next = next_run(v, &w.slug).unwrap();
    let due_again = (Utc::now() - Duration::minutes(1)).to_rfc3339();
    std::fs::write(v.join(&w.path), raw.replace(&next, &due_again)).unwrap();
    let w = current(v, &w.slug);
    let id = queue_cron(v, &w, Utc::now());
    handled(run_step(v, Utc::now(), None, &deps()));
    assert!(row(v, id)
        .error
        .unwrap()
        .starts_with(budgets::BUDGET_PAUSED_PREFIX));
    assert_eq!(pause_cards(v), 1);
}

/// Paperclip PR 9b: an inactive workspace skips CRON rows only. The skip
/// is written under the bookkeeping lease with one in-app notice, a
/// second occurrence coalesces into the same row, and manual and retry
/// rows for the same workflow are admitted.
#[test]
fn an_inactive_workspace_skips_cron_rows_but_admits_manual_and_retry_rows() {
    use crate::workflow_hygiene::tests::{current, quiet_vault, workspace_workflow};
    let dir = quiet_vault();
    let v = dir.path();
    let (workspace, w) = workspace_workflow(v, "Pump Inactive I");
    let brief = v.join(&workspace.path).join("WORKSPACE.md");
    let text = std::fs::read_to_string(&brief).unwrap();
    std::fs::write(&brief, text.replace("status: active", "status: done")).unwrap();
    let due_now = |v: &Path| {
        let raw = std::fs::read_to_string(v.join(&w.path)).unwrap();
        let due = (Utc::now() - Duration::minutes(1)).to_rfc3339();
        let fixed = match current(v, &w.slug).next_run_at {
            Some(next) => raw.replace(&next, &due),
            None => raw.replacen("\n---\n", &format!("\nnext_run_at: {due}\n---\n"), 1),
        };
        std::fs::write(v.join(&w.path), fixed).unwrap();
        current(v, &w.slug)
    };

    let first = due_now(v);
    let id = queue_cron(v, &first, Utc::now());
    let (row_id, event, notice) = handled(run_step(v, Utc::now(), None, &deps()));
    assert_eq!((row_id, event), (id, Some("finished")));
    assert!(notice.is_none(), "an idle skip joins no missed/busy notice");
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    let note = ended.error.unwrap();
    assert!(
        note.starts_with(vault_workflows::INACTIVE_SKIP_PREFIX),
        "{note}"
    );
    let log = run_log(v, &w.slug);
    assert_eq!(
        log.last().unwrap().note,
        "workspace inactive: Launch plan is done"
    );
    assert_ne!(
        current(v, &w.slug).next_run_at,
        first.next_run_at,
        "the occurrence was consumed"
    );
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        0,
        "no history run"
    );
    let sent = || IDLE_NOTICES.with(|sent| sent.borrow().clone());
    let notices = sent();
    assert_eq!(notices.len(), 1, "one notice for the episode");
    assert_eq!(
        notices[0].title,
        "Pump Inactive I is paused while Launch plan is done"
    );
    assert!(!notices[0].notify_native);

    // A second occurrence coalesces into the same row, with no notice.
    let second = due_now(v);
    let id = queue_cron(v, &second, Utc::now());
    handled(run_step(v, Utc::now(), None, &deps()));
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    let log = run_log(v, &w.slug);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(
        log[0].note,
        "workspace inactive: Launch plan is done · 2 skips"
    );
    assert_eq!(sent().len(), 1, "coalesced skips send no second notice");

    // Run now is never gated.
    let manual = queue_manual(v, &w, Utc::now());
    let run = admitted(run_step(v, Utc::now(), None, &deps()));
    assert_eq!(run.row().id, manual);
    finish_dispatch(v, run, "success");

    // Nor is a retry.
    let started = Utc::now() - Duration::minutes(2);
    vault_workflows::mark_started(v, &w.slug, started).unwrap();
    let w = current(v, &w.slug);
    let (plan, retry) = queued_retry(v, &w, started, Utc::now());
    let run = admitted(run_step(v, plan.not_before, None, &deps()));
    assert_eq!(run.row().id, retry);
    finish_dispatch(v, run, "error");
}

/// Paperclip PR 9b: the gates run in the claim, on cron rows only, after
/// the permission check and before the budget preflight.
#[test]
fn idle_gates_run_after_the_permission_check_and_before_the_budget() {
    const SRC: &str = include_str!("run_pump.rs");
    let claim = fn_body(SRC, concat!("pub(crate) fn ", "claim_verdict("));
    let verified = claim
        .find(concat!("(deps.", "precheck)("))
        .expect("phase 2");
    let gated = claim
        .find(concat!("idle_", "verdict("))
        .expect("the idle gates");
    let budget = claim.find(concat!("budget_", "verdict(")).expect("phase 3");
    assert!(verified < gated && gated < budget);
    assert!(claim.contains(concat!("row.source == RunSource::", "Cron")));
}

fn typed_chat_in(vault: &Path, cwd: PathBuf, now: DateTime<Utc>) -> chat::AcceptedTurn {
    let mut request = chat_request(vault);
    request.cwd = cwd;
    request.typed = true;
    chat::accept_turn(vault, request, now).unwrap()
}

/// COO rebuild L7: the CFO holds only unattended work: subagents (whatever
/// their `unattended` flag), night items, agent sends, Hollow's own
/// check-ins and wakes, scheduled and triggered automations and their
/// retries. Never the person's own sends, Run now or a retry of a Run now.
#[test]
fn the_cfo_holds_only_unattended_rows() {
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let chat = |payload: serde_json::Value| {
        let id = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261110-0000cfo1"), "Chat", "chat")
                .with_payload(payload),
            now,
        )
        .unwrap()
        .id();
        row(v, id)
    };
    let mine = chat(serde_json::json!({ "agent": "claude" }));
    assert!(!cfo_unattended(v, &mine), "the person's own send");
    let night = chat(serde_json::json!({ "agent": "codex", "unattended": true }));
    assert!(cfo_unattended(v, &night));
    let sub = chat(
        serde_json::json!({ "agent": "codex", "unattended": false, "parent": "s-20261110-0000par1" }),
    );
    assert!(cfo_unattended(v, &sub), "a subagent whatever its flag");
    let asked = chat(serde_json::json!({ "agent": "grok", "appDepth": 1 }));
    assert!(cfo_unattended(v, &asked));
    let wake = chat(serde_json::json!({ "agent": "claude", "hostAsker": "checkin" }));
    assert!(cfo_unattended(v, &wake), "Hollow's own check-in");
    let empty_asker = chat(serde_json::json!({ "agent": "claude", "hostAsker": "" }));
    assert!(!cfo_unattended(v, &empty_asker));

    // A retry is what its failed run was.
    let w = seed_workflow(v, "Pump CFO Retry", "manual", "BODY");
    let manual = queue_manual(v, &w, now);
    let cron = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Cron, Some(&w.slug), &w.title, "scheduled"),
        now,
    )
    .unwrap()
    .id();
    let retry_of = |of: i64| {
        let mut retry = NewRow::new(RunSource::Retry, Some(&w.slug), &w.title, "retry");
        retry.retry_of = Some(of);
        row(v, run_queue::enqueue(v, retry, now).unwrap().id())
    };
    assert!(!cfo_unattended(v, &retry_of(manual)), "a retry of Run now");
    assert!(cfo_unattended(v, &retry_of(cron)));
    assert!(!cfo_unattended(v, &row(v, manual)), "Run now");

    // With every account past the line, only unattended rows are held.
    let hold = |_: &Path, agent: &str| {
        Some(format!(
            "{}{agent} is full",
            crate::cfo::accounts::HOLD_MARK
        ))
    };
    let held = StepDeps {
        cfo_hold: &hold,
        ..deps()
    };
    assert!(cfo_hold(v, &night, None, now, &held).is_some());
    assert!(cfo_hold(v, &mine, None, now, &held).is_none());
    assert!(cfo_hold(v, &night, None, now, &deps()).is_none());
}

/// Review H1: the hold is judged on the accounts the run may really start
/// on. A Work account past the line holds a subagent even while a Personal
/// account in another organisation has room; once crossing organisations is
/// allowed (Personal joins the set), it runs.
#[test]
fn the_cfo_holds_on_the_accounts_a_run_may_start_on() {
    use crate::cfo::accounts::{hold_for, pool, Reading, Window};
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let resets = (now + Duration::hours(2)).to_rfc3339();
    let reading = |account: &str, used: f64| Reading {
        agent: "claude".into(),
        account: account.into(),
        label: account.into(),
        enabled: true,
        five_hour: Some(Window {
            used,
            resets_at: Some(resets.clone()),
        }),
        weekly: None,
        observed_at: Some(now),
    };
    let readings = vec![reading("work", 95.0), reading("personal", 5.0)];
    let settings = crate::cfo::CfoSettings::default();
    let judge = |eligible: Vec<String>| {
        let readings = readings.clone();
        let settings = settings.clone();
        move |_: &Path, agent: &str| {
            hold_for(
                &settings,
                &pool(&readings, agent, Some(&eligible)),
                agent,
                now,
            )
        }
    };
    let same_org = judge(vec!["work".into()]);
    let crossing = judge(vec!["work".into(), "personal".into()]);

    let id = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Chat, Some("s-20261110-0000cfo2"), "Chat", "chat").with_payload(
            serde_json::json!({ "agent": "claude", "parent": "s-20261110-0000par2" }),
        ),
        now,
    )
    .unwrap()
    .id();
    let claimed = run_queue::claim(v, id, now).unwrap().expect("claimed");
    let deps_same = StepDeps {
        cfo_hold: &same_org,
        ..deps()
    };
    match claim_verdict(v, &claimed, now, &deps_same) {
        ClaimVerdict::Defer {
            held: Some(note), ..
        } => {
            assert!(note.starts_with(crate::cfo::accounts::HOLD_MARK), "{note}");
            assert!(note.contains("work"), "{note}");
        }
        _ => panic!("held: the account in another organisation can't take it"),
    }
    let deps_crossing = StepDeps {
        cfo_hold: &crossing,
        ..deps()
    };
    assert!(matches!(
        claim_verdict(v, &claimed, now, &deps_crossing),
        ClaimVerdict::RunChat
    ));
}

/// Review 6: a scheduled automation the CFO held past its grace is skipped
/// with its own run-log row and a Reserve notice (one notification naming
/// the reserve), never consumed as a silent misfire; Run now is never held.
#[test]
fn a_cfo_held_cron_past_its_grace_is_skipped_with_a_reserve_notice() {
    let now = Utc::now();
    let hold = |_: &Path, _: &str| {
        Some(format!(
            "{}every account is full",
            crate::cfo::accounts::HOLD_MARK
        ))
    };
    let held_deps = StepDeps {
        cfo_hold: &hold,
        ..deps()
    };
    let dir = temp_vault();
    let v = dir.path();
    let c = due_cron(v, "Pump CFO Held", now - Duration::minutes(1));
    let id = queue_cron(v, &c, now);
    let (_, event, notice) = handled(run_step(v, now, None, &held_deps));
    assert_eq!((event, notice), (Some("held"), None));
    assert!(cfo_held(&row(v, id)));
    let (_, _, notice) = handled(run_step(v, now + Duration::hours(3), None, &held_deps));
    let notice = notice.expect("announced");
    assert_eq!(notice.kind, NoticeKind::Reserve);
    assert_eq!(notice.title, c.title);
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    assert_eq!(
        run_log(v, &c.slug).pop().unwrap().note,
        format!("not run: {CFO_SKIP_NOTE}")
    );
    assert_ne!(next_run(v, &c.slug), c.next_run_at);
    let why = reserve_skip_why(20);
    assert!(
        why.contains("20% reserve for your own prompts") && why.contains("Settings, Usage"),
        "{why}"
    );
    assert_eq!(
        announced_titles(&[notice], NoticeKind::Reserve),
        vec![c.title.clone()]
    );

    // Run now is the person's own: never held.
    let m = seed_workflow(v, "Pump CFO Manual", "manual", "BODY");
    queue_manual(v, &m, now);
    let run = admitted(run_step(v, now + Duration::hours(3), None, &held_deps));
    assert_eq!(dispatch_slug(&run), Some(m.slug.as_str()));
    finish_dispatch(v, run, "success");
}

/// L11 (11-10): the company code is gone, its tables are not. A vault whose
/// database already holds company tables with rows opens and migrates, keeps
/// those rows untouched, creates no company table it lacks, and an ordinary
/// automation and an ordinary chat turn still claim and run.
#[test]
fn a_database_with_company_rows_opens_and_ordinary_work_still_runs() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let path = crate::session_store::db_path_for(v).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE company_assignments(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, record TEXT NOT NULL);
             CREATE TABLE company_work(id TEXT PRIMARY KEY, assignment_id TEXT NOT NULL REFERENCES company_assignments(id), queue_id INTEGER, session_key TEXT NOT NULL, status TEXT NOT NULL);
             CREATE TABLE company_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE workflow_priority_pauses(id TEXT PRIMARY KEY, record TEXT NOT NULL);
             INSERT INTO company_assignments VALUES('a1', 3, '{\"status\":\"recorded\"}');
             INSERT INTO company_work VALUES('w1', 'a1', 7, 's-company-0001', 'queued');
             INSERT INTO company_settings VALUES('routing', '{\"openRoute\":true,\"allow\":[]}', 'x');
             INSERT INTO workflow_priority_pauses VALUES('p1', '{}');",
        )
        .unwrap();
    }
    // The first store use opens the file and runs every migration on it.
    let m = seed_workflow(v, "Pump Company Rows Manual", "manual", "BODY");
    let manual = run_queue::enqueue(
        v,
        NewRow::new(RunSource::Manual, Some(&m.slug), &m.title, "run now"),
        now,
    )
    .unwrap()
    .id();
    let run = admitted(run_step(v, now, None, &deps()));
    assert_eq!(run.row().id, manual);
    assert_eq!(dispatch_slug(&run), Some(m.slug.as_str()));
    finish_dispatch(v, run, "success");
    assert_eq!(row(v, manual).status, RowStatus::Done);

    let queued = chat::accept_turn(v, chat_request(v), now).unwrap();
    let turn = admitted(run_step(v, now, None, &deps()));
    assert_eq!(turn.row().id, queued.queue_id);
    assert!(matches!(turn, Dispatch::Chat { .. }));
    finish_dispatch(v, turn, "success");
    assert_eq!(row(v, queued.queue_id).status, RowStatus::Done);

    let (assignments, work, pauses, settings, created): (i64, i64, i64, i64, i64) =
        crate::session_store::with_read(v, |conn| {
            conn.query_row(
                "SELECT (SELECT count(*) FROM company_assignments WHERE revision = 3),
                        (SELECT count(*) FROM company_work WHERE status = 'queued'),
                        (SELECT count(*) FROM workflow_priority_pauses),
                        (SELECT count(*) FROM company_settings),
                        (SELECT count(*) FROM sqlite_master WHERE type = 'table'
                         AND name IN ('company_holds', 'company_employees', 'workflow_execution_configs'))",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    assert_eq!(
        (assignments, work, pauses, settings),
        (1, 1, 1, 1),
        "rows kept"
    );
    assert_eq!(created, 0, "no company table is created any more");
}

/// L11: a row the removed company code queued (a priority continuation, an
/// assignment's chat turn on its restricted agent marker) is settled once,
/// failed with [`COMPANY_REMOVED`], and never run or retried.
#[test]
fn a_queued_row_of_removed_company_work_is_settled_once_and_never_retried() {
    let _locks = chat_locks();
    let dir = chat_vault();
    let v = dir.path();
    let now = Utc::now();
    let resume = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::RestrictedResume,
            Some("company-report"),
            "Company report",
            "priority continuation",
        ),
        now,
    )
    .unwrap()
    .id();
    let chat_row = run_queue::enqueue(
        v,
        NewRow::new(
            RunSource::Chat,
            Some("s-company-0123456789abcdef0123456789abcdef"),
            "Chat",
            "assignment",
        )
        .with_payload(serde_json::json!({
            "prompt": "report", "asked": "report",
            "agent": "hollow-company-restricted-v3", "cwd": v,
        })),
        now,
    )
    .unwrap()
    .id();
    let mut ended = Vec::new();
    for _ in 0..2 {
        let (id, event, _) = handled(run_step(v, now, None, &deps()));
        assert_eq!(event, Some("finished"));
        ended.push(id);
    }
    ended.sort();
    assert_eq!(ended, [resume, chat_row]);
    for id in [resume, chat_row] {
        let settled = row(v, id);
        assert_eq!(settled.status, RowStatus::Failed);
        assert_eq!(settled.error.as_deref(), Some(COMPANY_REMOVED));
        assert!(settled.history_run_id.is_none(), "never started");
    }
    // Nothing is retried or left behind, an hour on either.
    assert!(matches!(
        run_step(v, now + Duration::hours(1), None, &deps()),
        PumpStep::Idle
    ));
    assert!(run_queue::live_rows(v).unwrap().is_empty());
    assert!(automated_history::list_runs(v, 10).unwrap().is_empty());
    // A cancel that landed first ends it cancelled instead, still once.
    let mut cancelled = row(v, resume);
    cancelled.cancel_requested = true;
    assert!(matches!(
        removed_company_verdict(&cancelled),
        ClaimVerdict::Cancel { .. }
    ));
}

/// M4: a cron row queued before a synced edit moved its automation to
/// another Mac ends skipped at claim, writing nothing; Run now still runs.
#[test]
fn a_row_for_an_automation_owned_elsewhere_is_skipped_at_claim() {
    use crate::runs_on::{with_seat, Seat};
    let dir = temp_vault();
    let v = dir.path();
    let now = Utc::now();
    let w = seed_workflow_with(v, "Owner Claim M4", "0 0 7 * * * *", "BODY", |fm| {
        fm.next_run_at = Some((now - Duration::minutes(1)).to_rfc3339());
        fm.runs_on = Some("mini".into());
    });
    let cron = row(v, queue_cron(v, &w, now));
    let before = std::fs::read(v.join(&w.path)).unwrap();
    let mini = Seat::mac("mini", "m-00000000000000aa");
    let paired = Seat::mac("macbook-pro", "m-00000000000000bb").with_peers(&[&mini]);
    with_seat(paired, || {
        match claim_verdict(v, &cron, now, &deps()) {
            ClaimVerdict::Skip {
                why, bookkeeping, ..
            } => {
                assert!(why.contains("runs on mini"), "{why}");
                assert!(bookkeeping.is_none(), "nothing is written to its file");
            }
            _ => panic!("a row owned elsewhere must be skipped"),
        }
        let manual = row(v, queue_manual(v, &w, now));
        assert!(
            !matches!(
                claim_verdict(v, &manual, now, &deps()),
                ClaimVerdict::Skip { .. }
            ),
            "Run now runs where it is clicked"
        );
    });
    with_seat(Seat::mac("mini", "m-00000000000000aa"), || {
        assert!(
            !matches!(
                claim_verdict(v, &cron, now, &deps()),
                ClaimVerdict::Skip { .. }
            ),
            "the owner claims it"
        );
    });
    assert_eq!(std::fs::read(v.join(&w.path)).unwrap(), before);
}

// OC13: the `precheck` gate at the claim (its own file: `workflow_precheck`).
#[path = "run_pump_precheck_tests.rs"]
mod precheck_gate;
