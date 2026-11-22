//! OC13: an automation's `precheck` command at the run-queue claim. A failing
//! one records a skipped run and starts no turn; a passing one admits the turn;
//! Run now and retry rows are never prechecked.

use super::*;

fn seed(v: &Path, title: &str, command: &str) -> Workflow {
    let w = seed_workflow_with(v, title, "0 0 7 * * * *", "BODY", |fm| {
        fm.precheck = Some(command.to_string());
        fm.next_run_at = Some((Utc::now() - Duration::minutes(1)).to_rfc3339());
    });
    vault_workflows::read_full(v, &w.slug).unwrap().0
}

/// Step the pump until the row's precheck (its own thread) settles: a
/// pending precheck defers the row, so each step moves the clock past the
/// one-second deferral.
fn settled_step(v: &Path, deps: &StepDeps<'_>) -> PumpStep {
    let base = Utc::now();
    for i in 0..400 {
        let now = base + Duration::seconds(2 * i);
        match run_step(v, now, None, deps) {
            PumpStep::Handled {
                event: Some("requeued"),
                ..
            }
            | PumpStep::Sleep(_)
            | PumpStep::Idle => {
                // sleep-ok: the precheck runs on a real thread; wait for its verdict.
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            done => return done,
        }
    }
    panic!("the precheck never settled");
}

#[test]
fn a_failing_precheck_records_a_skipped_run_and_starts_no_turn() {
    let dir = temp_vault();
    let v = dir.path();
    let w = seed(v, "Precheck Fails Oc13", "exit 1");
    assert_eq!(w.precheck.as_deref(), Some("exit 1"));
    let due = w.next_run_at.clone().expect("due");
    let id = queue_cron(v, &w, Utc::now());

    let (row_id, event, notice) = handled(settled_step(v, &deps()));
    assert_eq!((row_id, event), (id, Some("finished")));
    assert!(notice.is_none());
    let ended = row(v, id);
    assert_eq!(ended.status, RowStatus::Skipped);
    assert_eq!(
        ended.error.as_deref(),
        Some(crate::workflow_precheck::FAILED_NOTE)
    );
    let newest = run_log(v, &w.slug).pop().unwrap();
    assert_eq!(newest.status, vault_workflows::SKIPPED_STATUS);
    assert!(
        newest.note.starts_with("skipped_precheck:"),
        "{}",
        newest.note
    );
    assert_eq!(newest.run_id, None, "no run happened");
    assert_ne!(
        next_run(v, &w.slug),
        Some(due),
        "the occurrence was consumed"
    );
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        0,
        "no history run, so no agent turn"
    );
    // A second failing occurrence folds into the same row, like a quiet skip.
    let raw = std::fs::read_to_string(v.join(&w.path)).unwrap();
    let next = next_run(v, &w.slug).unwrap();
    let due_again = (Utc::now() - Duration::minutes(1)).to_rfc3339();
    std::fs::write(v.join(&w.path), raw.replace(&next, &due_again)).unwrap();
    let again = vault_workflows::read_full(v, &w.slug).unwrap().0;
    let id = queue_cron(v, &again, Utc::now());
    handled(settled_step(v, &deps()));
    assert_eq!(row(v, id).status, RowStatus::Skipped);
    let log = run_log(v, &w.slug);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(
        log[0].note,
        format!("{} · 2 skips", crate::workflow_precheck::FAILED_NOTE)
    );
}

#[test]
fn a_passing_precheck_admits_the_turn() {
    let dir = temp_vault();
    let v = dir.path();
    let w = seed(v, "Precheck Passes Oc13", "exit 0");
    let id = queue_cron(v, &w, Utc::now());
    let run = admitted(settled_step(v, &deps()));
    assert_eq!(run.row().id, id);
    finish_dispatch(v, run, "success");
}

#[test]
fn run_now_and_retries_are_not_prechecked() {
    let dir = temp_vault();
    let v = dir.path();
    let w = seed(v, "Precheck Manual Oc13", "exit 1");
    let manual = queue_manual(v, &w, Utc::now());
    let run = admitted(run_step(v, Utc::now(), None, &deps()));
    assert_eq!(run.row().id, manual);
    finish_dispatch(v, run, "success");
}

#[test]
fn a_slow_precheck_defers_the_row_and_the_claim_does_not_wait_for_it() {
    let dir = temp_vault();
    let v = dir.path();
    // sleep-ok: a command that outlives the claim; the step must not wait for it.
    let w = seed(v, "Precheck Slow Oc13", "sleep 5");
    let id = queue_cron(v, &w, Utc::now());
    let started = std::time::Instant::now();
    let (row_id, event, _) = handled(run_step(v, Utc::now(), None, &deps()));
    assert!(
        // timing: the precheck sleeps 5 s, so a 4 s bound only fails if the claim waited on it.
        started.elapsed() < std::time::Duration::from_secs(4),
        "the claim waited"
    );
    assert_eq!((row_id, event), (id, Some("requeued")));
    let queued = row(v, id);
    assert_eq!(queued.status, RowStatus::Queued);
    assert!(
        run_log(v, &w.slug).is_empty(),
        "nothing is recorded while it is pending"
    );
    // A Stop on the row kills the child and ends the row cancelled.
    run_queue::cancel(v, id, Utc::now()).unwrap();
    let (row_id, event, _) = handled(run_step(
        v,
        Utc::now() + Duration::seconds(5),
        None,
        &deps(),
    ));
    assert_eq!((row_id, event), (id, Some("cancelled")));
    assert_eq!(row(v, id).status, RowStatus::Cancelled);
}

#[test]
fn a_misspelt_precheck_fails_the_row_and_opens_the_streak_instead_of_skipping() {
    let dir = temp_vault();
    let v = dir.path();
    let w = seed(v, "Precheck Typo Oc13", "hollow-no-such-program-oc13 --x");
    let id = queue_cron(v, &w, Utc::now());
    let (row_id, event, _) = handled(settled_step(v, &deps()));
    assert_eq!((row_id, event), (id, Some("finished")));
    let ended = row(v, id);
    assert_eq!(
        ended.status,
        RowStatus::Failed,
        "needs attention, not a skip"
    );
    let note = ended.error.clone().unwrap_or_default();
    assert!(
        note.starts_with("precheck failed:") && note.contains("127"),
        "{note}"
    );
    let newest = run_log(v, &w.slug).pop().unwrap();
    assert_eq!(newest.status, "error");
    assert!(!vault_workflows::is_benign_skip_note(&newest.note));
    assert_eq!(
        automated_history::list_runs(v, usize::MAX).unwrap().len(),
        0,
        "no agent turn"
    );
    // Three in a row open the Workflows failure streak (the health warning),
    // and each failure is its own row, never coalesced.
    for _ in 0..2 {
        let raw = std::fs::read_to_string(v.join(&w.path)).unwrap();
        let next = next_run(v, &w.slug).unwrap();
        let due = (Utc::now() - Duration::minutes(1)).to_rfc3339();
        std::fs::write(v.join(&w.path), raw.replace(&next, &due)).unwrap();
        let again = vault_workflows::read_full(v, &w.slug).unwrap().0;
        assert_eq!(again.next_run_at.as_deref(), Some(due.as_str()));
        queue_cron(v, &again, Utc::now());
        handled(settled_step(v, &deps()));
    }
    let log = run_log(v, &w.slug);
    assert_eq!(log.len(), 3, "{log:#?}");
    assert!(log.iter().all(|e| e.status == "error"));
    let streak = workflow_hygiene::failure_streak(&log).expect("a streak");
    assert_eq!(streak.count, 3);
    assert!(
        IDLE_NOTICES.with(|sent| sent.borrow().iter().any(|n| n.kind == "error")),
        "the person is told"
    );
}
