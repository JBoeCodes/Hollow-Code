//! Publish to Main (row U8): run history, the proposed build, the runner
//! contract, the nudge's dedupe and the always-asking agent action. A fake
//! runner stands in for ssh and `promote.mjs`; nothing leaves the test.

use super::publish::*;
use super::settings::UpdateSettings;
use crate::app_actions::{self, asks_in_mode, Args, Tier, ALWAYS_ASK};
use serde_json::{json, Value};
use std::sync::Mutex;

const HOUR: u64 = 60 * 60 * 1000;

/// Answers each `promote.mjs` call with the next scripted result line.
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
    answers: Mutex<Vec<Result<Value, String>>>,
}

impl FakeRunner {
    fn new(answers: Vec<Result<Value, String>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answers: Mutex::new(answers),
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl Runner for FakeRunner {
    fn host(&self) -> &str {
        "runner-host-test"
    }

    fn promote(&self, args: &[String]) -> Result<RunnerOutput, String> {
        self.calls.lock().unwrap().push(args.to_vec());
        let next = self.answers.lock().unwrap().remove(0)?;
        Ok(RunnerOutput {
            code: Some(if next["ok"] == json!(true) { 0 } else { 1 }),
            stdout: format!("progress line\n{RESULT_PREFIX}{next}\n"),
            stderr: String::new(),
        })
    }
}

fn publisher() -> UpdateSettings {
    UpdateSettings {
        publisher: true,
        ..Default::default()
    }
}

fn run(version: &str, from: u64, to: u64) -> RunRecord {
    RunRecord {
        version: version.into(),
        first_seen_ms: from,
        last_seen_ms: to,
    }
}

const C30: &str = "3333333333333333333333333333333333333333";

fn status_answer(nudge: bool) -> Value {
    json!({
        "ok": true,
        "nightly": { "version": "0.6.30", "commit": C30, "pubDate": "2026-11-15T10:00:00Z" },
        "main": { "version": "0.6.10", "commit": "1".repeat(40), "pubDate": "2026-11-01T10:00:00Z" },
        "builds": [
            { "version": "0.6.30", "commit": C30, "pubDate": "2026-11-15T10:00:00Z" },
            { "version": "0.6.25", "commit": "2".repeat(40), "pubDate": "2026-11-14T10:00:00Z" },
            { "version": "0.6.10", "commit": "1".repeat(40), "pubDate": "2026-11-01T10:00:00Z" }
        ],
        "history": [{ "version": "0.6.10", "action": "publish", "promotedAt": "2026-11-01T11:00:00Z", "from": null }],
        "proposed": "0.6.25",
        "proposedWhy": null,
        "notes": {
            "pullRequests": [{ "number": 101, "title": "Add the frobnicator", "merge": "abcdef12", "mergedAt": "2026-11-14T00:00:00Z" }],
            "whatsNew": [{ "title": "Frobs", "what": "Frobs things." }],
            "commits": [{ "sha": "12345678", "subject": "direct" }],
            "note": null
        },
        "changesAhead": 23,
        "daysAhead": 4,
        "nudge": nudge,
        "previousMain": null,
        "canWithdraw": false
    })
}

#[test]
fn the_run_history_keeps_continuous_stretches_and_a_cap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updates").join(RUN_HISTORY);
    record_run(&path, "0.6.10", 1_000).unwrap();
    record_run(&path, "0.6.10", 1_000 + HOUR).unwrap();
    record_run(&path, "0.6.11", 2 * HOUR).unwrap();
    record_run(&path, "0.6.10", 3 * HOUR).unwrap();
    let runs = read_runs(&path).runs;
    assert_eq!(
        runs,
        vec![
            run("0.6.10", 1_000, 1_000 + HOUR),
            run("0.6.11", 2 * HOUR, 2 * HOUR),
            run("0.6.10", 3 * HOUR, 3 * HOUR),
        ],
        "coming back to a version starts a new stretch"
    );
    // Quit or asleep for longer than about two ticks: a new stretch, so the
    // hour must be one continuous run.
    let gap = dir.path().join("gap.json");
    let half = HOUR / 2;
    let back = half + MAX_RUN_GAP_MS + 1;
    record_run(&gap, "0.6.20", 0).unwrap();
    record_run(&gap, "0.6.20", half).unwrap();
    record_run(&gap, "0.6.20", back).unwrap();
    record_run(&gap, "0.6.20", back + half).unwrap();
    assert_eq!(
        read_runs(&gap).runs,
        vec![run("0.6.20", 0, half), run("0.6.20", back, back + half)],
        "a gap over MAX_RUN_GAP_MS breaks the stretch"
    );
    assert!(
        eligible_versions(&read_runs(&gap).runs, None).is_empty(),
        "two half hours with a gap are not an hour"
    );
    for i in 0..(MAX_RUNS as u64 + 5) {
        record_run(&path, &format!("0.6.{}", 100 + i), 10 * HOUR + i).unwrap();
    }
    assert_eq!(read_runs(&path).runs.len(), MAX_RUNS);
}

#[test]
fn the_proposed_build_ran_here_an_hour_without_a_go_back() {
    let runs = vec![
        run("0.6.10", 0, 2 * HOUR),
        run("0.6.12", 2 * HOUR, 2 * HOUR + 59 * 60 * 1000), // under an hour
        run("0.6.13", 3 * HOUR, 5 * HOUR),                  // went back from it (0.6.11 next)
        run("0.6.11", 5 * HOUR, 7 * HOUR),
        run("0.5.15", 7 * HOUR, 9 * HOUR), // not a channel version
        run("0.6.14", 9 * HOUR, 11 * HOUR),
        run("0.6.15", 11 * HOUR, 13 * HOUR),
    ];
    // 0.6.13 was followed by 0.6.11 (lower): went back.
    assert_eq!(
        eligible_versions(&runs, Some("0.6.15")),
        vec!["0.6.14", "0.6.11", "0.6.10"],
        "the skipped version (Go back, row U6) and a version left for a lower one are out"
    );
    assert_eq!(eligible_versions(&runs, None)[0], "0.6.15");
    assert_eq!(release_number("0.6.07"), None);
    assert_eq!(release_number("0.6.1x"), None);
    assert_eq!(release_number("0.6.1234"), Some(1234));
}

#[test]
fn status_asks_the_runner_only_for_a_publisher_with_one() {
    let runner = FakeRunner::new(vec![]);
    let off = publish_status(&UpdateSettings::default(), Some(&runner), &[], None);
    assert!(!off.publisher && !off.available);
    assert_eq!(off.unavailable_reason.as_deref(), Some(NOT_PUBLISHER));
    let none = publish_status(&publisher(), None, &[], None);
    assert_eq!(none.unavailable_reason.as_deref(), Some(NO_RUNNER));
    assert!(
        runner.calls().is_empty(),
        "no ssh for a Mac that doesn't publish"
    );

    let runner = FakeRunner::new(vec![Ok(status_answer(true))]);
    let runs = vec![
        run("0.6.25", 0, 2 * HOUR),
        run("0.6.30", 2 * HOUR, 2 * HOUR + 1),
    ];
    let s = publish_status(&publisher(), Some(&runner), &runs, None);
    assert_eq!(
        runner.calls(),
        vec![vec![
            "status".to_string(),
            "--eligible".into(),
            "0.6.25".into()
        ]]
    );
    assert!(s.available, "{:?}", s.unavailable_reason);
    assert_eq!(s.runner.as_deref(), Some("runner-host-test"));
    assert_eq!(s.proposed.as_deref(), Some("0.6.25"));
    assert_eq!(s.main.as_ref().unwrap().version, "0.6.10");
    assert_eq!(s.builds.len(), 3);
    assert!(s.builds[1].ran_here && !s.builds[0].ran_here);
    assert!(s.builds[0].newer_than_main && !s.builds[2].newer_than_main);
    assert_eq!(s.notes.as_ref().unwrap().pull_requests[0].number, 101);
    assert_eq!(s.changes_ahead, Some(23));
    assert_eq!(s.history[0].action, "publish");

    let picked = FakeRunner::new(vec![Ok(status_answer(false))]);
    publish_status(&publisher(), Some(&picked), &[], Some("0.6.30"));
    assert_eq!(
        picked.calls()[0],
        vec!["status".to_string(), "--pick".into(), "0.6.30".into()]
    );
    let bad = publish_status(&publisher(), Some(&picked), &[], Some("0.6.30; rm -rf /"));
    assert!(!bad.available);
    assert_eq!(
        picked.calls().len(),
        1,
        "a bad pick never reaches the runner"
    );

    let down = FakeRunner::new(vec![Err(
        "The runner (runner-host-test) didn't answer.".into()
    )]);
    let s = publish_status(&publisher(), Some(&down), &[], None);
    assert!(!s.available);
    assert!(s.unavailable_reason.unwrap().contains("didn't answer"));
    let refused = FakeRunner::new(vec![Ok(
        json!({ "ok": false, "code": "config", "error": "no builder" }),
    )]);
    let s = publish_status(&publisher(), Some(&refused), &[], None);
    assert_eq!(s.unavailable_reason.as_deref(), Some("no builder"));
}

#[test]
fn publish_and_withdraw_go_through_the_runner_with_their_binding() {
    let runner = FakeRunner::new(vec![]);
    let c25 = "2".repeat(40);
    let refused = publish_main(
        &UpdateSettings::default(),
        Some(&runner),
        "0.6.25",
        &c25,
        Some("0.6.10"),
    )
    .unwrap();
    assert!(
        matches!(refused, UpdatePublishOutcome::Refused { ref code, .. } if code == "not-publisher")
    );
    let refused = withdraw_main(&UpdateSettings::default(), Some(&runner), None).unwrap();
    assert!(
        matches!(refused, UpdatePublishOutcome::Refused { ref code, .. } if code == "not-publisher")
    );
    let bad = publish_main(&publisher(), Some(&runner), "latest", &c25, None).unwrap();
    assert!(matches!(bad, UpdatePublishOutcome::Refused { ref code, .. } if code == "usage"));
    let bad = publish_main(&publisher(), Some(&runner), "0.6.25", "abc", None).unwrap();
    assert!(matches!(bad, UpdatePublishOutcome::Refused { ref code, .. } if code == "usage"));
    let bad = publish_main(&publisher(), Some(&runner), "0.6.25", "", None).unwrap();
    assert!(
        matches!(bad, UpdatePublishOutcome::Refused { ref code, .. } if code == "usage"),
        "the commit is required"
    );
    let bad = publish_main(&publisher(), Some(&runner), "0.6.25", &c25, Some("x")).unwrap();
    assert!(matches!(bad, UpdatePublishOutcome::Refused { ref code, .. } if code == "usage"));
    assert!(runner.calls().is_empty());

    let runner = FakeRunner::new(vec![
        Ok(
            json!({ "ok": true, "action": "publish", "version": "0.6.25", "from": "0.6.10", "at": "2026-11-15T12:00:00Z", "historyError": null }),
        ),
        Ok(json!({ "ok": false, "code": "signature", "error": "the .sig doesn't match" })),
        Ok(
            json!({ "ok": true, "action": "withdraw", "version": "0.6.10", "from": "0.6.25", "at": "2026-11-15T12:05:00Z", "historyError": null }),
        ),
    ]);
    let done = publish_main(&publisher(), Some(&runner), "0.6.25", &c25, Some("0.6.10")).unwrap();
    assert_eq!(
        done,
        UpdatePublishOutcome::Published {
            version: "0.6.25".into(),
            from: Some("0.6.10".into()),
            at: "2026-11-15T12:00:00Z".into(),
            history_warning: None
        }
    );
    let no = publish_main(&publisher(), Some(&runner), "0.6.25", &c25, None).unwrap();
    assert!(matches!(no, UpdatePublishOutcome::Refused { ref code, .. } if code == "signature"));
    let back = withdraw_main(&publisher(), Some(&runner), Some("0.6.25")).unwrap();
    assert!(
        matches!(back, UpdatePublishOutcome::Withdrawn { ref version, ref from, .. } if version == "0.6.10" && from == "0.6.25")
    );
    assert_eq!(
        runner.calls(),
        vec![
            vec![
                "promote".to_string(),
                "0.6.25".into(),
                "--expect-commit".into(),
                c25.clone(),
                "--expect-main".into(),
                "0.6.10".into(),
            ],
            vec![
                "promote".to_string(),
                "0.6.25".into(),
                "--expect-commit".into(),
                c25.clone(),
                "--expect-main".into(),
                "none".into(),
            ],
            vec![
                "withdraw".to_string(),
                "--expect-version".into(),
                "0.6.25".into()
            ],
        ]
    );
    // The serialized shape the window reads.
    assert_eq!(
        serde_json::to_value(&back).unwrap()["outcome"],
        json!("withdrawn")
    );
}

/// Answers every call with one fixed output (exit code and all).
struct RawRunner(RunnerOutput);

impl Runner for RawRunner {
    fn host(&self) -> &str {
        "runner-host-test"
    }

    fn promote(&self, _args: &[String]) -> Result<RunnerOutput, String> {
        Ok(self.0.clone())
    }
}

fn raw(code: Option<i32>, result: Value) -> RawRunner {
    RawRunner(RunnerOutput {
        code,
        stdout: format!("{RESULT_PREFIX}{result}\n"),
        stderr: String::new(),
    })
}

#[test]
fn a_result_without_a_version_or_with_a_failing_exit_is_not_success() {
    let c = "2".repeat(40);
    let done = json!({ "ok": true, "action": "publish", "version": "0.6.25", "from": null, "at": "2026-11-15T12:00:00Z" });
    let cases = [
        (Some(1), done.clone(), "a non-zero exit"),
        (None, done.clone(), "killed by a signal"),
        (
            Some(0),
            json!({ "ok": true, "action": "publish", "at": "2026-11-15T12:00:00Z" }),
            "no version",
        ),
        (
            Some(0),
            json!({ "ok": true, "action": "publish", "version": "", "at": "2026-11-15T12:00:00Z" }),
            "an empty version",
        ),
        (
            Some(0),
            json!({ "ok": true, "action": "publish", "version": "0.6.25" }),
            "no time",
        ),
        (
            Some(0),
            json!({ "ok": true, "version": "0.6.25", "at": "2026-11-15T12:00:00Z" }),
            "no action",
        ),
    ];
    for (code, result, why) in cases {
        let out = publish_main(
            &publisher(),
            Some(&raw(code, result.clone())),
            "0.6.25",
            &c,
            None,
        )
        .unwrap();
        assert!(
            matches!(out, UpdatePublishOutcome::Refused { ref code, .. } if code == "failed"),
            "{why}: {out:?}"
        );
        let out = withdraw_main(&publisher(), Some(&raw(code, result)), None).unwrap();
        assert!(
            matches!(out, UpdatePublishOutcome::Refused { ref code, .. } if code == "failed"),
            "withdraw, {why}: {out:?}"
        );
    }
    assert!(matches!(
        publish_main(&publisher(), Some(&raw(Some(0), done)), "0.6.25", &c, None).unwrap(),
        UpdatePublishOutcome::Published { .. }
    ));
}

#[test]
fn the_runners_stderr_is_redacted_before_it_reaches_the_card() {
    let token = format!("ghp_{}", "A".repeat(36));
    let runner = RawRunner(RunnerOutput {
        code: Some(1),
        stdout: String::new(),
        stderr: format!("fatal: could not read from https://x:{token}@github.com/o/r"),
    });
    let s = publish_status(&publisher(), Some(&runner), &[], None);
    let why = s.unavailable_reason.unwrap();
    assert!(why.contains("said nothing Hollow could read"), "{why}");
    assert!(!why.contains(&token), "{why}");
}

#[test]
fn a_failed_fetch_on_the_runner_stops_the_call_and_says_so() {
    // The real remote command under /bin/sh, with a builder env file whose
    // clone's origin doesn't exist: the fetch fails at once (no network).
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("repo");
    std::fs::create_dir_all(home.path().join(".config/hollow-nightly")).unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    };
    std::fs::create_dir_all(&repo).unwrap();
    git(&["init", "-q"]);
    git(&[
        "remote",
        "add",
        "origin",
        home.path().join("missing").to_str().unwrap(),
    ]);
    std::fs::write(
        home.path().join(".config/hollow-nightly/env"),
        format!("HOLLOW_NIGHTLY_REPO={}\n", repo.display()),
    )
    .unwrap();
    let out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(remote_command(&["status".into()]))
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .output()
        .unwrap();
    let out = RunnerOutput {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    };
    assert_eq!(out.code, Some(2), "{out:?}");
    let v = parse_result(&out).unwrap();
    assert_eq!(v["code"], json!("fetch"), "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("could not fetch origin/main within 60 s"));
    let s = publish_status(&publisher(), Some(&RawRunner(out)), &[], None);
    assert!(s.unavailable_reason.unwrap().contains("could not fetch"));
    assert!(stalled_message("runner-host-test").contains("within 120 s"));
}

#[test]
fn the_remote_command_quotes_every_argument_and_holds_no_secret() {
    let cmd = remote_command(&["promote".into(), "0.6.1'; touch /tmp/x; '".into()]);
    assert!(cmd.contains("'0.6.1'\\''; touch /tmp/x; '\\'''"), "{cmd}");
    assert!(cmd.contains(".config/hollow-nightly/env"));
    assert!(cmd.contains("archive refs/remotes/origin/main scripts/release"));
    assert!(
        cmd.contains("( sleep 60; kill $p )"),
        "the fetch has a deadline"
    );
    assert!(!cmd.contains("HOLLOW_R2_SECRET"));
    let out = RunnerOutput {
        code: Some(127),
        stdout: String::new(),
        stderr: "zsh: command not found: node".into(),
    };
    assert!(parse_result(&out)
        .unwrap_err()
        .contains("command not found: node"));
}

#[test]
fn the_nudge_posts_once_per_main_a_day_apart_and_asks_the_runner_at_most_every_six_hours() {
    let dir = tempfile::tempdir().unwrap();
    let paths = PublishPaths::in_dir(dir.path());
    let mut main_moved = status_answer(true);
    main_moved["nightly"]["version"] = json!("0.6.31");
    main_moved["main"]["version"] = json!("0.6.25");
    let runner = FakeRunner::new(vec![
        Ok(status_answer(true)),
        Ok(status_answer(true)),
        Err("down".into()),
        Ok({
            let mut v = status_answer(true);
            v["nightly"]["version"] = json!("0.6.31");
            v
        }),
        Ok(main_moved.clone()),
        Ok(main_moved),
    ]);
    let posted: std::cell::RefCell<Vec<String>> = Default::default();
    let mut notify = |title: String, _body: String| posted.borrow_mut().push(title);
    let six = NUDGE_CHECK_EVERY.as_millis() as u64;
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000,
        &mut notify,
    );
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + HOUR,
        &mut notify,
    );
    assert_eq!(runner.calls().len(), 1, "not again within six hours");
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + six,
        &mut notify,
    );
    assert_eq!(runner.calls().len(), 2);
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + 2 * six,
        &mut notify,
    );
    assert_eq!(
        runner.calls().len(),
        3,
        "a runner that didn't answer is asked again next tick"
    );
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + 2 * six + 1,
        &mut notify,
    );
    assert_eq!(runner.calls().len(), 4);
    assert_eq!(
        posted.borrow().len(),
        1,
        "a new Nightly against the same Main is not a new nudge"
    );
    // Main moved to 0.6.25, 18 hours after the nudge: still under a day.
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + 3 * six + 1,
        &mut notify,
    );
    assert_eq!(runner.calls().len(), 5);
    assert_eq!(posted.borrow().len(), 1, "at least 24 hours between nudges");
    tick(
        &publisher(),
        &paths,
        Some(&runner),
        "0.6.30",
        1_000 + 4 * six + 1,
        &mut notify,
    );
    assert_eq!(
        *posted.borrow(),
        vec![
            "Nightly is 23 changes ahead of Main: review and publish".to_string(),
            "Nightly is 23 changes ahead of Main: review and publish".to_string(),
        ],
        "once against Main 0.6.10, once against 0.6.25 a day later"
    );
    let state = read_nudge(&paths.nudge);
    assert_eq!(state.nudged_for.as_deref(), Some("0.6.31"));
    assert_eq!(state.nudged_main.as_deref(), Some("0.6.25"));
    assert_eq!(state.last_nudge_ms, Some(1_000 + 4 * six + 1));
    assert_eq!(
        read_runs(&paths.runs).runs[0].version,
        "0.6.30",
        "the tick records the running version"
    );

    // Not a publisher: only the run history, no runner call.
    let quiet = FakeRunner::new(vec![]);
    tick(
        &UpdateSettings::default(),
        &paths,
        Some(&quiet),
        "0.6.30",
        9 * six,
        &mut |_, _| panic!("no nudge"),
    );
    assert!(quiet.calls().is_empty());

    // The rule itself.
    let mut status = publish_status(
        &publisher(),
        Some(&FakeRunner::new(vec![Ok(status_answer(false))])),
        &[],
        None,
    );
    assert_eq!(
        nudge_for(&status, &NudgeState::default(), 0),
        None,
        "not far enough ahead"
    );
    status.nudge = true;
    status.changes_ahead = None;
    let (_, title, body) = nudge_for(&status, &NudgeState::default(), 0).unwrap();
    assert_eq!(title, "Nightly is 4 days ahead of Main: review and publish");
    assert!(body.contains("Main is 0.6.10"));
    assert!(
        nudge_check_due(
            &NudgeState {
                last_check_ms: Some(5_000),
                ..Default::default()
            },
            1_000
        ),
        "a clock set back checks"
    );
}

#[test]
fn the_card_names_the_build_and_binds_its_commit() {
    let status = publish_status(
        &publisher(),
        Some(&FakeRunner::new(vec![Ok(status_answer(false))])),
        &[],
        None,
    );
    let card = publish_card_for(&status, "0.6.30").unwrap();
    let text = card.text();
    assert!(
        text.contains("Publish Hollow 0.6.30 (commit 333333333333) to Main"),
        "{text}"
    );
    assert!(text.contains("Main now serves 0.6.10."));
    assert!(text.contains("#101 Add the frobnicator"));
    assert!(text.contains("It hasn't run on this Mac for an hour yet."));
    assert!(text.contains("Destination: the update host: channels/main.json"));
    assert_eq!(card.kind(), "app-external");
    assert_eq!(
        card.bind,
        Some(json!({ "version": "0.6.30", "commit": C30, "main": "0.6.10" }))
    );
    let same = publish_card_for(&status, "0.6.10").unwrap_err();
    assert!(same.contains("Main already serves 0.6.10"), "{same}");
    let mut with_older = status.clone();
    with_older.builds.push(UpdatePublishBuild {
        version: "0.6.8".into(),
        commit: Some("8".repeat(40)),
        pub_date: None,
        ran_here: true,
        newer_than_main: false,
    });
    let older = publish_card_for(&with_older, "0.6.8").unwrap_err();
    assert!(
        older.contains("0.6.8 is older than the live Main 0.6.10"),
        "{older}"
    );
    assert!(publish_card_for(&status, "0.6.99")
        .unwrap_err()
        .contains("isn't a kept Nightly"));
    let off = publish_status(&UpdateSettings::default(), None, &[], None);
    assert!(publish_card_for(&off, "0.6.30")
        .unwrap_err()
        .contains("doesn't publish to Main"));

    // The handler runs only with the card's own build.
    assert!(approved_publish(None, "0.6.30")
        .unwrap_err()
        .contains("runs only with the build its card recorded"));
    assert!(approved_publish(Some(&card), "0.6.25")
        .unwrap_err()
        .contains("The approved build is 0.6.30"));
    let bind = approved_publish(Some(&card), "0.6.30").unwrap();
    assert_eq!(bind.commit, C30);
    assert_eq!(
        bind.main.as_deref(),
        Some("0.6.10"),
        "the handler passes --expect-main"
    );
}

/// Publishing to Main asks on its card in every mode, Bypass included, like
/// installing Hollow (design section 10: only his click publishes).
#[test]
fn publishing_to_main_asks_in_every_mode_bypass_included() {
    let none = Args(serde_json::Map::new());
    let action = app_actions::find("build.publish_main").expect("the action exists");
    assert!(ALWAYS_ASK.contains(&"build.publish_main"));
    assert_eq!(action.tier, Tier::External);
    assert!(action.card.is_some());
    assert_eq!(action.command, Some("update_publish_main"));
    for mode in [
        "bypassPermissions",
        "auto",
        "acceptEdits",
        "default",
        "plan",
    ] {
        assert!(asks_in_mode(action, &none, mode), "asks in {mode}");
    }
    assert!(
        crate::mode_contract::app_cards("bypassPermissions", false).contains("build.publish_main")
    );
    let dir = tempfile::tempdir().unwrap();
    assert!(crate::main_agent::unattended::allows(
        dir.path(),
        "build.publish_main",
        false,
        &serde_json::Map::new(),
        true
    )
    .is_err());
    let read = app_actions::find("build.publish_status").expect("the read exists");
    assert_eq!(read.tier, Tier::Read);
}
