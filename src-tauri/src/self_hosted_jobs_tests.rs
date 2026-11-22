//! Self-hosted models Q10: background jobs on a self-hosted model (attached
//! from `self_hosted_jobs.rs` as `#[path] mod tests`, so `super::` is the
//! module). Every file here is in a temp folder, never the app's own, and
//! nothing calls an endpoint but a stub on loopback.
use super::*;
use crate::self_hosted_registry::{preset, SelfHostedAuth};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

#[test]
fn automatic_capture_waits_as_one_private_job_per_chat_and_keeps_the_newest_run() {
    let files = Files::new();
    let mut first = Job::learning(
        Path::new("/fixture-vault"),
        "A first request",
        &[],
        "Answer",
        Some("capture-one"),
        false,
        Some("private-model"),
    );
    if let Job::Learning {
        capture_session, ..
    } = &mut first
    {
        *capture_session = Some("s-capture-chat".into());
    }
    let mut newest = first.clone();
    if let Job::Learning { run_id, user, .. } = &mut newest {
        *run_id = Some("capture-two".into());
        *user = "A newer request".into();
    }
    assert_eq!(first.key(), newest.key());
    enqueue_at(&files.queue(), "private-model", first, Utc::now()).unwrap();
    enqueue_at(&files.queue(), "private-model", newest, Utc::now()).unwrap();
    let queued = files.queued();
    assert_eq!(queued.len(), 1);
    let Job::Learning {
        run_id,
        user,
        only_on,
        capture_session,
        ..
    } = &queued[0].job
    else {
        panic!("capture job");
    };
    assert_eq!(run_id.as_deref(), Some("capture-two"));
    assert_eq!(user, "A newer request");
    assert_eq!(only_on.as_deref(), Some("private-model"));
    assert_eq!(capture_session.as_deref(), Some("s-capture-chat"));
}

/// A temp folder holding the setting and the queue.
struct Files {
    dir: tempfile::TempDir,
}

impl Files {
    fn new() -> Files {
        Files {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn jobs(&self) -> PathBuf {
        self.dir.path().join(JOBS_FILE)
    }
    fn queue(&self) -> PathBuf {
        self.dir.path().join(QUEUE_FILE)
    }
    fn route(&self, kind: JobKind, id: &str) {
        self.set(kind, Some(id));
    }
    /// Send `kind` to `id`, or As now (`None`), without checking the list.
    fn set(&self, kind: JobKind, id: Option<&str>) {
        let mut jobs = read_jobs_at(&self.jobs()).unwrap_or_default();
        match id {
            Some(id) => jobs.insert(kind.id().into(), id.into()),
            None => jobs.remove(kind.id()),
        };
        let text = serde_json::json!({ "version": 1, "jobs": jobs }).to_string();
        std::fs::write(self.jobs(), text).unwrap();
    }
    fn queued(&self) -> Vec<Queued> {
        read_queue_at(&self.queue()).unwrap()
    }
}

/// A saved entry `id` on loopback (never contacted unless a test says so).
fn entry(id: &str, enabled: bool) -> SelfHostedEntry {
    SelfHostedEntry {
        id: id.into(),
        name: format!("Model {id}"),
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        enabled,
        ..preset()
    }
}

/// A unique entry id for tests that start a forwarder.
fn unique(name: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "q10-{name}-{}-{}",
        std::process::id() % 10_000,
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

fn decide_in(
    files: &Files,
    kind: JobKind,
    only_on: Option<&str>,
    job: Option<Job>,
    entries: &dyn Fn() -> Result<Vec<SelfHostedEntry>, String>,
    warm: bool,
) -> Decision {
    decide_with(
        kind,
        only_on,
        || job,
        &Env {
            jobs: &files.jobs(),
            queue: &files.queue(),
            entries,
            warm: &|_: &str| warm,
            now: Utc::now(),
            asked: Asked::Auto,
        },
    )
}

fn profile(vault: &str) -> Job {
    Job::Profile {
        vault: PathBuf::from(vault),
    }
}

// ── The setting ─────────────────────────────────────────────────────────

/// Review: a setting that exists but can't be read fails closed. Every kind
/// is skipped with a note (never As now, so nothing private goes to
/// Anthropic or Codex), the registry isn't read, Settings says the jobs are
/// held and still shows what it can read of the routes, and no change
/// overwrites the file. Only a missing file is As now.
#[test]
fn a_damaged_setting_holds_every_kind() {
    let files = Files::new();
    assert_eq!(read_jobs_at(&files.jobs()), Ok(BTreeMap::new()));
    let none = || Ok(vec![entry("qwen", true)]);
    for kind in JobKind::ALL {
        assert_eq!(
            decide_in(&files, kind, None, Some(profile("/v")), &none, false),
            Decision::AsNow,
            "a missing file is As now"
        );
    }
    std::fs::write(
        files.jobs(),
        r#"{"version":1,"jobs":{"extraction":"qwen","later":"x"}}"#,
    )
    .unwrap();
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(entry_for(&jobs, JobKind::Extraction), Some("qwen"));
    assert_eq!(entry_for(&jobs, JobKind::Titles), None);
    for (text, says, shown) in [
        ("{not json", "isn't valid JSON", None),
        (
            r#"{"version":2,"jobs":{"extraction":"qwen"}}"#,
            "is version 2",
            Some("qwen"),
        ),
        (
            r#"{"jobs":{"extraction":"qwen"}}"#,
            "has no version number",
            Some("qwen"),
        ),
        (
            r#"{"version":1,"jobs":["qwen"]}"#,
            "doesn't match version 1",
            None,
        ),
    ] {
        std::fs::write(files.jobs(), text).unwrap();
        let why = read_jobs_at(&files.jobs()).unwrap_err();
        assert!(
            why.contains(says) && why.contains(JOBS_FILE),
            "{text}: {why}"
        );
        let reads = AtomicUsize::new(0);
        let entries = || {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok(vec![entry("qwen", true)])
        };
        for kind in JobKind::ALL {
            for only_on in [None, Some("qwen")] {
                let Decision::Held(Held::Skipped(note)) =
                    decide_in(&files, kind, only_on, Some(profile("/v")), &entries, true)
                else {
                    panic!("{text}: {kind:?} isn't held");
                };
                assert!(note.starts_with(kind.label()), "{note}");
                assert!(
                    note.contains("held while self-hosted-jobs.json can't be read"),
                    "{note}"
                );
                assert!(note.contains(says), "{note}");
            }
        }
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(!files.queue().exists(), "nothing queued either");
        // Every change is refused, and the file is never overwritten.
        let refused = set_at(
            &files.jobs(),
            Ok(vec![entry("qwen", true)]),
            JobKind::Titles,
            Some("qwen"),
        )
        .unwrap_err();
        assert!(refused.contains(says), "{refused}");
        assert!(set_at(&files.jobs(), Ok(vec![]), JobKind::Titles, None).is_err());
        assert_eq!(std::fs::read_to_string(files.jobs()).unwrap(), text);
        // Settings: the error, every row held, the routes it can still read.
        let view = view_with(&files.jobs(), &files.queue(), Ok(vec![entry("qwen", true)]));
        assert!(view.error.as_deref().is_some_and(|e| e.contains(says)));
        for row in &view.kinds {
            assert!(
                row.problem
                    .as_deref()
                    .is_some_and(|p| p.starts_with("Held:")),
                "{row:?}"
            );
        }
        let extraction = view
            .kinds
            .iter()
            .find(|r| r.kind == JobKind::Extraction)
            .unwrap();
        assert_eq!(extraction.entry.as_deref(), shown, "{text}");
        assert!(!routes_to_at(&files.jobs(), JobKind::Titles, "qwen"));
    }
}

#[test]
fn a_change_saves_version_one_and_checks_the_entry() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true), entry("off", false)]);
    set_at(&files.jobs(), list(), JobKind::Extraction, Some("qwen")).unwrap();
    set_at(&files.jobs(), list(), JobKind::Learning, Some(" qwen ")).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(files.jobs()).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"version": 1, "jobs": {"extraction": "qwen", "learning": "qwen"}})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(files.jobs())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(routes_to_at(&files.jobs(), JobKind::Learning, "qwen"));
    assert!(!routes_to_at(&files.jobs(), JobKind::Titles, "qwen"));
    // As now removes the line.
    set_at(&files.jobs(), list(), JobKind::Learning, None).unwrap();
    assert_eq!(read_jobs_at(&files.jobs()).unwrap().len(), 1);
    // Only a listed, switched-on Anthropic-style entry.
    for (id, list, says) in [
        ("nope", list(), "there is no self-hosted model \"nope\""),
        ("off", list(), "Model off is switched off"),
        (
            "qwen",
            Ok(vec![SelfHostedEntry {
                api_style: "openai".into(),
                ..entry("qwen", true)
            }]),
            "uses the openai API style",
        ),
        ("qwen", Err("bad file".into()), "can't be read (bad file)"),
    ] {
        let refused = set_at(&files.jobs(), list, JobKind::Titles, Some(id)).unwrap_err();
        assert!(
            refused.contains(says) && refused.ends_with("Nothing changed."),
            "{refused}"
        );
    }
    assert_eq!(read_jobs_at(&files.jobs()).unwrap().len(), 1);
    // A key this Hollow doesn't know is kept through a change.
    std::fs::write(files.jobs(), r#"{"version":1,"jobs":{"later":"qwen"}}"#).unwrap();
    set_at(&files.jobs(), list(), JobKind::Heartbeat, Some("qwen")).unwrap();
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(jobs.get("later").map(String::as_str), Some("qwen"));
    assert_eq!(entry_for(&jobs, JobKind::Heartbeat), Some("qwen"));
}

#[test]
fn job_kinds_round_trip_their_ids() {
    for kind in JobKind::ALL {
        assert_eq!(JobKind::from_id(kind.id()), Some(kind));
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::json!(kind.id())
        );
    }
    assert_eq!(JobKind::from_id("gardener"), None);
}

// ── Routing ─────────────────────────────────────────────────────────────

/// As now: no setting (or a kind it doesn't name) never reads the registry
/// or the queue, and the one-shot's spec is the one it always was.
#[test]
fn as_now_is_unchanged_byte_for_byte() {
    let files = Files::new();
    let reads = AtomicUsize::new(0);
    let entries = || {
        reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![entry("qwen", true)])
    };
    files.route(JobKind::Heartbeat, "qwen");
    for kind in JobKind::ALL
        .into_iter()
        .filter(|k| *k != JobKind::Heartbeat)
    {
        assert_eq!(
            decide_in(&files, kind, None, Some(profile("/v")), &entries, false),
            Decision::AsNow
        );
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!files.queue().exists());

    // The one-shot spec as Q10 left it, against the literal before Q10.
    let dir = tempfile::tempdir().unwrap();
    let opts = crate::claude_runner::RunOpts {
        cwd: dir.path(),
        timeout: Duration::from_secs(60),
        allowed_tools: &[],
        disallowed_tools: crate::claude_runner::NO_TOOLS,
        model: None,
        debug_log: None,
        meter: crate::usage_ledger::Meter::oneshot("learning_extract"),
        self_hosted: None,
    };
    let now =
        crate::claude_runner::oneshot_spec(Some("claude-haiku-5-5".into()), "Title it", &opts);
    let before = crate::agent::RunSpec {
        prompt: "Title it".into(),
        resume: None,
        model: Some("claude-haiku-5-5".into()),
        perm: "bypassPermissions".into(),
        cwd: dir.path().to_path_buf(),
        allowed_tools: vec![],
        disallowed_tools: crate::claude_runner::NO_TOOLS
            .iter()
            .map(|s| s.to_string())
            .collect(),
        interactive: false,
        mcp_config: None,
        append_system_prompt: None,
        process_scope: crate::agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        extras: crate::agent::RunExtras::default(),
    };
    assert_eq!(now.extras, before.extras);
    assert_eq!(
        (
            now.prompt.as_str(),
            now.model.as_deref(),
            now.perm.as_str(),
            &now.cwd
        ),
        (
            before.prompt.as_str(),
            before.model.as_deref(),
            before.perm.as_str(),
            &before.cwd
        )
    );
    assert_eq!(
        (&now.allowed_tools, &now.disallowed_tools),
        (&before.allowed_tools, &before.disallowed_tools)
    );
    assert!(now.resume.is_none() && !now.interactive && now.mcp_config.is_none());
    assert!(now.append_system_prompt.is_none() && !now.usage_stream);
    assert!(now.max_budget_usd.is_none());
    assert_eq!(now.process_scope, before.process_scope);
    let argv = |spec: &crate::agent::RunSpec| -> Vec<String> {
        let cmd =
            crate::agent::command_without_codex_listing(crate::agent::AgentKind::Claude, spec);
        // R5b: the appended prompt's descriptor number differs per build, so
        // compare it as the inline flag.
        crate::secret_fds::expand_append(
            cmd.as_std()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
        )
    };
    assert_eq!(argv(&now), argv(&before));
}

/// A routed job on an awake entry: the Claude harness on this Mac, the
/// served model, the forwarder and a key minted for the job, no Anthropic
/// credential; the key is refused once the job lets go of its run.
#[tokio::test]
async fn a_routed_job_runs_on_the_entry_with_its_own_key() {
    let files = Files::new();
    let id = unique("routed");
    let saved = entry(&id, true);
    let list = {
        let saved = saved.clone();
        move || Ok(vec![saved.clone()])
    };
    files.route(JobKind::Learning, &id);
    let warm = match decide_in(&files, JobKind::Learning, None, None, &list, true) {
        Decision::Warm(warm) => warm,
        other => panic!("{other:?}"),
    };
    assert_eq!(warm.entry, saved);
    let run = attach_with(warm, &list).await.unwrap();
    let key = run.target.run_key.as_str().to_string();
    let fwd = forwarder::get(&id).expect("the entry's forwarder");
    assert_eq!(fwd.live_keys(), 1);
    assert_eq!(run.target.served_model, "qwen3.8-27b");

    let dir = tempfile::tempdir().unwrap();
    let opts = crate::claude_runner::RunOpts {
        cwd: dir.path(),
        timeout: Duration::from_secs(60),
        allowed_tools: &[],
        disallowed_tools: crate::claude_runner::NO_TOOLS,
        model: None,
        debug_log: None,
        meter: crate::usage_ledger::Meter::oneshot("learning_extract"),
        self_hosted: None,
    };
    let (agent, model, opts) = on_run(&run, opts);
    assert_eq!(agent, crate::agent::AgentKind::Claude);
    let spec = crate::claude_runner::oneshot_spec(model, "Learn", &opts);
    assert_eq!(spec.model.as_deref(), Some("qwen3.8-27b"));
    assert_eq!(spec.extras.self_hosted.as_ref(), Some(&run.target));
    assert!(crate::agent::validate_run_spec(agent, &spec).is_ok());
    assert_eq!(
        crate::agent::account_route(agent, &spec),
        crate::agent::AccountRoute::None
    );
    let cmd = crate::agent::command_without_codex_listing(agent, &spec);
    assert!(!crate::agent::is_unavailable_command(&cmd));
    let env: HashMap<String, Option<String>> = cmd
        .as_std()
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    let set = |name: &str| env.get(name).cloned().flatten();
    assert_eq!(set("ANTHROPIC_AUTH_TOKEN").as_deref(), Some(key.as_str()));
    assert_eq!(set("ANTHROPIC_BASE_URL"), Some(fwd.base_url()));
    assert_eq!(set("ANTHROPIC_MODEL").as_deref(), Some("qwen3.8-27b"));
    for gone in ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"] {
        assert_eq!(env.get(gone), Some(&None), "{gone} is removed");
    }
    let argv: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at = argv.iter().position(|a| a == "--model").unwrap();
    assert_eq!(argv[at + 1], "qwen3.8-27b");
    assert!(!argv.iter().any(|a| a.contains(&key)), "never on argv");

    drop(opts);
    drop(spec);
    drop(cmd);
    drop(run);
    assert_eq!(fwd.live_keys(), 0, "the job's key is revoked when it ends");
    forwarder::stop(&id);
}

/// A routed job whose entry can't run it is skipped with a note, never run
/// as now and never queued.
#[test]
fn a_missing_or_switched_off_entry_skips_the_job() {
    let files = Files::new();
    files.route(JobKind::Extraction, "qwen");
    let cases: Vec<(Result<Vec<SelfHostedEntry>, String>, &str)> = vec![
        (Ok(vec![]), "there is no self-hosted model \"qwen\""),
        (Ok(vec![entry("qwen", false)]), "Model qwen is switched off"),
        (
            Ok(vec![SelfHostedEntry {
                api_style: "openai".into(),
                ..entry("qwen", true)
            }]),
            // Self-hosted models Q9: an `openai` entry runs on Codex, so a
            // kind sent to one before it changed style is held, never run
            // As now.
            "background jobs run on Claude, so choose a model with Anthropic's Messages API",
        ),
        (Err("broken".into()), "can't be read (broken)"),
    ];
    for (list, says) in cases {
        for warm in [false, true] {
            let asked = AtomicUsize::new(0);
            let decision = decide_with(
                JobKind::Extraction,
                None,
                || {
                    asked.fetch_add(1, Ordering::SeqCst);
                    Some(profile("/v"))
                },
                &Env {
                    jobs: &files.jobs(),
                    queue: &files.queue(),
                    entries: &|| list.clone(),
                    warm: &|_: &str| warm,
                    now: Utc::now(),
                    asked: Asked::Auto,
                },
            );
            let Decision::Held(Held::Skipped(note)) = decision else {
                panic!("{says}: {decision:?}");
            };
            assert!(
                note.starts_with("Recording extraction was skipped"),
                "{note}"
            );
            assert!(note.contains(says), "{note}");
            assert!(note.contains("never runs on another model"), "{note}");
            assert_eq!(asked.load(Ordering::SeqCst), 0);
        }
    }
    assert!(!files.queue().exists());
    // Settings' row shows the problem.
    let view = view_with(
        &files.jobs(),
        &files.queue(),
        Ok(vec![entry("qwen", false)]),
    );
    let row = view
        .kinds
        .iter()
        .find(|r| r.kind == JobKind::Extraction)
        .unwrap();
    assert_eq!(row.entry.as_deref(), Some("qwen"));
    assert!(row
        .problem
        .as_deref()
        .is_some_and(|p| p.starts_with("Skipped while Model qwen is switched off")));
    assert!(view
        .kinds
        .iter()
        .filter(|r| r.kind != JobKind::Extraction)
        .all(|r| r.entry.is_none() && r.problem.is_none()));
}

/// Q10 decision 5: a self-hosted chat's own retitle and learning run on its
/// entry only when that kind goes there, and stay skipped otherwise.
#[test]
fn a_self_hosted_chats_own_jobs_run_only_on_its_entry() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true), entry("other", true)]);
    for kind in [JobKind::Titles, JobKind::Learning] {
        let skipped = |d: Decision| matches!(d, Decision::Held(Held::Skipped(note)) if note.contains("for this chat stays skipped"));
        // As now: skipped, never on Anthropic.
        assert!(skipped(decide_in(
            &files,
            kind,
            Some("qwen"),
            None,
            &list,
            true
        )));
        // Another entry: skipped.
        files.route(kind, "other");
        assert!(skipped(decide_in(
            &files,
            kind,
            Some("qwen"),
            None,
            &list,
            true
        )));
        // Its own entry: runs there (or waits there).
        files.route(kind, "qwen");
        assert!(matches!(
            decide_in(&files, kind, Some("qwen"), None, &list, true),
            Decision::Warm(Warm { entry, .. }) if entry.id == "qwen"
        ));
        assert!(routes_to_at(&files.jobs(), kind, "qwen"));
    }
}

// ── The queue ───────────────────────────────────────────────────────────

#[test]
fn a_cold_entry_queues_the_job_and_a_warm_one_runs_it() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true)]);
    files.route(JobKind::Profile, "qwen");
    files.route(JobKind::Learning, "qwen");
    // Awake: runs at once, nothing queued.
    assert!(matches!(
        decide_in(
            &files,
            JobKind::Profile,
            None,
            Some(profile("/v")),
            &list,
            true
        ),
        Decision::Warm(_)
    ));
    assert!(!files.queue().exists());
    // Asleep: queued, with a note saying when it runs.
    let Decision::Held(Held::Queued { message, added }) = decide_in(
        &files,
        JobKind::Profile,
        None,
        Some(profile("/v")),
        &list,
        false,
    ) else {
        panic!("queued");
    };
    assert!(added);
    assert!(
        message.starts_with("About-you scan waits for Model qwen, which is asleep"),
        "{message}"
    );
    assert!(message.contains("Run them now"), "{message}");
    // The same job again keeps its place; another job is added.
    let again = decide_in(
        &files,
        JobKind::Profile,
        None,
        Some(profile("/v")),
        &list,
        false,
    );
    assert!(matches!(
        again,
        Decision::Held(Held::Queued { added: false, .. })
    ));
    let learning = Job::learning(
        Path::new("/v"),
        &"u".repeat(TEXT_KEPT + 50),
        &["people/Ana.md".into()],
        "a",
        Some("run-1"),
        true,
        None,
    );
    decide_in(
        &files,
        JobKind::Learning,
        None,
        Some(learning.clone()),
        &list,
        false,
    );
    // A reload reads them back as they were queued.
    let queued = files.queued();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].entry, "qwen");
    assert_eq!(queued[0].job, profile("/v"));
    assert_eq!(queued[1].job, learning);
    assert_ne!(queued[0].id, queued[1].id);
    let Job::Learning { user, .. } = &queued[1].job else {
        panic!()
    };
    assert_eq!(user.chars().count(), TEXT_KEPT);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(files.queue())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "a job can hold a chat's words");
    }
    let text = std::fs::read_to_string(files.queue()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["jobs"][0]["job"]["kind"], "profile");
    assert_eq!(value["jobs"][1]["job"]["runId"], "run-1");
    // Settings counts them per entry.
    let view = view_with(&files.jobs(), &files.queue(), list());
    assert_eq!(view.waiting.len(), 1);
    assert_eq!(view.waiting[0].count, 2);
    assert_eq!(view.waiting[0].name.as_deref(), Some("Model qwen"));
    // A job that can't wait is skipped.
    assert!(matches!(
        decide_in(&files, JobKind::Learning, None, None, &list, false),
        Decision::Held(Held::Skipped(note)) if note.contains("can't wait")
    ));
}

#[test]
fn a_damaged_or_full_queue_skips_the_job_and_is_kept() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true)]);
    files.route(JobKind::Profile, "qwen");
    std::fs::write(files.queue(), "{oops").unwrap();
    let Decision::Held(Held::Skipped(note)) = decide_in(
        &files,
        JobKind::Profile,
        None,
        Some(profile("/v")),
        &list,
        false,
    ) else {
        panic!("skipped");
    };
    assert!(
        note.contains("couldn't be queued") && note.contains(QUEUE_FILE),
        "{note}"
    );
    assert_eq!(std::fs::read_to_string(files.queue()).unwrap(), "{oops");
    let view = view_with(&files.jobs(), &files.queue(), list());
    assert!(view.queue_error.is_some());

    std::fs::remove_file(files.queue()).unwrap();
    for n in 0..MAX_QUEUED {
        enqueue_at(
            &files.queue(),
            "qwen",
            profile(&format!("/v{n}")),
            Utc::now(),
        )
        .unwrap();
    }
    assert!(matches!(
        decide_in(&files, JobKind::Profile, None, Some(profile("/new")), &list, false),
        Decision::Held(Held::Skipped(note)) if note.contains("200 jobs are waiting")
    ));
    assert_eq!(files.queued().len(), MAX_QUEUED);
}

#[test]
fn every_job_survives_the_queue_file() {
    let vault = PathBuf::from("/Users/me/vault");
    let jobs = vec![
        Job::Extraction {
            vault: vault.clone(),
            source: crate::extract::Source::Recording {
                path: "workspaces/hr/sources/claims.md".into(),
            },
        },
        Job::ChatTitle {
            vault: vault.clone(),
            session_key: "s-1".into(),
            turn_ord: 1,
            run_id: "r-1".into(),
            only_on: Some("qwen".into()),
        },
        Job::SessionTitle {
            projects: "/Users/me/.claude/projects".into(),
            project_dir: "-x".into(),
            session_id: "abc".into(),
        },
        Job::learning(
            &vault,
            "I prefer tea",
            &[],
            "Noted",
            Some("r-2"),
            false,
            None,
        ),
        Job::PrepPack {
            vault: vault.clone(),
            meeting_id: "evt-1".into(),
            regenerate: true,
            expected_hash: Some("h".into()),
            run_id: None,
        },
        Job::Recap {
            vault: vault.clone(),
            meeting_id: "evt-2".into(),
            around: Utc::now(),
            regenerate: false,
            recording_id: Some("rec-1".into()),
            expected_hash: None,
        },
        Job::Profile {
            vault: vault.clone(),
        },
        Job::Heartbeat {
            vault,
            manual_at: Some(Utc::now()),
        },
    ];
    let files = Files::new();
    for job in &jobs {
        assert!(enqueue_at(&files.queue(), "qwen", job.clone(), Utc::now()).unwrap());
    }
    let back: Vec<Job> = files.queued().into_iter().map(|q| q.job).collect();
    assert_eq!(back, jobs);
    let kinds: Vec<JobKind> = jobs.iter().map(Job::kind).collect();
    assert_eq!(
        kinds,
        [
            JobKind::Extraction,
            JobKind::Titles,
            JobKind::Titles,
            JobKind::Learning,
            JobKind::Meetings,
            JobKind::Meetings,
            JobKind::Profile,
            JobKind::Heartbeat
        ]
    );
    // Each is its own job; the same one twice waits once.
    let keys: HashSet<String> = jobs.iter().map(Job::key).collect();
    assert_eq!(keys.len(), jobs.len());
    assert!(!enqueue_at(&files.queue(), "qwen", jobs[4].clone(), Utc::now()).unwrap());
    // Another entry's queue is its own.
    assert!(enqueue_at(&files.queue(), "other", jobs[4].clone(), Utc::now()).unwrap());
    let taken = take_at(&files.queue(), &files.queued()[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(taken.job, jobs[0]);
    assert_eq!(files.queued().len(), jobs.len());
}

// ── Draining ────────────────────────────────────────────────────────────

fn waiting_since(entry: &str, minutes: i64, now: DateTime<Utc>) -> Queued {
    Queued {
        id: format!("{entry}-{minutes}"),
        entry: entry.into(),
        queued_at: now - chrono::Duration::minutes(minutes),
        job: profile("/v"),
        attempts: 0,
        last_error: None,
        retry_at: None,
        claim: None,
    }
}

fn routes(pairs: &[(JobKind, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, e)| (k.id().to_string(), e.to_string()))
        .collect()
}

#[test]
fn a_drain_is_due_after_thirty_minutes_or_when_the_entry_answered() {
    let now = Utc::now();
    let jobs = routes(&[(JobKind::Profile, "qwen")]);
    assert_eq!(
        due(&[], &jobs, "qwen", now, true, None),
        None,
        "nothing queued, no wake"
    );
    let other = [waiting_since("other", 90, now)];
    assert_eq!(due(&other, &jobs, "qwen", now, true, None), None);
    let young = [
        waiting_since("qwen", 29, now),
        waiting_since("other", 90, now),
    ];
    assert_eq!(due(&young, &jobs, "qwen", now, false, None), None);
    assert_eq!(
        due(&young, &jobs, "qwen", now, true, None),
        Some(Trigger::Answered)
    );
    let old = [
        waiting_since("qwen", 5, now),
        waiting_since("qwen", 30, now),
    ];
    assert_eq!(
        due(&old, &jobs, "qwen", now, false, None),
        Some(Trigger::Waited)
    );
    // A drain that failed waits for its retry time.
    let later = Some(now + chrono::Duration::minutes(10));
    assert_eq!(due(&old, &jobs, "qwen", now, false, later), None);
    assert_eq!(due(&old, &jobs, "qwen", now, true, later), None);
    let past = Some(now - chrono::Duration::minutes(1));
    assert_eq!(
        due(&old, &jobs, "qwen", now, false, past),
        Some(Trigger::Waited)
    );
    // Review: held jobs (their kind As now) and jobs that gave up never
    // start a drain, however old.
    let as_now = BTreeMap::new();
    assert_eq!(due(&old, &as_now, "qwen", now, true, None), None);
    let gave_up: Vec<Queued> = old
        .iter()
        .cloned()
        .map(|q| Queued {
            attempts: MAX_ATTEMPTS,
            ..q
        })
        .collect();
    assert_eq!(due(&gave_up, &jobs, "qwen", now, true, None), None);
    // A kind moved to another entry still drains here, to carry it over.
    let moved = routes(&[(JobKind::Profile, "other")]);
    assert_eq!(
        due(&old, &moved, "qwen", now, false, None),
        Some(Trigger::Waited)
    );
    // Review round 2 (M2): a failed job backing off starts no drain, not
    // even right after the model answered; once its time is past it does.
    let backing: Vec<Queued> = old
        .iter()
        .cloned()
        .map(|q| Queued {
            attempts: 1,
            retry_at: Some(now + chrono::Duration::minutes(20)),
            ..q
        })
        .collect();
    assert_eq!(due(&backing, &jobs, "qwen", now, true, None), None);
    assert_eq!(due(&backing, &jobs, "qwen", now, false, None), None);
    let later = now + chrono::Duration::minutes(21);
    assert_eq!(
        due(&backing, &jobs, "qwen", later, false, None),
        Some(Trigger::Waited)
    );
    // A fresh job beside it still drains at once when the model answered.
    let mut mixed = backing.clone();
    mixed.push(waiting_since("qwen", 1, now));
    assert_eq!(
        due(&mixed, &jobs, "qwen", now, true, None),
        Some(Trigger::Answered)
    );
}

/// The app's look at the queue: one drain per due entry, none for an entry
/// draining already or waiting for its retry time; a drain's end sets or
/// clears the retry time.
#[test]
fn a_look_at_the_queue_starts_each_due_drain_once() {
    let now = Utc::now();
    let jobs = routes(&[(JobKind::Profile, "qwen")]);
    // "spare" holds a young job whose kind goes to qwen now (a drain there
    // carries it over).
    let mut queue = vec![
        waiting_since("qwen", 45, now),
        waiting_since("spare", 2, now),
    ];
    queue[1].job = profile("/s");
    let mut state = DrainState::default();
    let cold = |_: &str| false;
    let warm_spare = |e: &str| e == "spare";
    assert_eq!(
        state.due_drains(&queue, &jobs, now, cold),
        vec![("qwen".to_string(), Trigger::Waited)]
    );
    assert_eq!(
        state.due_drains(&queue, &jobs, now, warm_spare),
        vec![
            ("qwen".to_string(), Trigger::Waited),
            ("spare".to_string(), Trigger::Answered)
        ],
        "right after spare answered"
    );
    assert!(state.begin("qwen"));
    assert!(!state.begin("qwen"), "one drain per entry");
    assert_eq!(state.due_drains(&queue, &jobs, now, cold), vec![]);
    let failed = DrainOutcome {
        error: Some("Model qwen didn't wake: its health check answered 503".into()),
        left: 1,
        ..Default::default()
    };
    state.finish("qwen", &failed, now);
    assert!(!state.running.contains("qwen"));
    assert_eq!(
        state.last_error.get("qwen").map(String::as_str),
        failed.error.as_deref()
    );
    assert_eq!(
        state.due_drains(&queue, &jobs, now, |_| true),
        vec![("spare".to_string(), Trigger::Answered)]
    );
    // Past the retry time (and spare's job is past 30 minutes too).
    let later = now + chrono::Duration::minutes(31);
    assert_eq!(
        state.due_drains(&queue, &jobs, later, cold),
        vec![
            ("qwen".to_string(), Trigger::Waited),
            ("spare".to_string(), Trigger::Waited)
        ]
    );
    state.finish("qwen", &DrainOutcome::default(), later);
    assert!(state.retry_at.is_empty() && state.last_error.is_empty());
}

/// Counts wakes and reruns; its wake fails when told to, a rerun fails for
/// the jobs it names, and it wants every job but those it names.
#[derive(Default)]
struct Fake {
    wakes: AtomicUsize,
    replays: Mutex<Vec<Job>>,
    wake_fails: bool,
    fails: Vec<Job>,
    unwanted: Vec<Job>,
    /// The entry whose `awake` each rerun notes in `awake_seen`.
    awake_of: String,
    awake_seen: Mutex<Vec<bool>>,
}

impl Drainer for Fake {
    fn wanted(&self, q: &Queued) -> bool {
        !self.unwanted.contains(&q.job)
    }
    async fn wake(&self, _entry: &SelfHostedEntry) -> Result<(), String> {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        if self.wake_fails {
            Err("its health check answered 503".into())
        } else {
            Ok(())
        }
    }
    async fn replay(&self, job: Job) -> Result<(), String> {
        self.replays.lock().unwrap().push(job.clone());
        self.awake_seen.lock().unwrap().push(awake(&self.awake_of));
        if self.fails.contains(&job) {
            Err("the model answered 500".into())
        } else {
            Ok(())
        }
    }
}

impl Fake {
    fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }
    fn replayed(&self) -> Vec<Job> {
        self.replays.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn a_drain_wakes_once_and_runs_every_job_back_to_back() {
    // Its own entry ids: `awake` is one set for the whole process.
    let (id, other) = (unique("batch"), unique("batch-other"));
    let files = Files::new();
    let list = || Ok(vec![entry(&id, true), entry(&other, true)]);
    files.route(JobKind::Profile, &id);
    files.route(JobKind::Heartbeat, &id);
    // Nothing queued: no wake.
    let fake = Fake {
        awake_of: id.clone(),
        ..Fake::default()
    };
    let out = drain_with(
        &id,
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!(out, DrainOutcome::default());
    assert_eq!(fake.wakes(), 0);

    let now = Utc::now();
    let mine = [
        profile("/a"),
        Job::Heartbeat {
            vault: "/a".into(),
            manual_at: None,
        },
        profile("/b"),
    ];
    for job in &mine {
        enqueue_at(&files.queue(), &id, job.clone(), now).unwrap();
    }
    enqueue_at(&files.queue(), &other, profile("/c"), now).unwrap();
    let out = drain_with(
        &id,
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!(fake.wakes(), 1, "one wake for the whole batch");
    assert_eq!(fake.replayed(), mine.to_vec(), "in the order queued");
    assert_eq!(
        *fake.awake_seen.lock().unwrap(),
        vec![true; 3],
        "the entry counts as awake for every rerun"
    );
    assert!(!awake(&id), "and not after the drain");
    assert_eq!(
        out,
        DrainOutcome {
            woke: true,
            ran: 3,
            ..Default::default()
        }
    );
    let left: Vec<String> = files.queued().into_iter().map(|q| q.entry).collect();
    assert_eq!(left, [other], "another entry's jobs stay");
}

/// Review: before waking, jobs no longer wanted are dropped, and with none
/// left there is no wake at all.
#[tokio::test]
async fn a_drain_drops_stale_jobs_and_never_wakes_for_nothing() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true)]);
    files.route(JobKind::Profile, "qwen");
    let now = Utc::now();
    for job in [profile("/a"), profile("/b")] {
        enqueue_at(&files.queue(), "qwen", job, now).unwrap();
    }
    let fake = Fake {
        unwanted: vec![profile("/a"), profile("/b")],
        ..Fake::default()
    };
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((fake.wakes(), out.dropped, out.ran, out.left), (0, 2, 0, 0));
    assert!(files.queued().is_empty());
    // One stale, one wanted: a wake for the wanted one only.
    for job in [profile("/a"), profile("/b")] {
        enqueue_at(&files.queue(), "qwen", job, now).unwrap();
    }
    let fake = Fake {
        unwanted: vec![profile("/a")],
        ..Fake::default()
    };
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((fake.wakes(), out.dropped, out.ran), (1, 1, 1));
    assert_eq!(fake.replayed(), vec![profile("/b")]);
}

/// Make every queued job's backoff end now (time passing, in a test).
fn backoff_over(files: &Files) {
    let mut queue = files.queued();
    for q in &mut queue {
        if q.retry_at.is_some() {
            q.retry_at = Some(Utc::now() - chrono::Duration::seconds(1));
        }
    }
    write_queue_at(&files.queue(), &queue).unwrap();
}

/// `drain_with`, again while it answers `busy` (it did nothing then). The
/// drain's flock is per open file description: a process another test
/// forks while it is held keeps it until that child execs, which under load
/// can outlast the next call and turn it into a busy no-op, `(false, 0, 0)`.
async fn drain_unbusy<D: Drainer>(
    entry_id: &str,
    trigger: Trigger,
    jobs_path: &Path,
    queue_path: &Path,
    entries: impl Fn() -> Result<Vec<SelfHostedEntry>, String>,
    drainer: &D,
) -> DrainOutcome {
    for _ in 0..1000 {
        let out = drain_with(entry_id, trigger, jobs_path, queue_path, &entries, drainer).await;
        if !out.busy {
            return out;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    drain_with(entry_id, trigger, jobs_path, queue_path, &entries, drainer).await
}

/// Review: a job leaves the queue only when its rerun succeeds. A failure
/// keeps it with its attempts and error and (review round 2, M2) a
/// backoff of 30 then 60 minutes, which a drain respects and Run them now
/// doesn't; after three it waits for the person, and only Try again (M4)
/// runs it again.
#[tokio::test]
async fn a_failed_rerun_keeps_the_job_until_it_gives_up() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true)]);
    files.route(JobKind::Profile, "qwen");
    enqueue_at(&files.queue(), "qwen", profile("/a"), Utc::now()).unwrap();
    enqueue_at(&files.queue(), "qwen", profile("/b"), Utc::now()).unwrap();
    let fake = Fake {
        fails: vec![profile("/a")],
        ..Fake::default()
    };
    for attempt in 1..=MAX_ATTEMPTS {
        let before = Utc::now();
        let out = drain_unbusy(
            "qwen",
            Trigger::Waited,
            &files.jobs(),
            &files.queue(),
            list,
            &fake,
        )
        .await;
        assert_eq!(out.failed, 1);
        let queued = files.queued();
        assert_eq!(queued.len(), 1, "the one that worked left");
        assert_eq!(queued[0].job, profile("/a"));
        assert_eq!(queued[0].attempts, attempt);
        assert_eq!(
            queued[0].last_error.as_deref(),
            Some("the model answered 500")
        );
        assert_eq!(queued[0].claim, None, "its claim ended with the rerun");
        let retry = queued[0].retry_at.expect("a backoff");
        assert!(retry >= before + backoff(attempt) && retry <= Utc::now() + backoff(attempt));
        if attempt < MAX_ATTEMPTS {
            // Backing off: the next drain (the model just answered) leaves
            // it alone, and without other work wakes nothing.
            let wakes = fake.wakes();
            let out = drain_unbusy(
                "qwen",
                Trigger::Answered,
                &files.jobs(),
                &files.queue(),
                list,
                &fake,
            )
            .await;
            assert_eq!((out.woke, out.ran, out.left), (false, 0, 1));
            assert_eq!(fake.wakes(), wakes);
            assert_eq!(files.queued()[0].attempts, attempt);
            backoff_over(&files);
        }
    }
    assert_eq!(backoff(1), chrono::Duration::minutes(30));
    assert_eq!(backoff(2), chrono::Duration::minutes(60));
    let gave_up = files.queued().remove(0);
    assert!(gave_up.gave_up());
    backoff_over(&files);
    for trigger in [Trigger::Waited, Trigger::RunNow] {
        let out = drain_unbusy("qwen", trigger, &files.jobs(), &files.queue(), list, &fake).await;
        assert_eq!(
            (out.woke, out.ran, out.left),
            (false, 0, 0),
            "no wake for a job that gave up ({trigger:?})"
        );
    }
    // Settings lists it for the person, with Try again (M4).
    let view = view_with(&files.jobs(), &files.queue(), list());
    assert!(view.waiting.is_empty());
    assert_eq!(view.held.len(), 1);
    let row = &view.held[0];
    assert!(row.gave_up && row.attempts == MAX_ATTEMPTS);
    assert!(row.can_try_again && !row.can_run_as_now && !row.running);
    assert_eq!(row.last_error.as_deref(), Some("the model answered 500"));
    assert_eq!(
        run_now_problem(&files.queued(), &read_jobs_at(&files.jobs()), "qwen").as_deref(),
        Some("Nothing is waiting for this model."),
        "no Run them now row for it"
    );
    // Try again: waiting again, attempts cleared; a drain runs it.
    assert_eq!(
        try_again_at(&files.queue(), &files.jobs(), &gave_up.id, Utc::now()),
        Ok("qwen".to_string())
    );
    let again = files.queued().remove(0);
    assert_eq!(
        (again.attempts, again.last_error, again.retry_at),
        (0, None, None)
    );
    let fake = Fake::default();
    let out = drain_unbusy(
        "qwen",
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((out.woke, out.ran, out.failed), (true, 1, 0));
    assert!(files.queued().is_empty());
}

/// Run them now doesn't wait for a failed job's backoff (the person asked).
#[tokio::test]
async fn run_them_now_reruns_a_job_backing_off() {
    let files = Files::new();
    let id = unique("backoff");
    let list = || Ok(vec![entry(&id, true)]);
    files.route(JobKind::Profile, &id);
    enqueue_at(&files.queue(), &id, profile("/a"), Utc::now()).unwrap();
    let q = files.queued().remove(0);
    fail_at(&files.queue(), &q.id, "the model answered 500", Utc::now()).unwrap();
    let fake = Fake::default();
    let out = drain_unbusy(
        &id,
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((out.woke, out.ran), (false, 0));
    let out = drain_unbusy(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((out.woke, out.ran, out.failed), (true, 1, 0));
    assert!(files.queued().is_empty());
}

#[tokio::test]
async fn a_drain_that_cant_wake_or_run_its_entry_leaves_the_jobs() {
    let files = Files::new();
    files.route(JobKind::Profile, "qwen");
    enqueue_at(&files.queue(), "qwen", profile("/a"), Utc::now()).unwrap();
    // The wake fails: nothing runs.
    let fake = Fake {
        wake_fails: true,
        ..Fake::default()
    };
    let list = || Ok(vec![entry("qwen", true)]);
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((fake.wakes(), fake.replayed().len()), (1, 0));
    assert!(out
        .error
        .as_deref()
        .is_some_and(|e| e.contains("didn't wake: its health check answered 503")));
    assert_eq!(out.left, 1);
    // Switched off: no wake at all.
    let fake = Fake::default();
    let off = || Ok(vec![entry("qwen", false)]);
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        off,
        &fake,
    )
    .await;
    assert_eq!((fake.wakes(), fake.replayed().len()), (0, 0));
    assert!(out
        .error
        .as_deref()
        .is_some_and(|e| e.contains("switched off")));
    // A setting that can't be read: nothing moves.
    let text = std::fs::read_to_string(files.jobs()).unwrap();
    std::fs::write(files.jobs(), "{").unwrap();
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((fake.wakes(), fake.replayed().len(), out.left), (0, 0, 1));
    assert!(out.error.is_some());
    std::fs::write(files.jobs(), text).unwrap();
    assert_eq!(files.queued().len(), 1);
}

/// Flake root cause (build run 11-18): `fork` duplicates every open
/// descriptor, and a flock belongs to the open file description, so a child
/// another test spawns on a parallel thread held a lock the drain had just
/// dropped until its `exec`: the next drain read "busy" and did nothing
/// (`left` 0 where 1 was expected). Dropping the lock now unlocks it
/// explicitly, so no child's copy keeps it.
#[test]
fn a_dropped_drain_lock_is_free_even_while_other_threads_spawn_children() {
    /// Stops the spawner threads even when an assertion unwinds.
    struct Stop(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let files = Files::new();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _stopper = Stop(Arc::clone(&stop));
    let spawners: Vec<_> = (0..3)
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = std::process::Command::new("true").status();
                }
            })
        })
        .collect();
    let mut busy = 0_usize;
    for _ in 0..400 {
        // Taken and dropped each time: any `None` is a lock a child still held.
        let lock = try_drain_lock(&files.queue(), "qwen").unwrap();
        busy += usize::from(lock.is_none());
    }
    drop(_stopper);
    for t in spawners {
        t.join().unwrap();
    }
    assert_eq!(busy, 0, "a dropped drain lock still read busy");
}

/// Review: a kind sent back to As now holds its waiting jobs (no rerun, no
/// wake, never sent to Anthropic by a drain): Settings lists them for Run
/// as now or Discard. A kind sent to another entry moves its jobs there.
#[tokio::test]
async fn jobs_of_a_kind_back_to_as_now_are_held_for_the_person() {
    let files = Files::new();
    let list = || Ok(vec![entry("qwen", true), entry("other", true)]);
    files.route(JobKind::Profile, "qwen");
    files.route(JobKind::Learning, "qwen");
    let learning = Job::learning(
        Path::new("/v"),
        "remember that I prefer tea",
        &[],
        "Noted",
        Some("r-1"),
        false,
        None,
    );
    enqueue_at(&files.queue(), "qwen", profile("/a"), Utc::now()).unwrap();
    enqueue_at(&files.queue(), "qwen", learning.clone(), Utc::now()).unwrap();
    // About-you scan back to As now; Learning extraction to another entry.
    set_at(&files.jobs(), list(), JobKind::Profile, None).unwrap();
    set_at(&files.jobs(), list(), JobKind::Learning, Some("other")).unwrap();
    let fake = Fake::default();
    let out = drain_with(
        "qwen",
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!(fake.wakes(), 0);
    assert!(fake.replayed().is_empty(), "a drain reruns neither");
    assert_eq!((out.held, out.moved), (1, 1));
    let queued = files.queued();
    let held = queued.iter().find(|q| q.job == profile("/a")).unwrap();
    assert_eq!(held.entry, "qwen", "held where it was");
    assert_eq!(
        queued.iter().find(|q| q.job == learning).unwrap().entry,
        "other",
        "moved to the kind's new entry, not run"
    );
    let view = view_with(&files.jobs(), &files.queue(), list());
    assert_eq!(view.held.len(), 1);
    assert_eq!(view.held[0].id, held.id);
    assert!(!view.held[0].gave_up);
    assert_eq!(view.waiting.len(), 1);
    assert_eq!(view.waiting[0].entry, "other");
    // Run as now only for a held job, only under a setting that reads.
    let jobs = read_jobs_at(&files.jobs());
    assert_eq!(as_now_problem(&jobs, held), None);
    let moved = queued.iter().find(|q| q.job == learning).unwrap();
    assert!(as_now_problem(&jobs, moved).is_some_and(
        |w| w.contains("goes to \"other\" now") || w.contains("goes to this model now")
    ));
    assert!(as_now_problem(&Err("bad".into()), held).is_some_and(|w| w.contains("held while")));
    // Discard takes it off.
    assert_eq!(
        discard_at(&files.queue(), &held.id, Utc::now()).unwrap().id,
        held.id
    );
    assert_eq!(files.queued().len(), 1);
    assert_eq!(
        discard_at(&files.queue(), &held.id, Utc::now()).unwrap_err(),
        "That job isn't waiting any more."
    );
}

/// The app's drainer: its wake is one health call through the entry's
/// forwarder (a 2xx wakes, anything else is an error), which marks the
/// entry answered.
#[tokio::test]
async fn the_app_drainer_wakes_through_the_health_path() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (status, ok) in [(200u16, true), (503, false)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = paths.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                seen.lock()
                    .unwrap()
                    .push(head.split_whitespace().nth(1).unwrap_or("").to_string());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        let id = unique("wake");
        let saved = SelfHostedEntry {
            base_url: base,
            health_path: "/up".into(),
            ..entry(&id, true)
        };
        let woke = AppDrainer.wake(&saved).await;
        assert_eq!(woke.is_ok(), ok, "{status}: {woke:?}");
        if !ok {
            assert_eq!(woke.unwrap_err(), "its health check answered 503");
        }
        assert_eq!(*paths.lock().unwrap(), vec!["/up".to_string()]);
        assert_eq!(
            warm_at(&id, SystemTime::now()),
            ok,
            "a 2xx marks it answered"
        );
        forwarder::stop(&id);
    }
}

/// Q10's commands answer the main window only; the COO reaches each through
/// its own `settings.agents.*` action (COO parity L13), never the command.
#[test]
fn background_jobs_commands_are_main_window_only_and_have_coo_actions() {
    let source = include_str!("commands/self_hosted_api.rs");
    for command in [
        "set_self_hosted_job",
        "run_self_hosted_jobs_now",
        "run_self_hosted_job_as_now",
        "retry_self_hosted_job",
        "discard_self_hosted_job",
    ] {
        let body = source
            .split(&format!("pub async fn {command}("))
            .nth(1)
            .unwrap_or_else(|| panic!("{command}"));
        let body = body.split("#[tauri::command]").next().unwrap();
        assert!(
            body.contains("webview: tauri::Webview") && body.contains("main_window(&webview)?;"),
            "{command}"
        );
    }
    let map = include_str!("../../docs/HOLLOW_APP_COMMAND_MAP.tsv");
    for (command, tier) in [
        ("self_hosted_jobs", "read"),
        ("set_self_hosted_job", "confirm"),
        ("run_self_hosted_jobs_now", "external"),
        ("run_self_hosted_job_as_now", "confirm"),
        ("retry_self_hosted_job", "external"),
        ("discard_self_hosted_job", "confirm"),
    ] {
        assert!(
            map.lines()
                .any(|l| l.starts_with(&format!("{command}\t"))
                    && l.split('\t').nth(3) == Some(tier)),
            "{command}"
        );
        let id = format!("settings.agents.{command}");
        let action = crate::app_actions::find(&id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(action.command, Some(command), "{id}");
        assert_eq!(action.tier.as_str(), tier, "{id}");
        assert!(
            !crate::app_actions::USER_ONLY.contains(&command),
            "{command}"
        );
    }
}

/// The warm check reads the forwarder's last answer on the wall clock.
#[test]
fn an_entry_is_warm_for_eight_minutes_after_it_answered() {
    let id = unique("warm");
    let now = SystemTime::now();
    assert!(!warm_at(&id, now));
    forwarder::set_last_answered(&id, Some(now - Duration::from_secs(7 * 60)));
    assert!(warm_at(&id, now));
    forwarder::set_last_answered(&id, Some(now - Duration::from_secs(8 * 60)));
    assert!(!warm_at(&id, now));
    forwarder::set_last_answered(&id, None);
}

/// A switch-off between the decision and the start refuses the job and
/// leaves no forwarder running.
#[tokio::test]
async fn a_switch_off_after_the_decision_refuses_the_start() {
    let id = unique("raced");
    let saved = entry(&id, true);
    let since = forwarder::stops(&id);
    forwarder::stop(&id);
    let refused = attach_with(
        Warm {
            entry: saved.clone(),
            since,
        },
        || Ok(vec![saved.clone()]),
    )
    .await
    .unwrap_err();
    assert!(refused.contains(forwarder::STOPPED), "{refused}");
    assert!(forwarder::get(&id).is_none());
    // Off by the second read: refused, and its forwarder stopped.
    let since = forwarder::stops(&id);
    let off = entry(&id, false);
    let refused = attach_with(
        Warm {
            entry: saved,
            since,
        },
        move || Ok(vec![off.clone()]),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("switched off"), "{refused}");
    assert!(forwarder::get(&id).is_none());
}

// ── The call sites ──────────────────────────────────────────────────────

/// Each call site's Held arm: a kind sent to a model that isn't in the list
/// returns the skip note as the job's own error and never runs as now (an
/// As now run would fail differently, or call an agent). Through `replay`
/// too, the drain's way back into each entry point.
#[tokio::test]
async fn every_call_site_returns_its_skip_and_never_runs_as_now() {
    let files = Files::new();
    let _dir = test_dir(files.dir.path());
    let gone = unique("gone");
    for kind in JobKind::ALL {
        files.route(kind, &gone);
    }
    let says = |kind: JobKind, e: &str| {
        e.starts_with(kind.label())
            && e.contains("was skipped")
            && e.contains(&format!("\"{gone}\""))
            && e.contains("never runs on another model")
    };
    let vault = tempfile::tempdir().unwrap();
    let v = vault.path();

    // Recording extraction: Keep transcript, extract_run and extract.run all
    // go through `ConfiguredModel`.
    std::fs::write(v.join("ops.md"), crate::extract::tests::TRANSCRIPT).unwrap();
    let source = crate::extract::Source::Recording {
        path: "ops.md".into(),
    };
    let job = Job::Extraction {
        vault: v.to_path_buf(),
        source: source.clone(),
    };
    let model = crate::extract::ConfiguredModel::new(None, Some(job.clone()));
    let e = crate::extract::run(v, &source, &model, "Alex", Utc::now())
        .await
        .unwrap_err();
    assert!(says(JobKind::Extraction, &e), "{e}");
    let e = replay(job).await.unwrap_err();
    assert!(says(JobKind::Extraction, &e), "{e}");
    assert!(
        crate::extract::ledger(v).unwrap().sources.is_empty(),
        "nothing extracted"
    );

    // Learning extraction.
    let e = crate::vault_learning::propose_from_exchange(
        v,
        "Remember that I prefer tea in the morning.",
        &[],
        "Noted.",
        Some("run-q10-held"),
        false,
        None,
    )
    .await
    .unwrap_err();
    assert!(says(JobKind::Learning, &e), "{e}");
    let job = Job::learning(
        v,
        "Remember that I prefer tea in the morning.",
        &[],
        "Noted.",
        Some("run-q10-held"),
        false,
        None,
    );
    let e = replay(job).await.unwrap_err();
    assert!(says(JobKind::Learning, &e), "{e}");

    // Session-list titles.
    let projects = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(projects.path().join("-q10")).unwrap();
    let session = unique("session");
    std::fs::write(
        projects.path().join("-q10").join(format!("{session}.jsonl")),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Plan the payroll review for March\"}}\n",
    )
    .unwrap();
    let e = crate::sessions::generate(projects.path(), "-q10", &session)
        .await
        .unwrap_err();
    assert!(says(JobKind::Titles, &e), "{e}");

    // Meeting recaps and prep packs: Prepare, Recap and a check-in's pack all
    // write through `AgentWriter`.
    use crate::prep_pack::Writer;
    let writer =
        crate::prep_pack::AgentWriter::new(None, "prep_pack", Duration::from_secs(150), None);
    let e = writer.complete("Write the pack.").await.unwrap_err();
    assert!(says(JobKind::Meetings, &e), "{e}");
    assert!(!writer.private());

    // The heartbeat: held in `prepare_beat`, before the lease and the
    // budget charge, with a line in its log.
    let e = match crate::heartbeat::prepare_beat(v, crate::heartbeat::BeatRequest::Scheduled) {
        Err(e) => e,
        Ok(_) => panic!("the beat is held"),
    };
    assert!(says(JobKind::Heartbeat, &e), "{e}");
    let log = std::fs::read_to_string(v.join(".heartbeat-log.jsonl")).unwrap();
    assert!(log.contains("\"status\":\"skipped\""), "{log}");

    // Nothing was queued for any of them, and no forwarder started.
    assert!(!files.queue().exists());
    assert!(forwarder::get(&gone).is_none());
}

/// Every call site keeps its old call as its As now arm and maps Held to the
/// job's own error (pinned in source, so a later edit can't route a held
/// job to Anthropic).
#[test]
fn every_call_site_keeps_as_now_and_errs_when_held() {
    for (name, source, as_now) in [
        (
            "extract.rs",
            include_str!("extract.rs"),
            "crate::claude_runner::run_oneshot_selected(kind, model, prompt, opts).await",
        ),
        (
            "chat_title.rs",
            include_str!("chat_title.rs"),
            "crate::claude_runner::run_oneshot_selected(kind, model, prompt, opts).await",
        ),
        (
            "sessions.rs",
            include_str!("sessions.rs"),
            "Routed::AsNow => crate::claude_runner::run_oneshot(&prompt, opts).await?",
        ),
        (
            "vault_learning.rs",
            include_str!("vault_learning.rs"),
            "crate::claude_runner::run_oneshot(&prompt, opts).await?",
        ),
        (
            "prep_pack.rs",
            include_str!("prep_pack.rs"),
            "Routed::AsNow => crate::claude_runner::run_oneshot(prompt, opts).await",
        ),
        (
            "profile_builder.rs",
            include_str!("profile_builder.rs"),
            "Routed::AsNow => crate::claude_runner::run_oneshot(&prompt, opts).await",
        ),
    ] {
        assert!(
            crate::source_pins::contains(source, as_now),
            "{name}: {as_now}"
        );
        let held =
            crate::source_pins::count(source, "Routed::Held(held) => Err(held.into_message())")
                + crate::source_pins::count(
                    source,
                    "Routed::Held(held) => return Err(held.into_message())",
                );
        assert_eq!(held, 1, "{name}");
        assert_eq!(
            source.matches("self_hosted_jobs::route(").count(),
            1,
            "{name}"
        );
    }
    // The heartbeat: decided in `prepare_beat`, attached in `run_beat` after
    // admission, and a failed attach is the beat's failure before anything
    // runs.
    let beat = include_str!("heartbeat.rs");
    let prepare = &beat[beat
        .find(concat!("pub(crate) fn ", "prepare_beat("))
        .unwrap()..];
    let prepare = &prepare[..prepare.find("\n}\n").unwrap()];
    assert!(prepare.contains("Decision::Held(held) => {"));
    assert!(prepare.contains("return Err(held.into_message());"));
    let run = &beat[beat
        .find(concat!("pub(crate) async fn ", "run_beat("))
        .unwrap()..];
    let run = &run[..run.find("\n}\n").unwrap()];
    let attach = run.find("self_hosted_jobs::attach(warm)").unwrap();
    let failed = run
        .find(concat!(
            "if let Err(error) = &self_hosted {\n        agent::Collected::",
            "failed("
        ))
        .unwrap();
    let spawn = run.find(concat!("agent::", "run_collect(")).unwrap();
    assert!(attach < failed && failed < spawn);
}

// ── Review round 2 ──────────────────────────────────────────────────────

/// A drainer whose rerun decides as a job's entry point does
/// (`decide_with` on the test's files, the drained entry counting as awake
/// while the drain runs), after the person's switch of `kind`'s setting:
/// during the wake (`on_wake`) or between the claim and the decision
/// (`in_replay`). A held decision is the rerun's error, or (`ok_when_held`,
/// as a held heartbeat's gate answers) no error at all.
struct EntryPoint<'a> {
    files: &'a Files,
    entries: Vec<SelfHostedEntry>,
    on_wake: Option<(JobKind, Option<String>)>,
    in_replay: Option<(JobKind, Option<String>)>,
    ok_when_held: bool,
    as_now: Mutex<Vec<Job>>,
    ran_on: Mutex<Vec<String>>,
    held: Mutex<Vec<String>>,
}

impl<'a> EntryPoint<'a> {
    fn new(files: &'a Files, entries: Vec<SelfHostedEntry>) -> Self {
        EntryPoint {
            files,
            entries,
            on_wake: None,
            in_replay: None,
            ok_when_held: false,
            as_now: Mutex::default(),
            ran_on: Mutex::default(),
            held: Mutex::default(),
        }
    }

    fn rerun(&self, job: Job) -> Result<(), String> {
        if let Some((kind, to)) = &self.in_replay {
            self.files.set(*kind, to.as_deref());
        }
        let entries = self.entries.clone();
        let decision = decide_with(
            job.kind(),
            job.only_on(),
            || Some(job.clone()),
            &Env {
                jobs: &self.files.jobs(),
                queue: &self.files.queue(),
                entries: &move || Ok(entries.clone()),
                warm: &|id: &str| awake(id),
                now: Utc::now(),
                asked: Asked::Auto,
            },
        );
        match decision {
            Decision::AsNow => {
                self.as_now.lock().unwrap().push(job);
                Ok(())
            }
            Decision::Warm(warm) => {
                self.ran_on.lock().unwrap().push(warm.entry.id);
                Ok(())
            }
            Decision::Held(held) => {
                self.held.lock().unwrap().push(held.message().to_string());
                if self.ok_when_held {
                    Ok(())
                } else {
                    Err(held.into_message())
                }
            }
        }
    }
}

impl Drainer for EntryPoint<'_> {
    fn wanted(&self, _q: &Queued) -> bool {
        true
    }
    async fn wake(&self, _entry: &SelfHostedEntry) -> Result<(), String> {
        if let Some((kind, to)) = &self.on_wake {
            self.files.set(*kind, to.as_deref());
        }
        Ok(())
    }
    async fn replay(&self, job: Job) -> Result<(), String> {
        self.rerun(job)
    }
}

/// B1: switching a kind to As now while its model wakes never sends its
/// queued jobs to Anthropic. Switched before a job's rerun starts, the
/// rerun's claim (the setting read again) leaves it; switched after,
/// inside the entry point, the rerun is pinned to the drained entry and
/// is held. Either way the job stays for Run as now or Discard, and it
/// isn't a failed attempt: no attempt, no error, no backoff, no claim. A
/// kind sent to another entry meanwhile is held too and moves there.
#[tokio::test]
async fn a_switch_to_as_now_during_a_drain_holds_the_jobs() {
    let id = unique("b1");
    let other = unique("b1-other");
    let list = vec![entry(&id, true), entry(&other, true)];
    // Not a failed attempt: no attempt, no error, no claim; a held rerun
    // backs off 30 minutes (review round 3, N1), one never claimed doesn't.
    let kept_as_it_was = |files: &Files, n: usize| {
        let queued = files.queued();
        assert_eq!(queued.len(), n);
        for q in &queued {
            assert_eq!(
                (q.attempts, &q.last_error, &q.claim),
                (0, &None, &None),
                "not a failed attempt"
            );
            assert!(q
                .retry_at
                .is_none_or(|at| at <= Utc::now() + backoff(1) && at > Utc::now()));
        }
    };
    for heartbeat_like in [false, true] {
        // During the wake: no rerun at all.
        let files = Files::new();
        files.route(JobKind::Learning, &id);
        files.route(JobKind::Profile, &id);
        let learning = Job::learning(
            Path::new("/b1"),
            "remember tea",
            &[],
            "ok",
            Some("r-b1"),
            false,
            None,
        );
        enqueue_at(&files.queue(), &id, learning.clone(), Utc::now()).unwrap();
        enqueue_at(&files.queue(), &id, profile("/b1"), Utc::now()).unwrap();
        let mut point = EntryPoint::new(&files, list.clone());
        point.ok_when_held = heartbeat_like;
        point.on_wake = Some((JobKind::Learning, None));
        let l = list.clone();
        let out = drain_with(
            &id,
            Trigger::Waited,
            &files.jobs(),
            &files.queue(),
            move || Ok(l.clone()),
            &point,
        )
        .await;
        assert!(
            point.as_now.lock().unwrap().is_empty(),
            "nothing ran as now"
        );
        assert_eq!(
            *point.ran_on.lock().unwrap(),
            vec![id.clone()],
            "the About-you scan still ran there"
        );
        assert_eq!((out.ran, out.failed, out.held, out.left), (1, 0, 1, 0));
        kept_as_it_was(&files, 1);
        assert_eq!(files.queued()[0].job, learning);
        let view = view_with(&files.jobs(), &files.queue(), Ok(list.clone()));
        assert!(view.held[0].can_run_as_now && !view.held[0].gave_up);

        // Inside the entry point, after the claim: pinned, held.
        let files = Files::new();
        files.route(JobKind::Learning, &id);
        enqueue_at(&files.queue(), &id, learning.clone(), Utc::now()).unwrap();
        let mut point = EntryPoint::new(&files, list.clone());
        point.ok_when_held = heartbeat_like;
        point.in_replay = Some((JobKind::Learning, None));
        let l = list.clone();
        let out = drain_with(
            &id,
            Trigger::RunNow,
            &files.jobs(),
            &files.queue(),
            move || Ok(l.clone()),
            &point,
        )
        .await;
        assert!(point.as_now.lock().unwrap().is_empty(), "never As now");
        let held = point.held.lock().unwrap().clone();
        assert_eq!(held.len(), 1);
        assert!(
            held[0].starts_with("Learning extraction was held")
                && held[0].contains("runs As now since"),
            "{}",
            held[0]
        );
        assert_eq!((out.ran, out.failed, out.held), (0, 0, 1));
        kept_as_it_was(&files, 1);

        // Sent to another entry inside the entry point: held, then moved.
        let files = Files::new();
        files.route(JobKind::Learning, &id);
        enqueue_at(&files.queue(), &id, learning.clone(), Utc::now()).unwrap();
        let mut point = EntryPoint::new(&files, list.clone());
        point.ok_when_held = heartbeat_like;
        point.in_replay = Some((JobKind::Learning, Some(other.clone())));
        let l = list.clone();
        let out = drain_with(
            &id,
            Trigger::RunNow,
            &files.jobs(),
            &files.queue(),
            move || Ok(l.clone()),
            &point,
        )
        .await;
        assert!(point.as_now.lock().unwrap().is_empty() && point.ran_on.lock().unwrap().is_empty());
        assert_eq!((out.ran, out.failed), (0, 0));
        kept_as_it_was(&files, 1);
        let point = EntryPoint::new(&files, list.clone());
        let l = list.clone();
        let out = drain_with(
            &id,
            Trigger::Waited,
            &files.jobs(),
            &files.queue(),
            move || Ok(l.clone()),
            &point,
        )
        .await;
        assert_eq!(out.moved, 1);
        assert_eq!(files.queued()[0].entry, other);
    }
}

/// B1: a pinned decision runs a job only where its pin says. Pinned to an
/// entry: there while its kind goes there and it is awake; held (never As
/// now, never queued again) when the kind runs As now, goes elsewhere or
/// the model is asleep. Pinned to As now (Run as now): As now while the
/// kind runs As now, held once it goes to a model, and a self-hosted chat's
/// own job held. The pin notes each hold, belongs to its queue file only,
/// pins a job once, and ends when dropped.
#[test]
fn a_pinned_decision_runs_only_where_it_was_sent() {
    let files = Files::new();
    let id = unique("pin");
    let list = vec![entry(&id, true)];
    let entries = || Ok(list.clone());
    let job = profile("/pinned");
    let decide = |warm: bool| {
        decide_in(
            &files,
            JobKind::Profile,
            None,
            Some(job.clone()),
            &entries,
            warm,
        )
    };

    let pin = Pinned::new(&files.queue(), &job, Some(&id)).unwrap();
    assert!(
        Pinned::new(&files.queue(), &job, Some(&id)).is_none(),
        "pinned once"
    );
    files.route(JobKind::Profile, &id);
    assert!(matches!(decide(true), Decision::Warm(_)));
    assert_eq!(pin.held(), None);
    assert!(
        matches!(decide(false), Decision::Held(Held::Skipped(n)) if n.contains("went back to sleep"))
    );
    assert!(!files.queue().exists(), "never queued again");
    files.set(JobKind::Profile, None);
    let Decision::Held(Held::Skipped(note)) = decide(true) else {
        panic!("held, not As now");
    };
    assert_eq!(pin.held().as_deref(), Some(note.as_str()));
    // Another queue's pin doesn't touch this one, nor another job.
    let elsewhere = Files::new();
    assert_eq!(
        decide_in(
            &elsewhere,
            JobKind::Profile,
            None,
            Some(job.clone()),
            &entries,
            true
        ),
        Decision::AsNow
    );
    assert_eq!(
        decide_in(
            &files,
            JobKind::Profile,
            None,
            Some(profile("/free")),
            &entries,
            true
        ),
        Decision::AsNow
    );
    drop(pin);
    assert_eq!(decide(true), Decision::AsNow, "unpinned again");

    // Pinned to As now (Run as now).
    let pin = Pinned::new(&files.queue(), &job, None).unwrap();
    assert_eq!(decide(true), Decision::AsNow);
    files.route(JobKind::Profile, &id);
    assert!(
        matches!(decide(true), Decision::Held(Held::Skipped(n)) if n.contains("didn't run as now"))
    );
    assert!(pin.held().is_some());
    drop(pin);
    let own = Job::ChatTitle {
        vault: "/pinned".into(),
        session_key: "s-own".into(),
        turn_ord: 1,
        run_id: "r".into(),
        only_on: Some(id.clone()),
    };
    files.set(JobKind::Titles, None);
    let pin = Pinned::new(&files.queue(), &own, None).unwrap();
    assert!(matches!(
        decide_in(
            &files,
            JobKind::Titles,
            own.only_on(),
            Some(own.clone()),
            &entries,
            true
        ),
        Decision::Held(Held::Skipped(_))
    ));
    assert!(pin.held().is_some());
}

/// B1 through the real entry points: the pin is found by the job's key,
/// so it holds on a blocking thread too (Prepare and Recap decide inside
/// `spawn_blocking`), and a held beat's preparation is an error, which the
/// queued beat's gate keeps queued.
#[tokio::test]
async fn a_pin_holds_through_the_entry_points() {
    let files = Files::new();
    let _dir = test_dir(files.dir.path());
    let vault = tempfile::tempdir().unwrap();
    let v = vault.path().to_path_buf();
    let id = unique("pin-entry");
    // Every kind As now: unpinned, Prepare and the beat would run as now.
    let pack = Job::PrepPack {
        vault: v.clone(),
        meeting_id: "m-pin".into(),
        regenerate: false,
        expected_hash: None,
        run_id: None,
    };
    let pin = Pinned::new(&files.queue(), &pack, Some(&id)).unwrap();
    let job = pack.clone();
    let queue = files.queue();
    let dir = files.dir.path().to_path_buf();
    let e = tauri::async_runtime::spawn_blocking(move || {
        let _dir = test_dir(&dir);
        tauri::async_runtime::block_on(async {
            use crate::prep_pack::Writer;
            let writer = crate::prep_pack::AgentWriter::new(
                None,
                "prep_pack",
                Duration::from_secs(150),
                Some(job),
            );
            writer.complete("Write the pack.").await
        })
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        e.starts_with("Meeting recaps and prep packs was held"),
        "{e}"
    );
    assert_eq!(pin.held().as_deref(), Some(e.as_str()));
    assert!(!queue.exists());
    drop(pin);

    let beat = Job::Heartbeat {
        vault: v.clone(),
        manual_at: None,
    };
    let pin = Pinned::new(&files.queue(), &beat, Some(&id)).unwrap();
    // A queued Run now's rerun finds the same pin: `manual_at` isn't in
    // the key.
    let e = match crate::heartbeat::prepare_beat(&v, crate::heartbeat::BeatRequest::QueuedRunNow) {
        Err(e) => e,
        Ok(_) => panic!("the beat is held"),
    };
    assert!(e.starts_with("Heartbeat was held"), "{e}");
    assert!(pin.held().is_some());
}

/// M1: a new copy of a job that gave up (the About-you scan and the
/// heartbeat have one key per vault) starts it over, attempts, error and
/// backoff cleared, so it runs again instead of being absorbed. Review
/// round 3 (N2): it waits from the new request as a new job would, so the
/// next look doesn't find it overdue and wake a cold model.
#[test]
fn a_new_copy_of_a_job_that_gave_up_runs_again() {
    let files = Files::new();
    let now = Utc::now();
    let beat = Job::Heartbeat {
        vault: "/m1".into(),
        manual_at: None,
    };
    assert!(enqueue_at(&files.queue(), "qwen", beat.clone(), now).unwrap());
    let id = files.queued()[0].id.clone();
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &id, "the model answered 500", now).unwrap();
    }
    assert!(files.queued()[0].gave_up());
    let jobs = routes(&[(JobKind::Heartbeat, "qwen")]);
    let later = now + chrono::Duration::hours(3);
    assert_eq!(due(&files.queued(), &jobs, "qwen", later, true, None), None);
    assert!(
        !enqueue_at(&files.queue(), "qwen", beat, later).unwrap(),
        "the same job"
    );
    let q = files.queued().remove(0);
    assert_eq!(
        (q.id.as_str(), q.queued_at),
        (id.as_str(), later),
        "the same job, waiting from the new request"
    );
    assert_eq!((q.attempts, q.last_error, q.retry_at), (0, None, None));
    assert_eq!(
        due(&files.queued(), &jobs, "qwen", later, false, None),
        None
    );
    assert_eq!(
        due(&files.queued(), &jobs, "qwen", later, true, None),
        Some(Trigger::Answered)
    );
    let after_wait = later + chrono::Duration::minutes(30);
    assert_eq!(
        due(&files.queued(), &jobs, "qwen", after_wait, false, None),
        Some(Trigger::Waited)
    );
}

/// Review round 3 (N2): asking again for a job that is backing off after a
/// failure (a Busy heartbeat re-asked every 10 minutes, a title re-asked on
/// every backfill) replaces its details but keeps its attempts, error and
/// backoff: no early wake, and it still gives up after three failures.
#[test]
fn a_new_request_never_cuts_a_backoff_short() {
    let files = Files::new();
    let now = Utc::now();
    let jobs = routes(&[(JobKind::Titles, "qwen")]);
    let title = |turn: i64| Job::ChatTitle {
        vault: "/n2".into(),
        session_key: "s-n2".into(),
        turn_ord: turn,
        run_id: format!("r-{turn}"),
        only_on: None,
    };
    enqueue_at(
        &files.queue(),
        "qwen",
        title(1),
        now - chrono::Duration::hours(1),
    )
    .unwrap();
    let id = files.queued()[0].id.clone();
    for attempt in 1..MAX_ATTEMPTS {
        fail_at(&files.queue(), &id, "the model answered 500", now).unwrap();
        assert!(!enqueue_at(&files.queue(), "qwen", title(attempt as i64 + 1), now).unwrap());
        let q = files.queued().remove(0);
        assert_eq!(q.job, title(attempt as i64 + 1), "the newest details");
        assert_eq!(q.attempts, attempt, "attempts kept");
        assert_eq!(q.last_error.as_deref(), Some("the model answered 500"));
        assert_eq!(q.retry_at, Some(now + backoff(attempt)), "backoff kept");
        assert_eq!(
            due(&files.queued(), &jobs, "qwen", now, true, None),
            None,
            "no wake before its time, warm or not"
        );
        assert_eq!(due(&files.queued(), &jobs, "qwen", now, false, None), None);
    }
    fail_at(&files.queue(), &id, "the model answered 500", now).unwrap();
    assert!(files.queued()[0].gave_up(), "it still gives up");
}

/// M3: one drain of an entry at a time in every Hollow process. While its
/// drain lock is held (by another process's drain, here another
/// descriptor), a drain does nothing (`busy`: no wake, no rerun, no retry
/// time), and Run them now's check says so.
#[tokio::test]
async fn a_second_drain_of_an_entry_does_nothing() {
    let files = Files::new();
    let id = unique("lock");
    files.route(JobKind::Profile, &id);
    enqueue_at(&files.queue(), &id, profile("/lock"), Utc::now()).unwrap();
    assert!(!drain_busy_at(&files.queue(), &id).unwrap());
    let held = try_drain_lock(&files.queue(), &id).unwrap().unwrap();
    assert!(try_drain_lock(&files.queue(), &id).unwrap().is_none());
    assert!(drain_busy_at(&files.queue(), &id).unwrap());
    assert!(
        !drain_busy_at(&files.queue(), "another").unwrap(),
        "per entry"
    );
    let fake = Fake::default();
    let list = || Ok(vec![entry(&id, true)]);
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!(
        out,
        DrainOutcome {
            busy: true,
            ..Default::default()
        }
    );
    assert_eq!((fake.wakes(), fake.replayed().len()), (0, 0));
    let mut state = DrainState::default();
    assert!(state.begin(&id));
    state.finish(&id, &out, Utc::now());
    assert!(state.retry_at.is_empty() && state.last_error.is_empty() && state.running.is_empty());
    drop(held);
    // Right after the release: a child another test forked while the lock was
    // held keeps it until it execs (see `drain_unbusy`), so retry a busy answer.
    let out = drain_unbusy(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &fake,
    )
    .await;
    assert_eq!((out.busy, out.ran), (false, 1));
}

/// M3: a job claimed by a rerun (Run as now here, in this process) is
/// running for Discard, a drain and another Run as now, in any process;
/// a claim whose process is gone, or older than two hours, isn't.
#[tokio::test]
async fn a_claimed_job_is_never_run_twice_or_discarded() {
    let files = Files::new();
    let id = unique("claim");
    let list = vec![entry(&id, true)];
    enqueue_at(&files.queue(), &id, profile("/claim"), Utc::now()).unwrap();
    let q = files.queued().remove(0);
    // Its kind runs As now: Run as now claims it.
    let rerun = start_as_now_at(&files.queue(), &files.jobs(), &q.id).unwrap();
    let now = Utc::now();
    let claimed = files.queued().remove(0);
    assert_eq!(
        claimed.claim.as_ref().map(|c| c.started),
        Some(crate::proc_info::own_start_micros()),
        "the claim names this process's start (N4)"
    );
    assert_eq!(
        claimed.claim.as_ref().map(|c| c.pid),
        Some(std::process::id())
    );
    assert!(claimed.running(now));
    assert_eq!(
        discard_at(&files.queue(), &q.id, now).unwrap_err(),
        "Hollow is running it now."
    );
    assert_eq!(
        start_as_now_at(&files.queue(), &files.jobs(), &q.id)
            .err()
            .as_deref(),
        Some("Hollow is running it now.")
    );
    // The person sends the kind to the model meanwhile: a drain leaves the
    // claimed job alone (no double run).
    files.route(JobKind::Profile, &id);
    let point = EntryPoint::new(&files, list.clone());
    let l = list.clone();
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        move || Ok(l.clone()),
        &point,
    )
    .await;
    assert_eq!((out.woke, out.ran), (false, 0));
    assert!(point.ran_on.lock().unwrap().is_empty());
    let view = view_with(&files.jobs(), &files.queue(), Ok(list.clone()));
    assert_eq!(view.waiting[0].count, 1);
    // Run as now's own rerun is pinned to As now: the kind now goes to a
    // model, so it is held there, not run as now, and not an attempt.
    let at = &point;
    let end = rerun.run(|job| async move { at.rerun(job) }).await;
    assert!(
        matches!(&end, RerunEnd::Held(why) if why.contains("didn't run as now")),
        "{end:?}"
    );
    assert!(point.as_now.lock().unwrap().is_empty());
    let after = files.queued().remove(0);
    assert_eq!((after.attempts, &after.claim), (0, &None));
    assert!(after.retry_at.is_some(), "a held rerun backs off (N1)");
    // Claims that don't count: a gone process, a stale one, one this
    // process no longer holds.
    let claim = |pid: u32, at: DateTime<Utc>, started: Option<u64>| {
        let mut q = after.clone();
        q.claim = Some(Claim { pid, at, started });
        q
    };
    assert!(!claim(999_999_999, now, None).running(now));
    assert!(!claim(999_999_999, now, Some(1)).running(now));
    let own_start = crate::proc_info::own_start_micros();
    assert!(
        !claim(std::process::id(), now, own_start).running(now),
        "not held in this process"
    );
    // Review round 3 (N4): another process's claim counts while its pid
    // names the process that started when the claim says; a reused pid
    // (another start) doesn't, nor a pid whose owner can't be signalled.
    #[cfg(unix)]
    {
        let parent = crate::proc_info::parent_pid(std::process::id() as i32).unwrap();
        let started = crate::proc_info::process_start_micros(parent);
        assert!(
            started.is_some(),
            "the test runner's parent has a start time"
        );
        let parent = parent as u32;
        assert!(
            claim(parent, now, started).running(now),
            "a live process's claim"
        );
        assert!(
            !claim(parent, now, started.map(|s| s + 1)).running(now),
            "a reused pid"
        );
        assert!(
            !claim(parent, now - chrono::Duration::hours(3), started).running(now),
            "stale"
        );
        assert!(
            claim(parent, now, None).running(now),
            "no start recorded: the pid decides"
        );
        if crate::proc_info::pid_alive(1).is_none() {
            assert!(!claim(1, now, None).running(now), "another user's process");
        }
    }
}

/// M3: a job discarded (or moved) while a drain runs is left alone: each
/// rerun reads the queue again right before it starts.
#[tokio::test]
async fn a_job_discarded_during_a_drain_never_runs() {
    let files = Files::new();
    let id = unique("discard");
    files.route(JobKind::Profile, &id);
    for vault in ["/d1", "/d2"] {
        enqueue_at(&files.queue(), &id, profile(vault), Utc::now()).unwrap();
    }
    let second = files.queued()[1].id.clone();

    /// Discards `id` while the first job reruns.
    struct Discarding<'a> {
        files: &'a Files,
        id: String,
        replays: Mutex<Vec<Job>>,
    }
    impl Drainer for Discarding<'_> {
        fn wanted(&self, _q: &Queued) -> bool {
            true
        }
        async fn wake(&self, _entry: &SelfHostedEntry) -> Result<(), String> {
            Ok(())
        }
        async fn replay(&self, job: Job) -> Result<(), String> {
            self.replays.lock().unwrap().push(job);
            let _ = discard_at(&self.files.queue(), &self.id, Utc::now());
            Ok(())
        }
    }
    let drainer = Discarding {
        files: &files,
        id: second,
        replays: Mutex::default(),
    };
    let list = || Ok(vec![entry(&id, true)]);
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &drainer,
    )
    .await;
    assert_eq!(*drainer.replays.lock().unwrap(), vec![profile("/d1")]);
    assert_eq!(out.ran, 1);
    assert!(files.queued().is_empty());
}

/// Run as now (minor a): only a held job whose kind runs As now, under a
/// setting that reads, and never a self-hosted chat's own job; it leaves
/// the queue when it succeeds, and a failure counts an attempt with a
/// backoff. Settings shows Run as now only where it can run.
#[tokio::test]
async fn run_as_now_runs_only_a_held_job_and_records_how_it_went() {
    let files = Files::new();
    let id = unique("as-now");
    let list = vec![entry(&id, true)];
    let own = Job::Learning {
        vault: "/own".into(),
        user: "remember tea".into(),
        references: Vec::new(),
        assistant: "ok".into(),
        run_id: Some("r-own".into()),
        changed: false,
        only_on: Some(id.clone()),
        capture_session: None,
    };
    enqueue_at(&files.queue(), &id, profile("/ok"), Utc::now()).unwrap();
    enqueue_at(&files.queue(), &id, profile("/fails"), Utc::now()).unwrap();
    enqueue_at(&files.queue(), &id, own.clone(), Utc::now()).unwrap();
    let ids: Vec<String> = files.queued().into_iter().map(|q| q.id).collect();
    let view = view_with(&files.jobs(), &files.queue(), Ok(list.clone()));
    let row = |job_id: &str| view.held.iter().find(|h| h.id == job_id).unwrap().clone();
    assert!(row(&ids[0]).can_run_as_now && !row(&ids[0]).can_try_again);
    let own_row = row(&ids[2]);
    assert!(!own_row.can_run_as_now && own_row.only_on.as_deref() == Some(id.as_str()));
    assert!(start_as_now_at(&files.queue(), &files.jobs(), &ids[2])
        .err()
        .is_some_and(|e| e.contains("runs only there")));
    assert_eq!(files.queued()[2].claim, None, "a refusal claims nothing");
    // Succeeds: off the queue.
    let rerun = start_as_now_at(&files.queue(), &files.jobs(), &ids[0]).unwrap();
    assert_eq!(rerun.queued().job, profile("/ok"));
    assert_eq!(rerun.run(|_| async { Ok(()) }).await, RerunEnd::Done);
    // Fails: an attempt, its error and a backoff.
    let rerun = start_as_now_at(&files.queue(), &files.jobs(), &ids[1]).unwrap();
    assert_eq!(
        rerun.run(|_| async { Err("no agent".to_string()) }).await,
        RerunEnd::Failed("no agent".into())
    );
    let queued = files.queued();
    assert_eq!(queued.len(), 2);
    assert_eq!(
        (
            queued[0].attempts,
            queued[0].last_error.as_deref(),
            queued[0].claim.is_none()
        ),
        (1, Some("no agent"), true)
    );
    assert!(queued[0].retry_at.is_some());
    // Its kind sent to a model: Run as now refuses; a damaged setting too.
    files.route(JobKind::Profile, &id);
    assert!(start_as_now_at(&files.queue(), &files.jobs(), &ids[1])
        .err()
        .is_some_and(|e| e.contains("goes to this model now")));
    std::fs::write(files.jobs(), "{").unwrap();
    assert!(start_as_now_at(&files.queue(), &files.jobs(), &ids[1])
        .err()
        .is_some_and(|e| e.contains("held while")));
    assert_eq!(
        start_as_now_at(&files.queue(), &files.jobs(), "gone")
            .err()
            .as_deref(),
        Some("That job isn't waiting any more.")
    );
}

/// M4: Try again on a job that gave up: back to waiting with its attempts
/// cleared, on the entry its kind goes to now; refused for a job that
/// hasn't given up, one whose kind runs As now (Run as now is its way), and
/// under a setting that can't be read. A copy waiting on the new entry
/// already stands for it.
#[test]
fn try_again_puts_a_job_that_gave_up_back_in_line() {
    let files = Files::new();
    let now = Utc::now();
    files.route(JobKind::Profile, "qwen");
    for vault in ["/t1", "/t2"] {
        enqueue_at(&files.queue(), "qwen", profile(vault), now).unwrap();
    }
    let ids: Vec<String> = files.queued().into_iter().map(|q| q.id).collect();
    assert!(try_again_at(&files.queue(), &files.jobs(), &ids[0], now)
        .unwrap_err()
        .contains("hasn't given up"));
    for id in &ids {
        for _ in 0..MAX_ATTEMPTS {
            fail_at(&files.queue(), id, "500", now).unwrap();
        }
    }
    let list = Ok(vec![entry("qwen", true), entry("other", true)]);
    let view = view_with(&files.jobs(), &files.queue(), list.clone());
    assert!(view.held.iter().all(|h| h.can_try_again && h.gave_up));
    // Its kind went back to As now: Run as now instead.
    files.set(JobKind::Profile, None);
    assert!(try_again_at(&files.queue(), &files.jobs(), &ids[0], now)
        .unwrap_err()
        .contains("Run as now"));
    let view = view_with(&files.jobs(), &files.queue(), list.clone());
    assert!(view
        .held
        .iter()
        .all(|h| !h.can_try_again && h.can_run_as_now));
    // Its kind went to another entry: it moves there, waiting again.
    files.route(JobKind::Profile, "other");
    assert_eq!(
        try_again_at(&files.queue(), &files.jobs(), &ids[0], now),
        Ok("other".to_string())
    );
    let q = files.queued().into_iter().find(|q| q.id == ids[0]).unwrap();
    assert_eq!(
        (q.entry.as_str(), q.attempts, q.last_error),
        ("other", 0, None)
    );
    // A copy of the second waits there already: the second is dropped.
    enqueue_at(&files.queue(), "other", profile("/t2"), now).unwrap();
    assert_eq!(
        try_again_at(&files.queue(), &files.jobs(), &ids[1], now),
        Ok("other".to_string())
    );
    assert!(files.queued().iter().all(|q| q.id != ids[1]));
    std::fs::write(files.jobs(), "{").unwrap();
    assert!(try_again_at(&files.queue(), &files.jobs(), &ids[0], now).is_err());
}

/// The drain loop's look at the queue (`tick`): each due entry once; none
/// while the setting or the queue can't be read, or nothing is queued.
#[test]
fn a_look_at_the_queue_files_starts_the_due_drains() {
    let files = Files::new();
    let state = DrainState::default();
    let now = Utc::now();
    let cold = |_: &str| false;
    assert!(due_now(&files.queue(), &files.jobs(), &state, now, cold).is_empty());
    files.route(JobKind::Profile, "qwen");
    enqueue_at(
        &files.queue(),
        "qwen",
        profile("/tick"),
        now - chrono::Duration::minutes(31),
    )
    .unwrap();
    enqueue_at(&files.queue(), "spare", profile("/tock"), now).unwrap();
    files.route(JobKind::Heartbeat, "spare");
    assert_eq!(
        due_now(&files.queue(), &files.jobs(), &state, now, cold),
        vec![("qwen".to_string(), Trigger::Waited)]
    );
    // spare's job is an About-you scan, which goes to qwen: it moves there.
    assert_eq!(
        due_now(&files.queue(), &files.jobs(), &state, now, |e| e == "spare"),
        vec![
            ("qwen".to_string(), Trigger::Waited),
            ("spare".to_string(), Trigger::Answered)
        ]
    );
    let text = std::fs::read_to_string(files.jobs()).unwrap();
    std::fs::write(files.jobs(), "{").unwrap();
    assert!(due_now(&files.queue(), &files.jobs(), &state, now, cold).is_empty());
    std::fs::write(files.jobs(), text).unwrap();
    std::fs::write(files.queue(), "{").unwrap();
    assert!(due_now(&files.queue(), &files.jobs(), &state, now, cold).is_empty());
}

/// Run them now runs what its row lists: refused with nothing waiting, with
/// only jobs that gave up (their own Try again) or held (Run as now), and
/// under a setting that can't be read.
#[test]
fn run_them_now_needs_a_job_waiting() {
    let now = Utc::now();
    let jobs = Ok(routes(&[(JobKind::Profile, "qwen")]));
    let mut queue = vec![waiting_since("qwen", 5, now)];
    assert_eq!(run_now_problem(&queue, &jobs, "qwen"), None);
    assert_eq!(
        run_now_problem(&queue, &jobs, "other").as_deref(),
        Some("Nothing is waiting for this model.")
    );
    queue[0].attempts = MAX_ATTEMPTS;
    assert!(run_now_problem(&queue, &jobs, "qwen").is_some());
    queue[0].attempts = 1;
    queue[0].retry_at = Some(now + chrono::Duration::minutes(10));
    assert_eq!(
        run_now_problem(&queue, &jobs, "qwen"),
        None,
        "backing off still waits"
    );
    assert!(run_now_problem(&queue, &Ok(BTreeMap::new()), "qwen").is_some());
    assert!(run_now_problem(&queue, &Err("bad".into()), "qwen")
        .is_some_and(|e| e.contains("held while")));
}

/// Each kind's `still_wanted` asks that job's own cheap check.
#[test]
fn still_wanted_asks_each_kinds_own_check() {
    let now = Utc::now();
    let wants = |job: Job| {
        still_wanted(&Queued {
            id: "w".into(),
            entry: "qwen".into(),
            queued_at: now,
            job,
            attempts: 0,
            last_error: None,
            retry_at: None,
            claim: None,
        })
    };
    // Recording extraction: its source still reads and isn't extracted.
    let dir = crate::extract::tests::vault_with_transcript();
    let v = dir.path().to_path_buf();
    let source = crate::extract::tests::source();
    let extraction = Job::Extraction {
        vault: v.clone(),
        source: source.clone(),
    };
    assert!(wants(extraction.clone()));
    let fake = crate::extract::tests::Fake::new(crate::extract::tests::ANSWER);
    let me = crate::extract::me(&v);
    crate::extract::tests::block_on(crate::extract::run(&v, &source, &fake, &me, now)).unwrap();
    assert!(!wants(extraction), "extracted already");
    assert!(!wants(Job::Extraction {
        vault: v.clone(),
        source: crate::extract::Source::Recording {
            path: "workspaces/ops/sources/gone.md".into()
        },
    }));
    // Chat titles: a chat that doesn't exist needs none; a session title
    // missing from the cache does.
    assert!(!wants(Job::ChatTitle {
        vault: v.clone(),
        session_key: "s-missing".into(),
        turn_ord: 1,
        run_id: "r".into(),
        only_on: None,
    }));
    assert!(wants(Job::SessionTitle {
        projects: v.clone(),
        project_dir: "-x".into(),
        session_id: unique("no-title"),
    }));
    // Learning: a request that doesn't ask to learn anything.
    assert!(!wants(Job::learning(
        &v,
        "ok",
        &[],
        "Sure.",
        None,
        false,
        None
    )));
    // Prepare and Recap: wanted while none is kept, or when rewriting.
    let meetings = |folder: &str, regenerate: bool| match folder {
        "prep" => Job::PrepPack {
            vault: v.clone(),
            meeting_id: "m-kept".into(),
            regenerate,
            expected_hash: None,
            run_id: None,
        },
        _ => Job::Recap {
            vault: v.clone(),
            meeting_id: "m-kept".into(),
            around: now,
            regenerate,
            recording_id: None,
            expected_hash: None,
        },
    };
    let main = crate::workspaces::main_agent_workspace(&v).unwrap();
    for folder in ["prep", "recaps"] {
        assert!(wants(meetings(folder, false)), "{folder}: none kept");
        let out = v.join(&main.path).join("outputs").join(folder);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(
            out.join("kept.md"),
            "---\nmeeting-id: \"m-kept\"\n---\n\n# Kept\n",
        )
        .unwrap();
        assert!(!wants(meetings(folder, false)), "{folder}: kept");
        assert!(wants(meetings(folder, true)), "{folder}: a rewrite");
    }
    // About-you scan: no scan since it was queued.
    assert!(wants(Job::Profile { vault: v.clone() }));
    // Heartbeat: the beat's own gates; a Run now whatever they say (Q10
    // follow-up), while no beat ran since and no budget holds it.
    assert_eq!(
        wants(Job::Heartbeat {
            vault: v.clone(),
            manual_at: None
        }),
        crate::heartbeat::still_due(&v)
    );
    assert!(wants(Job::Heartbeat {
        vault: v.clone(),
        manual_at: Some(now)
    }));
}

// ── Review round 3 ──────────────────────────────────────────────────────

/// N1: a self-hosted chat's own job never leaves its chat's model. A chat
/// on model B queues its own retitle while B sleeps; Chat titles then goes
/// to model A. The job stays on B's queue, held for the person (Discard:
/// no Run as now, no Try again), neither model drains or wakes for it, and
/// a drain of A that somehow holds it (a queue an older build moved) never
/// claims it. Titles back to B: it waits there again.
#[tokio::test]
async fn a_chats_own_job_never_moves_to_another_model() {
    let files = Files::new();
    let (a, b) = (unique("n1-a"), unique("n1-b"));
    let list = vec![entry(&a, true), entry(&b, true)];
    files.route(JobKind::Titles, &b);
    let own = Job::ChatTitle {
        vault: "/n1".into(),
        session_key: "s-n1".into(),
        turn_ord: 1,
        run_id: "r-n1".into(),
        only_on: Some(b.clone()),
    };
    let long_ago = Utc::now() - chrono::Duration::hours(2);
    enqueue_at(&files.queue(), &b, own.clone(), long_ago).unwrap();
    enqueue_at(
        &files.queue(),
        &b,
        Job::SessionTitle {
            projects: "/p".into(),
            project_dir: "-n1".into(),
            session_id: "n1".into(),
        },
        long_ago,
    )
    .unwrap();
    // The person sends Chat titles to A (what `set` does).
    set_at(
        &files.jobs(),
        Ok(list.clone()),
        JobKind::Titles,
        Some(a.as_str()),
    )
    .unwrap();
    assert_eq!(
        move_kind_at(&files.queue(), JobKind::Titles, &a).unwrap(),
        1,
        "only the other job"
    );
    let queued = files.queued();
    let mine = queued.iter().find(|q| q.job == own).unwrap();
    assert_eq!(mine.entry, b, "stays on its chat's model");
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(place(&jobs, mine), Place::Held);
    let now = Utc::now();
    assert_eq!(
        due(&queued, &jobs, &b, now, true, None),
        None,
        "B never drains for it"
    );
    let view = view_with(&files.jobs(), &files.queue(), Ok(list.clone()));
    let row = view.held.iter().find(|h| h.id == mine.id).unwrap();
    assert!(!row.can_run_as_now && !row.can_try_again);
    assert_eq!(row.only_on.as_deref(), Some(b.as_str()));
    let point = EntryPoint::new(&files, list.clone());
    let l = list.clone();
    let out = drain_with(
        &b,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        move || Ok(l.clone()),
        &point,
    )
    .await;
    assert_eq!((out.woke, out.ran, out.held), (false, 0, 1));
    assert!(drain_check(mine, &b, &files.jobs(), Trigger::RunNow).is_err());
    // Gave up, it still can't be tried on A.
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &mine.id, "500", now).unwrap();
    }
    assert!(try_again_at(&files.queue(), &files.jobs(), &mine.id, now)
        .unwrap_err()
        .contains("runs only there"));
    // A queue an older build moved: on A, it is held there, and a drain of
    // A never claims it.
    let mut moved = files.queued();
    for q in &mut moved {
        if q.job == own {
            q.entry = a.clone();
            q.attempts = 0;
            q.retry_at = None;
        }
    }
    write_queue_at(&files.queue(), &moved).unwrap();
    let on_a = files.queued().into_iter().find(|q| q.job == own).unwrap();
    assert_eq!(place(&jobs, &on_a), Place::Held);
    assert!(drain_check(&on_a, &a, &files.jobs(), Trigger::RunNow)
        .unwrap_err()
        .contains("belongs to a chat"));
    // Titles back to B: it moves home and waits there.
    set_at(
        &files.jobs(),
        Ok(list.clone()),
        JobKind::Titles,
        Some(b.as_str()),
    )
    .unwrap();
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(place(&jobs, &on_a), Place::Elsewhere(b.clone()));
    move_kind_at(&files.queue(), JobKind::Titles, &b).unwrap();
    let home = files.queued().into_iter().find(|q| q.job == own).unwrap();
    assert_eq!(
        (home.entry.as_str(), place(&jobs, &home)),
        (b.as_str(), Place::Here)
    );
}

/// N1: no hold can make a drain loop. A rerun routing held (here the
/// model went back to sleep, so its pinned decision holds it) backs off
/// 30 minutes, so the next look, with the model warm from the wake, starts
/// no drain for it.
#[tokio::test]
async fn a_held_rerun_backs_off_so_no_hold_loops() {
    let files = Files::new();
    let id = unique("n1-loop");
    let list = vec![entry(&id, true)];
    files.route(JobKind::Profile, &id);
    enqueue_at(
        &files.queue(),
        &id,
        profile("/loop"),
        Utc::now() - chrono::Duration::hours(1),
    )
    .unwrap();
    /// Its rerun decides as if the model were asleep again.
    struct Asleep<'a>(&'a Files, Vec<SelfHostedEntry>);
    impl Drainer for Asleep<'_> {
        fn wanted(&self, _q: &Queued) -> bool {
            true
        }
        async fn wake(&self, _entry: &SelfHostedEntry) -> Result<(), String> {
            Ok(())
        }
        async fn replay(&self, job: Job) -> Result<(), String> {
            let entries = self.1.clone();
            match decide_in(
                self.0,
                job.kind(),
                None,
                Some(job.clone()),
                &move || Ok(entries.clone()),
                false,
            ) {
                Decision::Held(held) => Err(held.into_message()),
                other => panic!("held, not {other:?}"),
            }
        }
    }
    let l = list.clone();
    let out = drain_with(
        &id,
        Trigger::Waited,
        &files.jobs(),
        &files.queue(),
        move || Ok(l.clone()),
        &Asleep(&files, list.clone()),
    )
    .await;
    assert_eq!((out.woke, out.ran, out.failed, out.left), (true, 0, 0, 1));
    let q = files.queued().remove(0);
    assert_eq!((q.attempts, &q.claim), (0, &None));
    assert!(q.backing_off(Utc::now()));
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(
        due(&files.queued(), &jobs, &id, Utc::now(), true, None),
        None,
        "no loop"
    );
}

/// N3: another call of the same job (same key) that routing held never
/// marks a rerun that ran as held: success wins unless no decision for the
/// job ran it. A rerun whose only decision was held stays held even when
/// its entry point answers Ok.
#[tokio::test]
async fn another_calls_hold_never_marks_a_rerun_that_ran() {
    let files = Files::new();
    let id = unique("n3");
    let list = vec![entry(&id, true)];
    files.route(JobKind::Profile, &id);
    for vault in ["/n3a", "/n3b"] {
        enqueue_at(&files.queue(), &id, profile(vault), Utc::now()).unwrap();
    }
    let ids: Vec<String> = files.queued().into_iter().map(|q| q.id).collect();
    let entries = move || Ok(list.clone());
    let check = |_: &Queued| Ok(());
    let rerun = Rerun::claim(&files.queue(), &ids[0], Some(&id), check).unwrap();
    let end = rerun
        .run(|job| {
            // Another call of the same job, held (the model looked asleep
            // to it), then the rerun's own decision, which runs it.
            let other = decide_in(
                &files,
                JobKind::Profile,
                None,
                Some(job.clone()),
                &entries,
                false,
            );
            assert!(matches!(other, Decision::Held(_)));
            let own = decide_in(&files, JobKind::Profile, None, Some(job), &entries, true);
            assert!(matches!(own, Decision::Warm(_)));
            async { Ok(()) }
        })
        .await;
    assert_eq!(end, RerunEnd::Done);
    assert!(
        files.queued().iter().all(|q| q.id != ids[0]),
        "off the queue"
    );
    let rerun = Rerun::claim(&files.queue(), &ids[1], Some(&id), check).unwrap();
    let end = rerun
        .run(|job| {
            let only = decide_in(&files, JobKind::Profile, None, Some(job), &entries, false);
            assert!(matches!(only, Decision::Held(_)));
            async { Ok(()) }
        })
        .await;
    assert!(matches!(end, RerunEnd::Held(_)), "{end:?}");
    let q = files.queued().remove(0);
    assert_eq!((q.id.as_str(), q.attempts), (ids[1].as_str(), 0));
}

/// N5: each entry's drain lock is its own file, named by a hash of the id,
/// so ids that differ only in characters a name can't hold never share one.
#[test]
fn drain_locks_never_collide() {
    let files = Files::new();
    let held = try_drain_lock(&files.queue(), "a.b").unwrap().unwrap();
    assert!(drain_busy_at(&files.queue(), "a.b").unwrap());
    assert!(!drain_busy_at(&files.queue(), "a_b").unwrap());
    assert!(!drain_busy_at(&files.queue(), "a/b").unwrap());
    drop(held);
    let names: Vec<String> = std::fs::read_dir(files.dir.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.contains(".drain-"))
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names
        .iter()
        .all(|n| n.len() == QUEUE_FILE.len() + ".drain-".len() + 16 + ".lock".len()));
}

/// N6: Try again while a drain of the model runs (here, held by another
/// process's drain) puts the job back in line and starts no second drain;
/// the next drain runs it.
#[test]
fn try_again_during_a_drain_waits_for_the_next() {
    let files = Files::new();
    let _dir = test_dir(files.dir.path());
    let id = unique("n6");
    files.route(JobKind::Profile, &id);
    enqueue_at(&files.queue(), &id, profile("/n6"), Utc::now()).unwrap();
    let job = files.queued()[0].id.clone();
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &job, "500", Utc::now()).unwrap();
    }
    let busy = try_drain_lock(&files.queue(), &id).unwrap().unwrap();
    let retry = try_again(&job).unwrap();
    assert!(!retry.started, "no drain started now");
    assert!(!drains().running.contains(&id));
    let q = files.queued().remove(0);
    assert_eq!((q.attempts, q.retry_at), (0, None), "back in line");
    assert_eq!(retry.jobs.waiting[0].count, 1, "the view shows it waiting");
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert!(!q.gave_up() && place(&jobs, &q) == Place::Here);
    drop(busy);
}

/// Review round 4 (R3): once Try again put the job back in line, a drain
/// lock that can't even be probed doesn't fail the request (the person
/// would read "Could not try again" about a job that is waiting again): it
/// counts as busy, so the next drain runs the job.
#[test]
fn try_again_never_fails_once_the_job_is_back_in_line() {
    use sha2::{Digest, Sha256};
    let files = Files::new();
    let _dir = test_dir(files.dir.path());
    let id = unique("r3");
    files.route(JobKind::Profile, &id);
    enqueue_at(&files.queue(), &id, profile("/r3"), Utc::now()).unwrap();
    let job = files.queued()[0].id.clone();
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &job, "500", Utc::now()).unwrap();
    }
    // A folder where the drain lock's file goes: the probe fails.
    let hash = format!("{:x}", Sha256::digest(id.as_bytes()));
    let lock = PathBuf::from(format!(
        "{}.drain-{}.lock",
        files.queue().display(),
        &hash[..16]
    ));
    std::fs::create_dir_all(&lock).unwrap();
    assert!(drain_busy_at(&files.queue(), &id).is_err());
    let retry = try_again(&job).expect("not an error");
    assert!(!retry.started);
    assert_eq!(files.queued()[0].attempts, 0, "back in line");
}

/// Review round 4 (R1, R2): only a person's request starts over a job that
/// gave up; an automatic one (the session list's backfill, the heartbeat
/// timer) joins it, replacing its details, and says only Try again runs
/// it. A request joining a job that backs off names the time Hollow tries
/// it again, not "within 30 minutes".
#[test]
fn only_a_persons_request_restarts_a_job_that_gave_up() {
    let files = Files::new();
    // One clock for the notes and the expected times (Q10 follow-up), so a
    // run across midnight can't name another day than the note does.
    let now = Utc::now();
    let list = || Ok(vec![entry("qwen", true)]);
    files.route(JobKind::Titles, "qwen");
    let title = |session: &str| Job::SessionTitle {
        projects: "/p".into(),
        project_dir: "-r1".into(),
        session_id: session.into(),
    };
    let ask = |job: Job, asked: Asked| {
        decide_with(
            JobKind::Titles,
            None,
            || Some(job),
            &Env {
                jobs: &files.jobs(),
                queue: &files.queue(),
                entries: &list,
                warm: &|_: &str| false,
                now,
                asked,
            },
        )
    };
    let message = |d: Decision| match d {
        Decision::Held(Held::Queued { message, added }) => (message, added),
        other => panic!("queued, not {other:?}"),
    };
    let (first, added) = message(ask(title("s-1"), Asked::Auto));
    assert!(added && first.contains("within 30 minutes"), "{first}");
    let id = files.queued()[0].id.clone();
    // Backing off after a failure: the note names its retry time.
    let failed_at = now;
    fail_at(&files.queue(), &id, "the model answered 500", failed_at).unwrap();
    let (note, added) = message(ask(title("s-1"), Asked::Auto));
    // Q10 follow-up: with its day when it isn't today.
    let at = local_when(failed_at + backoff(1), now);
    assert!(!added);
    assert!(note.contains(&format!("tries it again {at} (")), "{note}");
    assert!(!note.contains("within 30 minutes"), "{note}");
    for _ in 1..MAX_ATTEMPTS {
        fail_at(&files.queue(), &id, "the model answered 500", Utc::now()).unwrap();
    }
    assert!(files.queued()[0].gave_up());
    // An automatic request: still given up, its details replaced.
    let (note, _) = message(ask(title("s-1"), Asked::Auto));
    assert!(note.contains("only when you press Try again"), "{note}");
    let q = files.queued().remove(0);
    assert!(q.gave_up() && q.last_error.is_some());
    let jobs = read_jobs_at(&files.jobs()).unwrap();
    assert_eq!(
        due(&files.queued(), &jobs, "qwen", Utc::now(), true, None),
        None,
        "no wake"
    );
    // A person's request starts it over, waiting from now.
    let (note, _) = message(ask(title("s-1"), Asked::Person));
    assert!(note.contains("within 30 minutes"), "{note}");
    let q = files.queued().remove(0);
    assert_eq!((q.attempts, q.last_error, q.retry_at), (0, None, None));
    assert!(Utc::now().signed_duration_since(q.queued_at) < chrono::Duration::minutes(1));
    // The same through `enqueue_as_at` directly.
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &id, "500", Utc::now()).unwrap();
    }
    let auto = enqueue_as_at(
        &files.queue(),
        "qwen",
        title("s-1"),
        Asked::Auto,
        Utc::now(),
    )
    .unwrap();
    assert!(!auto.added && auto.queued.gave_up());
    let person = enqueue_as_at(
        &files.queue(),
        "qwen",
        title("s-1"),
        Asked::Person,
        Utc::now(),
    )
    .unwrap();
    assert!(!person.added && person.queued.attempts == 0);
}

// ── Q7 and Q10 follow-ups ───────────────────────────────────────────────

/// Q10 follow-up: an automatic request merged into a Prepare or Recap
/// rewrite that waits (backing off, or given up) keeps the rewrite whole,
/// so on Try again the drain still wants it with a pack kept and runs it
/// with the person's confirmed hash. A person's newer request replaces the
/// details as before; a heartbeat stays a Run now. The Recap half covers
/// `Job::merged`'s defensive arm: every Recap is a person's today.
#[test]
fn an_automatic_request_never_weakens_a_waiting_rewrite() {
    let files = Files::new();
    files.route(JobKind::Meetings, "qwen");
    let dir = crate::extract::tests::vault_with_transcript();
    let v = dir.path().to_path_buf();
    let main = crate::workspaces::main_agent_workspace(&v).unwrap();
    let prep = |regenerate: bool, hash: Option<&str>, run: Option<&str>| Job::PrepPack {
        vault: v.clone(),
        meeting_id: "m-kept".into(),
        regenerate,
        expected_hash: hash.map(str::to_string),
        run_id: run.map(str::to_string),
    };
    let recap = |regenerate: bool, hash: Option<&str>| Job::Recap {
        vault: v.clone(),
        meeting_id: "m-kept".into(),
        around: Utc::now(),
        regenerate,
        recording_id: None,
        expected_hash: hash.map(str::to_string),
    };
    for (folder, rewrite, checkin) in [
        (
            "prep",
            prep(true, Some("H"), None),
            prep(false, None, Some("run-checkin")),
        ),
        ("recaps", recap(true, Some("H")), recap(false, None)),
    ] {
        let out = v.join(&main.path).join("outputs").join(folder);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(
            out.join("kept.md"),
            "---\nmeeting-id: \"m-kept\"\n---\n\n# Kept\n",
        )
        .unwrap();
        let queue = files.queue();
        let _ = std::fs::remove_file(&queue);
        enqueue_as_at(&queue, "qwen", rewrite.clone(), Asked::Person, Utc::now()).unwrap();
        let id = files.queued()[0].id.clone();
        // Backing off after one failure, then given up: a check-in's
        // request joins it each time and keeps the person's rewrite.
        fail_at(&queue, &id, "the model answered 500", Utc::now()).unwrap();
        let joined =
            enqueue_as_at(&queue, "qwen", checkin.clone(), Asked::Auto, Utc::now()).unwrap();
        assert!(!joined.added);
        assert_eq!(joined.queued.job, rewrite, "{folder}: backing off");
        for _ in 1..MAX_ATTEMPTS {
            fail_at(&queue, &id, "the model answered 500", Utc::now()).unwrap();
        }
        let joined =
            enqueue_as_at(&queue, "qwen", checkin.clone(), Asked::Auto, Utc::now()).unwrap();
        assert!(joined.queued.gave_up());
        assert_eq!(joined.queued.job, rewrite, "{folder}: given up");
        // Try again: back in line as the person's rewrite, and still wanted
        // with a pack kept (the old merge left a plain request it dropped).
        assert_eq!(
            try_again_at(&queue, &files.jobs(), &id, Utc::now()).unwrap(),
            "qwen"
        );
        let q = files.queued().remove(0);
        assert_eq!(q.job, rewrite);
        assert!(still_wanted(&q), "{folder}: the rewrite still runs");
        assert!(
            !still_wanted(&Queued {
                job: checkin.clone(),
                ..q.clone()
            }),
            "{folder}: the plain request a kept pack answers"
        );
        // The person's own newer request replaces it (a newer hash).
        let newer = match rewrite.clone() {
            Job::PrepPack { .. } => prep(true, Some("H2"), None),
            _ => recap(true, Some("H2")),
        };
        let person =
            enqueue_as_at(&queue, "qwen", newer.clone(), Asked::Person, Utc::now()).unwrap();
        assert_eq!(person.queued.job, newer);
        // An automatic rewrite joins a plain request as itself.
        let _ = std::fs::remove_file(&queue);
        enqueue_as_at(&queue, "qwen", checkin.clone(), Asked::Auto, Utc::now()).unwrap();
        let auto_rewrite = match checkin.clone() {
            Job::PrepPack { .. } => prep(true, None, Some("run-agent")),
            _ => recap(true, None),
        };
        let joined = enqueue_as_at(
            &queue,
            "qwen",
            auto_rewrite.clone(),
            Asked::Auto,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(joined.queued.job, auto_rewrite);
    }
    // The heartbeat: a Run now stays one whichever comes second, keeping
    // the later press.
    let (early, late) = (Utc::now() - chrono::Duration::minutes(5), Utc::now());
    let beat = |manual_at: Option<DateTime<Utc>>| Job::Heartbeat {
        vault: "/hb".into(),
        manual_at,
    };
    assert_eq!(
        beat(Some(early)).merged(beat(None), Asked::Auto),
        beat(Some(early))
    );
    assert_eq!(
        beat(None).merged(beat(Some(late)), Asked::Auto),
        beat(Some(late))
    );
    assert_eq!(
        beat(Some(late)).merged(beat(Some(early)), Asked::Auto),
        beat(Some(late))
    );
    assert_eq!(beat(None).merged(beat(None), Asked::Auto), beat(None));
    assert_eq!(
        beat(Some(late)).key(),
        beat(None).key(),
        "one beat per vault"
    );
    // A queue file's scheduled beat (no `manualAt`) reads as one.
    let old: Job = serde_json::from_str(r#"{"kind":"heartbeat","vault":"/hb"}"#).unwrap();
    assert_eq!(old, beat(None));
}

/// Q10 follow-up: a waiting request's note names the retry time with its
/// day when it isn't today, and the later of the job's own backoff and a
/// failed drain's retry time (a drain error holds every drain of the model
/// until then).
#[test]
fn a_waiting_note_names_the_day_and_the_later_retry() {
    use chrono::TimeZone;
    let local = |h: u32, m: u32| {
        chrono::Local
            .with_ymd_and_hms(2026, 11, 3, h, m, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    };
    let now = local(23, 40);
    let minutes = chrono::Duration::minutes;
    assert_eq!(local_when(now + minutes(10), now), "at 23:50");
    assert_eq!(local_when(now + minutes(40), now), "tomorrow at 00:20");
    assert_eq!(
        local_when(now + chrono::Duration::days(2), now),
        "on 5 Nov at 23:40"
    );
    let q = |retry: Option<DateTime<Utc>>, attempts: u32| Queued {
        id: "n".into(),
        entry: "qwen".into(),
        queued_at: now,
        job: profile("/n"),
        attempts,
        last_error: None,
        retry_at: retry,
        claim: None,
    };
    let note = |q: &Queued, drain: Option<DateTime<Utc>>| {
        waiting_note(JobKind::Profile, "Qwen", q, drain, now)
    };
    // Its own backoff, past midnight.
    let own = note(&q(Some(now + minutes(40)), 1), None);
    assert!(
        own.contains("it failed there before, so Hollow tries it again tomorrow at 00:20 ("),
        "{own}"
    );
    // A failed drain's retry later than the job's: that one.
    let later = note(&q(Some(now + minutes(10)), 1), Some(now + minutes(40)));
    assert!(
        later.contains("Hollow's last run of its waiting jobs stopped with an error, so Hollow tries again tomorrow at 00:20 ("),
        "{later}"
    );
    // ... and with no backoff of its own.
    assert!(note(&q(None, 0), Some(now + minutes(5))).contains("tries again at 23:45 ("));
    // The job's own when it is the later; a past drain retry is ignored.
    let own = note(&q(Some(now + minutes(40)), 1), Some(now + minutes(10)));
    assert!(own.contains("tries it again tomorrow at 00:20"), "{own}");
    let past = note(&q(None, 0), Some(now - minutes(5)));
    assert!(past.contains("within 30 minutes"), "{past}");
    // Given up: only Try again, whatever the times.
    let gave_up = note(
        &q(Some(now + minutes(40)), MAX_ATTEMPTS),
        Some(now + minutes(50)),
    );
    assert!(
        gave_up.contains("only when you press Try again"),
        "{gave_up}"
    );
    assert!(!gave_up.contains("tries"), "{gave_up}");

    // Through routing: the drain's retry time from Hollow's drain state.
    let files = Files::new();
    let id = unique("drain-retry");
    files.route(JobKind::Profile, &id);
    let list = || Ok(vec![entry(&id, true)]);
    let at = Utc::now() + minutes(45);
    drains().retry_at.insert(id.clone(), at);
    let decided = decide_in(
        &files,
        JobKind::Profile,
        None,
        Some(profile("/d")),
        &list,
        false,
    );
    drains().retry_at.remove(&id);
    let (message, added) = match decided {
        Decision::Held(Held::Queued { message, added }) => (message, added),
        other => panic!("queued, not {other:?}"),
    };
    assert!(added);
    let when = local_when(at, Utc::now());
    assert!(
        message.contains(&format!("so Hollow tries again {when} (")),
        "{message}"
    );
}

/// Q10 follow-up: Try again names the model the job waits for now (Try
/// again moves a job whose kind goes to another model since), and says a
/// run started, one is under way, or (the drain lock couldn't be probed)
/// neither is known; then no drain starts.
#[test]
fn try_again_names_the_model_it_waits_for_now_and_what_runs_it() {
    let files = Files::new();
    files.route(JobKind::Profile, "qwen");
    enqueue_at(&files.queue(), "qwen", profile("/t"), Utc::now()).unwrap();
    let id = files.queued()[0].id.clone();
    for _ in 0..MAX_ATTEMPTS {
        fail_at(&files.queue(), &id, "500", Utc::now()).unwrap();
    }
    files.route(JobKind::Profile, "llama");
    let to = try_again_at(&files.queue(), &files.jobs(), &id, Utc::now()).unwrap();
    assert_eq!(to, "llama", "moved to where its kind goes now");
    let list = || -> Result<Vec<SelfHostedEntry>, String> {
        Ok(vec![entry("qwen", true), entry("llama", true)])
    };
    let view = |entries: Result<Vec<SelfHostedEntry>, String>| {
        view_with(&files.jobs(), &files.queue(), entries)
    };
    let started = retried(to.clone(), list(), Ok(false), |e| e == "llama", view);
    assert_eq!(
        (
            started.started,
            started.under_way,
            started.entry.as_str(),
            started.name.as_deref()
        ),
        (true, false, "llama", Some("Model llama"))
    );
    assert_eq!(started.jobs.waiting.len(), 1, "the view, read after");
    // A drain of it runs in this process already, or in another one.
    let here = retried(to.clone(), list(), Ok(false), |_| false, view);
    assert!(!here.started && here.under_way);
    let there = retried(
        to.clone(),
        list(),
        Ok(true),
        |_| panic!("no second drain"),
        view,
    );
    assert!(!there.started && there.under_way);
    // The lock couldn't be probed: no drain, and nothing claimed under way.
    let unknown = retried(
        to.clone(),
        list(),
        Err("lock: Is a directory".into()),
        |_| panic!("no drain"),
        view,
    );
    assert!(!unknown.started && !unknown.under_way);
    // A model that left the list is named by its id in Settings.
    let gone = retried(to, Ok(Vec::new()), Ok(true), |_| false, view);
    assert_eq!(gone.name, None);
}

// ── Q10 follow-ups, second review ───────────────────────────────────────

/// A drainer that asks the real `still_wanted` and counts wakes and reruns.
#[derive(Default)]
struct RealWanted {
    wakes: AtomicUsize,
    replays: AtomicUsize,
}

impl Drainer for RealWanted {
    fn wanted(&self, q: &Queued) -> bool {
        still_wanted(q)
    }
    async fn wake(&self, _entry: &SelfHostedEntry) -> Result<(), String> {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn replay(&self, _job: Job) -> Result<(), String> {
        self.replays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A heartbeat budget of 10 tokens for vault `v`, 500 used: it holds every
/// beat now.
fn block_heartbeat_budget(v: &Path) {
    let store = crate::budgets::Store::for_app(v);
    let def: crate::budgets::BudgetDef = serde_json::from_value(serde_json::json!({
        "id": "hb", "scope": "heartbeat", "metric": "tokens", "window": "day", "limit": 10
    }))
    .unwrap();
    store
        .as_ref()
        .unwrap()
        .save(&crate::budgets::BudgetsFile {
            budgets: vec![def],
            ..crate::budgets::BudgetsFile::default()
        })
        .unwrap();
    let mut row = crate::session_store::tests::usage_row_fixture(
        "hb-row",
        &crate::session_store::db_time(Utc::now()),
        "success",
    );
    row.surface = "heartbeat".into();
    row.input_tokens = Some(500);
    row.output_tokens = Some(0);
    row.cost_status = "unpriced".into();
    crate::session_store::upsert_usage(v, &row).unwrap();
}

fn heartbeat_log(v: &Path) -> String {
    std::fs::read_to_string(v.join(".heartbeat-log.jsonl")).unwrap_or_default()
}

/// Review M1: a Run now that waits for its model while a budget holds the
/// heartbeat is dropped before any wake, with a note in the heartbeat's
/// log (the Activity ledger), and never counts a failed attempt.
#[tokio::test]
async fn a_queued_run_now_a_budget_holds_is_dropped_before_any_wake() {
    let id = unique("run-now-budget");
    let files = Files::new();
    files.route(JobKind::Heartbeat, &id);
    let vault = tempfile::tempdir().unwrap();
    let v = vault.path().to_path_buf();
    block_heartbeat_budget(&v);
    let beat = Job::Heartbeat {
        vault: v.clone(),
        manual_at: Some(Utc::now()),
    };
    enqueue_at(&files.queue(), &id, beat, Utc::now()).unwrap();
    let drainer = RealWanted::default();
    let list = || Ok(vec![entry(&id, true)]);
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &drainer,
    )
    .await;
    assert_eq!(drainer.wakes.load(Ordering::SeqCst), 0, "no wake");
    assert_eq!(drainer.replays.load(Ordering::SeqCst), 0);
    assert_eq!((out.dropped, out.ran, out.failed), (1, 0, 0));
    assert!(files.queued().is_empty(), "dropped, not an attempt");
    let log = heartbeat_log(&v);
    assert!(
        log.contains("Run now waited for its self-hosted model and was dropped: Budget paused:"),
        "{log}"
    );
    assert!(log.contains("\"status\":\"skipped\""), "{log}");
}

/// Review M2: a Run now that waited is wanted only while no beat was
/// admitted after it was pressed (a second Run now once the model woke, or
/// the scheduled beat), checked before any wake and again right before its
/// rerun, so two beats never run back to back. One pressed after that beat
/// still runs.
#[tokio::test]
async fn a_queued_run_now_never_runs_a_second_beat_after_one_ran() {
    let id = unique("run-now-twice");
    let files = Files::new();
    files.route(JobKind::Heartbeat, &id);
    let vault = tempfile::tempdir().unwrap();
    let v = vault.path().to_path_buf();
    let asked = Utc::now() - chrono::Duration::minutes(5);
    let beat = Job::Heartbeat {
        vault: v.clone(),
        manual_at: Some(asked),
    };
    enqueue_at(&files.queue(), &id, beat, asked).unwrap();
    let q = files.queued().remove(0);
    assert!(still_wanted(&q), "nothing ran since");
    assert_eq!(drain_check(&q, &id, &files.jobs(), Trigger::Waited), Ok(()));
    // A beat is admitted meanwhile.
    let cfg = crate::heartbeat::HeartbeatConfig {
        work_hours_start: 0,
        work_hours_end: 24,
        min_interval_min: 30,
        daily_budget: 10,
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let admitted =
        crate::heartbeat::gate_queued_beat(&v, true, true, cfg, 12, &today, Utc::now(), |v| {
            Ok(crate::heartbeat::prepared_beat_for_test(v))
        });
    let Ok(Some((_, _, lease))) = admitted else {
        panic!("a beat is admitted");
    };
    lease.finish("success").unwrap();
    assert_eq!(
        drain_check(&q, &id, &files.jobs(), Trigger::Waited),
        Err("a beat ran after its Run now".to_string())
    );
    let drainer = RealWanted::default();
    let list = || Ok(vec![entry(&id, true)]);
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &drainer,
    )
    .await;
    assert_eq!(drainer.wakes.load(Ordering::SeqCst), 0, "no wake");
    assert_eq!(out.dropped, 1);
    assert!(files.queued().is_empty());
    assert!(heartbeat_log(&v).contains("a beat ran after you pressed Run now"));
    // Pressed after that beat: it still runs.
    let later = Queued {
        job: Job::Heartbeat {
            vault: v.clone(),
            manual_at: Some(Utc::now() + chrono::Duration::seconds(1)),
        },
        ..q
    };
    assert!(still_wanted(&later));
}

/// Review M2: switching the heartbeat off drops its beats that wait for a
/// self-hosted model, a Run now included, but not one a rerun is running,
/// nor any other kind's job; the start-up sync never does (only the
/// person's switch passes `switched`).
#[test]
fn switching_the_heartbeat_off_drops_its_waiting_beats() {
    let files = Files::new();
    let now = Utc::now();
    let beat = |vault: &str, manual: bool| Job::Heartbeat {
        vault: vault.into(),
        manual_at: manual.then_some(now),
    };
    for (entry, job) in [
        ("qwen", beat("/a", true)),
        ("qwen", beat("/b", false)),
        ("qwen", profile("/p")),
        ("llama", beat("/c", true)),
    ] {
        enqueue_at(&files.queue(), entry, job, now).unwrap();
    }
    let running = files
        .queued()
        .into_iter()
        .find(|q| q.job == beat("/c", true))
        .unwrap()
        .id;
    let (_q, _claimed) = claim_at(&files.queue(), &running, now, |_| Ok(())).unwrap();
    assert_eq!(drop_heartbeats_at(&files.queue(), now).unwrap(), 2);
    let left: Vec<Job> = files.queued().into_iter().map(|q| q.job).collect();
    assert_eq!(left, vec![profile("/p"), beat("/c", true)]);
    assert_eq!(drop_heartbeats_at(&files.queue(), now).unwrap(), 0);
    // Only the person's switch drops them.
    let commands = include_str!("commands.rs");
    let body = &commands[commands
        .find(concat!("pub fn ", "set_heartbeat_enabled("))
        .unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    assert!(
        body.contains("if crate::heartbeat::switched_off(was, enabled, switched) {"),
        "{body}"
    );
    assert!(
        body.contains("self_hosted_jobs::drop_heartbeats();"),
        "{body}"
    );
    let runtime = include_str!("../../src/store/runtime.ts");
    assert!(runtime.contains("api.setHeartbeatEnabled(v, true)"));
    assert!(runtime.contains("api.setHeartbeatEnabled(heartbeatEnabled()).catch"));
}

/// A queued Run now's rerun for the delta review's test: let in as a beat
/// (Run now's gate stamps it), then ended as `end` says.
async fn let_in_and_end(v: &Path, end: Result<(), String>) -> Result<(), String> {
    let cfg = crate::heartbeat::HeartbeatConfig {
        work_hours_start: 0,
        work_hours_end: 24,
        min_interval_min: 30,
        daily_budget: 10,
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let admitted =
        crate::heartbeat::gate_queued_beat(v, true, true, cfg, 12, &today, Utc::now(), |v| {
            Ok(crate::heartbeat::prepared_beat_for_test(v))
        });
    let Ok(Some((_, _, lease))) = admitted else {
        panic!("the Run now is let in");
    };
    lease
        .finish(if end.is_ok() { "success" } else { "error" })
        .unwrap();
    end
}

/// Delta review (medium): a Run now's own rerun is let in as a beat before
/// it fails, and that beat must not drop it as "a beat ran after you
/// pressed Run now". After the failure it is kept, backing off (its time
/// moved to the failure, `fail_at`); its next rerun that succeeds leaves the
/// queue, so no second beat follows.
#[tokio::test]
async fn a_failed_run_now_is_kept_and_runs_once_more() {
    let id = unique("run-now-fails");
    let files = Files::new();
    files.route(JobKind::Heartbeat, &id);
    let vault = tempfile::tempdir().unwrap();
    let v = vault.path().to_path_buf();
    let asked = Utc::now() - chrono::Duration::minutes(5);
    let beat = Job::Heartbeat {
        vault: v.clone(),
        manual_at: Some(asked),
    };
    enqueue_at(&files.queue(), &id, beat, asked).unwrap();
    let job = files.queued()[0].id.clone();
    let check = |q: &Queued| drain_check(q, &id, &files.jobs(), Trigger::RunNow);
    let rerun = Rerun::claim(&files.queue(), &job, Some(&id), check).unwrap();
    let end = rerun
        .run(|_| let_in_and_end(&v, Err("the model answered 500".into())))
        .await;
    assert!(matches!(end, RerunEnd::Failed(_)), "{end:?}");
    let q = files.queued().remove(0);
    assert_eq!(q.attempts, 1);
    assert!(q.backing_off(Utc::now()));
    assert!(matches!(q.job, Job::Heartbeat { manual_at: Some(at), .. } if at > asked));
    assert!(still_wanted(&q), "its own start is no beat after the press");
    assert_eq!(drain_check(&q, &id, &files.jobs(), Trigger::RunNow), Ok(()));
    assert!(!heartbeat_log(&v).contains("a beat ran after you pressed Run now"));
    // Its next rerun succeeds: off the queue, and a later look runs nothing.
    let check = |q: &Queued| drain_check(q, &id, &files.jobs(), Trigger::RunNow);
    let rerun = Rerun::claim(&files.queue(), &job, Some(&id), check).unwrap();
    assert_eq!(
        rerun.run(|_| let_in_and_end(&v, Ok(()))).await,
        RerunEnd::Done
    );
    assert!(files.queued().is_empty());
    let drainer = RealWanted::default();
    let list = || Ok(vec![entry(&id, true)]);
    let out = drain_with(
        &id,
        Trigger::RunNow,
        &files.jobs(),
        &files.queue(),
        list,
        &drainer,
    )
    .await;
    assert_eq!(
        (
            drainer.wakes.load(Ordering::SeqCst),
            drainer.replays.load(Ordering::SeqCst),
            out.ran
        ),
        (0, 0, 0),
        "no second beat"
    );
}
