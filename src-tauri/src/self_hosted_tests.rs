//! Tests for `self_hosted` (attached from `self_hosted.rs` as `#[path] mod
//! tests`, so `super::` is `crate::self_hosted`).
use super::*;
use crate::agent::{self, AgentKind, ProcessScope, RunExtras, RunSpec};
use std::path::PathBuf;

const KEY: &str = "hsr_0123456789abcdef0123456789abcdef";

fn target() -> RunTarget {
    RunTarget {
        entry: "qwen".into(),
        name: "Qwen3.8 27B".into(),
        base_url: "http://127.0.0.1:41234".into(),
        run_key: RunKey::from(KEY),
        served_model: "qwen3.8-27b".into(),
        context_window: 131_072,
        effort_levels: vec!["low".into(), "medium".into(), "xhigh".into()],
        api_style: crate::self_hosted_registry::API_ANTHROPIC.into(),
        codex_home: None,
    }
}

fn spec(scope: ProcessScope, self_hosted: Option<RunTarget>) -> RunSpec {
    RunSpec {
        prompt: "hello".into(),
        resume: None,
        model: Some("opus".into()),
        perm: "acceptEdits".into(),
        cwd: PathBuf::from("/tmp"),
        allowed_tools: vec![],
        disallowed_tools: vec![],
        interactive: true,
        mcp_config: None,
        append_system_prompt: None,
        process_scope: scope,
        usage_stream: false,
        max_budget_usd: None,
        extras: RunExtras {
            self_hosted,
            ..RunExtras::default()
        },
    }
}

/// The command's env as (name, value); `None` is a removal.
fn envs(cmd: &Command) -> Vec<(String, Option<String>)> {
    cmd.as_std()
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

fn env_set(cmd: &Command, key: &str) -> Option<String> {
    envs(cmd)
        .into_iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v)
}

fn args(cmd: &Command) -> Vec<String> {
    // R5b: a private appended prompt reads as the inline flag.
    crate::secret_fds::expand_append(
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
    )
}

#[test]
fn only_loopback_http_urls_are_forwarder_addresses() {
    for ok in [
        "http://127.0.0.1:41234",
        "http://[::1]:9000",
        "http://127.0.0.1:1/v1",
    ] {
        assert!(is_loopback_url(ok), "{ok}");
    }
    for bad in [
        "https://127.0.0.1:41234",
        "http://qwen-27b-x6jmsblvea-uc.a.run.app",
        "http://127.0.0.1.evil.com:80",
        "http://10.0.0.5:8080",
        "http://127.0.0.1:abc",
        "http://localhost:8080",
        "127.0.0.1:8080",
        "",
    ] {
        assert!(!is_loopback_url(bad), "{bad}");
    }
}

#[test]
fn the_run_key_never_prints() {
    let shown = format!("{:?}", target());
    assert!(!shown.contains(KEY), "{shown}");
    assert!(shown.contains("<redacted>"));
}

/// Q1's verify: a self-hosted run's command carries no Anthropic credential,
/// its base URL is loopback, and its model is the served id.
#[test]
fn a_self_hosted_run_carries_no_anthropic_credential() {
    let mut run = spec(ProcessScope::GuiOwned, Some(target()));
    // Whatever a launch point's extra env carried is overridden.
    run.extras.env = vec![
        (
            "ANTHROPIC_BASE_URL".into(),
            "https://api.anthropic.com".into(),
        ),
        ("ANTHROPIC_API_KEY".into(), "sk-ant-api-should-go".into()),
        (OAUTH_TOKEN_ENV.into(), "sk-ant-oat-should-go".into()),
    ];
    let cmd = agent::build_command(AgentKind::Claude, &run);
    assert!(!agent::is_unavailable_command(&cmd), "{:?}", args(&cmd));
    let all = envs(&cmd);
    for (name, value) in &all {
        if let Some(value) = value {
            assert!(!value.contains("sk-ant"), "{name} carries a credential");
        }
    }
    for removed in REMOVED_ENVS {
        assert_eq!(env_set(&cmd, removed), None, "{removed} is removed");
    }
    assert_eq!(env_set(&cmd, crate::claude_accounts::ACCOUNT_ENV), None);
    let base = env_set(&cmd, BASE_URL_ENV).expect("a base URL");
    assert!(is_loopback_url(&base), "{base}");
    assert_eq!(env_set(&cmd, AUTH_TOKEN_ENV).as_deref(), Some(KEY));
    assert_eq!(env_set(&cmd, MODEL_ENV).as_deref(), Some("qwen3.8-27b"));
    for alias in ALIAS_MODEL_ENVS {
        assert_eq!(env_set(&cmd, alias).as_deref(), Some("qwen3.8-27b"));
    }
    assert_eq!(env_set(&cmd, NONESSENTIAL_ENV).as_deref(), Some("1"));
    assert_eq!(env_set(&cmd, API_TIMEOUT_ENV).as_deref(), Some("900000"));
    assert_eq!(env_set(&cmd, TOOL_SEARCH_ENV).as_deref(), Some("false"));
    // Every ANTHROPIC_* variable that is set is one of ours.
    let ours = [BASE_URL_ENV, AUTH_TOKEN_ENV, MODEL_ENV]
        .into_iter()
        .chain(ALIAS_MODEL_ENVS)
        .collect::<Vec<_>>();
    for (name, value) in &all {
        if name.starts_with("ANTHROPIC_") && value.is_some() {
            assert!(ours.contains(&name.as_str()), "{name}");
        }
    }
    // The served model on argv, the run key never.
    let argv = args(&cmd);
    let at = argv.iter().position(|a| a == "--model").expect("--model");
    assert_eq!(argv[at + 1], "qwen3.8-27b");
    assert!(
        argv.iter().all(|a| !a.contains(KEY)),
        "the key is not on argv"
    );
}

/// Every `ANTHROPIC_*` variable the app inherited is removed, not only the
/// ones Hollow knows by name.
#[test]
fn inherited_anthropic_variables_are_removed() {
    let mut cmd = Command::new("/bin/true");
    let inherited = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
        "HOME",
    ]
    .map(std::ffi::OsString::from);
    apply_env_from(&mut cmd, &target(), inherited);
    let all = envs(&cmd);
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(
            all.iter().any(|(k, v)| k == name && v.is_none()),
            "{name} is removed"
        );
    }
    assert!(
        !all.iter().any(|(k, _)| k == "HOME"),
        "others are left alone"
    );
}

/// A normal Claude run is unchanged by Q1: the exact argv of a canonical
/// spec, and no self-hosted variable in its env.
#[test]
fn a_normal_claude_run_is_unchanged() {
    let run = spec(ProcessScope::Inherited, None);
    let cmd = agent::build_command(AgentKind::Claude, &run);
    let policy = agent::run_policy(AgentKind::Claude, &run);
    assert_eq!(
        args(&cmd),
        vec![
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "acceptEdits",
            "--include-partial-messages",
            "--model",
            "opus",
            "--allowed-tools",
            "WebFetch",
            "WebSearch",
            "Bash(gh repo view:*)",
            "Bash(gh pr view:*)",
            "Bash(gh pr list:*)",
            "Bash(gh pr diff:*)",
            "Bash(gh issue view:*)",
            "Bash(gh issue list:*)",
            "Bash(gh search:*)",
            "Bash(printenv HOLLOW_SCRATCH_DIR)",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--append-system-prompt",
            policy.as_str(),
        ]
    );
    let names: Vec<String> = envs(&cmd).into_iter().map(|(k, _)| k).collect();
    // NV9: and the two fixed run settings every Claude run carries.
    assert_eq!(
        names,
        vec![
            "BASH_MAX_OUTPUT_LENGTH",
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
            "CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK",
            "PATH"
        ],
        "{names:?}"
    );
    // GUI-owned: still no self-hosted variable, and no removal of the
    // account token beyond what account routing does.
    let gui = agent::build_command(AgentKind::Claude, &spec(ProcessScope::GuiOwned, None));
    for (name, _) in envs(&gui) {
        assert!(
            !name.starts_with("ANTHROPIC_")
                && name != TOOL_SEARCH_ENV
                && name != API_TIMEOUT_ENV
                && name != NONESSENTIAL_ENV,
            "{name}"
        );
    }
}

#[test]
fn a_self_hosted_run_fails_closed_off_claude_off_this_mac_or_off_loopback() {
    let unavailable = |cmd: &Command| {
        assert!(agent::is_unavailable_command(cmd));
        args(cmd).join(" ")
    };
    let codex = agent::build_command(
        AgentKind::Codex,
        &spec(ProcessScope::Inherited, Some(target())),
    );
    assert!(unavailable(&codex).contains("runs on Claude only"));

    let mut far = target();
    far.base_url = "https://qwen-27b-x6jmsblvea-uc.a.run.app".into();
    let far = agent::build_command(AgentKind::Claude, &spec(ProcessScope::Inherited, Some(far)));
    assert!(unavailable(&far).contains("no local forwarder address"));

    let mut keyless = target();
    keyless.run_key = RunKey::default();
    let keyless = agent::build_command(
        AgentKind::Claude,
        &spec(ProcessScope::Inherited, Some(keyless)),
    );
    assert!(unavailable(&keyless).contains("isn't ready"));

    // The Mac mini runner: refused before any remote work starts.
    let mut remote = spec(ProcessScope::Inherited, Some(target()));
    remote.extras.remote = Some(crate::remote_session::RemoteTarget::default());
    let remote = agent::build_command(AgentKind::Claude, &remote);
    assert!(unavailable(&remote).contains("runs on this Mac only"));
}

/// Q1 review: the account decision itself, so a regression that routes a
/// self-hosted run (even with a claude.ai connector or as long work) fails
/// here rather than passing on a test build with no account registry.
#[test]
fn a_self_hosted_run_gets_no_account_step() {
    use agent::AccountRoute;
    let mut run = spec(ProcessScope::GuiOwned, Some(target()));
    run.extras.long_work = true;
    run.extras.account = Some("acct-1".into());
    run.extras.account_affinity = Some("chat:s-1".into());
    run.extras.shared_connectors = vec![agent::SharedConnector {
        claude_login: true,
        ..Default::default()
    }];
    assert_eq!(
        agent::account_route(AgentKind::Claude, &run),
        AccountRoute::None
    );
    // The same spec without the endpoint takes the usual routes.
    run.extras.self_hosted = None;
    assert_eq!(
        agent::account_route(AgentKind::Claude, &run),
        AccountRoute::Login
    );
    run.extras.shared_connectors.clear();
    assert_eq!(
        agent::account_route(AgentKind::Claude, &run),
        AccountRoute::LongWork
    );
    run.extras.long_work = false;
    assert_eq!(
        agent::account_route(AgentKind::Claude, &run),
        AccountRoute::Routed
    );
    run.process_scope = ProcessScope::Inherited;
    assert_eq!(
        agent::account_route(AgentKind::Claude, &run),
        AccountRoute::None
    );
    // And the command: the account variables are removed, not set.
    let cmd = agent::build_command(
        AgentKind::Claude,
        &spec(ProcessScope::GuiOwned, Some(target())),
    );
    assert_eq!(env_set(&cmd, crate::claude_accounts::ACCOUNT_ENV), None);
    assert!(envs(&cmd)
        .iter()
        .any(|(k, v)| k == OAUTH_TOKEN_ENV && v.is_none()));
}

/// Q1 review: a settings file can't undo the env. The run's `--settings`
/// pins the endpoint and blanks every credential, and holds no secret.
#[test]
fn the_run_settings_pin_the_endpoint_and_hold_no_secret() {
    let json = settings_json(&target());
    assert!(!json.contains(KEY));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let env = &v["env"];
    assert_eq!(env[BASE_URL_ENV], "http://127.0.0.1:41234");
    assert_eq!(env[MODEL_ENV], "qwen3.8-27b");
    for alias in ALIAS_MODEL_ENVS {
        assert_eq!(env[alias], "qwen3.8-27b");
    }
    for blank in BLANKED_IN_SETTINGS {
        assert_eq!(env[blank], "", "{blank}");
    }
    assert_eq!(env[TOOL_SEARCH_ENV], "false");
    assert!(env.get(AUTH_TOKEN_ENV).is_none(), "the key stays off argv");
    let cmd = agent::build_command(
        AgentKind::Claude,
        &spec(ProcessScope::Inherited, Some(target())),
    );
    let argv = args(&cmd);
    let at = argv
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings");
    assert_eq!(argv[at + 1], json);
    // NO_PROXY keeps the loopback call off any inherited proxy.
    let no_proxy = env_set(&cmd, "NO_PROXY").expect("NO_PROXY");
    assert!(no_proxy.split(',').any(|h| h == "127.0.0.1"), "{no_proxy}");
    assert!(no_proxy.split(',').any(|h| h == "[::1]"), "{no_proxy}");
    assert_eq!(env["NO_PROXY"], NO_PROXY_HOSTS);
    // A run's own NO_PROXY is kept and extended.
    let mut own = spec(ProcessScope::Inherited, Some(target()));
    own.extras.env = vec![("NO_PROXY".into(), "corp.internal".into())];
    let own = agent::build_command(AgentKind::Claude, &own);
    let kept = env_set(&own, "NO_PROXY").unwrap();
    assert!(kept.starts_with("corp.internal,"), "{kept}");
}

#[test]
fn logs_mask_a_run_key() {
    let masked = crate::claude_runner::redact(&format!("key {KEY} end"));
    assert!(!masked.contains(KEY), "{masked}");
    assert!(masked.contains("[run-key]"), "{masked}");
    let glued = crate::claude_runner::redact(&format!(
        "{KEY}_tail {}",
        KEY.to_uppercase().replace("HSR_", "hsr_")
    ));
    assert!(!glued.contains(&KEY[4..]), "{glued}");
    assert!(!glued.to_lowercase().contains(&KEY[4..]), "{glued}");
    assert!(is_run_key(KEY));
    for bad in [
        "hsr_short",
        "sk-ant-0123456789abcdef0123456789abcdef",
        "hsr_0123456789ABCDEF0123456789ABCDEF",
    ] {
        assert!(!is_run_key(bad), "{bad}");
    }
}

// ── Q3: effort and context ───────────────────────────────────────────────

/// Q3's verify: every level maps to one the endpoint takes; `high` is
/// never sent to Qwen and `medium` is the default.
#[test]
fn effort_maps_each_level_to_one_the_endpoint_takes() {
    let t = target();
    for (asked, sent) in [
        (Some("low"), "low"),
        (Some("medium"), "medium"),
        (Some("high"), "medium"),
        (Some("xhigh"), "xhigh"),
        (Some("max"), "xhigh"),
        (Some("ultracode"), "xhigh"),
        (Some("minimal"), "low"),
        (Some("bogus"), "medium"),
        (None, "medium"),
    ] {
        assert_eq!(t.effort(asked).as_deref(), Some(sent), "{asked:?}");
    }
    // The command: `--effort medium` with nothing saved, and never `high`.
    for saved in [None, Some("high")] {
        let mut run = spec(ProcessScope::Inherited, Some(target()));
        run.extras.effort = saved.map(String::from);
        let argv = args(&agent::build_command(AgentKind::Claude, &run));
        let at = argv.iter().position(|a| a == "--effort").expect("--effort");
        assert_eq!(argv[at + 1], "medium", "{saved:?}");
    }
    // Remembered levels steer `effort::resolve` for the served id too.
    let model = format!("qwen-effort-test-{}", std::process::id());
    remember(&model, 131_072, &t.effort_levels);
    let resolve =
        |level| crate::effort::resolve(AgentKind::Claude, Some(&model), Some(level)).effort;
    assert_eq!(resolve("high").as_deref(), Some("medium"));
    assert_eq!(resolve("max").as_deref(), Some("xhigh"));
    assert_eq!(resolve("ultracode").as_deref(), Some("xhigh"));
    assert_eq!(resolve("low").as_deref(), Some("low"));
}

#[test]
fn the_window_compact_window_and_answer_budget_add_up() {
    let mut t = target();
    assert_eq!((t.compact_window(), t.answer_budget()), (100_000, 31_072));
    for (window, compact, answer) in [
        (110_000, 100_000, 10_000),
        (200_000, 168_000, 32_000),
        (1_000_000, 968_000, 32_000),
    ] {
        t.context_window = window;
        assert_eq!(
            (t.compact_window(), t.answer_budget()),
            (compact, answer),
            "{window}"
        );
        assert_eq!(t.compact_window() + t.answer_budget(), window);
    }
}

/// Q3: the run's window settings come from its entry, over Settings' own
/// compaction window (OB22), on the env and in the pinned settings.
#[test]
fn a_self_hosted_run_gets_its_window_over_settings() {
    let mut run = spec(ProcessScope::Inherited, Some(target()));
    run.extras.env = vec![(agent::CLAUDE_COMPACT_WINDOW_ENV.into(), "500000".into())];
    let cmd = agent::build_command(AgentKind::Claude, &run);
    assert_eq!(env_set(&cmd, MAX_CONTEXT_ENV).as_deref(), Some("131072"));
    assert_eq!(
        env_set(&cmd, agent::CLAUDE_COMPACT_WINDOW_ENV).as_deref(),
        Some("100000")
    );
    assert_eq!(env_set(&cmd, MAX_OUTPUT_ENV).as_deref(), Some("31072"));
    assert_eq!(
        agent::compact_window(AgentKind::Claude, &run),
        Some(100_000)
    );
    let pinned: serde_json::Value = serde_json::from_str(&settings_json(&target())).unwrap();
    assert_eq!(pinned["env"][MAX_CONTEXT_ENV], "131072");
    assert_eq!(pinned["env"][agent::CLAUDE_COMPACT_WINDOW_ENV], "100000");
    assert_eq!(pinned["env"][MAX_OUTPUT_ENV], "31072");
}

/// Q3: the context gauge (and so `sessions.context`) measures a
/// self-hosted model against its own window.
#[test]
fn the_gauge_knows_a_remembered_window() {
    let model = format!("qwen-gauge-test-{}", std::process::id());
    assert_eq!(
        crate::context_gauge::context_window(AgentKind::Claude, Some(&model), None),
        Some(200_000),
        "unknown ids keep Claude's default"
    );
    remember(&model, 131_072, &["medium".to_string()]);
    assert_eq!(
        crate::context_gauge::context_window(AgentKind::Claude, Some(&model), Some(120_000)),
        Some(131_072)
    );
    assert_eq!(
        crate::context_gauge::exact_window(AgentKind::Claude, Some(&model.to_uppercase())),
        Some(131_072)
    );
    assert_eq!(
        crate::context_gauge::exact_window(AgentKind::Codex, Some(&model)),
        None
    );
}

/// Q3's verify: a pack over the cap is cut with a note, keeping its head
/// and the person's message at its end; one under it is unchanged.
#[test]
fn a_pack_over_the_cap_is_trimmed_with_a_note() {
    let t = target();
    let cap = t.pack_cap_chars();
    assert_eq!(cap, (100_000 - HARNESS_RESERVE as usize) * CHARS_PER_TOKEN);
    let small = "<hollow-workspace>brief</hollow-workspace>\nWhat changed today?";
    assert_eq!(fit_pack(small, 1_000, &t), (small.to_string(), None));

    let mut big = String::from("HEAD-START\n");
    while big.len() < cap + 50_000 {
        big.push_str("evidence line with some words é\n");
    }
    big.push_str("Please summarise the plan. REQUEST-END");
    let (fitted, note) = fit_pack(&big, 10_000, &t);
    let note = note.expect("a note");
    assert!(
        fitted.len() <= cap - 10_000,
        "{} > {}",
        fitted.len(),
        cap - 10_000
    );
    assert!(fitted.starts_with("HEAD-START\n"));
    assert!(fitted.ends_with("Please summarise the plan. REQUEST-END"));
    assert!(fitted.contains(&note));
    assert!(
        note.contains("Qwen3.8 27B") && note.contains("131072"),
        "{note}"
    );
}

/// Q3 review: a cap too small for anything still keeps the person's
/// message, and an ultracode request says why it ran lower.
#[test]
fn the_request_survives_a_tiny_cap_and_ultracode_says_why() {
    let t = target();
    let mut big = "x".repeat(50_000);
    big.push_str("\nWhat should I do next? REQUEST-END");
    let (fitted, note) = fit_pack(&big, 10_000_000, &t);
    assert!(note.is_some());
    assert!(
        fitted.ends_with("What should I do next? REQUEST-END"),
        "{}",
        &fitted[fitted.len() - 80..]
    );
    assert!(fitted.len() < MIN_TAIL + 400, "{}", fitted.len());

    let mut run = spec(ProcessScope::Inherited, Some(target()));
    run.extras.effort = Some("ultracode".into());
    let choice = agent::run_effort(AgentKind::Claude, &run);
    assert_eq!(choice.effort.as_deref(), Some("xhigh"));
    assert!(choice
        .note
        .unwrap()
        .contains("Ultracode isn't available on Qwen3.8 27B"));
    run.extras.effort = Some("high".into());
    assert_eq!(agent::run_effort(AgentKind::Claude, &run).note, None);
}

/// Q11: a self-hosted turn (a bake-off variant's too) waits out a cold
/// start: a watchdog or deadline set shorter than one request's timeout is
/// raised to it and a minute more; off stays off, longer stays, and any
/// other turn's limits are untouched.
#[test]
fn a_self_hosted_turns_limits_wait_out_a_cold_start() {
    use crate::agent_settings::RunLimits;
    let min = |m: u64| Some(Duration::from_secs(m * 60));
    assert_eq!(COLD_START_FLOOR, Duration::from_secs(16 * 60));
    assert!(COLD_START_FLOOR > Duration::from_millis(u64::from(API_TIMEOUT_MS)));
    let short = RunLimits {
        deadline: min(5),
        stall_after: min(3),
    };
    let t = target();
    let raised = cold_start_limits(short, Some(&t));
    assert_eq!(raised.stall_after, Some(COLD_START_FLOOR));
    assert_eq!(raised.deadline, Some(COLD_START_FLOOR));
    assert_eq!(cold_start_limits(short, None), short, "an ordinary turn");
    let long = RunLimits {
        deadline: min(90),
        stall_after: min(30),
    };
    assert_eq!(cold_start_limits(long, Some(&t)), long);
    let off = RunLimits {
        deadline: None,
        stall_after: None,
    };
    assert_eq!(cold_start_limits(off, Some(&t)), off, "off stays off");
}

// ── Q5: a chat turn on a self-hosted model ──────────────────────────────

use crate::self_hosted_registry::{preset, SelfHostedAuth, SelfHostedEntry};

/// An ordinary model's route must never read the registry.
fn unread() -> Result<Vec<SelfHostedEntry>, String> {
    panic!("an ordinary turn never reads the self-hosted registry")
}

fn refused(route: Route) -> String {
    match route {
        Route::Refused(why) => why,
        other => panic!("refused, not {other:?}"),
    }
}

#[test]
fn a_chat_model_id_names_its_registry_entry() {
    assert_eq!(model_id("qwen"), "self-hosted:qwen");
    assert_eq!(entry_id("self-hosted:qwen"), Some("qwen"));
    assert_eq!(entry_id(" self-hosted:qwen "), Some("qwen"));
    assert_eq!(entry_id("self-hosted:"), Some(""));
    for other in ["opus", "claude-opus-5-5", "qwen3.8-27b", "", "self-hosted"] {
        assert_eq!(entry_id(other), None, "{other}");
    }
    assert_eq!(menu_label("Qwen3.8 27B"), "Qwen3.8 27B · self-hosted");
}

/// Q5: a turn on `self-hosted:<id>` runs on that entry when it is saved,
/// switched on and Anthropic-style; otherwise it is refused with a reason
/// that names where to fix it, and never falls back to another model.
#[test]
fn a_turn_routes_to_its_entry_or_says_why_not() {
    let qwen = preset();
    let spare = SelfHostedEntry {
        id: "spare".into(),
        name: "Spare GPU".into(),
        enabled: false,
        ..preset()
    };
    let other_style = SelfHostedEntry {
        id: "oai".into(),
        name: "Other style".into(),
        api_style: "openai".into(),
        ..preset()
    };
    let saved = || Ok(vec![qwen.clone(), spare.clone(), other_style.clone()]);
    for model in [None, Some(""), Some("opus"), Some("claude-opus-5-5")] {
        assert_eq!(
            route(AgentKind::Claude, model, unread),
            Route::Ordinary,
            "{model:?}"
        );
        assert_eq!(
            route(AgentKind::Codex, model, unread),
            Route::Ordinary,
            "{model:?}"
        );
    }
    assert_eq!(
        route(AgentKind::Claude, Some("self-hosted:qwen"), saved),
        Route::Entry(qwen.clone())
    );

    let gone = refused(route(AgentKind::Claude, Some("self-hosted:gone"), saved));
    assert!(gone.contains("no self-hosted model \"gone\""), "{gone}");
    let off = refused(route(AgentKind::Claude, Some("self-hosted:spare"), saved));
    assert!(off.starts_with("Spare GPU is switched off"), "{off}");
    // Q9: an `openai` entry runs on Codex, so a Claude turn on it is
    // refused with where to choose it instead.
    let style = refused(route(AgentKind::Claude, Some("self-hosted:oai"), saved));
    assert!(style.contains("OpenAI's Responses API"), "{style}");
    assert!(style.contains("runs on Codex, not Claude"), "{style}");
    assert!(style.contains("under Codex in the model menu"), "{style}");
    let unknown = SelfHostedEntry {
        id: "odd".into(),
        name: "Odd style".into(),
        api_style: "gemini".into(),
        ..preset()
    };
    let odd = refused(route(AgentKind::Claude, Some("self-hosted:odd"), || {
        Ok(vec![unknown.clone()])
    }));
    assert!(odd.contains("gemini API style"), "{odd}");
    let damaged = refused(route(AgentKind::Claude, Some("self-hosted:qwen"), || {
        Err("self-hosted-models.json isn't valid JSON (x). Fix it or move it aside.".into())
    }));
    assert!(damaged.contains("can't be read"), "{damaged}");
    assert!(damaged.contains("isn't valid JSON"), "{damaged}");
    for why in [&gone, &off, &damaged] {
        assert!(why.contains(SETTINGS_PLACE), "{why}");
        assert!(why.contains("didn't start"), "{why}");
    }
    // Antigravity and Grok are refused before the registry is read.
    for agent in [AgentKind::Antigravity, AgentKind::Grok] {
        let why = refused(route(agent, Some("self-hosted:qwen"), unread));
        assert!(why.contains("runs on Claude or Codex only"), "{why}");
        assert!(why.contains(agent.label()), "{why}");
    }
    // Q9: Codex runs the `openai` entry, and refuses the `anthropic` one.
    assert_eq!(
        route(AgentKind::Codex, Some("self-hosted:oai"), saved),
        Route::Entry(other_style.clone())
    );
    let mismatch = refused(route(AgentKind::Codex, Some("self-hosted:qwen"), saved));
    assert!(mismatch.contains("Anthropic's Messages API"), "{mismatch}");
    assert!(mismatch.contains("runs on Claude, not Codex"), "{mismatch}");
    assert!(
        mismatch.contains("under Claude in the model menu"),
        "{mismatch}"
    );
    assert!(mismatch.contains("didn't start"), "{mismatch}");
}

/// Q5: only a chat turn, which sets the run's target, runs a self-hosted
/// model id; any other run of any agent refuses it rather than sending it
/// to the agent's own provider.
#[test]
fn only_a_chat_turn_runs_a_self_hosted_model_id() {
    let mut run = spec(ProcessScope::GuiOwned, None);
    run.model = Some("self-hosted:qwen".into());
    for kind in [AgentKind::Claude, AgentKind::Codex] {
        let why = agent::validate_run_spec(kind, &run).unwrap_err();
        assert!(why.contains("only a chat can use"), "{kind:?}: {why}");
        let cmd = agent::command_without_codex_listing(kind, &run);
        assert!(agent::is_unavailable_command(&cmd), "{kind:?}");
    }
    // The chat turn's spec: the served id and its target.
    run.model = Some("qwen3.8-27b".into());
    run.extras.self_hosted = Some(target());
    assert!(agent::validate_run_spec(AgentKind::Claude, &run).is_ok());
    let cmd = agent::command_without_codex_listing(AgentKind::Claude, &run);
    let argv = args(&cmd);
    let at = argv.iter().position(|a| a == "--model").unwrap();
    assert_eq!(argv[at + 1], "qwen3.8-27b");
}

/// Q5: a turn's key is minted on its entry's forwarder, one per turn, and
/// revoked when the turn lets go of it.
#[tokio::test]
async fn a_turns_key_lives_until_the_turn_lets_go() {
    let entry = SelfHostedEntry {
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        ..preset()
    };
    let forwarder = Arc::new(
        crate::self_hosted_forwarder::Forwarder::start(entry.upstream())
            .await
            .unwrap(),
    );
    let run = turn_run(&entry, forwarder.clone());
    let t = &run.target;
    assert_eq!(
        (t.entry.as_str(), t.name.as_str(), t.served_model.as_str()),
        ("qwen", "Qwen3.8 27B", "qwen3.8-27b")
    );
    assert_eq!(t.context_window, 131_072);
    assert_eq!(t.effort_levels, ["low", "medium", "xhigh"]);
    assert_eq!(t.base_url, forwarder.base_url());
    assert!(is_run_key(t.run_key.as_str()));
    assert_eq!(invalid(t), None);
    assert!(!format!("{run:?}").contains(t.run_key.as_str()));
    assert_eq!(forwarder.live_keys(), 1);
    let next = turn_run(&entry, forwarder.clone());
    assert_ne!(next.target.run_key, run.target.run_key);
    assert_eq!(forwarder.live_keys(), 2);
    drop(run);
    assert_eq!(forwarder.live_keys(), 1, "the first turn's key is revoked");
    drop(next);
    assert_eq!(forwarder.live_keys(), 0);
}

// ── Q5 review ────────────────────────────────────────────────────────────

/// The run marks its process tree, and a run with no target started
/// inside one (a teammate) loses every self-hosted variable, the run key
/// first; anywhere else nothing is stripped.
#[test]
fn a_delegate_never_inherits_the_endpoint_or_the_key() {
    let cmd = agent::build_command(
        AgentKind::Claude,
        &spec(ProcessScope::GuiOwned, Some(target())),
    );
    assert_eq!(env_set(&cmd, MARKER_ENV).as_deref(), Some("qwen"));

    let inside = |marker: Option<&str>, token: Option<&str>| {
        let (marker, token) = (marker.map(str::to_string), token.map(str::to_string));
        move |name: &str| match name {
            MARKER_ENV => marker.clone(),
            AUTH_TOKEN_ENV => token.clone(),
            _ => None,
        }
    };
    for (env, why) in [
        (inside(Some("qwen"), Some(KEY)), "marker and key"),
        (inside(Some("qwen"), None), "marker"),
        (inside(None, Some(KEY)), "a run key alone"),
    ] {
        let mut delegate = Command::new("codex");
        assert!(strip_inherited_from(&mut delegate, &env), "{why}");
        let removed: Vec<String> = envs(&delegate)
            .into_iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k)
            .collect();
        for name in INHERITED_ENVS {
            assert!(removed.iter().any(|r| r == name), "{why}: {name}");
        }
        assert!(inherited_from(&env), "{why}");
    }
    for env in [
        inside(None, None),
        inside(Some(" "), Some("sk-ant-oat-not-a-run-key")),
    ] {
        let mut ordinary = Command::new("codex");
        assert!(!strip_inherited_from(&mut ordinary, &env));
        assert!(envs(&ordinary).is_empty());
        assert!(!inherited_from(&env));
    }
}

/// A served id Hollow knows never goes to Anthropic without its target;
/// with it, it runs.
#[test]
fn a_known_served_id_needs_its_target() {
    remember("q5-review-served", 131_072, &["low".to_string()]);
    let mut run = spec(ProcessScope::GuiOwned, None);
    run.model = Some("q5-review-served".into());
    let why = agent::validate_run_spec(AgentKind::Claude, &run).unwrap_err();
    assert!(why.contains("only a chat can use"), "{why}");
    // Another agent's own model of that name isn't Claude's business.
    assert!(agent::validate_run_spec(AgentKind::Codex, &run).is_ok());
    run.extras.self_hosted = Some(target());
    assert!(agent::validate_run_spec(AgentKind::Claude, &run).is_ok());
    assert!(needs_target("self-hosted:qwen") && needs_target("q5-review-served"));
    assert!(!needs_target("opus"));
}

/// The template's model and target come from one call: the served id
/// only with its target, the chat's own model only without one.
#[tokio::test]
async fn the_served_id_never_goes_without_its_target() {
    assert_eq!(
        turn_spec_model(Some("opus"), None),
        (Some("opus".into()), None)
    );
    assert_eq!(turn_spec_model(None, None), (None, None));
    let entry = SelfHostedEntry {
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        ..preset()
    };
    let forwarder = Arc::new(
        crate::self_hosted_forwarder::Forwarder::start(entry.upstream())
            .await
            .unwrap(),
    );
    let run = turn_run(&entry, forwarder);
    let (model, target) = turn_spec_model(Some("self-hosted:qwen"), Some(&run));
    assert_eq!(model.as_deref(), Some("qwen3.8-27b"));
    assert_eq!(target.as_ref(), Some(&run.target));
}

/// A claude.ai connector (Anthropic's proxy on the person's login) never
/// goes with a self-hosted run: the builder refuses it either way it could
/// arrive, and an ordinary Hollow server is fine.
#[test]
fn a_self_hosted_run_takes_no_claude_ai_connector() {
    let proxy = r#"{"mcpServers":{"mail":{"type":"claudeai-proxy","url":"https://mcp.example.com/mcp","id":"mcpsrv_1"}}}"#;
    let stdio = r#"{"mcpServers":{"hollow_tasks":{"command":"/bin/echo","args":["tasks-mcp"]}}}"#;
    let mut run = spec(ProcessScope::Inherited, Some(target()));
    run.mcp_config = Some(proxy.into());
    assert!(claude_login_servers(&run));
    let cmd = agent::command_without_codex_listing(AgentKind::Claude, &run);
    assert!(agent::is_unavailable_command(&cmd));
    assert!(args(&cmd).iter().any(|a| a.contains("claude.ai connector")));

    run.mcp_config = Some(stdio.into());
    assert!(!claude_login_servers(&run));
    run.extras.shared_connectors = vec![agent::SharedConnector {
        id: "mail".into(),
        claude_login: true,
        ..agent::SharedConnector::default()
    }];
    assert!(claude_login_servers(&run));
    assert!(agent::is_unavailable_command(
        &agent::command_without_codex_listing(AgentKind::Claude, &run)
    ));
    run.extras.shared_connectors.clear();
    let cmd = agent::command_without_codex_listing(AgentKind::Claude, &run);
    assert!(!agent::is_unavailable_command(&cmd), "{:?}", args(&cmd));
}

/// A hand-edited registry can't put a served id Claude Code uses for its
/// own models on a run, nor into the remembered windows: such an entry is
/// left out of the menu, refused by the route with the reason, and marked
/// invalid with that reason in Settings (Q5 delta review: not hidden).
#[test]
fn entries_that_fail_validation_are_marked_and_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(crate::self_hosted_registry::FILE);
    let good = serde_json::to_value(preset()).unwrap();
    let mut sonnet = good.clone();
    sonnet["id"] = "sneaky".into();
    sonnet["name"] = "Sneaky".into();
    sonnet["servedModel"] = "sonnet".into();
    let mut plain_http = good.clone();
    plain_http["id"] = "far".into();
    plain_http["baseUrl"] = "http://10.0.0.5:8000".into();
    std::fs::write(
        &path,
        serde_json::json!({"version": 1, "entries": [good, sonnet, plain_http]}).to_string(),
    )
    .unwrap();
    let read = || crate::self_hosted_registry::read_at(&path);

    let listed = crate::self_hosted_registry::list_at(&path);
    assert_eq!(listed.entries.len(), 3, "Settings lists every saved entry");
    let marked: Vec<&str> = listed.invalid.keys().map(String::as_str).collect();
    assert_eq!(marked, ["far", "sneaky"]);
    assert!(
        listed.invalid["sneaky"].contains("Claude"),
        "{}",
        listed.invalid["sneaky"]
    );

    let rows = crate::agent_models::self_hosted_options(AgentKind::Claude, &read().unwrap());
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["self-hosted:qwen"]);
    assert_eq!(known_window("sonnet"), None, "never remembered");

    let why = refused(route(AgentKind::Claude, Some("self-hosted:sneaky"), read));
    assert!(why.starts_with("Sneaky in Settings"), "{why}");
    assert!(
        why.contains(&listed.invalid["sneaky"]),
        "the reason Settings shows: {why}"
    );
    assert!(!why.contains("no self-hosted model"), "{why}");
    assert!(matches!(
        route(AgentKind::Claude, Some("self-hosted:qwen"), read),
        Route::Entry(_)
    ));
}

/// A switch-off can't slip between the turn's registry read and its
/// forwarder start: a stop in between refuses the start, and an entry
/// found off on the second read stops the forwarder it started.
#[tokio::test]
async fn a_switch_off_while_a_turn_starts_refuses_it() {
    let entry = |id: &str, enabled: bool| SelfHostedEntry {
        id: id.into(),
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        enabled,
        ..preset()
    };
    // Off on the second read.
    let reads = std::cell::Cell::new(0);
    let flips = || {
        reads.set(reads.get() + 1);
        Ok::<_, String>(vec![entry("q5-flip", reads.get() == 1)])
    };
    let why = turn_route_with(AgentKind::Claude, Some("self-hosted:q5-flip"), &flips)
        .await
        .unwrap_err();
    assert!(why.contains("switched off"), "{why}");
    assert_eq!(reads.get(), 2);
    assert!(
        crate::self_hosted_forwarder::get("q5-flip").is_none(),
        "stopped"
    );

    // Stopped (Settings switched it off) after the stop count was read.
    let stops_now = || {
        crate::self_hosted_forwarder::stop("q5-race");
        Ok::<_, String>(vec![entry("q5-race", true)])
    };
    let why = turn_route_with(AgentKind::Claude, Some("self-hosted:q5-race"), &stops_now)
        .await
        .unwrap_err();
    assert!(why.contains("didn't start"), "{why}");
    assert!(crate::self_hosted_forwarder::get("q5-race").is_none());

    // Unreadable on the second read: this turn alone is refused, and the
    // forwarder other turns may be using keeps running (Q5 delta review).
    let reads = std::cell::Cell::new(0);
    let breaks = || {
        reads.set(reads.get() + 1);
        if reads.get() == 1 {
            Ok(vec![entry("q5-unread", true)])
        } else {
            Err("self-hosted-models.json isn't valid JSON (x).".to_string())
        }
    };
    let why = turn_route_with(AgentKind::Claude, Some("self-hosted:q5-unread"), &breaks)
        .await
        .unwrap_err();
    assert!(why.contains("can't be read"), "{why}");
    assert!(
        crate::self_hosted_forwarder::get("q5-unread").is_some(),
        "not stopped on a read error"
    );
    crate::self_hosted_forwarder::stop("q5-unread");

    // Steady: the turn gets its key.
    let steady = || Ok::<_, String>(vec![entry("q5-steady", true)]);
    let run = turn_route_with(AgentKind::Claude, Some("self-hosted:q5-steady"), &steady)
        .await
        .unwrap()
        .expect("a turn");
    assert_eq!(run.forwarder().live_keys(), 1);
    drop(run);
    crate::self_hosted_forwarder::stop("q5-steady");
}

/// Nothing that isn't a chat takes a self-hosted model id: the default
/// model, the extraction model (and what extraction says it asks) and an
/// automation's model refuse it with the same reason.
#[test]
fn setters_that_are_not_chats_refuse_a_self_hosted_id() {
    // The default model: one already saved is dropped (so the composer's
    // Effort row, which re-saves the whole object, still saves); a newly
    // asked one is refused (Q5 delta review).
    let mut settings = crate::agent_settings::AgentSettings::default();
    settings
        .models
        .insert("claude".into(), "self-hosted:qwen".into());
    settings.models.insert("codex".into(), "gpt-6".into());
    let saved = crate::agent_settings::normalize(settings.clone()).unwrap();
    assert_eq!(saved.models.get("claude"), None);
    assert_eq!(saved.models.get("codex").map(String::as_str), Some("gpt-6"));
    let none = std::collections::BTreeMap::new();
    assert_eq!(
        crate::agent_settings::newly_chat_only(&settings.models, &none).as_deref(),
        Some("self-hosted:qwen")
    );
    assert_eq!(
        crate::agent_settings::newly_chat_only(&settings.models, &settings.models),
        None
    );
    assert!(crate::self_hosted::not_for_this("self-hosted:qwen").contains("only a chat can use"));

    let why = crate::extract::set_extraction_model("claude", "self-hosted:qwen").unwrap_err();
    assert!(why.contains("Nothing changed"), "{why}");
    let mut saved = crate::extract::ExtractionSettings::default();
    saved
        .models
        .insert("claude".into(), "self-hosted:qwen".into());
    let asks = crate::extract::model_for(AgentKind::Claude, &saved).unwrap();
    assert!(!asks.starts_with(MODEL_PREFIX), "{asks}");

    let dir = tempfile::tempdir().unwrap();
    let input: crate::types::WorkflowInput = serde_json::from_value(serde_json::json!({
        "title": "Morning digest",
        "schedule": "daily 09:00",
        "model": "self-hosted:qwen",
        "prompt": "Summarize.",
    }))
    .unwrap();
    let why = crate::vault_workflows::preview_input(dir.path(), None, &input)
        .map(|_| ())
        .unwrap_err();
    assert!(why.contains("only a chat can use"), "{why}");
}

/// Q5 delta review: Hollow's own servers carry the self-hosted marker in
/// their config env (as the run tag), so `hollow team-mcp` sees it even when
/// the CLI passes its servers no inherited environment; a library connector
/// gets nothing, and the built command's config file carries it.
#[test]
fn hollow_servers_carry_the_marker_in_their_config() {
    let config = r#"{"mcpServers":{"hollow_team":{"command":"/bin/echo","args":["team-mcp"],"env":{"HOLLOW_VAULT":"/v"}},"hollow_tasks":{"command":"/bin/echo"},"mail":{"command":"/bin/echo","env":{}}}}"#;
    let marked: serde_json::Value = serde_json::from_str(&with_marker(config, "qwen")).unwrap();
    let servers = &marked["mcpServers"];
    assert_eq!(servers["hollow_team"]["env"][MARKER_ENV], "qwen");
    assert_eq!(servers["hollow_team"]["env"]["HOLLOW_VAULT"], "/v");
    assert_eq!(servers["hollow_tasks"]["env"][MARKER_ENV], "qwen");
    assert!(servers["mail"]["env"].get(MARKER_ENV).is_none());
    assert_eq!(with_marker("not json", "qwen"), "not json");

    let mut run = spec(ProcessScope::GuiOwned, Some(target()));
    run.mcp_config = Some(config.into());
    let cmd = agent::build_command(AgentKind::Claude, &run);
    assert!(!agent::is_unavailable_command(&cmd), "{:?}", args(&cmd));
    let argv = args(&cmd);
    let at = argv.iter().position(|a| a == "--mcp-config").unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&argv[at + 1]).unwrap()).unwrap();
    assert_eq!(
        written["mcpServers"]["hollow_team"]["env"][MARKER_ENV],
        "qwen"
    );
    // An ordinary run's servers get none.
    let mut plain = spec(ProcessScope::GuiOwned, None);
    plain.mcp_config = Some(config.into());
    let cmd = agent::build_command(AgentKind::Claude, &plain);
    let argv = args(&cmd);
    let at = argv.iter().position(|a| a == "--mcp-config").unwrap();
    let written = std::fs::read_to_string(&argv[at + 1]).unwrap();
    assert!(!written.contains(MARKER_ENV));
}

// ── Q7: Runs on ──────────────────────────────────────────────────────────

/// Q7: how the note of a turn kept on this Mac names the endpoint's
/// sign-in: `gcloud` by the token command's program name (bare or a path),
/// any other token command, or none.
#[test]
fn the_sign_in_kind_follows_the_entrys_auth() {
    let command = |program: &str| SelfHostedAuth::TokenCommand {
        program: program.into(),
        args: vec!["auth".into(), "print-identity-token".into()],
    };
    assert_eq!(sign_in_kind(&preset().auth), "gcloud");
    assert_eq!(sign_in_kind(&command("gcloud")), "gcloud");
    assert_eq!(
        sign_in_kind(&command("/opt/homebrew/share/google-cloud-sdk/bin/gcloud")),
        "gcloud"
    );
    assert_eq!(sign_in_kind(&command(" gcloud ")), "gcloud");
    assert_eq!(sign_in_kind(&command("gcloud-wrapper")), "command");
    assert_eq!(sign_in_kind(&command("/usr/local/bin/op")), "command");
    assert_eq!(sign_in_kind(&SelfHostedAuth::None), "none");
}

/// Q7: the first line of a self-hosted turn whose Runs on is the runner is
/// the chat's `stayed` notice with the model's name and sign-in (and, Q7
/// review, whether work an earlier turn left on the runner is still there),
/// and holds neither the run key nor the forwarder's address.
#[tokio::test]
async fn the_runner_note_names_the_model_and_holds_no_secret() {
    let entry = SelfHostedEntry {
        base_url: "http://127.0.0.1:9".into(),
        ..preset()
    };
    let forwarder = Arc::new(
        crate::self_hosted_forwarder::Forwarder::start(entry.upstream())
            .await
            .unwrap(),
    );
    let run = turn_run(&entry, forwarder.clone());
    let note = runner_note("runner-host", &run, None, "s-20261103-0000q7q7");
    assert_eq!(
        note,
        serde_json::json!({
            "type": "remoteRun",
            "phase": "stayed",
            "host": "runner-host",
            "selfHosted": { "model": "Qwen3.8 27B", "signIn": "gcloud", "leftOnRunner": null },
            "sessionKey": "s-20261103-0000q7q7",
        })
    );
    for left in ["pending", "runner"] {
        let note = runner_note("runner-host", &run, Some(left), "s-20261103-0000q7q7");
        assert_eq!(note["selfHosted"]["leftOnRunner"], left);
    }
    let text = note.to_string();
    assert!(!text.contains(run.target.run_key.as_str()));
    assert!(!text.contains(&forwarder.base_url()));
    assert!(!text.contains("hsr_"));
    // No runner set up: the same note with an empty host, which the chat
    // words as its own sentence (Q7 review), never naming a login.
    let open = turn_run(
        &SelfHostedEntry {
            auth: SelfHostedAuth::None,
            ..entry
        },
        forwarder,
    );
    let note = runner_note("", &open, None, "s-1");
    assert_eq!(note["host"], "");
    assert_eq!(note["selfHosted"]["signIn"], "none");
    assert_eq!(note["phase"], "stayed");
}

/// Q8: a run whose credential is a run key is billed by GPU time.
#[test]
fn a_self_hosted_command_classifies_as_self_hosted_billing() {
    let cmd = agent::build_command(
        AgentKind::Claude,
        &spec(ProcessScope::Inherited, Some(target())),
    );
    assert_eq!(
        crate::usage::classify(AgentKind::Claude, &cmd, false),
        crate::usage::Billing::SelfHosted
    );
    let plain = agent::build_command(AgentKind::Claude, &spec(ProcessScope::Inherited, None));
    assert_ne!(
        crate::usage::classify(AgentKind::Claude, &plain, false),
        crate::usage::Billing::SelfHosted
    );
    assert_eq!(crate::usage::Billing::SelfHosted.as_str(), "self-hosted");
}

/// Q8 second review: a Claude run on the person's own gateway (a base URL,
/// and an auth token that is not a Hollow run key) is not billed by GPU
/// time: it classifies as unknown billing, as before Q8.
#[test]
fn a_persons_own_gateway_is_not_self_hosted_billing() {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = agent::build_command(AgentKind::Claude, &spec(ProcessScope::Inherited, None));
    for key in [
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CONFIG_DIR",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("HOME", home.path());
    cmd.env("ANTHROPIC_BASE_URL", "https://gateway.example.test");
    cmd.env(
        "ANTHROPIC_AUTH_TOKEN",
        "sk-gateway-0123456789abcdef0123456789abcdef",
    );
    assert_eq!(
        crate::usage::classify(AgentKind::Claude, &cmd, false),
        crate::usage::Billing::Unknown
    );
    // `hsr_` alone isn't enough either: a run key is 32 or more hex digits.
    cmd.env("ANTHROPIC_AUTH_TOKEN", "hsr_not-a-run-key");
    assert_eq!(
        crate::usage::classify(AgentKind::Claude, &cmd, false),
        crate::usage::Billing::Unknown
    );
}

// ── Q9: Codex on a self-hosted model ─────────────────────────────────────

/// An `openai` entry's target, its Codex home in a temporary app-data
/// folder (`self-hosted-codex/<id>`, as `codex_home_for` makes it).
fn codex_target(app_data: &std::path::Path) -> RunTarget {
    RunTarget {
        entry: "qwen-codex".into(),
        name: "Qwen on Codex".into(),
        api_style: crate::self_hosted_registry::API_OPENAI.into(),
        codex_home: Some(app_data.join(CODEX_HOMES_DIR).join("qwen-codex")),
        ..target()
    }
}

/// A Codex chat turn's spec on `target`: the served id, an effort, and a
/// launch point's extra env full of credentials and Settings' own
/// compaction limit, all of which must give way.
fn codex_spec(target: RunTarget, effort: &str) -> RunSpec {
    let mut run = spec(ProcessScope::GuiOwned, Some(target));
    run.model = Some("qwen3.8-27b".into());
    run.extras.effort = Some(effort.into());
    run.extras.env = vec![
        ("OPENAI_API_KEY".into(), "sk-should-go-openai".into()),
        ("CODEX_API_KEY".into(), "sk-should-go-codex".into()),
        ("OPENAI_BASE_URL".into(), "https://api.openai.com/v1".into()),
        ("CODEX_HOME".into(), "/Users/someone/.codex".into()),
        ("ANTHROPIC_API_KEY".into(), "sk-ant-should-go".into()),
        (OAUTH_TOKEN_ENV.into(), "sk-ant-oat-should-go".into()),
        (agent::CODEX_COMPACT_LIMIT_ENV.into(), "90000".into()),
    ];
    run
}

/// The value after each `-c` on `argv`.
fn overrides(argv: &[String]) -> Vec<String> {
    argv.windows(2)
        .filter(|w| w[0] == "-c")
        .map(|w| w[1].clone())
        .collect()
}

/// Q9's verify: a self-hosted Codex run gets Hollow's own `CODEX_HOME` for
/// the entry (made, no sign-in), a custom provider at the loopback
/// forwarder whose key rides `env_key` (never argv), the served model and
/// its window, `--disable apps`, an effort the entry takes (never `high`)
/// and the entry's compaction limit, and no OpenAI, Codex or Anthropic
/// credential from anywhere.
#[test]
fn a_self_hosted_codex_run_gets_its_own_home_and_provider() {
    let app_data = tempfile::tempdir().unwrap();
    let target = codex_target(app_data.path());
    let home = target.codex_home.clone().unwrap();
    let run = codex_spec(target, "high");
    let cmd = agent::command_without_codex_listing(AgentKind::Codex, &run);
    assert!(!agent::is_unavailable_command(&cmd), "{:?}", args(&cmd));
    assert!(home.is_dir(), "the home was made");
    assert!(!home.join("auth.json").exists());

    // The environment: the entry's home, the run key, no credential.
    let all = envs(&cmd);
    assert_eq!(
        env_set(&cmd, "CODEX_HOME").as_deref(),
        Some(home.to_string_lossy().as_ref())
    );
    assert_eq!(env_set(&cmd, CODEX_KEY_ENV).as_deref(), Some(KEY));
    assert_eq!(env_set(&cmd, MARKER_ENV).as_deref(), Some("qwen-codex"));
    for (name, value) in &all {
        if let Some(value) = value.as_deref().filter(|_| name != "PATH") {
            assert!(!value.contains("sk-"), "{name} carries a credential");
            assert!(!value.contains("/.codex"), "{name} points at ~/.codex");
        }
        let vendor = ["OPENAI_", "CODEX_", "ANTHROPIC_", "AZURE_OPENAI_"]
            .iter()
            .any(|p| name.starts_with(p));
        if vendor && value.is_some() {
            assert_eq!(name, "CODEX_HOME", "{name} is set");
        }
    }
    for removed in ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"]
        .into_iter()
        .chain(REMOVED_ENVS)
    {
        assert!(
            all.iter().any(|(k, v)| k == removed && v.is_none()),
            "{removed} is removed"
        );
    }
    assert!(env_set(&cmd, "NO_PROXY").unwrap().contains("127.0.0.1"));
    assert_eq!(env_set(&cmd, AUTH_TOKEN_ENV), None, "no Claude variable");

    // Argv: the provider, model and window; the key never.
    let argv = args(&cmd);
    assert!(argv.windows(2).any(|w| w == ["--disable", "apps"]));
    assert!(
        argv.iter().all(|a| !a.contains(KEY)),
        "the key is not on argv"
    );
    assert!(argv.iter().all(|a| !a.contains("sk-")));
    let set = overrides(&argv);
    for want in [
        r#"model_provider="hollow-self-hosted""#,
        r#"model="qwen3.8-27b""#,
        "model_context_window=131072",
        r#"model_reasoning_effort="medium""#,
        "model_auto_compact_token_limit=100000",
        // Q9 review: Codex's own telemetry off (0.166.0 otherwise opens
        // ab.chatgpt.com), and the key kept out of the commands it runs.
        r#"otel.exporter="none""#,
        r#"otel.trace_exporter="none""#,
        r#"otel.metrics_exporter="none""#,
        "analytics.enabled=false",
        "feedback.enabled=false",
        "shell_environment_policy.ignore_default_excludes=false",
        r#"shell_environment_policy.exclude=["HOLLOW_SELF_HOSTED_KEY"]"#,
    ] {
        assert!(set.iter().any(|s| s == want), "{want} in {set:?}");
    }
    // No profile is pinned or selected: Codex 0.166.0 refuses a legacy
    // `profile` key and selects one only with `--profile`, which Hollow
    // never passes.
    assert!(!argv
        .iter()
        .any(|a| a == "--profile" || a.starts_with("profile=")));
    // Q8 for Codex: the run key in Codex's provider variable is billed by
    // GPU time; an ordinary Codex command isn't.
    assert_eq!(
        crate::usage::classify(AgentKind::Codex, &cmd, false),
        crate::usage::Billing::SelfHosted
    );
    let plain = agent::command_without_codex_listing(
        AgentKind::Codex,
        &spec(ProcessScope::Inherited, None),
    );
    assert_ne!(
        crate::usage::classify(AgentKind::Codex, &plain, false),
        crate::usage::Billing::SelfHosted
    );
    let provider = set
        .iter()
        .find(|s| s.starts_with("model_providers.hollow-self-hosted="))
        .expect("the provider table");
    for part in [
        r#"base_url="http://127.0.0.1:41234/v1""#,
        r#"wire_api="responses""#,
        r#"env_key="HOLLOW_SELF_HOSTED_KEY""#,
        "requires_openai_auth=false",
        "stream_idle_timeout_ms=900000",
    ] {
        assert!(provider.contains(part), "{part} in {provider}");
    }
    assert!(!set.iter().any(|s| s.contains("high")), "{set:?}");
    assert!(
        !set.iter().any(|s| s.ends_with("=90000")),
        "Settings' limit gives way"
    );
    let at = argv.iter().position(|a| a == "-m").expect("-m");
    assert_eq!(argv[at + 1], "qwen3.8-27b");
    // The overrides are top-level, before the subcommand (resume gets them).
    let exec = argv.iter().position(|a| a == "exec").unwrap();
    let provider_at = argv.iter().position(|a| a == provider).unwrap();
    assert!(provider_at < exec);

    // A steerable turn's `codex app-server` gets the same top-level flags.
    let mut live = run.clone();
    live.extras.live_input = true;
    let server = agent::command_without_codex_listing(AgentKind::Codex, &live);
    let server_argv = args(&server);
    assert_eq!(server_argv.last().map(String::as_str), Some("app-server"));
    assert_eq!(overrides(&server_argv), set);
    assert_eq!(
        env_set(&server, "CODEX_HOME").as_deref(),
        Some(home.to_string_lossy().as_ref())
    );
    assert_eq!(
        agent::codex_app_server_start(&live, false).model.as_deref(),
        Some("qwen3.8-27b")
    );
}

/// Q9: effort through the entry's levels with Q3's mapping, never `high`
/// for Qwen's levels and never `max` on Codex (Codex has none).
#[test]
fn a_self_hosted_codex_runs_effort_follows_the_entrys_levels() {
    let app_data = tempfile::tempdir().unwrap();
    for (asked, sent) in [
        ("low", "low"),
        ("medium", "medium"),
        ("high", "medium"),
        ("xhigh", "xhigh"),
        ("max", "xhigh"),
        (crate::effort::ULTRACODE, "xhigh"),
        ("minimal", "low"),
    ] {
        let run = codex_spec(codex_target(app_data.path()), asked);
        let choice = agent::run_effort(AgentKind::Codex, &run);
        assert_eq!(choice.effort.as_deref(), Some(sent), "{asked}");
    }
    let mut run = codex_spec(codex_target(app_data.path()), "medium");
    run.extras.effort = None;
    assert_eq!(
        agent::run_effort(AgentKind::Codex, &run).effort.as_deref(),
        Some(DEFAULT_EFFORT)
    );
    // A hand-made target listing `max` still never sends it on Codex.
    let mut maxed = codex_target(app_data.path());
    maxed.effort_levels = vec!["medium".into(), "max".into()];
    assert_eq!(maxed.effort(Some("max")).as_deref(), Some("medium"));
}

/// Q9: an inherited OpenAI, Codex or Anthropic variable is removed by
/// prefix, not only the ones Hollow knows by name, and `CODEX_HOME` is set
/// again to the entry's own.
#[test]
fn inherited_openai_and_codex_variables_are_removed() {
    let app_data = tempfile::tempdir().unwrap();
    let target = codex_target(app_data.path());
    let mut cmd = Command::new("/bin/true");
    let inherited = [
        "OPENAI_API_KEY",
        "OPENAI_ORG_ID",
        "OPENAI_BASE_URL",
        "CODEX_HOME",
        "CODEX_ACCESS_TOKEN",
        "AZURE_OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_USE_BEDROCK",
        "HOME",
    ]
    .map(std::ffi::OsString::from);
    apply_codex_env_from(&mut cmd, &target, inherited);
    let all = envs(&cmd);
    for name in [
        "OPENAI_API_KEY",
        "OPENAI_ORG_ID",
        "OPENAI_BASE_URL",
        "CODEX_ACCESS_TOKEN",
        "AZURE_OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(
            all.iter().any(|(k, v)| k == name && v.is_none()),
            "{name} is removed"
        );
    }
    assert_eq!(
        env_set(&cmd, "CODEX_HOME"),
        target
            .codex_home
            .as_ref()
            .map(|h| h.to_string_lossy().into_owned())
    );
    assert!(
        !all.iter().any(|(k, _)| k == "HOME"),
        "others are left alone"
    );
}

/// Q9: a normal Codex run is unchanged: the exact argv of a canonical spec,
/// and only `PATH` in its env (no self-hosted variable, no home).
#[test]
fn a_normal_codex_run_is_unchanged() {
    let run = spec(ProcessScope::Inherited, None);
    let cmd = agent::command_without_codex_listing(AgentKind::Codex, &run);
    let prompt = agent::contextual_prompt(AgentKind::Codex, &run);
    // acceptEdits runs the nested-`.git` profile (N22) in place of `-s`.
    let top = std::fs::canonicalize("/tmp").unwrap();
    let profile_fs = format!(
        "permissions.hollow_auto.filesystem={{\"{0}/*/.git\"=\"deny\",\"{0}/*/**/.git\"=\"deny\"}}",
        top.display()
    );
    assert_eq!(
        args(&cmd),
        vec![
            "-c",
            "mcp_servers={}",
            "--disable",
            "apps",
            "--disable",
            "plugins",
            "--disable",
            "remote_plugin",
            "-c",
            "default_permissions=\"hollow_auto\"",
            "-c",
            "permissions.hollow_auto.extends=\":workspace\"",
            "-c",
            &profile_fs,
            "-c",
            "permissions.hollow_auto.network.enabled=false",
            "exec",
            "--json",
            "--skip-git-repo-check",
            "-m",
            "opus",
            "-",
        ]
    );
    // Q-R33: the prompt rides stdin.
    assert_eq!(
        agent::take_last_stdin_prompt().as_deref(),
        Some(prompt.as_str())
    );
    let names: Vec<String> = envs(&cmd).into_iter().map(|(k, _)| k).collect();
    assert_eq!(names, vec!["PATH"], "{names:?}");
    // GUI-owned and steerable: still no provider override, home or key.
    let mut live = spec(ProcessScope::GuiOwned, None);
    live.extras.live_input = true;
    let cmd = agent::command_without_codex_listing(AgentKind::Codex, &live);
    let argv = args(&cmd);
    assert!(!argv
        .iter()
        .any(|a| a.contains("model_provider") || a.contains("model_context_window")));
    for (name, _) in envs(&cmd) {
        assert!(
            name != CODEX_KEY_ENV && name != "CODEX_HOME" && name != MARKER_ENV,
            "{name}"
        );
    }
}

/// Q9: the run fails closed on the wrong harness, off this Mac, with no
/// Codex home, or with a sign-in in its home.
#[test]
fn a_self_hosted_codex_run_fails_closed() {
    let app_data = tempfile::tempdir().unwrap();
    let unavailable = |cmd: &Command| {
        assert!(agent::is_unavailable_command(cmd), "{:?}", args(cmd));
        args(cmd).join(" ")
    };
    // An `openai` entry on Claude.
    let claude = agent::command_without_codex_listing(
        AgentKind::Claude,
        &codex_spec(codex_target(app_data.path()), "medium"),
    );
    let why = unavailable(&claude);
    assert!(
        why.contains("OpenAI's Responses API, which runs on Codex only"),
        "{why}"
    );
    // The Mac mini runner.
    let mut remote = codex_spec(codex_target(app_data.path()), "medium");
    remote.extras.remote = Some(crate::remote_session::RemoteTarget::default());
    let remote = agent::command_without_codex_listing(AgentKind::Codex, &remote);
    assert!(unavailable(&remote).contains("runs on this Mac only"));
    // No Codex home.
    let mut homeless = codex_target(app_data.path());
    homeless.codex_home = None;
    let homeless =
        agent::command_without_codex_listing(AgentKind::Codex, &codex_spec(homeless, "medium"));
    assert!(unavailable(&homeless).contains("has no Codex home"));
    // A sign-in someone left in the home: refused, and kept.
    let target = codex_target(app_data.path());
    let home = target.codex_home.clone().unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), "{}").unwrap();
    let signed_in =
        agent::command_without_codex_listing(AgentKind::Codex, &codex_spec(target, "medium"));
    assert!(unavailable(&signed_in).contains("holds a sign-in"));
    assert!(home.join("auth.json").exists(), "Hollow never deletes it");
    // The inventory refuses it the same way, before Codex is asked.
    let listing =
        crate::codex_inventory::installed(&codex_spec(codex_target(app_data.path()), "medium"));
    assert!(listing.servers.unwrap_err().contains("holds a sign-in"));
}

/// Q9: a turn on an `openai` entry gets its Codex home and style; an
/// `anthropic` entry's gets none. Each harness counts as a self-hosted
/// chat for its own style only through the route; the chat-level checks
/// (connectors, retitle, learning) cover Claude and Codex.
#[tokio::test]
async fn an_openai_turn_carries_its_codex_home() {
    let entry = SelfHostedEntry {
        id: "q9-home".into(),
        api_style: crate::self_hosted_registry::API_OPENAI.into(),
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        ..preset()
    };
    let forwarder = Arc::new(
        crate::self_hosted_forwarder::Forwarder::start(entry.upstream())
            .await
            .unwrap(),
    );
    let run = turn_run(&entry, forwarder.clone());
    assert_eq!(run.target.api_style, "openai");
    assert_eq!(run.target.agent(), Some(AgentKind::Codex));
    let home = run.target.codex_home.clone().expect("a Codex home");
    assert_eq!(Some(home.clone()), codex_home_for("q9-home"));
    assert!(home.ends_with("self-hosted-codex/q9-home"));
    assert!(is_self_hosted_codex_home(&home));
    assert_eq!(invalid(&run.target), None);
    let claude = turn_run(&preset(), forwarder.clone());
    assert_eq!(claude.target.codex_home, None);
    assert_eq!(claude.target.agent(), Some(AgentKind::Claude));
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        assert!(chat_on_self_hosted(agent, Some("self-hosted:q9-home")));
    }
    assert!(!chat_on_self_hosted(
        AgentKind::Grok,
        Some("self-hosted:q9-home")
    ));
    assert!(!chat_on_self_hosted(AgentKind::Codex, Some("gpt-6-luna")));
}

/// Q9: a Codex run on an `openai` entry's served id never goes to OpenAI
/// without its target; a Codex model of another name is untouched.
#[test]
fn a_known_openai_served_id_needs_its_target_on_codex() {
    crate::self_hosted_registry::save(
        SelfHostedEntry {
            id: "q9-review".into(),
            api_style: crate::self_hosted_registry::API_OPENAI.into(),
            served_model: "q9-review-served".into(),
            ..preset()
        },
        false,
    )
    .unwrap();
    let mut run = spec(ProcessScope::GuiOwned, None);
    run.model = Some("q9-review-served".into());
    for kind in [AgentKind::Codex, AgentKind::Claude] {
        let why = agent::validate_run_spec(kind, &run).unwrap_err();
        assert!(why.contains("only a chat can use"), "{kind:?}: {why}");
    }
    run.model = Some("gpt-6-luna".into());
    assert!(agent::validate_run_spec(AgentKind::Codex, &run).is_ok());
    let app_data = tempfile::tempdir().unwrap();
    let mut with_target = codex_spec(codex_target(app_data.path()), "low");
    with_target.model = Some("q9-review-served".into());
    assert!(agent::validate_run_spec(AgentKind::Codex, &with_target).is_ok());
}

/// Q9: a delegate inside a self-hosted Codex run loses the run key and the
/// entry's Codex home; a person's own `CODEX_HOME` is left alone.
#[test]
fn a_delegate_never_inherits_a_codex_runs_key_or_home() {
    let app_data = tempfile::tempdir().unwrap();
    let home = codex_target(app_data.path()).codex_home.unwrap();
    let home_text = home.to_string_lossy().into_owned();
    let inside = move |name: &str| match name {
        CODEX_KEY_ENV => Some(KEY.to_string()),
        "CODEX_HOME" => Some(home_text.clone()),
        _ => None,
    };
    let mut delegate = Command::new("codex");
    assert!(strip_inherited_from(&mut delegate, &inside));
    let removed: Vec<String> = envs(&delegate)
        .into_iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k)
        .collect();
    assert!(removed.iter().any(|r| r == CODEX_KEY_ENV));
    assert!(removed.iter().any(|r| r == "CODEX_HOME"));
    let own = |name: &str| match name {
        MARKER_ENV => Some("qwen".to_string()),
        "CODEX_HOME" => Some("/Users/someone/.codex".to_string()),
        _ => None,
    };
    let mut delegate = Command::new("codex");
    assert!(strip_inherited_from(&mut delegate, &own));
    assert!(!envs(&delegate).iter().any(|(k, _)| k == "CODEX_HOME"));
}

/// Q9 review: a Codex home holding a file Hollow never writes (a sign-in,
/// or config that could move the run), or a system or managed Codex config
/// that can outrank `-c`, refuses the run naming the path; nothing is
/// deleted. Probed with Codex 0.166.0: neither `exec` nor `app-server`
/// writes one of these into the home.
#[test]
fn a_codex_home_with_config_hollow_never_writes_refuses_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(CODEX_HOMES_DIR).join("qwen-codex");
    std::fs::create_dir_all(&home).unwrap();
    // What Codex itself leaves there is fine.
    for (name, text) in [("installation_id", "x"), ("state_5.sqlite", "")] {
        std::fs::write(home.join(name), text).unwrap();
    }
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    assert_eq!(codex_home_problem(&home, &[]), None);
    for name in CODEX_HOME_REFUSED {
        let file = home.join(name);
        std::fs::write(&file, "profile = \"evil\"\n").unwrap();
        let why = codex_home_problem(&home, &[]).expect("refused");
        assert!(why.contains(&file.display().to_string()), "{why}");
        assert!(why.contains("didn't start"), "{why}");
        std::fs::remove_file(&file).unwrap();
    }
    // A system or managed config, given here (the live list is
    // `codex_outranking_files`: /etc/codex and the managed preferences).
    let managed = dir.path().join("managed_config.toml");
    std::fs::write(&managed, "").unwrap();
    let why = codex_home_problem(&home, std::slice::from_ref(&managed)).unwrap();
    assert!(why.contains("can outrank what Hollow sets"), "{why}");
    assert!(why.contains(&managed.display().to_string()), "{why}");
    assert!(managed.exists(), "never deleted");
    let live = codex_outranking_files();
    for path in [
        "/etc/codex/config.toml",
        "/etc/codex/managed_config.toml",
        "/Library/Managed Preferences/com.openai.codex.plist",
    ] {
        assert!(
            live.iter().any(|f| f == std::path::Path::new(path)),
            "{path}"
        );
    }
}

/// Q9 delta review: Codex's served-id guard follows the app's saved
/// registry exactly, switched-off entries included: an edit of the served
/// id, a switch of the API style or a removal stops the old id being
/// refused at once (no restart), and the new one is refused. Claude's guard
/// keeps every id it has known (never one of Claude's own).
#[test]
fn the_codex_guard_follows_the_saved_registry() {
    use crate::self_hosted_registry::{remove, save, set_enabled, API_ANTHROPIC, API_OPENAI};
    let codex = |served: &str| needs_target_on(AgentKind::Codex, served);
    let entry = |style: &str, served: &str| SelfHostedEntry {
        id: "q9-guard".into(),
        api_style: style.into(),
        served_model: served.into(),
        enabled: false,
        ..preset()
    };
    assert!(!codex("q9-guard-a"));
    save(entry(API_OPENAI, "q9-guard-a"), false).unwrap();
    assert!(codex("q9-guard-a"), "saved switched off still counts");
    set_enabled("q9-guard", true).unwrap();
    assert!(codex("q9-guard-a"));
    save(entry(API_OPENAI, "q9-guard-b"), true).unwrap();
    assert!(!codex("q9-guard-a"), "the old served id is free at once");
    assert!(codex("q9-guard-b"));
    save(entry(API_ANTHROPIC, "q9-guard-b"), true).unwrap();
    assert!(
        !codex("q9-guard-b"),
        "an anthropic entry's id stays open to Codex"
    );
    assert!(needs_target_on(AgentKind::Claude, "q9-guard-b"));
    save(entry(API_OPENAI, "q9-guard-c"), true).unwrap();
    assert!(codex("q9-guard-c"));
    remove("q9-guard").unwrap();
    assert!(!codex("q9-guard-c"), "a removed entry's id is free at once");
    // A Codex entry's levels aren't registered as Codex effort levels.
    assert_eq!(
        crate::effort::resolve(AgentKind::Codex, Some("q9-guard-c"), Some("high"))
            .effort
            .as_deref(),
        Some("high")
    );
}

/// Q9 delta review: the ids the Codex guard takes from a registry list:
/// `openai` entries that pass the checks, switched on or not, lowercased;
/// never an `anthropic` entry's or one that fails the checks. Startup fills
/// the guard the same way (`agent_models::warm_effort_catalogs`).
#[test]
fn the_codex_guard_takes_only_valid_openai_entries() {
    let entry = |id: &str, style: &str, served: &str, enabled: bool| SelfHostedEntry {
        id: id.into(),
        api_style: style.into(),
        served_model: served.into(),
        enabled,
        ..preset()
    };
    let ids = codex_guard_ids(&[
        entry("a", "openai", "Q9-Ids-On", true),
        entry("b", "openai", "q9-ids-off", false),
        entry("c", "anthropic", "q9-ids-claude", true),
        entry("d", "openai", "gpt-q9-broken", true),
    ]);
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort();
    assert_eq!(ids, ["q9-ids-off", "q9-ids-on"]);
}

/// Q9 review: Codex's MCP listing runs only for a target Codex can run, in
/// that target's own home: an `anthropic` entry's target or one with no
/// home gets the refusal at once and nothing is listed (never the person's
/// `~/.codex`).
#[test]
fn the_codex_listing_never_runs_for_a_target_codex_cant_use() {
    let mut claude_style = spec(ProcessScope::GuiOwned, Some(target()));
    claude_style.model = Some("qwen3.8-27b".into());
    let listing = crate::codex_inventory::installed(&claude_style);
    let why = listing.servers.unwrap_err();
    assert!(why.contains("runs on Claude only"), "{why}");
    let app_data = tempfile::tempdir().unwrap();
    let mut homeless = codex_target(app_data.path());
    homeless.codex_home = None;
    let listing = crate::codex_inventory::installed(&codex_spec(homeless, "medium"));
    assert!(listing.servers.unwrap_err().contains("has no Codex home"));
}

/// Q9 review: the teammate refusal inside a self-hosted chat names no
/// harness, since the chat may be on Codex.
#[test]
fn the_teammate_refusal_suits_a_codex_chat_too() {
    assert!(NO_CLAUDE_TEAMMATE.contains("a model that isn't self-hosted"));
    assert!(!NO_CLAUDE_TEAMMATE.contains("to a Claude model"));
}

/// CX4b: a self-hosted turn with a personal root has exactly one
/// `--settings`, holding both the endpoint pins and `autoMemoryDirectory`
/// (a second flag would replace the first and drop the pins).
#[test]
fn a_self_hosted_turn_carries_pins_and_the_memory_dir_in_one_settings_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (clone, work) = (root.join("clone"), root.join("work"));
    std::fs::create_dir_all(&clone).unwrap();
    std::fs::create_dir_all(work.join(".git")).unwrap();
    let _root = crate::context_layers::test_root::set(clone.clone(), vec![]);
    let mut run = spec(ProcessScope::GuiOwned, Some(target()));
    run.cwd = work.clone();
    let cmd = agent::build_command(AgentKind::Claude, &run);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at: Vec<usize> = (0..args.len())
        .filter(|i| args[*i] == "--settings")
        .collect();
    assert_eq!(at.len(), 1, "{args:?}");
    let v: serde_json::Value = serde_json::from_str(&args[at[0] + 1]).unwrap();
    assert_eq!(v["env"][BASE_URL_ENV], "http://127.0.0.1:41234");
    let slug = crate::run_manifest::claude_project_slug(&work);
    assert_eq!(
        v["autoMemoryDirectory"],
        clone.join("memory").join(slug).to_string_lossy().as_ref()
    );
    // No personal root: the same turn has the pins alone.
    drop(_root);
    let cmd = agent::build_command(AgentKind::Claude, &run);
    let json = cmd
        .as_std()
        .get_args()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(!json.contains("autoMemoryDirectory"));
}
