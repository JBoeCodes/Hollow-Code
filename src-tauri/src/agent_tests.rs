//! The `agent` unit tests (split out of `agent.rs` by B13, 2026-10-16, no
//! behaviour change). Attached from `agent.rs` as `#[path] mod tests`, so
//! `super::` is still `crate::agent`.

/// The launch a pure override test renders with (nothing is written).
fn test_launch() -> super::CodexLaunch {
    super::CodexLaunch {
        exe: std::path::PathBuf::from("/x/hollow"),
        file: std::path::PathBuf::from("/run/r.json"),
        digest: "d1".into(),
        remote_home: None,
    }
}

/// [`super::codex_server_overrides`] with [`test_launch`], its overrides
/// alone, as the override tests read them.
fn codex_owned_servers(
    config: &str,
    perm: &str,
    shared: &[super::SharedConnector],
) -> Result<String, String> {
    super::codex_server_overrides(config, &test_launch(), perm, shared).map(|o| o.overrides)
}

/// The `command=…,args=[…]` a launched server `name` renders with.
fn launched(name: &str) -> String {
    test_launch().fields(name).join(",")
}

/// The JSON a Claude command's `--mcp-config` names: the run's private
/// file (removed once read) or the inline empty config.
fn claude_config(cmd: &tokio::process::Command) -> serde_json::Value {
    let args = args_of(cmd);
    let at = args.iter().position(|a| a == "--mcp-config").unwrap();
    let arg = &args[at + 1];
    let config = crate::run_auth::config_from_arg(arg).expect("a readable MCP config");
    if !arg.starts_with('{') {
        let _ = std::fs::remove_file(arg);
    }
    config
}

/// The run file a Codex command's launched servers read (removed once
/// read), from the `mcp-launch` arguments on its argv.
fn codex_config(cmd: &tokio::process::Command) -> serde_json::Value {
    let argv = args_of(cmd).join(" ");
    let marker = format!("[\"{}\",\"", crate::run_auth::LAUNCH_SUBCOMMAND);
    let start = argv.find(&marker).expect("a launched server") + marker.len();
    let file = &argv[start..start + argv[start..].find('"').unwrap()];
    let config = crate::run_auth::config_from_arg(file).expect("a readable run file");
    let _ = std::fs::remove_file(file);
    config
}

fn test_command(kind: super::AgentKind, spec: &super::RunSpec) -> tokio::process::Command {
    super::build_with_codex_servers(
        kind,
        spec,
        Ok(vec!["mail-server".into(), "custom.slack".into()]),
    )
}

/// Agent effectiveness F8: only Codex runs a native MCP inventory when
/// its command is built, so only Codex's build has an inventory mark
/// for the timeline (`prep_timing::Step::CodexInventory`); the timed
/// build is the same command. (Codex's own listing isn't run here: it
/// would start the installed CLI.)
#[test]
fn only_codex_command_builds_time_a_native_inventory() {
    let spec = spec("acceptEdits", None, false);
    for kind in [AgentKind::Claude, AgentKind::Antigravity, AgentKind::Grok] {
        let (timed, inventory) = build_command_timed(kind, &spec);
        assert!(inventory.is_none(), "{kind:?} lists no inventory");
        let plain = build_command(kind, &spec);
        assert_eq!(
            args_of(&timed).len(),
            args_of(&plain).len(),
            "{kind:?}: the same command"
        );
    }
}

#[test]
fn command_boundary_isolates_connectors_and_preserves_workspace_policy() {
    let spec = spec("bypassPermissions", Some("old-session"), true);
    let claude = rendered(&test_command(AgentKind::Claude, &spec));
    assert!(claude.contains("--strict-mcp-config --mcp-config {\"mcpServers\":{}}"));
    assert!(claude.contains(WORKSPACE_POLICY));
    let codex = rendered(&test_command(AgentKind::Codex, &spec));
    assert!(codex.contains("\"mail-server\"={enabled=false}"));
    assert!(codex.contains("\"custom.slack\"={enabled=false}"));
    assert!(codex.contains("--disable apps --disable plugins --disable remote_plugin"));
    assert!(codex.contains(WORKSPACE_POLICY));
    let agy = rendered(&test_command(AgentKind::Antigravity, &spec));
    assert!(agy.contains(WORKSPACE_POLICY));
    let failed = rendered(&build_with_codex_servers(
        AgentKind::Codex,
        &spec,
        Err("configuration unavailable".into()),
    ));
    assert!(failed.contains("exit 1"));
    assert!(!failed.contains("exec resume"));
}

/// HB01 (evening 10-16): a chat turn names its mode in its message, so its
/// command carries the mode-free chat policy, the same in acceptEdits and
/// bypass for every adapter; any other run keeps its mode paragraph. The
/// mode flag itself still differs per process.
#[test]
fn a_chat_turns_policy_is_the_same_in_every_mode_that_acts() {
    let chat = |perm: &str| RunSpec {
        extras: RunExtras {
            mode_per_turn: true,
            ..RunExtras::default()
        },
        ..spec(perm, Some("old-session"), true)
    };
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Antigravity,
        AgentKind::Grok,
    ] {
        let accept = rendered(&test_command(kind, &chat("acceptEdits")));
        let bypass = rendered(&test_command(kind, &chat("bypassPermissions")));
        assert!(
            accept.contains(crate::mode_contract::MODE_POINTER),
            "{kind:?}"
        );
        assert!(
            bypass.contains(crate::mode_contract::MODE_POINTER),
            "{kind:?}"
        );
        for text in [&accept, &bypass] {
            assert!(!text.contains("Permission mode: bypass."), "{kind:?}");
            assert!(!text.contains("Permission mode: accept edits."), "{kind:?}");
        }
        let other = rendered(&test_command(kind, &spec("bypassPermissions", None, true)));
        assert!(other.contains("Permission mode: bypass."), "{kind:?}");
        assert!(
            !other.contains(crate::mode_contract::MODE_POINTER),
            "{kind:?}"
        );
    }
    let claude = rendered(&test_command(AgentKind::Claude, &chat("acceptEdits")));
    assert!(claude.contains("--permission-mode acceptEdits"), "{claude}");
    let plan = rendered(&test_command(AgentKind::Claude, &chat("plan")));
    assert!(
        plan.contains("Permission mode: plan."),
        "plan keeps its paragraph"
    );
}

#[test]
fn codex_configuration_listing_is_strict_and_does_not_need_credentials() {
    assert_eq!(codex_server_names(br#"[{"name":"mail","enabled":true,"transport":{"env":{"TOKEN":"redacted"}}},{"name":"custom.slack","enabled":false}]"#).unwrap(), ["mail", "custom.slack"]);
    assert!(codex_server_names(b"{}").is_err());
    assert!(codex_server_names(br#"[{}]"#).is_err());
    assert!(codex_server_names(b"not json").is_err());
    assert_eq!(codex_server_names(b"[]").unwrap(), Vec::<String>::new());
}

#[test]
fn restrictions_are_enforced_or_rejected_before_spawn() {
    let mut run = spec("bypassPermissions", None, false);
    run.disallowed_tools = crate::claude_runner::NO_TOOLS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(validate_run_spec(AgentKind::Claude, &run).is_ok());
    let command = test_command(AgentKind::Claude, &run);
    let args: Vec<_> = command.as_std().get_args().collect();
    let position = args.iter().position(|s| *s == "--tools").unwrap();
    assert_eq!(args[position + 1], "");
    for kind in [AgentKind::Codex, AgentKind::Antigravity] {
        assert!(validate_run_spec(kind, &run)
            .unwrap_err()
            .contains("tool restrictions"));
        assert!(rendered(&test_command(kind, &run)).contains("exit 1"));
    }
    run.disallowed_tools.clear();
    run.allowed_tools = vec!["Read".into(), "Write".into()];
    assert!(rendered(&test_command(AgentKind::Claude, &run)).contains("--tools Read,Write"));
    assert!(validate_run_spec(AgentKind::Codex, &run).is_err());
}

/// OB23: a Claude run with tools (a chat here, built through the whole
/// `build_command`, a GUI-owned spec with its own env) carries the inline
/// Bash output cap, as env, never argv; the manifest's decision matches.
#[test]
fn a_claude_run_with_tools_caps_bash_output_inline() {
    let mut chat = spec("acceptEdits", None, true);
    chat.process_scope = ProcessScope::GuiOwned;
    chat.disallowed_tools = vec!["Bash(osascript:*)".into()];
    chat.extras.env = vec![("CARGO_TARGET_DIR".into(), "/tmp/t".into())];
    let cap = |cmd: &Command| env_value(cmd, "BASH_MAX_OUTPUT_LENGTH");
    let cmd = build_command(AgentKind::Claude, &chat);
    assert_eq!(cap(&cmd).as_deref(), Some("16000"));
    let argv = rendered(&cmd);
    assert!(!argv.contains("BASH_MAX"), "env, not argv: {argv}");
    assert_eq!(bash_output_cap(AgentKind::Claude, &chat), Some(16_000));
    // A restricted tool set that keeps Bash still gets it.
    let mut restricted = spec("acceptEdits", None, false);
    restricted.allowed_tools = vec!["Bash".into(), "Read".into()];
    let restricted = test_command(AgentKind::Claude, &restricted);
    assert_eq!(cap(&restricted).as_deref(), Some("16000"));
    // No Bash, nothing to cap: disallowed (the heartbeat's tool pass), or
    // left out of a restricted tool set.
    let mut no_bash = spec("acceptEdits", None, false);
    no_bash.disallowed_tools = vec!["Bash".into()];
    assert_eq!(cap(&test_command(AgentKind::Claude, &no_bash)), None);
    assert_eq!(bash_output_cap(AgentKind::Claude, &no_bash), None);
    let mut read_only = spec("acceptEdits", None, false);
    read_only.allowed_tools = vec!["Read".into(), "Grep".into()];
    assert_eq!(cap(&test_command(AgentKind::Claude, &read_only)), None);
    // Only Claude's command carries it.
    let plain = spec("acceptEdits", None, false);
    for kind in [AgentKind::Codex, AgentKind::Antigravity, AgentKind::Grok] {
        assert_eq!(bash_output_cap(kind, &plain), None, "{kind:?}");
    }
}

/// OB23: a tool-free one-shot (every `NO_TOOLS` entry disallowed, so
/// `--tools ""`) has no Bash to cap and gets no cap.
#[test]
fn a_tool_free_claude_one_shot_gets_no_bash_output_cap() {
    let mut run = spec("bypassPermissions", None, false);
    run.disallowed_tools = crate::claude_runner::NO_TOOLS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let cmd = test_command(AgentKind::Claude, &run);
    assert!(rendered(&cmd).contains("--disable-slash-commands"));
    assert_eq!(env_value(&cmd, "BASH_MAX_OUTPUT_LENGTH"), None);
    assert_eq!(bash_output_cap(AgentKind::Claude, &run), None);
}

/// NV9: every Claude run carries both fixed run settings in its environment
/// (never argv), whatever the extras say; Codex, Grok and Antigravity carry
/// neither. A runner run's leader copies the command's environment
/// (`remote_session::into_leader_with`), so the same two reach the Mac mini.
#[test]
fn claude_runs_carry_the_fixed_run_settings_and_no_other_agent_does() {
    let names = [
        CLAUDE_DISABLE_BACKGROUND_TASKS_ENV,
        CLAUDE_DISABLE_REFUSAL_FALLBACK_ENV,
    ];
    assert_eq!(
        names,
        [
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
            "CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK"
        ]
    );
    for base in [
        spec("acceptEdits", None, true),
        spec("acceptEdits", Some("thread-1"), true),
        spec("acceptEdits", None, false),
    ] {
        let claude = test_command(AgentKind::Claude, &base);
        for name in names {
            assert_eq!(env_value(&claude, name).as_deref(), Some("1"), "{name}");
            let argv = rendered(&claude);
            assert!(!argv.contains(name), "env, not argv: {argv}");
        }
        // The command's own environment, which a runner run's leader copies.
        let carried: Vec<String> = claude
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_some())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for name in names {
            assert!(carried.iter().any(|key| key == name), "{name}");
        }
    }

    // The run's own extras cannot turn either off.
    let mut off = spec("acceptEdits", None, true);
    off.extras.env = names
        .iter()
        .map(|n| (n.to_string(), "0".to_string()))
        .collect();
    let claude = test_command(AgentKind::Claude, &off);
    for name in names {
        assert_eq!(env_value(&claude, name).as_deref(), Some("1"), "{name}");
    }

    for kind in [AgentKind::Codex, AgentKind::Grok, AgentKind::Antigravity] {
        let cmd = test_command(kind, &spec("acceptEdits", None, true));
        for name in names {
            assert_eq!(env_value(&cmd, name), None, "{kind:?} {name}");
        }
    }
}

/// OB22: a compaction window from Settings reaches Claude as
/// `CLAUDE_CODE_AUTO_COMPACT_WINDOW` (env, never argv) and Codex as a
/// top-level `-c model_auto_compact_token_limit` before `exec`, fresh or
/// resumed, with Hollow's carrier kept off Codex's environment. Off, neither
/// command carries anything.
#[test]
fn the_compaction_window_reaches_claude_as_env_and_codex_as_config() {
    let on = crate::agent_settings::AgentSettings {
        compact_window: Some(400_000),
        ..Default::default()
    };
    let with_window = |base: RunSpec, kind: AgentKind| {
        let mut env = vec![("CARGO_TARGET_DIR".to_string(), "/tmp/t".to_string())];
        env.extend(crate::agent_settings::compaction_env_with(kind, &on));
        RunSpec {
            extras: RunExtras {
                env,
                ..RunExtras::default()
            },
            ..base
        }
    };
    let claude_run = with_window(spec("acceptEdits", None, true), AgentKind::Claude);
    let claude = test_command(AgentKind::Claude, &claude_run);
    let window = env_value(&claude, CLAUDE_COMPACT_WINDOW_ENV);
    assert_eq!(window.as_deref(), Some("400000"));
    assert_eq!(
        env_value(&claude, "CARGO_TARGET_DIR").as_deref(),
        Some("/tmp/t")
    );
    let argv = rendered(&claude);
    assert!(
        !argv.contains(CLAUDE_COMPACT_WINDOW_ENV),
        "env, not argv: {argv}"
    );
    assert!(!argv.contains("model_auto_compact"), "{argv}");
    assert_eq!(
        compact_window(AgentKind::Claude, &claude_run),
        Some(400_000)
    );

    for base in [
        spec("acceptEdits", None, true),
        spec("acceptEdits", Some("thread-1"), true),
    ] {
        let run = with_window(base, AgentKind::Codex);
        let codex = test_command(AgentKind::Codex, &run);
        let argv = rendered(&codex);
        let at = argv
            .find("model_auto_compact_token_limit=400000")
            .expect("the override");
        assert!(argv[..at].ends_with("-c "), "{argv}");
        assert!(
            at < argv.find(" exec").expect("exec"),
            "before exec: {argv}"
        );
        assert_eq!(env_value(&codex, CODEX_COMPACT_LIMIT_ENV), None);
        assert_eq!(
            env_value(&codex, "CARGO_TARGET_DIR").as_deref(),
            Some("/tmp/t")
        );
        assert_eq!(compact_window(AgentKind::Codex, &run), Some(400_000));
    }

    // Off (the default): no env var, no override, nothing recorded.
    let plain = spec("acceptEdits", None, true);
    let claude = test_command(AgentKind::Claude, &plain);
    assert_eq!(env_value(&claude, CLAUDE_COMPACT_WINDOW_ENV), None);
    let codex = rendered(&test_command(AgentKind::Codex, &plain));
    assert!(!codex.contains("model_auto_compact_token_limit"), "{codex}");
    for kind in EVERY_AGENT {
        assert_eq!(compact_window(kind, &plain), None, "{kind:?}");
    }
}

#[test]
fn codex_attaches_only_owned_mcp_and_escapes_values() {
    let config = r#"{"mcpServers":{"hollow_vault":{"command":"/Applications/Hollow.app/Contents/MacOS/hollow","args":["vault-mcp","/a path"],"env":{"KEY":"quote\"value"}}}}"#;
    let mut run = spec("acceptEdits", None, true);
    run.mcp_config = Some(config.into());
    let command = rendered(&build_with_codex_servers(
        AgentKind::Codex,
        &run,
        Ok(vec!["mail".into(), "hollow_vault".into()]),
    ));
    assert!(command.contains("\"mail\"={enabled=false}"));
    assert!(command.contains("\"hollow_vault\"={command="));
    assert!(!command.contains("\"hollow_vault\"={enabled=false}"));
    // Merged plan slice 6: the server's arguments and environment are in
    // the run's private file, read by `mcp-launch`; argv has neither.
    assert!(!command.contains("quote"), "{command}");
    assert!(!command.contains("vault-mcp"), "{command}");
    let cmd = build_with_codex_servers(AgentKind::Codex, &run, Ok(vec![]));
    let file = codex_config(&cmd);
    let server = &file["mcpServers"]["hollow_vault"];
    assert_eq!(server["env"]["KEY"], json!("quote\"value"));
    assert_eq!(server["args"], json!(["vault-mcp", "/a path"]));
    assert!(codex_owned_servers(
        r#"{"mcpServers":{"external":{"command":"external"}}}"#,
        "acceptEdits",
        &[]
    )
    .is_err());
    assert!(codex_owned_servers(
        r#"{"mcpServers":{"hollow_bad":{"args":[]}}}"#,
        "acceptEdits",
        &[]
    )
    .is_err());
    // PR A5: Hollow app calls may wait on a card and a sign-in, past
    // Codex's 60 s default; no other server's timeout changes.
    let app = codex_owned_servers(r#"{"mcpServers":{"hollow_app":{"command":"/x/hollow","args":["app-mcp"]},"hollow_tasks":{"command":"/x/hollow","args":["tasks-mcp"]},"hollow_team":{"command":"/x/hollow","args":["team-mcp"]}}}"#, "acceptEdits", &[]).unwrap();
    let timeout = crate::app_mcp::CODEX_TOOL_TIMEOUT_SECS;
    let entry = format!(
        "\"hollow_app\"={{{},enabled=true,default_tools_approval_mode=\"approve\",tool_timeout_sec={timeout}",
        launched("hollow_app")
    );
    assert!(app.contains(&entry), "{app}");
    // F10 (D1): a consultation's permit wait and deadline fit in the
    // team server's Codex timeout.
    let team = crate::team_mcp::CODEX_TOOL_TIMEOUT_SECS;
    let entry = format!(
        "\"hollow_team\"={{{},enabled=true,default_tools_approval_mode=\"approve\",tool_timeout_sec={team}",
        launched("hollow_team")
    );
    assert!(app.contains(&entry), "{app}");
    assert!(
        team > crate::delegation::CONSULTATION_SECONDS + 30,
        "deadline plus the permit wait"
    );
    assert_eq!(app.matches("tool_timeout_sec").count(), 2, "{app}");
    assert!(validate_run_spec(AgentKind::Antigravity, &run)
        .unwrap_err()
        .contains("Hollow-owned connectors"));
}

#[test]
fn antigravity_forwards_selected_model_and_exact_session() {
    let mut run = spec("plan", Some("conversation-one"), true);
    run.model = Some("provider/new-model".into());
    let command = rendered(&test_command(AgentKind::Antigravity, &run));
    assert!(command.contains("--model provider/new-model"));
    assert!(command.contains("--conversation conversation-one"));
    assert!(!command.contains("--continue"));
}

#[test]
fn grok_command_streams_resumes_and_maps_permissions() {
    let mut run = spec("plan", Some("01a0df79-39e3-7560-86f9-3b598ba5ff05"), true);
    run.model = Some("grok-4.7".into());
    let command = rendered(&test_command(AgentKind::Grok, &run));
    assert!(command.contains("--output-format streaming-messages-json"));
    assert!(command.contains("--include-partial-messages"));
    assert!(command.contains("--resume 01a0df79-39e3-7560-86f9-3b598ba5ff05"));
    assert!(command.contains("--model grok-4.7"));
    assert!(command.contains("--permission-mode plan"));
    assert!(!command.contains("--continue"));
    // A one-shot never streams partials or resumes.
    let oneshot = rendered(&test_command(
        AgentKind::Grok,
        &spec("bypassPermissions", Some("x"), false),
    ));
    assert!(!oneshot.contains("--include-partial-messages"));
    assert!(!oneshot.contains("--resume"));
    assert!(oneshot.contains("--permission-mode bypassPermissions"));
}

#[test]
fn grok_refuses_owned_mcp_and_tool_restrictions() {
    let mut run = spec("acceptEdits", None, true);
    run.mcp_config = Some(r#"{"mcpServers":{"hollow_vault":{"command":"x"}}}"#.into());
    assert!(validate_run_spec(AgentKind::Grok, &run).is_err());
    let mut run = spec("acceptEdits", None, true);
    run.allowed_tools = vec!["Read".into()];
    assert!(validate_run_spec(AgentKind::Grok, &run).is_err());
    assert!(validate_run_spec(AgentKind::Grok, &spec("acceptEdits", None, true)).is_ok());
}

/// Merged plan slice 2. Grok's headless `-p` ran every command in plan,
/// default and acceptEdits (probed 2026-10-02), so those modes run in its
/// OS sandbox, fresh and resumed alike (a resumed session refuses a
/// different profile, which `result_kind` turns into a fresh retry). Auto
/// and bypass pass `off`, so a chat switched up from a sandboxed mode
/// retries fresh instead of keeping its old profile. The computer-control pair goes in as `--deny`;
/// host-only exclusions are inert and left out; any other deny refuses.
#[test]
fn grok_sandboxes_ask_modes_and_denies_desktop_commands() {
    let desktop: Vec<String> = crate::computer_mcp::BASH_DENY
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (perm, profile) in [
        ("plan", Some("read-only")),
        ("default", Some("read-only")),
        ("acceptEdits", Some("workspace")),
        ("auto", Some("off")),
        ("bypassPermissions", Some("off")),
    ] {
        for resume in [None, Some("01a0df79-39e3-7560-86f9-3b598ba5ff05")] {
            for interactive in [true, false] {
                let case = format!("{perm} resume={resume:?} interactive={interactive}");
                let mut run = spec(perm, resume, interactive);
                run.disallowed_tools = vec!["SendUserFile".into()];
                run.disallowed_tools.extend(desktop.clone());
                validate_run_spec(AgentKind::Grok, &run).unwrap_or_else(|e| panic!("{case}: {e}"));
                let args: Vec<String> = test_command(AgentKind::Grok, &run)
                    .as_std()
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                let sandbox = args
                    .iter()
                    .position(|a| a == "--sandbox")
                    .map(|i| args[i + 1].as_str());
                assert_eq!(sandbox, profile, "{case}: {args:?}");
                for rule in &desktop {
                    assert!(
                        args.windows(2).any(|w| w[0] == "--deny" && w[1] == *rule),
                        "{case}: {rule} in {args:?}"
                    );
                }
                assert!(
                    !args.iter().any(|a| a == "SendUserFile"),
                    "{case}: inert, never passed"
                );
            }
        }
    }
    // Unknown modes run as acceptEdits, sandbox included.
    assert!(
        rendered(&test_command(AgentKind::Grok, &spec("yolo", None, true)))
            .contains("--sandbox workspace")
    );
    // Other denies are unprobed for Grok and still refuse.
    for rule in ["Bash(rm:*)", "WebFetch"] {
        let mut run = spec("auto", None, true);
        run.disallowed_tools = vec![rule.into()];
        assert!(validate_run_spec(AgentKind::Grok, &run).is_err(), "{rule}");
    }
    // Codex and Antigravity still can't take the pair as a flag.
    for kind in [AgentKind::Codex, AgentKind::Antigravity] {
        let mut run = spec("auto", None, true);
        run.disallowed_tools = desktop.clone();
        assert!(validate_run_spec(kind, &run).is_err(), "{kind:?}");
    }
}

fn grok_events(fixture: &str) -> Vec<AgentEvent> {
    let mut state = ParseState::default();
    fixture
        .lines()
        .flat_map(|l| parse_line(AgentKind::Grok, l, &mut state))
        .collect()
}

#[test]
fn grok_recorded_streams_parse_like_claude() {
    let oneshot = grok_events(include_str!("../fixtures/grok/oneshot.jsonl"));
    assert!(oneshot.iter().any(
        |e| matches!(e, AgentEvent::Session(s) if s == "01a0df79-39e3-7560-86f9-3b598ba5ff05")
    ));
    assert!(oneshot.iter().any(
        |e| matches!(e, AgentEvent::Result { text: Some(t), is_error: false, .. } if t == "OK")
    ));

    let tool = grok_events(include_str!("../fixtures/grok/tool-call.jsonl"));
    assert!(tool
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolUse { name, .. } if name == "read_file")));
    assert!(tool.iter().any(|e| matches!(
        e,
        AgentEvent::ToolResult {
            is_error: false,
            ..
        }
    )));
    assert!(tool
        .iter()
        .any(|e| matches!(e, AgentEvent::Result { text: Some(t), .. } if t == "banana")));

    let mut state = ParseState::default();
    let partial: Vec<AgentEvent> = include_str!("../fixtures/grok/partial.jsonl")
        .lines()
        .flat_map(|l| parse_line(AgentKind::Grok, l, &mut state))
        .collect();
    let streamed: String = partial
        .iter()
        .filter_map(|e| match e {
            AgentEvent::AssistantDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "1\n2\n3\n4\n5");

    let resumed = grok_events(include_str!("../fixtures/grok/resume.jsonl"));
    assert!(resumed.iter().any(
        |e| matches!(e, AgentEvent::Session(s) if s == "01a0df79-39e3-7560-86f9-3b598ba5ff05")
    ));
}

/// OB08a: the model's thinking is an event of its own, never text, for
/// every agent whose stream carries it: Grok's recorded partial stream
/// (before its first text), Claude's recorded thinking part, and a Codex
/// `reasoning` item (SYNTHETIC: no recorded Codex fixture has one).
/// Antigravity's recorded stream has none.
#[test]
fn thinking_is_an_event_for_every_agent_that_streams_it() {
    let events = |kind: AgentKind, fixture: &str| -> Vec<AgentEvent> {
        let mut state = ParseState::default();
        fixture
            .lines()
            .flat_map(|l| parse_line(kind, l, &mut state))
            .collect()
    };
    let thinks = |e: &AgentEvent| matches!(e, AgentEvent::Thinking);

    let partial = grok_events(include_str!("../fixtures/grok/partial.jsonl"));
    let thinking = partial.iter().position(thinks).expect("Grok thinks");
    let text = partial
        .iter()
        .position(|e| matches!(e, AgentEvent::AssistantDelta(_)))
        .expect("Grok writes");
    assert!(thinking < text, "thinking streams first");

    let claude = include_str!("../fixtures/usage/claude-max-budget.jsonl");
    assert!(events(AgentKind::Claude, claude).iter().any(thinks));

    let started = r#"{"type":"item.started","item":{"id":"item_0","type":"reasoning","text":""}}"#;
    let completed =
        r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"**Plan**"}}"#;
    for line in [started, completed] {
        let evts = events(AgentKind::Codex, line);
        assert!(evts.iter().any(thinks));
        if line == completed {
            assert!(evts
                .iter()
                .any(|e| matches!(e, AgentEvent::Reasoning(text) if text == "**Plan**")));
        }
    }

    let agy = include_str!("../fixtures/results/antigravity-denied-write.jsonl");
    assert!(!events(AgentKind::Antigravity, agy).iter().any(thinks));
}

#[test]
fn gemini_is_not_an_agent_alias() {
    assert_eq!(AgentKind::from_opt("gemini"), None);
    assert_eq!(AgentKind::from_opt("agy"), Some(AgentKind::Antigravity));
    assert_eq!(AgentKind::from_opt("grok"), Some(AgentKind::Grok));
}

use super::*;

#[test]
fn kind_round_trips_through_string() {
    for k in AgentKind::all() {
        assert_eq!(AgentKind::from_opt(k.as_str()), Some(k));
    }
}

/// A·R3: the event `drive` delivers in place of an over-long stdout line
/// is recognized and yields nothing, for every CLI's parser; prose that
/// merely mentions its type is not mistaken for it.
#[test]
fn parse_line_skips_the_dropped_stdout_line_event() {
    use crate::process_group::{dropped_line_bytes, dropped_line_event, STDOUT_LINE_MAX};
    let line = dropped_line_event(20_000_000, STDOUT_LINE_MAX);
    assert_eq!(dropped_line_bytes(&line), Some(20_000_000));
    for kind in AgentKind::all() {
        let mut state = ParseState::default();
        assert!(parse_line(kind, &line, &mut state).is_empty(), "{kind:?}");
    }
    let prose = r#"{"type":"assistant","text":"hollow_stdout_line_dropped"}"#;
    assert_eq!(dropped_line_bytes(prose), None);
    assert_eq!(dropped_line_bytes("hollow_stdout_line_dropped"), None);
}

#[test]
fn serde_uses_plain_lowercase_strings() {
    assert_eq!(
        serde_json::to_string(&AgentKind::Codex).unwrap(),
        "\"codex\""
    );
    let k: AgentKind = serde_json::from_str("\"antigravity\"").unwrap();
    assert_eq!(k, AgentKind::Antigravity);
}

fn spec(perm: &str, resume: Option<&str>, interactive: bool) -> RunSpec {
    RunSpec {
        prompt: "hello".into(),
        resume: resume.map(String::from),
        model: None,
        perm: perm.into(),
        cwd: PathBuf::from("/tmp"),
        allowed_tools: vec![],
        disallowed_tools: vec![],
        interactive,
        mcp_config: None,
        append_system_prompt: None,
        // Keeps the rendered-argument tests independent of run tagging.
        process_scope: ProcessScope::Inherited,
        usage_stream: false,
        max_budget_usd: None,
        extras: RunExtras::default(),
    }
}

fn env_value(cmd: &Command, key: &str) -> Option<String> {
    cmd.as_std()
        .get_envs()
        .find(|(name, _)| *name == std::ffi::OsStr::new(key))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
}

#[test]
fn build_command_isolates_and_tags_only_gui_owned_specs() {
    use crate::process_group::RUN_TAG_ENV;
    let mut run = spec("acceptEdits", None, true);
    run.process_scope = ProcessScope::GuiOwned;
    run.mcp_config = Some(r#"{"mcpServers":{"hollow_vault":{"command":"x","args":[]}}}"#.into());

    // Claude: the tag rides the CLI's env and every owned server's env.
    let claude = build_command(AgentKind::Claude, &run);
    let tag = env_value(&claude, RUN_TAG_ENV).expect("GUI-owned runs carry a run tag");
    assert_eq!(tag.len(), 32);
    assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
    let config = claude_config(&claude);
    assert_eq!(
        config["mcpServers"]["hollow_vault"]["env"][RUN_TAG_ENV],
        json!(tag)
    );

    // Codex: the tag reaches the owned server through the run's file,
    // which `mcp-launch` applies.
    let codex = build_scoped(AgentKind::Codex, &run, Ok(vec![]));
    let codex_tag = env_value(&codex, RUN_TAG_ENV).expect("tagged codex run");
    let file = codex_config(&codex);
    assert_eq!(
        file["mcpServers"]["hollow_vault"]["env"][RUN_TAG_ENV],
        json!(codex_tag)
    );
    assert_ne!(tag, codex_tag, "every run gets a fresh tag");

    // Inherited: no tag anywhere, arguments unchanged.
    run.process_scope = ProcessScope::Inherited;
    let plain = build_command(AgentKind::Claude, &run);
    assert!(env_value(&plain, RUN_TAG_ENV).is_none());
    assert!(!rendered(&plain).contains(RUN_TAG_ENV));
}

/// Merged plan slice 6 (reviews A·S5, B·F11): no provider's argv carries
/// a broker proof or a connector secret, where any process on the Mac
/// could read it (`ps -ww`). Claude and Codex still hand every one to
/// their servers, through the run's private file (and, for an HTTP
/// header, Codex's own environment), and the command names the run ids
/// the brokers' peer check registers for its process tree.
#[test]
fn no_agent_argv_carries_a_proof_or_secret() {
    use crate::run_auth::BOUND_RUNS_ENV;
    let secrets = [
        "proof-app-1",
        "proof-tasks-2",
        "proof-svc-3",
        "lib-env-4",
        "hdr-5",
    ];
    let config = json!({"mcpServers": {
        "hollow_app": {"command": "/x/hollow", "args": ["app-mcp"], "env": {"HOLLOW_RUN_ID": "run-9", "HOLLOW_APP_PROOF": "proof-app-1"}},
        "hollow_tasks": {"command": "/x/hollow", "args": ["tasks-mcp"], "env": {"HOLLOW_RUN_ID": "run-9", "HOLLOW_TASKS_PROOF": "proof-tasks-2"}},
        "hollow_services": {"command": "/x/hollow", "args": ["services-mcp"], "env": {"HOLLOW_SERVICES_RUN": "run-9", "HOLLOW_SERVICES_PROOF": "proof-svc-3"}},
        "crm": {"command": "/usr/bin/crm", "args": ["serve"], "env": {"API_KEY": "lib-env-4"}},
        "mail": {"type": "http", "url": "https://mail.example/mcp", "headers": {"Authorization": "Bearer hdr-5"}}
    }});
    let shared = |id: &str| SharedConnector {
        id: id.into(),
        ..Default::default()
    };
    for kind in AgentKind::all() {
        let mut run = spec("acceptEdits", None, true);
        run.process_scope = ProcessScope::GuiOwned;
        run.mcp_config = Some(config.to_string());
        run.extras.shared_connectors = vec![shared("crm"), shared("mail")];
        let cmd = build_scoped(kind, &run, Ok(vec![]));
        let argv = rendered(&cmd);
        for secret in secrets {
            assert!(!argv.contains(secret), "{kind:?} argv has {secret}: {argv}");
        }
        let bound = env_value(&cmd, BOUND_RUNS_ENV);
        match kind {
            AgentKind::Claude => {
                let file = claude_config(&cmd).to_string();
                for secret in secrets {
                    assert!(file.contains(secret), "{secret} reaches its server");
                }
                assert_eq!(bound.as_deref(), Some(r#"["run-9"]"#));
            }
            AgentKind::Codex => {
                let file = codex_config(&cmd).to_string();
                for secret in &secrets[..4] {
                    assert!(file.contains(secret), "{secret} reaches its server");
                }
                let header = env_value(&cmd, "HOLLOW_MCP_SECRET_HEADER_0");
                assert_eq!(header.as_deref(), Some("Bearer hdr-5"));
                assert_eq!(bound.as_deref(), Some(r#"["run-9"]"#));
            }
            AgentKind::Antigravity | AgentKind::Grok => {
                assert!(is_unavailable_command(&cmd), "{kind:?} takes no MCP config");
                assert_eq!(bound, None);
            }
        }
    }
}

#[test]
fn run_tag_is_injected_into_owned_mcp_server_env() {
    use crate::process_group::{with_run_tag, RunTag, RUN_TAG_ENV};
    let tag = RunTag::new();
    let tagged = with_run_tag(
        r#"{"mcpServers":{"hollow_a":{"command":"a"},"hollow_b":{"command":"b","env":{"KEEP":"1"}}}}"#,
        &tag,
    );
    let value: Value = serde_json::from_str(&tagged).unwrap();
    assert_eq!(
        value["mcpServers"]["hollow_a"]["env"][RUN_TAG_ENV],
        json!(tag.as_str())
    );
    assert_eq!(value["mcpServers"]["hollow_b"]["env"]["KEEP"], json!("1"));
    assert_eq!(
        value["mcpServers"]["hollow_b"]["env"][RUN_TAG_ENV],
        json!(tag.as_str())
    );
    assert_eq!(with_run_tag("not json", &tag), "not json");
    // Codex's overrides carry no env at all: `mcp-launch` applies the
    // tagged env from the run's file (merged plan slice 6).
    assert!(!codex_owned_servers(&tagged, "acceptEdits", &[])
        .unwrap()
        .contains(RUN_TAG_ENV));
}

/// Paperclip PR 19: exact roots only, judged after canonicalization, and
/// the configured vault is never refused.
#[cfg(target_os = "macos")]
#[test]
fn unsafe_working_dirs_are_exact_roots_after_canonicalization_and_never_the_vault() {
    let home = dirs::home_dir().expect("a home folder");
    let vault = tempfile::Builder::new()
        .prefix("vault-cwd-")
        .tempdir()
        .unwrap();
    let project = tempfile::Builder::new()
        .prefix("project-cwd-")
        .tempdir()
        .unwrap();
    for denied in [
        PathBuf::from("/"),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/usr"),
        PathBuf::from("/System"),
        PathBuf::from("/Library"),
        home.clone(),
    ] {
        let reason = unsafe_working_dir(&denied, Some(vault.path()))
            .unwrap_or_else(|| panic!("{} must be refused", denied.display()));
        assert!(reason.contains("system or shared folder"), "{reason}");
    }
    let tmp_child = PathBuf::from("/tmp").join(format!("hollow-cwd-probe-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_child).unwrap();
    for allowed in [
        tmp_child.clone(),
        project.path().to_path_buf(),
        home.join("Documents"),
        PathBuf::from("/definitely/missing/hollow-folder"),
    ] {
        assert_eq!(
            unsafe_working_dir(&allowed, Some(vault.path())),
            None,
            "{}",
            allowed.display()
        );
    }
    let _ = std::fs::remove_dir(&tmp_child);
    // A vault that IS the home folder (or any denied root) is never refused.
    assert_eq!(unsafe_working_dir(&home, Some(&home)), None);
    assert_eq!(
        unsafe_working_dir(Path::new("/tmp"), Some(Path::new("/private/tmp"))),
        None
    );

    let error = unsafe_cwd_error("/tmp is a system or shared folder");
    assert!(error.starts_with(UNSAFE_CWD_ERROR), "{error}");
    assert!(!error.starts_with(crate::chat::CHAT_CONTEXT_RECOVERED_ERROR));
    assert!(!error.contains('\u{2014}') && !error.contains('<') && !error.contains('>'));
}

/// Pins the per-site process-group decision: every GUI spawn leads its own
/// group, stdio team delegates inherit, and HTTP team delegates own a group.
/// No MCP subprocess isolates. Count field assignments, not eligibility comparisons.
#[test]
fn gui_run_specs_isolate_and_team_mcp_inherits() {
    let count = |source: &str, needle: &str| source.matches(needle).count();
    let gui_fields = |source: &str| {
        source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("process_scope:"))
            .filter(|value| {
                matches!(
                    value.trim(),
                    "agent::ProcessScope::GuiOwned," | "crate::agent::ProcessScope::GuiOwned,"
                )
            })
            .count()
    };
    // A comparison must neither count as a field nor hide a missing assignment.
    assert_eq!(
        gui_fields("spec.process_scope == agent::ProcessScope::GuiOwned"),
        0
    );
    assert_eq!(
        gui_fields("process_scope: agent::ProcessScope::Inherited,"),
        0
    );
    assert_eq!(
        gui_fields(concat!(
            "process_scope: agent::ProcessScope::GuiOwned,\n",
            "process_scope: crate::agent::ProcessScope::GuiOwned,\n",
            "spec.process_scope == agent::ProcessScope::GuiOwned\n"
        )),
        2
    );
    for (name, source) in [
        // chat_queue.rs is chat.rs's queued-turn half (B14): scan both.
        (
            "chat.rs + chat_queue.rs",
            concat!(include_str!("chat.rs"), include_str!("chat_queue.rs")),
        ),
        ("workflows.rs", include_str!("workflows.rs")),
        ("ingest.rs", include_str!("ingest.rs")),
        ("heartbeat.rs", include_str!("heartbeat.rs")),
        ("claude_runner.rs", include_str!("claude_runner.rs")),
    ] {
        let specs = count(source, "RunSpec {");
        assert!(specs > 0, "{name} builds no RunSpec");
        assert_eq!(
            specs,
            gui_fields(source),
            "{name}: every GUI spawn is GuiOwned"
        );
        assert_eq!(count(source, "ProcessScope::Inherited"), 0, "{name}");
    }
    let team = include_str!("team_mcp.rs");
    assert!(count(team, "RunSpec {") > 0);
    assert_eq!(
        count(team, "ProcessScope::Inherited"),
        count(team, "RunSpec {")
    );
    assert_eq!(count(team, "ProcessScope::GuiOwned"), 1);
    assert!(
        team.contains(concat!(
            "process_scope: if REQUEST.try_with(|_| ()).is_ok() {\n",
            "            agent::ProcessScope::GuiOwned\n",
            "        } else {\n",
            "            agent::ProcessScope::Inherited\n",
            "        }"
        )),
        "only an HTTP request owns its team group"
    );
    // Paperclip PR 8b: team-mcp never probes the Claude CLI itself (the
    // spending backstop is GUI-only).
    assert!(!team.contains(concat!("claude_max_budget", "_supported")));
    for (name, source, extra) in OWNED_MCP_SOURCES {
        for banned in MCP_SOURCE_BANS.iter().chain(extra.iter()) {
            assert!(
                !source.contains(*banned),
                "{name} must never contain `{banned}`"
            );
        }
    }
    // proc_info only probes liveness: every signal it sends is signal 0.
    let proc_info = include_str!("proc_info.rs");
    let kills = proc_info.matches("libc::kill(").count();
    assert!(kills > 0);
    assert_eq!(
        kills,
        proc_info.matches("libc::kill(pid, 0)").count(),
        "proc_info.rs must never deliver a signal"
    );
}

/// What no code that runs inside a Hollow MCP subprocess may contain: it
/// must never create or signal a process group (PR 1), never touch the
/// Keychain-reading quota widget (PR 8c), and never run the GUI-only
/// repository capture (PR 16: it spawns git under the GUI's process
/// cleanup and writes the run's history sidecar).
const MCP_SOURCE_BANS: &[&str] = &[
    "process_group::isolate",
    "process_group::run_to_completion",
    "claude_quota::",
    "claude_accounts::",
    "repo_diff::",
];

/// Every source file that runs inside a Hollow MCP subprocess, with the
/// extra substrings each must never contain (Paperclip PR 13's shared
/// table; PR 12 adds `tasks_mcp.rs`, PR 16 `repo_diff::`). A new MCP
/// server adds its row here. The question server is stateless: it never
/// reaches the session store, the interactions store, the run queue or
/// the vault. Its own test builds these needles with `concat!`, so the
/// file never matches itself.
const OWNED_MCP_SOURCES: &[(&str, &str, &[&str])] = &[
    ("team_mcp.rs", include_str!("team_mcp.rs"), &[]),
    ("computer_mcp.rs", include_str!("computer_mcp.rs"), &[]),
    ("vault_mcp.rs", include_str!("vault_mcp.rs"), &[]),
    // team-mcp calls into it (`budgets::preflight_readonly`, PR 8b).
    ("budgets.rs", include_str!("budgets.rs"), &[]),
    // team-mcp takes run permits, and the permit samples memory
    // pressure through proc_info (Paperclip PR 20).
    ("run_permit.rs", include_str!("run_permit.rs"), &[]),
    ("proc_info.rs", include_str!("proc_info.rs"), &[]),
    // team-mcp validates its parent's scratch folder (Paperclip PR 15a).
    ("scratch.rs", include_str!("scratch.rs"), &[]),
    (
        "question_mcp.rs",
        include_str!("question_mcp.rs"),
        &[
            "session_store::",
            "interactions::",
            "run_queue::",
            "HOLLOW_VAULT",
            "automated_history::",
        ],
    ),
    // The question server's only dependency.
    (
        "interaction_schema.rs",
        include_str!("interaction_schema.rs"),
        &[
            "session_store::",
            "interactions::",
            "run_queue::",
            "automated_history::",
        ],
    ),
    // Build PR B6: the services server forwards every call to the GUI's
    // broker; it never starts, signals or waits for a process itself.
    (
        "services_mcp.rs",
        include_str!("services_mcp.rs"),
        &[
            "Command::new",
            "process::Command",
            "libc::",
            "killpg",
            "spawn(",
            "services::",
            "automated_history::",
        ],
    ),
    // PR A1: the Hollow app server forwards every call to the GUI's
    // broker; it never runs a handler or reaches the GUI's state itself.
    (
        "app_mcp.rs",
        include_str!("app_mcp.rs"),
        &[
            "commands::",
            "automated_history::",
            "app_broker::dispatch",
            "app_broker::start",
            "atomic_write",
        ],
    ),
    // Paperclip PR 12: the task server reads the vault root and forwards
    // every write to the GUI's broker; it never writes the vault, the
    // review queue or history itself.
    (
        "tasks_mcp.rs",
        include_str!("tasks_mcp.rs"),
        &[
            "vault::mutate_note",
            "write_note",
            "atomic_write",
            "create_note",
            "review_queue::",
            "vault_tasks::upsert",
            "vault_tasks::mutate",
            "vault_tasks::set_status",
            "vault_tasks::apply_update",
            "vault_tasks::create_from_input",
            "vault_tasks::create_agent_task",
            "vault_tasks::append_activity",
            "task_broker::dispatch",
            "task_broker::start",
            "automated_history::",
        ],
    ),
];

/// Paperclip PR 13 (cross-m34 H1): Codex MCP calls are normalized to
/// Claude's `mcp__<server>__<tool>` naming, from the recorded stream.
#[test]
fn parse_codex_normalizes_recorded_mcp_tool_calls() {
    let approved = fixture_events(
        AgentKind::Codex,
        include_str!("../fixtures/interactions/codex-mcp-tool-call.jsonl"),
    );
    let uses: Vec<_> = approved
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::ToolUse { id, name, input } => {
                Some((id.clone(), name.clone(), input.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        uses[0],
        (
            "item_1".into(),
            "mcp__hollow_vault__search_vault".into(),
            json!({"query": "zebra"})
        )
    );
    assert_eq!(uses[1].1, "Bash", "command executions are unchanged");
    let results: Vec<_> = approved
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } => Some((id.clone(), content.clone(), *is_error)),
            _ => None,
        })
        .collect();
    assert_eq!(results[0].0, "item_1");
    assert!(
        results[0].1.starts_with("Found 1 match"),
        "{}",
        results[0].1
    );
    assert!(!results[0].2);
    // One ToolUse per call: the completion does not repeat it.
    assert_eq!(uses.iter().filter(|u| u.0 == "item_1").count(), 1);

    let refused = fixture_events(
        AgentKind::Codex,
        include_str!("../fixtures/interactions/codex-mcp-tool-call-unapproved.jsonl"),
    );
    let failed = refused
        .iter()
        .find_map(|evt| match evt {
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } if id == "item_1" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("the failed call's result");
    assert!(failed.1, "status failed is an error");
    assert!(failed.0.contains("requires approval"), "{}", failed.0);
}

/// The live Claude Code 2.1.283 captures (2026-09-25, `-p` in
/// `acceptEdits`): with the server preapproved, ToolSearch loads the
/// deferred tool and the MCP call succeeds; without it, the call is
/// denied (`permission_denied`, an `is_error` result) while the run still
/// ends `success`. Main-agent lines carry `parent_tool_use_id: null`, so
/// none reads as nested, and neither stream produces a question card.
#[test]
fn parse_claude_replays_recorded_mcp_captures() {
    let preapproved = include_str!("../fixtures/interactions/claude-mcp-preapproved.jsonl");
    let evts = fixture_events(AgentKind::Claude, preapproved);
    let uses: Vec<_> = evts
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::ToolUse { id, name, input } => {
                Some((id.clone(), name.clone(), input.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(uses[0].1, "ToolSearch", "{uses:?}");
    assert_eq!(
        uses[1],
        (
            "toolu-002".into(),
            "mcp__hollow_vault__search_vault".into(),
            json!({"query": "zebra"})
        )
    );
    let result = evts
        .iter()
        .find_map(|evt| match evt {
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } if id == "toolu-002" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("the MCP call's result");
    assert!(!result.1, "a preapproved call succeeds");
    assert!(result.0.contains("Found 1 match"), "{}", result.0);
    for line in preapproved.lines() {
        assert!(!crate::interaction_schema::claude_is_nested(line), "{line}");
    }
    let (calls, _) = crate::interaction_schema::tests::replay(AgentKind::Claude, preapproved, None);
    assert!(calls.is_empty(), "a vault search is not a question card");

    let unapproved = include_str!("../fixtures/interactions/claude-mcp-unapproved.jsonl");
    let evts = fixture_events(AgentKind::Claude, unapproved);
    let denied = evts
        .iter()
        .find_map(|evt| match evt {
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } if id == "toolu-002" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("the denied call's result");
    assert!(denied.1, "the denial is an error result");
    assert!(denied.0.contains("haven't granted it yet"), "{}", denied.0);
    assert!(
        evts.iter().any(|evt| matches!(
            evt,
            AgentEvent::Result {
                is_error: false,
                ..
            }
        )),
        "the run still ends success, so only the preapproval makes the tool usable"
    );
    let (calls, _) = crate::interaction_schema::tests::replay(AgentKind::Claude, unapproved, None);
    assert!(calls.is_empty());
}

#[test]
fn parse_codex_mcp_fallbacks() {
    // A completion whose start was never seen: the ToolUse comes first.
    let mut st = ParseState::default();
    let done = r#"{"type":"item.completed","item":{"id":"item_4","type":"mcp_tool_call","server":"hollow_team","tool":"ask_agent","arguments":"{\"agent\":\"claude\"}","result":{"content":[{"type":"text","text":"ok"}],"structured_content":null},"error":null,"status":"completed"}}"#;
    let evts = parse_codex(done, &mut st);
    match (&evts[0], &evts[1]) {
        (
            AgentEvent::ToolUse { id, name, input },
            AgentEvent::ToolResult {
                content, is_error, ..
            },
        ) => {
            assert_eq!(id, "item_4");
            assert_eq!(name, "mcp__hollow_team__ask_agent");
            assert_eq!(
                input,
                &json!({"agent": "claude"}),
                "string arguments are parsed"
            );
            assert_eq!(content, "ok");
            assert!(!is_error);
        }
        other => panic!("expected a normalized pair, got {other:?}"),
    }
    // A non-null error fails the call even when the status says completed.
    let errored = r#"{"type":"item.completed","item":{"id":"item_5","type":"mcp_tool_call","server":"hollow_vault","tool":"search_vault","arguments":{},"result":null,"error":{"message":"boom"},"status":"completed"}}"#;
    match parse_codex(errored, &mut st).last() {
        Some(AgentEvent::ToolResult {
            content, is_error, ..
        }) => {
            assert!(*is_error);
            assert_eq!(content, "boom");
        }
        other => panic!("expected an error result, got {other:?}"),
    }
    // No server or tool names: the legacy generic pair.
    let legacy = r#"{"type":"item.completed","item":{"id":"item_6","type":"mcp_tool_call","status":"completed"}}"#;
    match parse_codex(legacy, &mut st).as_slice() {
        [AgentEvent::ToolUse { name, .. }, AgentEvent::ToolResult { is_error, .. }] => {
            assert_eq!(name, "mcp_tool_call");
            assert!(!is_error);
        }
        other => panic!("expected the legacy pair, got {other:?}"),
    }
}

fn with_servers(names: &[&str]) -> String {
    let servers: serde_json::Map<String, Value> = names
        .iter()
        .map(|name| {
            (
                name.to_string(),
                json!({ "command": "/bin/hollow", "args": ["x"] }),
            )
        })
        .collect();
    json!({ "mcpServers": servers }).to_string()
}

#[test]
fn owned_preapprovals_are_table_driven_and_exclude_computer() {
    let config = with_servers(&[
        "hollow_computer",
        "hollow_team",
        "hollow_vault",
        "hollow_questions",
    ]);
    assert_eq!(
        owned_preapprovals(Some(&config), "acceptEdits"),
        vec![
            "mcp__hollow_questions",
            "mcp__hollow_vault",
            "mcp__hollow_team"
        ]
    );
    // A read-only run never preapproves teamwork: its delegate can write.
    assert_eq!(
        owned_preapprovals(Some(&config), "plan"),
        vec!["mcp__hollow_questions", "mcp__hollow_vault"]
    );
    assert_eq!(
        owned_preapprovals(Some(&with_servers(&["hollow_tasks"])), "acceptEdits"),
        vec!["mcp__hollow_tasks"]
    );
    assert!(
        owned_preapprovals(Some(&with_servers(&["hollow_computer"])), "acceptEdits").is_empty()
    );
    assert!(owned_preapprovals(None, "acceptEdits").is_empty());
    assert!(owned_preapprovals(Some("not json"), "acceptEdits").is_empty());
}

#[test]
fn claude_preapproves_owned_servers_without_restricting_builtins() {
    let mut run = spec("acceptEdits", None, true);
    run.mcp_config = Some(with_servers(&[
        "hollow_questions",
        "hollow_computer",
        "hollow_vault",
    ]));
    let chat = rendered(&test_command(AgentKind::Claude, &run));
    assert!(
        chat.contains("--allowed-tools mcp__hollow_questions mcp__hollow_vault WebFetch"),
        "{chat}"
    );
    assert!(
        !chat.contains("--allowed-tools mcp__hollow_computer"),
        "{chat}"
    );
    assert!(
        !chat.contains("mcp__hollow_computer "),
        "computer is not preapproved in acceptEdits: {chat}"
    );
    assert!(
        !chat.contains("--tools "),
        "an unrestricted run keeps every built-in: {chat}"
    );

    // Plan mode: teamwork attached but never preapproved.
    let mut plan = spec("plan", None, true);
    plan.mcp_config = Some(with_servers(&["hollow_questions", "hollow_team"]));
    let plan = rendered(&test_command(AgentKind::Claude, &plan));
    assert!(
        plan.contains("--allowed-tools mcp__hollow_questions --"),
        "{plan}"
    );
    assert!(
        !plan.contains("mcp__hollow_team"),
        "a read-only run never preapproves a writing delegate: {plan}"
    );

    // No owned server: only the read-only research preapprovals, no MCP token.
    let plain = rendered(&test_command(
        AgentKind::Claude,
        &spec("acceptEdits", None, true),
    ));
    assert!(!plain.contains("mcp__"), "{plain}");
    assert!(
        plain.contains("--allowed-tools WebFetch WebSearch"),
        "{plain}"
    );
    assert!(!plain.contains("gh api"), "{plain}");
}

/// Paperclip PR 12: `hollow_tasks` is preapproved without restricting the
/// built-ins, and a restricted workflow lists its token once.
#[test]
fn claude_preapproves_hollow_tasks_without_restricting_builtins() {
    let mut chat = spec("acceptEdits", None, true);
    chat.mcp_config = Some(with_servers(&["hollow_tasks"]));
    let open = rendered(&test_command(AgentKind::Claude, &chat));
    assert!(
        open.contains("--allowed-tools mcp__hollow_tasks WebFetch"),
        "{open}"
    );
    assert!(!open.contains("--tools "), "{open}");

    let mut restricted = spec("acceptEdits", None, false);
    restricted.allowed_tools = vec!["Read".into(), crate::tasks_mcp::TASKS_TOOL_ALLOW.into()];
    restricted.mcp_config = Some(with_servers(&["hollow_tasks"]));
    let workflow = rendered(&test_command(AgentKind::Claude, &restricted));
    assert!(
        workflow.contains("--allowed-tools Read mcp__hollow_tasks WebFetch"),
        "{workflow}"
    );
    assert!(workflow.contains("--tools Read,ToolSearch"), "{workflow}");
    assert_eq!(
        workflow.matches("mcp__hollow_tasks").count(),
        1,
        "{workflow}"
    );

    let none = rendered(&test_command(
        AgentKind::Claude,
        &spec("acceptEdits", None, true),
    ));
    assert!(!none.contains("mcp__hollow_tasks"), "{none}");
    let codex = codex_owned_servers(&with_servers(&["hollow_tasks"]), "acceptEdits", &[]).unwrap();
    assert!(
        codex.contains("default_tools_approval_mode=\"approve\""),
        "{codex}"
    );
}

#[test]
fn a_restricted_claude_run_with_mcp_keeps_tool_search() {
    let mut run = spec("acceptEdits", None, false);
    run.allowed_tools = vec!["Read".into(), crate::team_mcp::TEAM_TOOL_ALLOW.into()];
    run.mcp_config = Some(with_servers(&["hollow_team"]));
    let workflow = rendered(&test_command(AgentKind::Claude, &run));
    assert!(workflow.contains("--tools Read,ToolSearch"), "{workflow}");
    assert_eq!(
        workflow.matches("mcp__hollow_team").count(),
        1,
        "the spec's own token and the preapproval are one entry: {workflow}"
    );
    // Without an MCP server the restricted list is unchanged.
    run.mcp_config = None;
    let bare = rendered(&test_command(AgentKind::Claude, &run));
    assert!(bare.contains("--tools Read "), "{bare}");
    assert!(!bare.contains("ToolSearch"), "{bare}");
}

/// Overnight S8 review: the built-in Plaud's `login` and `logout` tools
/// (a browser sign-in, a revoked grant) are never an agent's to call, on
/// Claude (`--disallowed-tools`) or Codex (`disabled_tools`), whatever the
/// mode; its other tools stay.
#[test]
fn the_plaud_built_ins_sign_in_tools_are_never_given_to_agents() {
    let mut run = spec("bypassPermissions", None, false);
    run.mcp_config = Some(with_servers(&["hollow_plaud", "hollow_vault"]));
    let claude = rendered(&test_command(AgentKind::Claude, &run));
    let deny = claude.find("--disallowed-tools").expect("a deny list");
    assert!(
        claude[deny..].contains("mcp__hollow_plaud__login"),
        "{claude}"
    );
    assert!(
        claude[deny..].contains("mcp__hollow_plaud__logout"),
        "{claude}"
    );
    assert!(!claude.contains("mcp__hollow_vault__login"), "{claude}");
    // The spec's own deny list is kept alongside.
    run.disallowed_tools = vec!["SendUserFile".into()];
    let both = rendered(&test_command(AgentKind::Claude, &run));
    assert_eq!(both.matches("--disallowed-tools").count(), 1, "{both}");
    assert!(
        both.contains("SendUserFile") && both.contains("mcp__hollow_plaud__logout"),
        "{both}"
    );
    // Without Plaud attached nothing is added.
    run.disallowed_tools.clear();
    run.mcp_config = Some(with_servers(&["hollow_vault"]));
    assert!(!rendered(&test_command(AgentKind::Claude, &run)).contains("--disallowed-tools"));

    let codex = codex_owned_servers(&with_servers(&["hollow_plaud"]), "acceptEdits", &[]).unwrap();
    assert!(
        codex.contains(r#"disabled_tools=["login","logout"]"#),
        "{codex}"
    );
    let vault = codex_owned_servers(&with_servers(&["hollow_vault"]), "acceptEdits", &[]).unwrap();
    assert!(!vault.contains("disabled_tools"), "{vault}");
}

#[test]
fn codex_accepts_library_connectors_listed_in_the_spec() {
    let config = r#"{"mcpServers":{"mail":{"type":"http","url":"https://mail.example/mcp","headers":{"Authorization":"Bearer x"}},"crm":{"command":"npx","args":["crm"]},"rogue":{"command":"x"}}}"#;
    let shared = [
        SharedConnector {
            id: "mail".into(),
            auto_approve: true,
            claude_login: false,
            name: "Mail".into(),
            ..Default::default()
        },
        SharedConnector {
            id: "crm".into(),
            auto_approve: false,
            claude_login: false,
            name: "CRM".into(),
            ..Default::default()
        },
    ];
    // Anything not listed is still refused.
    assert!(codex_owned_servers(config, "acceptEdits", &shared).is_err());
    let config = r#"{"mcpServers":{"mail":{"type":"http","url":"https://mail.example/mcp","headers":{"Authorization":"Bearer x"}},"crm":{"command":"npx","args":["crm"]}}}"#;
    let all = super::codex_server_overrides(config, &test_launch(), "acceptEdits", &shared);
    let all = all.unwrap();
    let out = &all.overrides;
    // Merged plan slice 6: the header's value is in Codex's environment,
    // named on argv; the stdio connector starts through `mcp-launch`.
    assert!(out.contains(r#""mail"={url="https://mail.example/mcp",enabled=true,default_tools_approval_mode="approve",env_http_headers={"Authorization"="HOLLOW_MCP_SECRET_HEADER_0"}}"#), "{out}");
    assert!(!out.contains("Bearer"), "{out}");
    let header = (
        "HOLLOW_MCP_SECRET_HEADER_0".to_string(),
        "Bearer x".to_string(),
    );
    assert_eq!(all.env, [header]);
    let crm = format!("\"crm\"={{{},enabled=true}}", launched("crm"));
    assert!(out.contains(&crm), "{out}");
}

#[test]
fn codex_preapproves_only_the_preapproved_owned_servers() {
    let owned = codex_owned_servers(
        &with_servers(&[
            "hollow_vault",
            "hollow_computer",
            "hollow_questions",
            "hollow_team",
        ]),
        "acceptEdits",
        &[],
    )
    .unwrap();
    let entry = |name: &str| {
        let start = owned
            .find(&format!("\"{name}\"={{"))
            .unwrap_or_else(|| panic!("{name}: {owned}"));
        let end = start + owned[start..].find('}').unwrap();
        owned[start..=end].to_string()
    };
    assert!(
        entry("hollow_vault").contains("default_tools_approval_mode=\"approve\""),
        "{owned}"
    );
    assert!(
        entry("hollow_questions").contains("default_tools_approval_mode=\"approve\""),
        "{owned}"
    );
    assert!(
        !entry("hollow_computer").contains("default_tools_approval_mode"),
        "{owned}"
    );
    assert!(
        entry("hollow_team").contains("default_tools_approval_mode=\"approve\""),
        "{owned}"
    );

    // Plan mode (`-s read-only`): teamwork is not approved, so Codex fails
    // a delegation instead of letting a read-only run write through it.
    let plan = codex_owned_servers(
        &with_servers(&["hollow_team", "hollow_questions"]),
        "plan",
        &[],
    )
    .unwrap();
    let team = &plan[plan.find("\"hollow_team\"={").unwrap()..];
    let team = &team[..=team.find('}').unwrap()];
    assert!(!team.contains("default_tools_approval_mode"), "{plan}");
    assert!(
        plan.contains("default_tools_approval_mode=\"approve\""),
        "questions stay approved: {plan}"
    );
}

// Render a built Command's program + args to a string for assertions,
// then the prompt the last build on this thread put on its stdin (Q-R33),
// so a prompt check reads the same whether it rides argv or stdin. Call it
// right after building `cmd`.
fn rendered(cmd: &Command) -> String {
    let std = cmd.as_std();
    let mut parts: Vec<String> = vec![std.get_program().to_string_lossy().into_owned()];
    parts.extend(crate::secret_fds::expand_append(
        std.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
    ));
    parts.extend(take_last_stdin_prompt());
    parts.join(" ")
}

#[test]
fn claude_chat_command_has_partial_messages_and_resume() {
    let cmd = test_command(AgentKind::Claude, &spec("acceptEdits", Some("sid-1"), true));
    let s = rendered(&cmd);
    assert!(s.contains("-p --output-format stream-json"));
    assert!(s.ends_with(" hello"), "the prompt is on stdin: {s}");
    assert!(s.contains("--include-partial-messages"));
    assert!(s.contains("--resume sid-1"));
    assert!(s.contains("--permission-mode acceptEdits"));
}

#[test]
fn claude_workflow_command_disables_session_persistence() {
    let cmd = test_command(AgentKind::Claude, &spec("acceptEdits", None, false));
    let s = rendered(&cmd);
    assert!(s.contains("--no-session-persistence"));
    assert!(!s.contains("--include-partial-messages"));
}

#[test]
fn codex_command_maps_permissions_and_resume() {
    let bypass = rendered(&test_command(
        AgentKind::Codex,
        &spec("bypassPermissions", None, true),
    ));
    assert!(bypass.contains("exec"));
    assert!(bypass.contains("--json"));
    assert!(bypass.contains("--skip-git-repo-check"));
    assert!(bypass.contains("--dangerously-bypass-approvals-and-sandbox"));
    assert!(
        bypass.trim_end().ends_with("hello"),
        "prompt is positional last: {bypass}"
    );

    let plan = rendered(&test_command(AgentKind::Codex, &spec("plan", None, true)));
    assert!(plan.contains("-s read-only"));
    // `codex exec` has no approval flag — sandbox policy alone governs it.
    assert!(!plan.contains("-a never"));

    let edits = rendered(&test_command(
        AgentKind::Codex,
        &spec("acceptEdits", None, true),
    ));
    // N22: the nested-`.git` profile in place of `-s workspace-write`; its
    // `:workspace` base writes the cwd and has no network.
    assert!(
        edits.contains("default_permissions=\"hollow_auto\""),
        "{edits}"
    );
    assert!(edits.contains("network.enabled=false"), "{edits}");
    assert!(!edits.contains("-s "), "{edits}");

    let resumed = rendered(&test_command(
        AgentKind::Codex,
        &spec("acceptEdits", Some("th-9"), true),
    ));
    assert!(resumed.contains("exec resume th-9"));
    // `codex exec resume` rejects -s/--sandbox — it must not leak in.
    assert!(
        !resumed.contains("-s "),
        "resume must not pass a sandbox flag: {resumed}"
    );
    assert!(resumed.contains("--skip-git-repo-check"));

    // bypass on resume re-asserts only the bypass escape hatch (no -s).
    let resumed_bypass = rendered(&test_command(
        AgentKind::Codex,
        &spec("bypassPermissions", Some("th-9"), true),
    ));
    assert!(resumed_bypass.contains("--dangerously-bypass-approvals-and-sandbox"));
    assert!(!resumed_bypass.contains("-s "));
}

/// 2026-10-02 (user decision): auto and bypass mean the same thing in
/// every agent. Auto approves everything through the agent's own
/// automatic policy (Claude's auto mode, Codex's "Approve for me",
/// Grok's auto, Antigravity's own `--mode auto`); bypass runs
/// everything with no prompt. A resumed Codex chat follows a mode switch.
#[test]
fn every_agent_maps_auto_and_bypass_to_its_own_equivalent() {
    let run = |kind, perm: &str, resume: Option<&str>| {
        rendered(&test_command(kind, &spec(perm, resume, true)))
    };

    assert!(run(AgentKind::Claude, "auto", None).contains("--permission-mode auto"));
    assert!(run(AgentKind::Claude, "bypassPermissions", None)
        .contains("--permission-mode bypassPermissions"));

    let auto = run(AgentKind::Codex, "auto", None);
    assert!(
        auto.contains("-c approval_policy=\"on-request\" -c approvals_reviewer=\"auto_review\""),
        "{auto}"
    );
    assert!(
        auto.contains("-s workspace-write") && !auto.contains("--dangerously"),
        "{auto}"
    );
    let edits = run(AgentKind::Codex, "acceptEdits", None);
    assert!(
        !edits.contains("approvals_reviewer"),
        "only auto uses the reviewer: {edits}"
    );
    let resumed_auto = run(AgentKind::Codex, "auto", Some("th-1"));
    assert!(
        resumed_auto.contains("approvals_reviewer=\"auto_review\"")
            && resumed_auto.contains("sandbox_mode=\"workspace-write\""),
        "{resumed_auto}"
    );
    let resumed_plan = run(AgentKind::Codex, "plan", Some("th-1"));
    assert!(
        resumed_plan.contains("sandbox_mode=\"read-only\""),
        "a resumed chat follows the switch to plan: {resumed_plan}"
    );
    let resumed_bypass = run(AgentKind::Codex, "bypassPermissions", Some("th-1"));
    assert!(!resumed_bypass.contains("sandbox_mode"), "{resumed_bypass}");
    let exec = resumed_auto.find(" exec ").expect("exec");
    assert!(
        resumed_auto.find("sandbox_mode").unwrap() < exec,
        "config overrides precede the subcommand: {resumed_auto}"
    );

    // Antigravity: a chat turn runs `agy acp` (`antigravity_acp_runs_chats`),
    // a one-shot `agy -p` with its own `--mode auto`, never the skip flag.
    let agy_oneshot = |perm: &str| {
        rendered(&test_command(
            AgentKind::Antigravity,
            &spec(perm, None, false),
        ))
    };
    let agy_auto = agy_oneshot("auto");
    assert!(
        agy_auto.contains("--sandbox --mode auto")
            && !agy_auto.contains("--dangerously-skip-permissions"),
        "{agy_auto}"
    );
    let agy_bypass = agy_oneshot("bypassPermissions");
    assert!(
        agy_bypass.contains("--dangerously-skip-permissions") && !agy_bypass.contains("--sandbox"),
        "{agy_bypass}"
    );
    assert!(agy_oneshot("acceptEdits").contains("--sandbox --mode accept-edits"));
    assert!(agy_oneshot("plan").contains("--sandbox --mode plan"));

    for perm in [
        "default",
        "acceptEdits",
        "auto",
        "bypassPermissions",
        "plan",
    ] {
        assert!(
            run(AgentKind::Grok, perm, None).contains(&format!("--permission-mode {perm}")),
            "{perm}"
        );
    }
}

/// Auto and bypass preapprove every Hollow-owned server, computer
/// control and services included; other modes keep them behind the
/// mode. Bypass also preapproves every shared connector; auto none
/// ([`auto_attaches_library_connectors_without_preapproving_them`]).
#[test]
fn auto_and_bypass_preapprove_every_owned_server() {
    let config = with_servers(&["hollow_app", "hollow_computer", "hollow_services"]);
    for perm in ["auto", "bypassPermissions"] {
        let tokens = owned_preapprovals(Some(&config), perm);
        assert!(
            tokens.contains(&"mcp__hollow_computer") && tokens.contains(&"mcp__hollow_services"),
            "{perm}: {tokens:?}"
        );
    }
    for perm in ["acceptEdits", "default", "plan"] {
        let tokens = owned_preapprovals(Some(&config), perm);
        assert!(
            !tokens
                .iter()
                .any(|t| t.contains("computer") || t.contains("services")),
            "{perm}: {tokens:?}"
        );
    }
    let codex = codex_owned_servers(
        &with_servers(&["hollow_computer"]),
        "bypassPermissions",
        &[],
    )
    .unwrap();
    assert!(
        codex.contains("default_tools_approval_mode=\"approve\""),
        "Codex bypass runs computer control: {codex}"
    );
    let manual = SharedConnector {
        id: "crm".into(),
        auto_approve: false,
        ..Default::default()
    };
    assert!(connector_preapproved(&manual, "bypassPermissions"));
    assert!(
        !connector_preapproved(&manual, "auto") && !connector_preapproved(&manual, "acceptEdits")
    );
    let trusted = SharedConnector {
        id: "mail".into(),
        auto_approve: true,
        ..Default::default()
    };
    assert!(
        connector_preapproved(&trusted, "acceptEdits")
            && connector_preapproved(&trusted, "default")
    );
    assert!(!connector_preapproved(&trusted, "auto") && !connector_preapproved(&trusted, "plan"));
}

/// Review C (merged plan slice 2): auto attaches a library connector
/// the user marked auto-approve but never preapproves it, so Claude's
/// auto-mode classifier and Codex's automatic reviewer see each call.
/// Bypass still preapproves it; acceptEdits honours the mark.
#[test]
fn auto_attaches_library_connectors_without_preapproving_them() {
    let config = r#"{"mcpServers":{"mail":{"type":"http","url":"https://mail.example/mcp"},"crm":{"command":"npx","args":["crm"]},"hollow_vault":{"command":"/bin/hollow","args":["x"]}}}"#;
    let shared = vec![
        SharedConnector {
            id: "mail".into(),
            auto_approve: true,
            ..Default::default()
        },
        SharedConnector {
            id: "crm".into(),
            auto_approve: true,
            ..Default::default()
        },
    ];
    let claude = |perm: &str| {
        let mut run = spec(perm, None, true);
        run.mcp_config = Some(config.to_string());
        run.extras.shared_connectors = shared.clone();
        rendered(&test_command(AgentKind::Claude, &run))
    };
    let codex = |perm: &str| codex_owned_servers(config, perm, &shared).unwrap();
    let entry = |owned: &str, name: &str| {
        let start = owned
            .find(&format!("\"{name}\"={{"))
            .unwrap_or_else(|| panic!("{name}: {owned}"));
        let end = start + owned[start..].find('}').unwrap();
        owned[start..=end].to_string()
    };

    // Auto: attached (Codex lists them), never preapproved.
    let auto = claude("auto");
    assert!(auto.contains("--permission-mode auto"), "{auto}");
    assert!(
        auto.contains("mcp__hollow_vault"),
        "Hollow's own read-only server stays preapproved: {auto}"
    );
    assert!(
        !auto.contains("mcp__mail") && !auto.contains("mcp__crm"),
        "auto must not preapprove a library connector: {auto}"
    );
    let auto = codex("auto");
    assert!(
        entry(&auto, "hollow_vault").contains("default_tools_approval_mode=\"approve\""),
        "{auto}"
    );
    for name in ["mail", "crm"] {
        let e = entry(&auto, name);
        assert!(
            e.contains("enabled=true"),
            "auto still attaches {name}: {auto}"
        );
        assert!(
            !e.contains("default_tools_approval_mode"),
            "auto must not preapprove {name}: {auto}"
        );
    }

    // Bypass: every library connector preapproved.
    let bypass = claude("bypassPermissions");
    assert!(
        bypass.contains("mcp__mail") && bypass.contains("mcp__crm"),
        "{bypass}"
    );
    let bypass = codex("bypassPermissions");
    for name in ["mail", "crm"] {
        assert!(
            entry(&bypass, name).contains("default_tools_approval_mode=\"approve\""),
            "{name}: {bypass}"
        );
    }

    // acceptEdits: the auto-approve mark still preapproves (a headless
    // run there has no other way to call the tools).
    let edits = claude("acceptEdits");
    assert!(
        edits.contains("mcp__mail") && edits.contains("mcp__crm"),
        "{edits}"
    );
    let edits = codex("acceptEdits");
    assert!(
        entry(&edits, "mail").contains("default_tools_approval_mode=\"approve\""),
        "{edits}"
    );
}

/// 2026-11-20: Antigravity's Auto is back (agy 1.5.13's own `--mode auto`;
/// the probe is in agent.rs's probe comment). Every agent runs every
/// mode, no picker lists a reason, and no mode but bypass ever gets the
/// `--dangerously-skip-permissions` flag, a one-shot or a chat.
#[test]
fn antigravity_runs_auto_natively_and_only_bypass_skips_permissions() {
    for kind in AgentKind::all() {
        for perm in [
            "plan",
            "default",
            "acceptEdits",
            "auto",
            "bypassPermissions",
        ] {
            assert_eq!(unavailable_mode(kind, perm), None, "{kind:?} {perm}");
            assert!(
                validate_run_spec(kind, &spec(perm, None, true)).is_ok(),
                "{kind:?} {perm}"
            );
        }
        assert!(capabilities(kind).unavailable_modes.is_empty(), "{kind:?}");
    }
    for interactive in [true, false] {
        for resume in [None, Some("conversation-one")] {
            for perm in ["auto", " auto ", "plan", "acceptEdits", "default"] {
                let cmd = test_command(AgentKind::Antigravity, &spec(perm, resume, interactive));
                assert!(!is_unavailable_command(&cmd), "{perm:?}");
                let args = args_of(&cmd);
                assert!(
                    !args.iter().any(|a| a == "--dangerously-skip-permissions"),
                    "{perm:?} {interactive} {args:?}"
                );
                assert!(args.iter().any(|a| a == "--sandbox"), "{args:?}");
            }
        }
    }
    let bypass = args_of(&test_command(
        AgentKind::Antigravity,
        &spec("bypassPermissions", None, true),
    ));
    assert!(bypass.iter().any(|a| a == "--dangerously-skip-permissions"));
    assert!(!bypass.iter().any(|a| a == "--sandbox"), "{bypass:?}");
}

/// A chat turn runs over `agy acp`, with the prompt, model, mode and servers
/// kept off argv; a one-shot, a runner turn and a self-hosted one stay on
/// `agy -p`; the environment switch is the way back.
#[test]
fn antigravity_acp_runs_chats_and_keeps_one_shots_headless() {
    let mut chat = spec("acceptEdits", None, true);
    // Without the chat's flag the same spec is headless (every other caller).
    assert!(!antigravity_acp(&chat));
    chat.extras.antigravity_acp = true;
    let cmd = args_of(&test_command(AgentKind::Antigravity, &chat));
    assert_eq!(cmd.first().map(String::as_str), Some("acp"), "{cmd:?}");
    assert!(
        !cmd.iter()
            .any(|a| a == "-p=" || a == "--model" || a == "--mode"),
        "{cmd:?}"
    );
    let oneshot = args_of(&test_command(
        AgentKind::Antigravity,
        &spec("acceptEdits", None, false),
    ));
    assert!(oneshot.iter().any(|a| a == "-p="), "{oneshot:?}");
    assert!(antigravity_acp(&chat));
    let mut oneshot_flagged = spec("acceptEdits", None, false);
    oneshot_flagged.extras.antigravity_acp = true;
    assert!(
        !antigravity_acp(&oneshot_flagged),
        "a one-shot is never a chat turn"
    );
    // The stdin prompt is only for the headless path.
    assert!(stdin_prompt(AgentKind::Antigravity, &chat).is_none());
    assert!(stdin_prompt(AgentKind::Antigravity, &spec("acceptEdits", None, false)).is_some());
    // Modes: agy's own ids, bypass has none (the flag instead).
    for (perm, want) in [
        ("plan", Some("plan")),
        ("default", Some("default")),
        ("acceptEdits", Some("accept-edits")),
        ("auto", Some("auto")),
        ("bypassPermissions", None),
    ] {
        assert_eq!(antigravity_acp_mode(perm), want, "{perm}");
    }
    let start = antigravity_acp_start(&chat);
    assert_eq!(start.mode.as_deref(), Some("accept-edits"));
    assert!(start.ask && start.agent == AgentKind::Antigravity);
    assert!(!antigravity_acp_start(&spec("bypassPermissions", None, true)).ask);
}

#[test]
fn antigravity_command_is_print_mode() {
    let sandboxed = rendered(&test_command(
        AgentKind::Antigravity,
        &spec("acceptEdits", None, true),
    ));
    // The prompt is the stdin line the render appends, after `-p=` (R5).
    assert!(sandboxed.contains("-p= --input-format stream-json"));
    assert!(sandboxed.contains("Hollow is a local"));
    assert!(sandboxed.contains("--sandbox"));
    assert!(sandboxed.contains("--output-format stream-json"));

    let bypass = rendered(&test_command(
        AgentKind::Antigravity,
        &spec("bypassPermissions", Some("x"), true),
    ));
    assert!(bypass.contains("--dangerously-skip-permissions"));
    assert!(bypass.contains("--conversation x"));
    assert!(!bypass.contains("--continue"));
}

#[test]
fn host_only_tools_are_omitted_only_where_they_do_not_exist() {
    // The embedded chat disallows host-only tools (SendUserFile,
    // AskUserQuestion). Only Claude has a --disallowed-tools flag; codex /
    // antigravity must silently ignore the list, not choke on it.
    let mut s = spec("acceptEdits", None, true);
    s.disallowed_tools = vec!["SendUserFile".into(), "AskUserQuestion".into()];

    let claude = rendered(&test_command(AgentKind::Claude, &s));
    assert!(
        claude.contains("--disallowed-tools SendUserFile AskUserQuestion"),
        "claude must forward the disallow list: {claude}"
    );

    let codex = rendered(&test_command(AgentKind::Codex, &s));
    assert!(
        !codex.contains("--disallowed-tools"),
        "codex has no such flag: {codex}"
    );
    let agy = rendered(&test_command(AgentKind::Antigravity, &s));
    assert!(
        !agy.contains("--disallowed-tools"),
        "antigravity has no such flag: {agy}"
    );
}

#[test]
fn every_adapter_receives_environment_context() {
    // Equivalent context reaches every adapter using its native mechanism.
    let mut s = spec("acceptEdits", None, true);
    s.append_system_prompt = Some("Prefer inline HTML.".into());

    let claude = rendered(&test_command(AgentKind::Claude, &s));
    assert!(
        claude.contains("--append-system-prompt Prefer inline HTML."),
        "claude must forward the system-prompt append: {claude}"
    );

    let codex = rendered(&test_command(AgentKind::Codex, &s));
    assert!(
        !codex.contains("--append-system-prompt"),
        "codex has no such flag: {codex}"
    );
    assert!(codex.contains("Prefer inline HTML."));
    let agy = rendered(&test_command(AgentKind::Antigravity, &s));
    assert!(agy.contains("Prefer inline HTML."));
    assert!(
        !agy.contains("--append-system-prompt"),
        "antigravity has no such flag: {agy}"
    );

    // Empty/None → flag omitted entirely.
    let none = rendered(&test_command(
        AgentKind::Claude,
        &spec("acceptEdits", None, true),
    ));
    assert!(
        none.contains("--append-system-prompt Hollow is a local"),
        "workspace boundary is always present: {none}"
    );
}

#[test]
fn parse_codex_agent_message_yields_turn_and_text() {
    let mut st = ParseState::default();
    let line =
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hi"}}"#;
    let evts = parse_codex(line, &mut st);
    assert!(matches!(evts[0], AgentEvent::TurnBoundary));
    match &evts[1] {
        AgentEvent::AssistantText(t) => assert_eq!(t, "hi"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn parse_codex_command_execution_pairs_tooluse_and_result() {
    let mut st = ParseState::default();
    let started = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"echo hi","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
    let started_evts = parse_codex(started, &mut st);
    match &started_evts[0] {
        AgentEvent::ToolUse { id, name, .. } => {
            assert_eq!(id, "item_1");
            assert_eq!(name, "Bash");
        }
        other => panic!("expected tool use, got {other:?}"),
    }
    let done = r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"echo hi","aggregated_output":"hi\n","exit_code":0,"status":"completed"}}"#;
    let done_evts = parse_codex(done, &mut st);
    match &done_evts[0] {
        AgentEvent::ToolResult {
            id,
            content,
            is_error,
        } => {
            assert_eq!(id, "item_1");
            assert_eq!(content, "hi\n");
            assert!(!is_error);
        }
        other => panic!("expected tool result, got {other:?}"),
    }
}

/// OF18 (red first): a Codex command's result carried its output alone, so
/// a failure that printed nothing (a redirect, a quiet predicate) read as a
/// blank error, and `rg` finding nothing was a failure.
#[test]
fn parse_codex_command_results_carry_the_exit_code_and_no_match_is_not_an_error() {
    let result = |command: &str, output: &str, exit: &str| {
        let line = json!({
            "type": "item.completed",
            "item": {
                "id": "item_9",
                "type": "command_execution",
                "command": command,
                "aggregated_output": output,
                "exit_code": serde_json::from_str::<Value>(exit).unwrap(),
                "status": "completed",
            },
        })
        .to_string();
        match parse_codex(&line, &mut ParseState::default()).remove(0) {
            AgentEvent::ToolResult {
                content, is_error, ..
            } => (content, is_error),
            other => panic!("expected tool result, got {other:?}"),
        }
    };
    assert_eq!(
        result("/bin/zsh -lc 'npm run build > /tmp/b.log 2>&1'", "", "1"),
        ("Exit code 1 (no output)".to_string(), true)
    );
    assert_eq!(
        result("/bin/zsh -lc 'cargo test'", "error[E0425]: x\n", "2"),
        ("Exit code 2\nerror[E0425]: x\n".to_string(), true)
    );
    // A search or comparison that ends the script and finds nothing.
    for command in [
        "rg foo src",
        "/bin/zsh -lc 'rg foo src'",
        "bash -lc 'cat x | grep foo'",
        "/bin/bash -lc \"cd src; /usr/bin/grep -rn foo .\"",
        "zsh -lc '[ -f x ]'",
    ] {
        let (content, is_error) = result(command, "", "1");
        assert!(!is_error, "{command}");
        assert_eq!(content, "Exit code 1 (no match)", "{command}");
    }
    let (content, is_error) = result("diff a b", "1c1\n< x\n---\n> y\n", "1");
    assert!(!is_error);
    assert_eq!(content, "Exit code 1 (no match)\n1c1\n< x\n---\n> y\n");
    // Not a no-match: another exit, a predicate that doesn't end the
    // script, too much output, or another command.
    assert!(result("rg foo src", "rg: src: No such file or directory\n", "2").1);
    assert!(result("bash -lc 'grep foo x | wc -l'", "", "1").1);
    assert!(result("rg foo src", &"x".repeat(201), "1").1);
    assert!(result("npx vitest run", "", "1").1);
    // A clean exit, or none reported, is unchanged.
    assert_eq!(
        result("rg foo src", "src/a.rs:1:foo\n", "0"),
        ("src/a.rs:1:foo\n".to_string(), false)
    );
    assert_eq!(
        result("echo hi", "hi\n", "null"),
        ("hi\n".to_string(), false)
    );
}

/// OF18 review (red first): the no-match rule only asked which command was
/// last, not whether its exit was the script's. `npx vitest run > v.log
/// 2>&1 && grep -q passed v.log` with vitest failing never runs the `grep`,
/// prints nothing, and exits 1: it read as a no-match, and the receipt then
/// passed the vitest check. A predicate's exit 1 is a no-match only when
/// nothing else can have made it: not after `&&`, no exit-taker before it,
/// no errexit or pipefail in the script.
#[test]
fn codex_no_match_needs_the_predicates_exit_to_be_the_scripts() {
    let no_match = |command: &str, output: &str| {
        let (content, is_error) = codex_command_result(command, output, Some(1));
        assert!(content.starts_with("Exit code 1"), "{command}: {content}");
        !is_error
    };
    // The output bound is inclusive: 200 bytes is a no-match, 201 an error.
    assert!(no_match("rg foo src", &"x".repeat(200)));
    assert!(!no_match("rg foo src", &"x".repeat(201)));
    for command in [
        "/bin/zsh -lc 'npx vitest run > /tmp/v.log 2>&1 && grep -q passed /tmp/v.log'",
        "bash -lc 'npm run build >/tmp/b.log 2>&1 && test -f dist/index.html'",
        "zsh -lc 'cd /nope && rg foo'",
        "a && grep foo x",
        "a && cat x | grep foo",
        "zsh -lc 'test -f x || exit 1\ngrep foo x'",
        "zsh -lc '[ -d node_modules ] || exit 1\nrg foo src'",
        "zsh -lc 'set -e\nnpm run build >/tmp/l 2>&1\ngrep x /tmp/l'",
        "bash -lc 'set -e; npm run build >/tmp/b.log 2>&1; grep err /tmp/b.log'",
        "bash -lc 'set -euo pipefail; cat x | grep foo'",
        "bash -lc 'set -o errexit; a; grep foo x'",
        "zsh -lc 'setopt ERR_EXIT; a; grep foo x'",
        "zsh -lc 'set -o pipefail; npx vitest run 2>&1 | grep -c FAIL'",
        "zsh -lc 'setopt pipefail; npx vitest run 2>&1 | grep -c FAIL'",
        "rg foo src &",
    ] {
        assert!(!no_match(command, ""), "{command}");
    }
    // OF18 re-review: an exit-taker or errexit behind a shell keyword, a
    // sourced or eval'd script, a negated pipeline, a nested `sh -c`.
    for command in [
        "zsh -lc 'if ! npx vitest run >/tmp/v.log 2>&1; then exit 1; fi; grep -n Tests /tmp/v.log'",
        "zsh -lc 'if [ -f x ]; then :; else exit 1; fi; grep foo x'",
        "zsh -lc 'for t in a b; do exit 1; done; rg foo'",
        "bash -lc '[ -d x ] || { exit 1; }; grep foo x'",
        "bash -lc 'if true; then set -e; fi; a; grep foo x'",
        "bash -lc '{ set -e; }; a; grep foo x'",
        "zsh -lc 'function f { exit 1; }; f; grep foo x'",
        "bash -lc 'source .env; grep foo x'",
        "bash -lc '. ./env.sh; grep foo x'",
        "bash -lc 'eval \"$CHECK\"; grep foo x'",
        "! cat x | grep -q error",
        "zsh -lc '! cat x | grep -q error'",
        "zsh -lc '! grep -q error x'",
        "zsh -lc \"sh -c 'test -d node_modules && npm run build >/tmp/b.log 2>&1'\"",
        "bash -lc \"/bin/bash -lc 'grep -q x y && npm run build >/tmp/b.log 2>&1'\"",
    ] {
        assert!(!no_match(command, ""), "{command}");
    }
    // `;`, `||` and a plain pipe leave the predicate's status the script's.
    for command in [
        "bash -lc 'cat x | grep foo'",
        "zsh -lc 'cd src; rg foo'",
        "zsh -lc 'cd src\nrg foo'",
        "zsh -lc 'test -f x || grep foo y'",
        "zsh -lc 'set -x; rg foo'",
        "zsh -lc 'cd src; grep -rn foo .'",
    ] {
        assert!(no_match(command, ""), "{command}");
    }
}

#[test]
fn parse_codex_session_and_turn_completed() {
    let mut st = ParseState::default();
    let started = r#"{"type":"thread.started","thread_id":"abc-123"}"#;
    match &parse_codex(started, &mut st)[0] {
        AgentEvent::Session(id) => assert_eq!(id, "abc-123"),
        other => panic!("expected session, got {other:?}"),
    }
    let done = r#"{"type":"turn.completed","usage":{"output_tokens":25}}"#;
    match &parse_codex(done, &mut st)[0] {
        AgentEvent::Result {
            is_error, usage, ..
        } => {
            assert!(!is_error);
            // Input missing means no data, never zeros.
            assert_eq!(usage.tokens, None);
            assert_eq!(usage.cost_usd, None);
        }
        other => panic!("expected result, got {other:?}"),
    }
}

fn fixture_events(kind: AgentKind, text: &str) -> Vec<AgentEvent> {
    let mut state = ParseState::default();
    text.lines()
        .flat_map(|line| parse_line(kind, line, &mut state))
        .collect()
}

fn message_usages(evts: &[AgentEvent]) -> Vec<(String, Option<String>, bool)> {
    evts.iter()
        .filter_map(|evt| match evt {
            AgentEvent::MessageUsage {
                message_id,
                model,
                is_final,
                ..
            } => Some((message_id.clone(), model.clone(), *is_final)),
            _ => None,
        })
        .collect()
}

#[test]
fn parse_claude_fixture_emits_message_usage_and_result_usage() {
    use crate::usage::tests::{CHAT_TOOL, ONESHOT};
    // Non-interactive: one assistant snapshot, then the result.
    let oneshot = fixture_events(AgentKind::Claude, ONESHOT);
    let opus = Some("claude-opus-5-5".to_string());
    assert_eq!(
        message_usages(&oneshot),
        vec![("msg-006".to_string(), opus.clone(), false)]
    );
    let results: Vec<_> = oneshot
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::Result { usage, .. } => Some(usage),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1);
    assert!(results[0].tokens.is_some());

    // Chat (partial messages): message_start snapshot, assistant snapshot,
    // message_delta final, per message; the final keeps the id and model
    // tracked from message_start.
    let chat = fixture_events(AgentKind::Claude, CHAT_TOOL);
    assert_eq!(
        message_usages(&chat),
        vec![
            ("msg-003".to_string(), opus.clone(), false),
            ("msg-003".to_string(), opus.clone(), false),
            ("msg-003".to_string(), opus.clone(), true),
            ("msg-004".to_string(), opus.clone(), false),
            ("msg-004".to_string(), opus.clone(), false),
            ("msg-004".to_string(), opus, true),
        ]
    );
    let finals: Vec<u64> = chat
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::MessageUsage {
                tokens,
                is_final: true,
                ..
            } => Some(tokens.output),
            _ => None,
        })
        .collect();
    assert_eq!(
        finals,
        vec![74, 3],
        "the final output, not the snapshot's 16"
    );
}

/// OB05: a message carries its `parent_tool_use_id` as `thread` (`None` on
/// the main thread) from `message_start`, the `assistant` snapshot and the
/// `message_delta` final, and its cache writes apart from `tokens.input`.
#[test]
fn parse_claude_message_usage_carries_the_subagent_thread() {
    use crate::usage::tests::SUBAGENT;
    let evts = fixture_events(AgentKind::Claude, SUBAGENT);
    let threads: Vec<(String, Option<String>, u64)> = evts
        .into_iter()
        .filter_map(|evt| match evt {
            AgentEvent::MessageUsage {
                message_id,
                thread,
                cache_write,
                ..
            } => Some((message_id, thread, cache_write)),
            _ => None,
        })
        .collect();
    let main = |id: &str, writes: u64| (id.to_string(), None::<String>, writes);
    let sub = |id: &str, writes: u64| (id.to_string(), Some("toolu-task-1".to_string()), writes);
    assert_eq!(
        threads,
        vec![
            main("msg-sa-1", 1_000),
            main("msg-sa-1", 1_000),
            main("msg-sa-1", 1_000),
            sub("msg-sub-1", 5_000),
            sub("msg-sub-1", 5_000),
            sub("msg-sub-1", 5_000),
            sub("msg-sub-2", 200),
            sub("msg-sub-2", 200),
            sub("msg-sub-2", 200),
            main("msg-sa-2", 400),
            main("msg-sa-2", 400),
            main("msg-sa-2", 400),
        ]
    );
}

#[test]
fn parse_claude_reads_rate_limit_windows() {
    use crate::claude_quota::{StreamQuota, StreamWindow};
    use crate::usage::tests::{CHAT_RESUME, CHAT_TOOL, MAX_BUDGET, ONESHOT};
    // Every recorded Claude run printed the same single rate_limit_event.
    let expected = StreamQuota {
        five_hour: Some(StreamWindow {
            utilization: 1.01,
            resets_at: Some(1_790_374_200),
        }),
        seven_day: Some(StreamWindow {
            utilization: 0.3,
            resets_at: Some(1_790_665_200),
        }),
        status: Some("rejected".to_string()),
        using_overage: Some(true),
    };
    for (name, text) in [
        ("oneshot", ONESHOT),
        ("chat-tool", CHAT_TOOL),
        ("chat-resume", CHAT_RESUME),
        ("max-budget", MAX_BUDGET),
    ] {
        let events = fixture_events(AgentKind::Claude, text);
        let quotas: Vec<&StreamQuota> = events
            .iter()
            .filter_map(|evt| match evt {
                AgentEvent::RateLimit(q) => Some(q),
                _ => None,
            })
            .collect();
        assert_eq!(quotas, vec![&expected], "{name}");

        // Nothing else changes: the other events equal a parse of the
        // same run with its rate_limit_event line removed.
        let others: Vec<String> = events
            .iter()
            .filter(|evt| !matches!(evt, AgentEvent::RateLimit(_)))
            .map(|evt| format!("{evt:?}"))
            .collect();
        let without: String = text
            .lines()
            .filter(|line| !line.contains("\"type\":\"rate_limit_event\""))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(without.lines().count() + 1, text.lines().count(), "{name}");
        let baseline: Vec<String> = fixture_events(AgentKind::Claude, &without)
            .iter()
            .map(|evt| format!("{evt:?}"))
            .collect();
        assert_eq!(others, baseline, "{name}");
    }
    // Codex never produces one.
    let mut st = ParseState::default();
    let line = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.5}}}}"#;
    assert!(parse_codex(line, &mut st).is_empty());
}

// ── collect_command: one usage row on every ending, with real processes.

#[cfg(unix)]
fn fixture_path(name: &str) -> String {
    format!("{}/fixtures/usage/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[cfg(unix)]
fn test_ledger(vault: &std::path::Path, kind: AgentKind) -> crate::usage_ledger::UsageRun {
    crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            vault,
            crate::usage_ledger::Surface::Oneshot,
            Some("test".into()),
            None,
        ),
        Some(kind),
        None,
    )
}

#[cfg(unix)]
fn only_row(vault: &std::path::Path) -> crate::session_store::UsageRow {
    let rows = crate::session_store::usage_rows_since(vault, "").unwrap();
    assert_eq!(rows.len(), 1, "exactly one row: {rows:?}");
    rows.into_iter().next().unwrap()
}

/// A stream cut off before its `result` line, written to a file.
#[cfg(unix)]
fn killed_fixture(dir: &std::path::Path, text: &str) -> String {
    let path = dir.join("killed.jsonl");
    std::fs::write(&path, crate::usage::tests::before_result(text)).unwrap();
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
#[tokio::test]
async fn collect_command_records_one_complete_row_on_success() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_path("claude-oneshot.jsonl");
    let cmd = isolated_shell("cat \"$F\"", &[("F", fixture.as_str())]);
    let mut ledger = test_ledger(dir.path(), AgentKind::Claude);
    ledger.set_billing(crate::usage::Billing::Subscription);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        ledger,
    )
    .await;
    assert_eq!(collected.result.as_deref(), Ok("ok"));
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "success");
    assert!(row.spawned);
    assert_eq!(row.cost_status, "estimated");
    assert_eq!(row.basis, "per_run");
    assert_eq!(
        (row.input_tokens, row.cached_input_tokens, row.output_tokens),
        (Some(20_390), Some(10_704), Some(12))
    );
    assert!((row.cost_usd.unwrap() - 0.1590798).abs() < 1e-9);
    assert_eq!(row.model.as_deref(), Some("claude-opus-5-5"));
    let usage = collected.usage.expect("the runner reports its row");
    assert_eq!(usage.cost_status, crate::usage::CostStatus::Estimated);
    assert_eq!(usage.tokens.map(|t| t.input), Some(20_390));
}

#[cfg(unix)]
#[tokio::test]
async fn collect_command_records_partial_on_timeout() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    // Chat's stream (partial messages): the message_delta finals count.
    let killed = killed_fixture(dir.path(), crate::usage::tests::CHAT_TOOL);
    let cmd = isolated_shell("cat \"$K\"; exec /bin/sleep 30", &[("K", killed.as_str())]);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_millis(300)),
        None,
        test_ledger(dir.path(), AgentKind::Claude),
    )
    .await;
    assert!(collected.timed_out);
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "timeout");
    assert_eq!(row.cost_status, "partial");
    assert_eq!(
        (row.input_tokens, row.cached_input_tokens, row.output_tokens),
        (Some(18_119), Some(42_412), Some(77))
    );
    assert_eq!(row.cost_usd, None);
}

#[cfg(unix)]
#[tokio::test]
async fn collect_command_records_cancelled_partial_on_stop() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    // A non-interactive stream: only the start-of-message snapshot exists
    // mid-run, so a killed run's output undercounts (a known limit).
    let killed = killed_fixture(dir.path(), crate::usage::tests::ONESHOT);
    let cmd = isolated_shell("cat \"$K\"; exec /bin/sleep 30", &[("K", killed.as_str())]);
    let stop = crate::process_group::StopSignal::new();
    let (collected, _) = tokio::join!(
        collect_command(
            AgentKind::Claude,
            cmd,
            ParseState::default(),
            Some(Duration::from_secs(20)),
            Some(&stop),
            test_ledger(dir.path(), AgentKind::Claude),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stop.request();
        }
    );
    assert!(collected.stopped);
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "cancelled");
    assert_eq!(row.cost_status, "partial");
    assert_eq!(row.output_tokens, Some(4));
}

/// OF04: a run its deadline cut off keeps what it wrote, whichever CLI's
/// stream it was: every Claude text block in order, a Codex agent message,
/// Antigravity's deltas. The result is still the timeout's `Err`, so a
/// caller that never asks for the partial text sees a failure as before; a
/// run that exits on its own keeps none.
#[cfg(unix)]
#[tokio::test]
async fn collect_command_keeps_the_text_a_timed_out_run_wrote() {
    use crate::process_group::test_support::isolated_shell;
    let claude = [
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reading a.rs."}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a.rs calls b."}]}}"#,
    ]
    .join("\n");
    let codex = r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Codex so far."}}"#;
    let agy = [
        r#"{"event":"step_update","step_update":{"step_type":"agent_response","text_delta":"Gemini "}}"#,
        r#"{"event":"step_update","step_update":{"step_type":"agent_response","text_delta":"so far."}}"#,
    ]
    .join("\n");
    for (kind, lines, expected) in [
        (
            AgentKind::Claude,
            claude.as_str(),
            "Reading a.rs.\n\na.rs calls b.",
        ),
        (AgentKind::Codex, codex, "Codex so far."),
        (AgentKind::Antigravity, agy.as_str(), "Gemini so far."),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let cmd = isolated_shell("printf '%s\\n' \"$L\"; exec /bin/sleep 30", &[("L", lines)]);
        let collected = collect_command(
            kind,
            cmd,
            ParseState::default(),
            Some(Duration::from_millis(500)),
            None,
            test_ledger(dir.path(), kind),
        )
        .await;
        assert!(collected.timed_out, "{kind:?}");
        assert!(collected.result.is_err(), "{kind:?}: still a failure");
        assert_eq!(collected.partial_text, expected, "{kind:?}");
    }

    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_path("claude-oneshot.jsonl");
    let cmd = isolated_shell("cat \"$F\"", &[("F", fixture.as_str())]);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        test_ledger(dir.path(), AgentKind::Claude),
    )
    .await;
    assert_eq!(collected.result.as_deref(), Ok("ok"));
    assert_eq!(collected.partial_text, "", "a finished run has no partial");
}

/// OF04: error text is never a cut-off run's answer. A Claude run that wrote
/// prose, then an API-error block and an error result, then hung keeps only
/// the prose; a Codex run whose only output was an error keeps nothing.
#[cfg(unix)]
#[tokio::test]
async fn collect_command_keeps_no_error_text_in_a_timed_out_runs_partial() {
    use crate::process_group::test_support::isolated_shell;
    let claude = [
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reading a.rs."}]}}"#,
        r#"{"type":"assistant","is_api_error_message":true,"api_error_status":529,"message":{"content":[{"type":"text","text":"API Error: overloaded"}]}}"#,
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"API Error: overloaded"}"#,
    ]
    .join("\n");
    let codex = r#"{"type":"error","message":"stream disconnected"}"#;
    for (kind, lines, expected) in [
        (AgentKind::Claude, claude.as_str(), "Reading a.rs."),
        (AgentKind::Codex, codex, ""),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let cmd = isolated_shell("printf '%s\\n' \"$L\"; exec /bin/sleep 30", &[("L", lines)]);
        let collected = collect_command(
            kind,
            cmd,
            ParseState::default(),
            Some(Duration::from_millis(500)),
            None,
            test_ledger(dir.path(), kind),
        )
        .await;
        assert!(collected.timed_out, "{kind:?}");
        assert_eq!(collected.partial_text, expected, "{kind:?}");
        assert!(
            !collected.partial_text.contains("overloaded")
                && !collected.partial_text.contains("disconnected"),
            "{kind:?}: no error text"
        );
    }
}

/// OF04: streamed deltas from separate messages are separated by a blank
/// line rather than run together, in a cut-off run's partial text (Grok's
/// `message_start`, Antigravity's step index) and in a finished run's answer;
/// no separator trails the text when a boundary has nothing after it.
#[cfg(unix)]
#[tokio::test]
async fn collect_command_separates_streamed_messages_with_a_blank_line() {
    use crate::process_group::test_support::isolated_shell;
    let grok = [
        r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"First "}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"part."}}}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Second part."}}}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
    ]
    .join("\n");
    let agy = [
        r#"{"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","text_delta":"First "}}"#,
        r#"{"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","text_delta":"part."}}"#,
        r#"{"event":"step_update","step_update":{"step_index":3,"step_type":"agent_response","text_delta":"Second part."}}"#,
    ]
    .join("\n");
    let expected = "First part.\n\nSecond part.";
    for (kind, lines) in [
        (AgentKind::Grok, grok.as_str()),
        (AgentKind::Antigravity, agy.as_str()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let cmd = isolated_shell("printf '%s\\n' \"$L\"; exec /bin/sleep 30", &[("L", lines)]);
        let collected = collect_command(
            kind,
            cmd,
            ParseState::default(),
            Some(Duration::from_millis(500)),
            None,
            test_ledger(dir.path(), kind),
        )
        .await;
        assert!(collected.timed_out, "{kind:?}");
        assert_eq!(collected.partial_text, expected, "{kind:?}");
    }

    let dir = tempfile::tempdir().unwrap();
    let cmd = isolated_shell("printf '%s\\n' \"$L\"", &[("L", agy.as_str())]);
    let collected = collect_command(
        AgentKind::Antigravity,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        test_ledger(dir.path(), AgentKind::Antigravity),
    )
    .await;
    assert_eq!(collected.result.as_deref(), Ok(expected), "a finished run");
}

#[cfg(unix)]
#[tokio::test]
async fn collect_command_records_unknown_not_zero_when_nothing_is_reported() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let collected = collect_command(
        AgentKind::Claude,
        isolated_shell("exit 3", &[]),
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        test_ledger(dir.path(), AgentKind::Claude),
    )
    .await;
    assert!(collected.result.is_err());
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "error");
    assert!(row.spawned);
    assert_eq!(
        (
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
            row.cost_usd
        ),
        (None, None, None, None)
    );
    assert_eq!(row.cost_status, "unknown");
}

#[cfg(unix)]
#[tokio::test]
async fn collect_command_records_spawn_failure_and_the_sentinel_as_not_started() {
    let dir = tempfile::tempdir().unwrap();
    let collected = collect_command(
        AgentKind::Claude,
        Command::new("/nonexistent/hollow-usage-test"),
        ParseState::default(),
        Some(Duration::from_secs(5)),
        None,
        test_ledger(dir.path(), AgentKind::Claude),
    )
    .await;
    assert!(collected.result.is_err());
    let row = only_row(dir.path());
    assert_eq!((row.outcome.as_str(), row.spawned), ("spawn_failed", false));
    assert_eq!(row.cost_status, "unknown");

    // The stand-in is spawned (its stderr is the user-visible error), but
    // it is not an agent.
    let other = tempfile::tempdir().unwrap();
    let mut stand_in = failed_command(&spec("acceptEdits", None, false), "unsupported");
    crate::process_group::isolate(&mut stand_in);
    let collected = collect_command(
        AgentKind::Codex,
        stand_in,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        test_ledger(other.path(), AgentKind::Codex),
    )
    .await;
    assert!(collected.result.unwrap_err().contains("unsupported"));
    let row = only_row(other.path());
    assert_eq!((row.outcome.as_str(), row.spawned), ("spawn_failed", false));
}

// ── Paperclip PR 8b: budget metering ────────────────────────────────

// PR 8 plan test 3: usage-only partial messages add each message's final
// `message_delta` usage without changing the text a run collects.
#[test]
fn usage_only_mode_reads_message_delta_output_without_changing_text_events() {
    use crate::usage::tests::CHAT_TOOL;
    let metered = RunSpec {
        usage_stream: true,
        ..spec("acceptEdits", None, false)
    };
    let mut state = ParseState::for_spec(AgentKind::Claude, &metered);
    let evts: Vec<AgentEvent> = CHAT_TOOL
        .lines()
        .flat_map(|line| parse_line(AgentKind::Claude, line, &mut state))
        .collect();
    let finals: Vec<(String, u64)> = evts
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::MessageUsage {
                message_id,
                tokens,
                is_final: true,
                ..
            } => Some((message_id.clone(), tokens.output)),
            _ => None,
        })
        .collect();
    assert_eq!(finals, vec![("msg-003".into(), 74), ("msg-004".into(), 3)]);
    assert!(!evts
        .iter()
        .any(|e| matches!(e, AgentEvent::AssistantDelta(_))));
    assert!(!evts.iter().any(|e| matches!(e, AgentEvent::TurnBoundary)));
    let texts: Vec<&str> = evts
        .iter()
        .filter_map(|e| match e {
            AgentEvent::AssistantText(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts.len(),
        1,
        "msg-004's assistant text block still arrives"
    );

    // Chat (interactive) keeps its deltas and turn boundaries.
    let chat = spec("acceptEdits", None, true);
    let mut state = ParseState::for_spec(
        AgentKind::Claude,
        &RunSpec {
            usage_stream: true,
            ..chat
        },
    );
    let evts: Vec<AgentEvent> = CHAT_TOOL
        .lines()
        .flat_map(|line| parse_line(AgentKind::Claude, line, &mut state))
        .collect();
    assert!(evts
        .iter()
        .any(|e| matches!(e, AgentEvent::AssistantDelta(_))));
    assert!(evts.iter().any(|e| matches!(e, AgentEvent::TurnBoundary)));
    // Other agents never parse in usage-only mode.
    let codex = ParseState::for_spec(AgentKind::Codex, &metered);
    assert!(!codex.usage_only_partials);
}

// PR 8 plan test 5: the flags appear only on a fresh non-interactive
// Claude run, and every other combination is refused before spawn.
#[test]
fn claude_usage_stream_and_backstop_flags_only_for_fresh_non_interactive_runs() {
    let metered = RunSpec {
        usage_stream: true,
        max_budget_usd: Some(0.5),
        ..spec("acceptEdits", None, false)
    };
    let s = rendered(&test_command(AgentKind::Claude, &metered));
    assert!(s.contains("--no-session-persistence"));
    assert_eq!(s.matches("--include-partial-messages").count(), 1);
    assert!(s.contains("--max-budget-usd 0.50"), "{s}");
    // A backstop below a cent is floored at one.
    let tiny = RunSpec {
        max_budget_usd: Some(0.001),
        ..spec("acceptEdits", None, false)
    };
    assert!(rendered(&test_command(AgentKind::Claude, &tiny)).contains("--max-budget-usd 0.01"));
    // Chat already streams partials once and never gets a backstop.
    let chat = RunSpec {
        usage_stream: true,
        ..spec("acceptEdits", Some("sid"), true)
    };
    let s = rendered(&test_command(AgentKind::Claude, &chat));
    assert_eq!(s.matches("--include-partial-messages").count(), 1);
    assert!(!s.contains("--max-budget-usd"));
    // Unmetered specs are unchanged.
    let plain = rendered(&test_command(
        AgentKind::Claude,
        &spec("acceptEdits", None, false),
    ));
    assert!(!plain.contains("--include-partial-messages") && !plain.contains("--max-budget-usd"));

    assert!(validate_run_spec(AgentKind::Claude, &metered).is_ok());
    for kind in [AgentKind::Codex, AgentKind::Antigravity] {
        let err = validate_run_spec(
            kind,
            &RunSpec {
                usage_stream: true,
                ..spec("acceptEdits", None, false)
            },
        )
        .unwrap_err();
        assert!(err.contains("budget metering"), "{err}");
        assert!(validate_run_spec(
            kind,
            &RunSpec {
                max_budget_usd: Some(1.0),
                ..spec("acceptEdits", None, false)
            }
        )
        .is_err());
    }
    let resumed = RunSpec {
        max_budget_usd: Some(1.0),
        ..spec("acceptEdits", Some("sid"), false)
    };
    assert!(
        validate_run_spec(AgentKind::Claude, &resumed).is_err(),
        "never with a resume id"
    );
    let interactive = RunSpec {
        max_budget_usd: Some(1.0),
        ..spec("acceptEdits", None, true)
    };
    assert!(validate_run_spec(AgentKind::Claude, &interactive).is_err());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let spec = RunSpec {
            max_budget_usd: Some(bad),
            ..spec("acceptEdits", None, false)
        };
        assert!(
            validate_run_spec(AgentKind::Claude, &spec).is_err(),
            "{bad}"
        );
    }
}

#[cfg(unix)]
fn budgeted_ledger(
    vault: &std::path::Path,
    budget: crate::budgets::RunBudget,
) -> crate::usage_ledger::UsageRun {
    crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            vault,
            crate::usage_ledger::Surface::Workflow,
            Some("brief".into()),
            None,
        )
        .with_budget(budget),
        Some(AgentKind::Claude),
        None,
    )
}

/// The chat-tool capture up to msg-003's `message_delta`, then a sleep:
/// the guard must end the run, not its own exit.
#[cfg(unix)]
fn cut_after_first_delta(dir: &std::path::Path) -> String {
    let path = dir.join("first-delta.jsonl");
    let head: String = crate::usage::tests::CHAT_TOOL
        .lines()
        .take(19)
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(head.contains("message_delta"));
    std::fs::write(&path, head).unwrap();
    path.to_string_lossy().into_owned()
}

// PR 8 plan tests 6, 11 and 13(1): an enforced token limit crossed
// mid-run ends the run's WHOLE process tree (the forked grandchild too,
// verified) as a budget stop and records ONE partial row with what
// streamed (msg-003's final, 18,012 input + 74 output; its start snapshot
// of 16 output did not cross 18,050).
#[cfg(unix)]
#[tokio::test]
async fn collect_command_stops_at_an_enforced_token_limit_with_one_partial_row() {
    use crate::process_group::test_support::{isolated_shell, wait_gone};
    let dir = tempfile::tempdir().unwrap();
    let fixture = cut_after_first_delta(dir.path());
    let pid_file = dir.path().join("grandchild.pid");
    let pid_path = pid_file.to_string_lossy().into_owned();
    let cmd = isolated_shell(
        "/bin/sleep 60 & echo $! > \"$P\"; cat \"$F\"; wait",
        &[("F", fixture.as_str()), ("P", pid_path.as_str())],
    );
    let budget = crate::budgets::token_budget_for_test(
        crate::budgets::RunIntent::workflow("brief", Some(AgentKind::Claude)),
        18_050.0,
        0.0,
        true,
    );
    // The cause is asserted below (no timeout, a budget stop, a verified
    // tree stop), not the clock: a stop runs on production `Timings`
    // (grace, reap, drains) that stretch under two-slot load. The
    // deadline is generous and `elapsed` is only a no-hang guard.
    let deadline = Duration::from_secs(120);
    let started = std::time::Instant::now();
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(deadline),
        None,
        budgeted_ledger(dir.path(), budget),
    )
    .await;
    assert!(started.elapsed() < deadline, "no hang");
    assert!(
        !collected.stopped && !collected.timed_out,
        "the guard ended it, not the deadline"
    );
    assert!(collected.budget_stop.is_some(), "a budget stop ended it");
    assert_eq!(
        collected.outcome,
        Some(process_group::StopOutcome::Verified),
        "the tree was ended"
    );
    let grandchild: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_gone(grandchild, Duration::from_secs(3)).await,
        "the grandchild went with it"
    );
    let notice = collected.budget_stop.expect("a budget stop");
    assert!(
        notice
            .message
            .starts_with(crate::budgets::BUDGET_STOP_PREFIX),
        "{}",
        notice.message
    );
    assert_eq!(collected.result.unwrap_err(), notice.message);
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "budget_stopped");
    assert_eq!(row.cost_status, "partial");
    assert_eq!(
        (row.input_tokens, row.output_tokens),
        (Some(18_012), Some(74))
    );
    assert_eq!(row.cost_usd, None);
    assert_eq!(
        collected.usage.map(|u| u.outcome),
        Some(crate::usage::LedgerOutcome::BudgetStopped)
    );
}

// PR 8 plan test 15: a delegate under chat is only warned, never killed.
#[cfg(unix)]
#[tokio::test]
async fn collect_command_never_stops_a_warn_only_run() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_path("claude-oneshot.jsonl");
    let cmd = isolated_shell("cat \"$F\"", &[("F", fixture.as_str())]);
    let under_chat = crate::budgets::RunIntent::delegate(
        Some(AgentKind::Claude),
        crate::budgets::ParentRun {
            surface: Some(crate::usage_ledger::Surface::Chat),
            scope: Some("s-1".into()),
        },
    );
    let budget = crate::budgets::token_budget_for_test(under_chat, 100.0, 0.0, true);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        budgeted_ledger(dir.path(), budget),
    )
    .await;
    assert_eq!(collected.result.as_deref(), Ok("ok"));
    assert!(collected.budget_stop.is_none());
    // The limit it crossed is reported once, never as a stop.
    assert_eq!(
        collected.budget_warnings.len(),
        1,
        "{:?}",
        collected.budget_warnings
    );
    assert_eq!(
        collected.budget_warnings[0].level,
        crate::budgets::NoticeLevel::Limit
    );
    assert!(!collected.budget_warnings[0].blocking);
    assert_eq!(only_row(dir.path()).outcome, "success");
}

// PR 8 plan test 14: a delegate (an inherited, untagged spawn inside
// team-mcp) under an automation loses only its direct process. It joins
// a parent leader P's process group, as a delegate shares its server's
// group; afterwards the delegate is gone and P still lives, because
// nothing signals a process group for an inherited run.
#[cfg(unix)]
#[tokio::test]
async fn an_inherited_run_over_its_limit_loses_only_its_direct_process() {
    use crate::process_group::test_support::{spawn_isolated, wait_gone};
    let dir = tempfile::tempdir().unwrap();
    let fixture = cut_after_first_delta(dir.path());
    let pid_file = dir.path().join("delegate.pid");
    let (_parent_cmd, mut parent) = spawn_isolated("exec /bin/sleep 60", &[]);
    let leader = parent.id().unwrap() as i32;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg("echo $$ > \"$P\"; cat \"$F\"; exec /bin/sleep 30")
        .env("F", &fixture)
        .env("P", &pid_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(leader);
    let under_workflow = crate::budgets::RunIntent::delegate(
        Some(AgentKind::Claude),
        crate::budgets::ParentRun {
            surface: Some(crate::usage_ledger::Surface::Workflow),
            scope: Some("brief".into()),
        },
    );
    let budget = crate::budgets::token_budget_for_test(under_workflow, 18_050.0, 0.0, true);
    // The cause (no timeout, a budget stop) is asserted, not the clock;
    // `elapsed` is only a no-hang guard. Left alone the delegate would
    // exit on its own at 30 s with no budget stop, so that fails too.
    let deadline = Duration::from_secs(120);
    let started = std::time::Instant::now();
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(deadline),
        None,
        budgeted_ledger(dir.path(), budget),
    )
    .await;
    assert!(started.elapsed() < deadline, "no hang");
    assert!(
        !collected.stopped && !collected.timed_out,
        "the guard ended it, not the deadline"
    );
    assert!(collected.budget_stop.is_some(), "a budget stop ended it");
    assert_eq!(
        collected.outcome, None,
        "an inherited run has no group to verify"
    );
    assert_eq!(only_row(dir.path()).outcome, "budget_stopped");
    let delegate: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_ne!(delegate, leader);
    assert!(
        wait_gone(delegate, Duration::from_secs(5)).await,
        "the delegate was killed"
    );
    // SAFETY: signal 0 only probes for existence.
    assert_eq!(
        unsafe { libc::kill(leader, 0) },
        0,
        "the parent group survives"
    );
    let _ = parent.start_kill();
    let _ = parent.wait().await;
}

// PR 8 plan tests 12/13(2): Claude's own backstop exit (recorded shape:
// exit 1, no result text, full modelUsage, zeroed top-level usage) is a
// budget stop whose row is REPORTED from modelUsage, not partial.
#[cfg(unix)]
#[tokio::test]
async fn the_backstop_exit_is_a_budget_stop_with_a_reported_row() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture_path("claude-max-budget.jsonl");
    let cmd = isolated_shell("cat \"$F\"; exit 1", &[("F", fixture.as_str())]);
    let mut ledger = test_ledger(dir.path(), AgentKind::Claude);
    ledger.set_billing(crate::usage::Billing::Api);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        ledger,
    )
    .await;
    let notice = collected
        .budget_stop
        .expect("the backstop is a budget stop");
    assert_eq!(
        notice.message,
        "Budget limit reached: Claude stopped at this run's spending backstop."
    );
    assert_eq!(collected.result.unwrap_err(), notice.message);
    let row = only_row(dir.path());
    assert_eq!(row.outcome, "budget_stopped");
    assert_eq!(row.cost_status, "reported");
    assert_eq!(
        row.output_tokens,
        Some(1_288),
        "opus 1,276 + haiku 12 from modelUsage"
    );
    assert!((row.cost_usd.unwrap() - 0.173_125_2).abs() < 1e-6);
}

/// OB08a: `collect_command` (the heartbeat's runner) clocks its run as chat
/// and workflows do: the spawn, the first model event (here thinking), the
/// tool calls before the first text and that text, on `Collected::activity`.
#[cfg(unix)]
#[tokio::test]
async fn collect_command_clocks_the_first_model_event_and_tools_before_text() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let think = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#;
    let tool = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}]}}"#;
    let text =
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done."}]}}"#;
    let done = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done."}"#;
    let script = concat!(
        "printf '%s\\n' \"$A\"; sleep 0.4; printf '%s\\n' \"$B\"; sleep 0.6; ",
        "printf '%s\\n%s\\n' \"$C\" \"$D\""
    );
    let env = [("A", think), ("B", tool), ("C", text), ("D", done)];
    // A starved reader (a full suite on a loaded runner) can stamp every
    // line in one late batch, thinking at 0 ms and text at 0 ms: the run is
    // repeated up to three times, and only the last one must show the gap.
    let mut attempts = 0;
    let (collected, activity, first, model, text) = loop {
        attempts += 1;
        let cmd = isolated_shell(script, &env);
        let collected = collect_command(
            AgentKind::Claude,
            cmd,
            ParseState::default(),
            Some(Duration::from_secs(20)),
            None,
            test_ledger(dir.path(), AgentKind::Claude),
        )
        .await;
        assert_eq!(collected.result.as_deref(), Ok("Done."));
        let activity = collected
            .activity
            .clone()
            .expect("a spawned run is clocked");
        let first = activity.first_outputs();
        let (model, text) = (first.model.expect("thinking"), first.text.expect("text"));
        if text >= model + 300 || attempts == 3 {
            break (collected, activity, first, model, text);
        }
    };
    // The stub sleeps 1 s between the two; a loaded reader stamps the first
    // line late, so the bound keeps a wide margin.
    assert!(
        text >= model + 300,
        "thinking at {model} ms, text at {text} ms"
    );
    let _ = &collected;
    assert_eq!(first.tools_before_text, Some(1));
    assert!(first.stdout.is_some_and(|stdout| stdout <= model));
    assert!(activity.since_spawn_ms().is_some_and(|since| since >= text));
    // A refusal spawns nothing and has no clock.
    assert!(Collected::failed("refused".into()).activity.is_none());
}

// Review fix (Paperclip PR 8b): a result envelope is never checked. The
// one-shot capture's terminal `modelUsage` (side models included) is
// larger than its streamed messages; with a limit between the two, the
// run finishes normally, keeps its text and records `success`.
#[cfg(unix)]
#[tokio::test]
async fn a_limit_crossed_only_by_the_result_line_never_stops_a_finished_run() {
    use crate::process_group::test_support::isolated_shell;
    let probe_dir = tempfile::tempdir().unwrap();
    let mut probe = test_ledger(probe_dir.path(), AgentKind::Claude);
    let mut state = ParseState::default();
    let (mut streamed, mut terminal) = (0u64, None::<u64>);
    for line in include_str!("../fixtures/usage/claude-oneshot.jsonl").lines() {
        for evt in parse_line(AgentKind::Claude, line, &mut state) {
            let is_result = matches!(evt, AgentEvent::Result { .. });
            probe.observe(&evt);
            let total = probe.running_total().map_or(0, |t| t.input + t.output);
            if is_result {
                terminal = Some(total);
            } else if terminal.is_none() {
                streamed = total;
            }
        }
    }
    let terminal = terminal.expect("a result line");
    drop(probe);
    assert!(
        terminal > streamed,
        "the result adds side-model tokens: {streamed} -> {terminal}"
    );

    let dir = tempfile::tempdir().unwrap();
    let budget = crate::budgets::token_budget_for_test(
        crate::budgets::RunIntent::workflow("brief", Some(AgentKind::Claude)),
        (streamed + 1) as f64,
        0.0,
        true,
    );
    let fixture = fixture_path("claude-oneshot.jsonl");
    let cmd = isolated_shell("cat \"$F\"", &[("F", fixture.as_str())]);
    let collected = collect_command(
        AgentKind::Claude,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        budgeted_ledger(dir.path(), budget),
    )
    .await;
    assert_eq!(collected.result.as_deref(), Ok("ok"));
    assert!(collected.budget_stop.is_none());
    assert_eq!(only_row(dir.path()).outcome, "success");
}

// Review fix (Paperclip PR 8b): Codex is checked at the claim only. Its
// usage arrives once, in `turn.completed`, after the work is done, so
// even a limit that turn crosses never stops it.
#[cfg(unix)]
#[tokio::test]
async fn codex_runs_get_no_mid_run_guard() {
    use crate::process_group::test_support::isolated_shell;
    let dir = tempfile::tempdir().unwrap();
    let budget = crate::budgets::token_budget_for_test(
        crate::budgets::RunIntent::workflow("brief", Some(AgentKind::Codex)),
        1.0,
        0.0,
        true,
    );
    let ledger = crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            dir.path(),
            crate::usage_ledger::Surface::Workflow,
            Some("brief".into()),
            None,
        )
        .with_budget(budget),
        Some(AgentKind::Codex),
        None,
    );
    let fixture = fixture_path("codex-exec.jsonl");
    let cmd = isolated_shell("cat \"$F\"", &[("F", fixture.as_str())]);
    let collected = collect_command(
        AgentKind::Codex,
        cmd,
        ParseState::default(),
        Some(Duration::from_secs(20)),
        None,
        ledger,
    )
    .await;
    assert!(collected.result.is_ok(), "{:?}", collected.result);
    assert!(collected.budget_stop.is_none());
    assert!(collected.budget_warnings.is_empty());
    assert_eq!(only_row(dir.path()).outcome, "success");
}

fn never_probed() -> bool {
    panic!("the backstop probe must not run here")
}

// Paperclip PR 8b: what a claimed budget changes on a spec, for every
// launch point that applies it (workflows, the heartbeat).
#[tokio::test]
async fn apply_run_budget_meters_only_claude_and_backstops_only_fresh_runs() {
    let intent = crate::budgets::RunIntent::workflow("brief", Some(AgentKind::Claude));
    let mut budget = crate::budgets::token_budget_for_test(intent.clone(), 1000.0, 0.0, true);
    budget.max_budget_usd = Some(0.5);

    let mut fresh = spec("acceptEdits", None, false);
    apply_run_budget(AgentKind::Claude, &mut fresh, &budget, || async { true }).await;
    assert!(fresh.usage_stream);
    assert_eq!(fresh.max_budget_usd, Some(0.5));
    assert!(ParseState::for_spec(AgentKind::Claude, &fresh).usage_only());
    assert!(validate_run_spec(AgentKind::Claude, &fresh).is_ok());

    let mut unsupported = spec("acceptEdits", None, false);
    apply_run_budget(AgentKind::Claude, &mut unsupported, &budget, || async {
        false
    })
    .await;
    assert!(unsupported.usage_stream);
    assert_eq!(
        unsupported.max_budget_usd, None,
        "an old CLI gets no backstop"
    );

    for kind in [AgentKind::Codex, AgentKind::Antigravity] {
        let mut other = spec("acceptEdits", None, false);
        apply_run_budget(kind, &mut other, &budget, || async { never_probed() }).await;
        assert!(
            !other.usage_stream && other.max_budget_usd.is_none(),
            "{kind:?}"
        );
        assert!(validate_run_spec(kind, &other).is_ok());
    }
    for (resume, interactive) in [(Some("sid"), false), (None, true)] {
        let mut not_fresh = spec("acceptEdits", resume, interactive);
        apply_run_budget(AgentKind::Claude, &mut not_fresh, &budget, || async {
            never_probed()
        })
        .await;
        assert_eq!(not_fresh.max_budget_usd, None);
        assert!(validate_run_spec(AgentKind::Claude, &not_fresh).is_ok());
    }

    let tokens_only = crate::budgets::token_budget_for_test(intent.clone(), 1000.0, 0.0, true);
    let mut metered = spec("acceptEdits", None, false);
    apply_run_budget(AgentKind::Claude, &mut metered, &tokens_only, || async {
        never_probed()
    })
    .await;
    assert!(metered.usage_stream);
    assert_eq!(metered.max_budget_usd, None);

    let none = crate::budgets::RunBudget::none(intent);
    let mut plain = spec("acceptEdits", None, false);
    apply_run_budget(AgentKind::Claude, &mut plain, &none, || async {
        never_probed()
    })
    .await;
    assert!(
        !plain.usage_stream && plain.max_budget_usd.is_none(),
        "no budgets change nothing"
    );
}

// The backstop capability probe runs `--help` of the binary it is given
// (never the installed CLI in tests), outside the vault, once per binary.
#[cfg(unix)]
#[tokio::test]
async fn the_backstop_probe_reads_help_once_per_binary() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let marker = dir.path().join("probed");
    let with = script(
        "with-budget",
        &format!(
            "echo probed >> '{}'; echo '  --max-budget-usd <amount>  Maximum dollars'",
            marker.display()
        ),
    );
    let without = script("without-budget", "echo '  --model <model>'");
    assert!(claude_max_budget_supported_at(&with).await);
    assert!(claude_max_budget_supported_at(&with).await);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().lines().count(),
        1,
        "probed once"
    );
    assert!(!claude_max_budget_supported_at(&without).await);
    assert!(!claude_max_budget_supported_at(&dir.path().join("missing")).await);
}

/// Track U: the ultracode probe asks the CLI itself, once per binary,
/// with a model no API serves; it trusts only a CLI that got
/// as far as the model and said nothing about the effort. The output
/// shapes are Claude Code 2.1.286's (2026-10-02): a taken effort is
/// silent, an unknown one prints a warning and is ignored.
#[tokio::test]
async fn the_ultracode_probe_trusts_only_a_cli_that_took_the_effort() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let marker = dir.path().join("argv");
    let model_line = r#"[claude-code:unrecognized_model] {\"model\":\"hollow-probe-no-such-model\",\"query_source\":\"sdk\"}"#;
    let takes = script(
        "takes",
        &format!("echo \"$*\" >> '{}'; echo \"{model_line}\" >&2; echo '{{\"session_id\":\"x\",\"total_cost_usd\":0}}'", marker.display()),
    );
    let ignores = script(
        "ignores",
        &format!("echo \"Warning: Unknown --effort value 'ultracode' — ignoring it and using the default effort.\" >&2; echo \"{model_line}\" >&2; echo '{{\"session_id\":\"x\"}}'"),
    );
    let silent = script("silent", "exit 1");
    assert!(claude_ultracode_supported_at(&takes).await);
    assert!(claude_ultracode_supported_at(&takes).await);
    let argv = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(argv.lines().count(), 1, "probed once per binary");
    assert!(
        argv.contains("--effort ultracode") && argv.contains("--model hollow-probe-no-such-model"),
        "{argv}"
    );
    assert!(
        argv.contains("--tools  --strict-mcp-config"),
        "no tools, no MCP servers: {argv}"
    );
    assert!(
        !claude_ultracode_supported_at(&ignores).await,
        "a CLI that ignores it degrades"
    );
    assert!(
        !claude_ultracode_supported_at(&silent).await,
        "no proof is no support"
    );
    assert!(!claude_ultracode_supported_at(&dir.path().join("missing")).await);
}

/// Track U: what each agent's command carries for an ultracode turn,
/// fresh and resumed. Only Claude, and only once its CLI is known to take
/// it, sees the string; resumed turns keep the effort.
#[test]
fn ultracode_reaches_only_a_claude_cli_that_takes_it() {
    let _probe = crate::effort::tests::PROBE_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let with = |kind: AgentKind, resume: Option<&str>| {
        let mut run = spec("acceptEdits", resume, true);
        run.model = Some(
            match kind {
                AgentKind::Grok => "grok-test-u2",
                AgentKind::Antigravity => "gemini-u2-pro-low",
                _ => "m",
            }
            .into(),
        );
        run.extras.effort = Some(crate::effort::ULTRACODE.into());
        rendered(&test_command(kind, &run))
    };
    crate::effort::set_claude_ultracode(true);
    for resume in [None, Some("sid-u")] {
        let claude = with(AgentKind::Claude, resume);
        assert!(claude.contains("--effort ultracode"), "{claude}");
        assert_eq!(claude.contains("--resume sid-u"), resume.is_some());
    }
    crate::effort::set_claude_ultracode(false);
    let degraded = with(AgentKind::Claude, Some("sid-u"));
    assert!(
        degraded.contains("--effort xhigh") && !degraded.contains("ultracode"),
        "{degraded}"
    );
    crate::effort::remember(
        AgentKind::Grok,
        "grok-test-u2",
        vec!["low".into(), "high".into()],
    );
    crate::effort::remember_agy_catalog(["gemini-u2-pro-high", "gemini-u2-pro-low"]);
    for kind in [AgentKind::Codex, AgentKind::Grok, AgentKind::Antigravity] {
        for resume in [None, Some("th-u")] {
            let cmd = with(kind, resume);
            assert!(
                !cmd.contains("ultracode"),
                "{kind:?} never gets the string: {cmd}"
            );
        }
    }
    assert!(
        with(AgentKind::Codex, Some("th-u")).contains("model_reasoning_effort="),
        "a resumed Codex turn keeps its effort"
    );
    assert!(with(AgentKind::Grok, None).contains("--reasoning-effort high"));
    assert!(with(AgentKind::Antigravity, None).contains("--model gemini-u2-pro-high"));
}

// Refresh §3 item 6: a resumed chat turn's guard total is the sum of its
// streamed message finals (msg-002: 148 input + 3 output), never the
// result's session-cumulative modelUsage (~19,000 tokens).
#[test]
fn a_resumed_turns_guard_reads_only_its_streamed_messages() {
    use crate::usage::tests::CHAT_RESUME;
    let dir = tempfile::tempdir().unwrap();
    let mut run = crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            dir.path(),
            crate::usage_ledger::Surface::Chat,
            Some("s-1".into()),
            None,
        ),
        Some(AgentKind::Claude),
        None,
    );
    run.set_resumed(Some("id-001".into()));
    let budget = crate::budgets::token_budget_for_test(
        crate::budgets::RunIntent::chat(Some(AgentKind::Claude)),
        1_000.0,
        0.0,
        true,
    );
    let mut guard = crate::budgets::BudgetGuard::new(&budget).unwrap();
    let mut state = ParseState::for_spec(
        AgentKind::Claude,
        &spec("acceptEdits", Some("id-001"), true),
    );
    let mut warnings = 0;
    for line in CHAT_RESUME.lines() {
        for evt in parse_line(AgentKind::Claude, line, &mut state) {
            if run.observe(&evt) {
                let outcome = guard.observe(run.running_total());
                assert!(outcome.stop.is_none(), "chat is never stopped");
                warnings += outcome.warnings.len();
            }
        }
    }
    assert_eq!(warnings, 0, "151 tokens of 1,000: no level crossed");
    let total = run.running_total().expect("streamed usage");
    assert_eq!(total.input + total.output, 151);
    drop(run);
}

#[test]
fn the_unavailable_sentinel_is_detected() {
    let run = spec("acceptEdits", None, false);
    assert!(is_unavailable_command(&failed_command(&run, "x")));
    assert!(is_unavailable_command(&build_with_codex_servers(
        AgentKind::Codex,
        &run,
        Err("configuration unavailable".into()),
    )));
    assert!(!is_unavailable_command(&test_command(
        AgentKind::Claude,
        &run
    )));
    let mut plain = Command::new("/bin/sh");
    plain.args(["-c", "exit 0", "not-the-sentinel"]);
    assert!(!is_unavailable_command(&plain));
}

#[test]
fn parse_antigravity_streams_verified_step_deltas_and_sessions() {
    let mut st = ParseState::default();
    let init = parse_antigravity(
        r#"{"event":"init","conversation_id":"conversation-one","init":{"cwd":"/tmp"}}"#,
        &mut st,
    );
    assert!(matches!(&init[0], AgentEvent::Session(id) if id == "conversation-one"));
    let first = parse_antigravity(
        r#"{"event":"step_update","step_update":{"step_index":2,"state":"RUNNING","step_type":"agent_response","text_delta":"line one"}}"#,
        &mut st,
    );
    assert!(matches!(first[0], AgentEvent::TurnBoundary));
    match &first[1] {
        AgentEvent::AssistantDelta(t) => assert_eq!(t, "line one"),
        other => panic!("got {other:?}"),
    }
    let second = parse_antigravity(
        r#"{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"agent_response","text_delta":"\nline two"}}"#,
        &mut st,
    );
    match &second[0] {
        AgentEvent::AssistantDelta(t) => assert_eq!(t, "\nline two"),
        other => panic!("got {other:?}"),
    }
    let result = parse_antigravity(
        r#"{"event":"result","result":{"conversation_id":"conversation-one","status":"SUCCESS","response":"line one\nline two"}}"#,
        &mut st,
    );
    assert!(
        matches!(&result[1], AgentEvent::Result { is_error: false, text: Some(text), .. } if text == "line one\nline two")
    );
    let failed = parse_antigravity(
        r#"{"event":"result","result":{"status":"ERROR","response":"Authentication failed"}}"#,
        &mut st,
    );
    assert!(matches!(
        &failed[0],
        AgentEvent::Result { is_error: true, .. }
    ));
    assert!(parse_antigravity("not protocol", &mut st).is_empty());
    assert!(parse_antigravity(r#"{"event":"step_update","step_update":{"step_type":"user_input","text_delta":"private user text"}}"#, &mut st).is_empty());
}

#[test]
fn parse_claude_delta_then_skips_aggregate_text() {
    let mut st = ParseState::default();
    let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}"#;
    match &parse_claude(delta, &mut st)[0] {
        AgentEvent::AssistantDelta(t) => assert_eq!(t, "hel"),
        other => panic!("got {other:?}"),
    }
    assert!(st.got_delta);
    // The aggregated assistant block should now be suppressed.
    let agg = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}"#;
    assert!(parse_claude(agg, &mut st).is_empty());
}

/// S02 review: a `compact_boundary` system event marks the stream as
/// compacted; the init event and other system events don't.
#[test]
fn parse_claude_notes_a_compaction() {
    let mut st = ParseState::default();
    let init = r#"{"type":"system","subtype":"init","session_id":"s-1"}"#;
    let events = parse_claude(init, &mut st);
    assert!(matches!(&events[0], AgentEvent::Session(id) if id == "s-1"));
    let hook = r#"{"type":"system","subtype":"hook_started","session_id":"s-1"}"#;
    parse_claude(hook, &mut st);
    assert!(!st.compacted);
    let compact = r#"{"type":"system","subtype":"compact_boundary","session_id":"s-1","compact_metadata":{"trigger":"auto","pre_tokens":167000}}"#;
    parse_claude(compact, &mut st);
    assert!(st.compacted);
}

/// OB22: every `compact_boundary` counts, so a run that compacted twice
/// says 2; no other system event moves the count.
#[test]
fn parse_claude_counts_each_compaction() {
    let mut st = ParseState::default();
    assert_eq!(st.compactions, 0);
    let init = r#"{"type":"system","subtype":"init","session_id":"s-1"}"#;
    parse_claude(init, &mut st);
    assert_eq!(st.compactions, 0);
    let compact = r#"{"type":"system","subtype":"compact_boundary","session_id":"s-1","compact_metadata":{"trigger":"auto","pre_tokens":167000}}"#;
    parse_claude(compact, &mut st);
    assert_eq!(st.compactions, 1);
    parse_claude(compact, &mut st);
    assert_eq!(st.compactions, 2);
    assert!(st.compacted);
}

/// The `Result` event's detail at the end of a capture.
fn result_detail(kind: AgentKind, stdout: &str) -> ResultDetail {
    let mut state = ParseState::default();
    stdout
        .lines()
        .flat_map(|line| parse_line(kind, line, &mut state))
        .filter_map(|evt| match evt {
            AgentEvent::Result { detail, .. } => Some(detail),
            _ => None,
        })
        .last()
        .expect("a result event")
}

/// Paperclip PR 14a: the result envelope's structured fields are kept.
#[test]
fn claude_result_detail_keeps_stop_reason_errors_and_status() {
    let budget = result_detail(
        AgentKind::Claude,
        include_str!("../fixtures/usage/claude-max-budget.jsonl"),
    );
    assert_eq!(budget.terminal_reason.as_deref(), Some("budget_exhausted"));
    assert_eq!(budget.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(
        budget.errors,
        vec!["Reached maximum budget ($0.01)".to_string()]
    );
    assert_eq!(budget.api_error_status, None);

    // A success: no errors, no status (`api_error_status: null`).
    let ok = result_detail(
        AgentKind::Claude,
        include_str!("../fixtures/usage/claude-oneshot.jsonl"),
    );
    assert_eq!(ok.terminal_reason.as_deref(), Some("completed"));
    assert!(ok.errors.is_empty());
    assert_eq!(ok.api_error_status, None);
    assert!(ok.denied_actions.is_empty());

    let refusal = result_detail(
        AgentKind::Claude,
        include_str!("../fixtures/results/claude-refusal.jsonl"),
    );
    assert_eq!(refusal.stop_reason.as_deref(), Some("refusal"));
    let auth = result_detail(
        AgentKind::Claude,
        include_str!("../fixtures/results/claude-auth.jsonl"),
    );
    assert_eq!(auth.api_error_status, Some(401));

    // Objects and caps.
    let line = format!(
        r#"{{"type":"result","subtype":"error_during_execution","is_error":true,"stopReason":" Refusal ","errors":[{{"message":"a"}},{{"code":42}},{},{}]}}"#,
        serde_json::to_string(&"x".repeat(RESULT_ERROR_CHARS + 50)).unwrap(),
        (0..20)
            .map(|i| format!("\"e{i}\""))
            .collect::<Vec<_>>()
            .join(","),
    );
    let detail = result_detail(AgentKind::Claude, &line);
    assert_eq!(
        detail.stop_reason.as_deref(),
        Some("refusal"),
        "trimmed, lowercased, camelCase key"
    );
    assert_eq!(detail.errors.len(), RESULT_ERRORS_MAX);
    assert_eq!(detail.errors[0], "a");
    assert_eq!(detail.errors[1], "42");
    assert_eq!(detail.errors[2].chars().count(), RESULT_ERROR_CHARS);
}

/// Paperclip PR 14a: an assistant message wrapping an API error is an
/// envelope, never assistant prose.
#[test]
fn claude_api_error_assistant_message_is_an_envelope() {
    let mut state = ParseState::default();
    let events: Vec<AgentEvent> =
        include_str!("../fixtures/results/claude-poisoned-api-error.jsonl")
            .lines()
            .flat_map(|line| parse_line(AgentKind::Claude, line, &mut state))
            .collect();
    let api: Vec<_> = events
        .iter()
        .filter_map(|evt| match evt {
            AgentEvent::ApiError { text, status } => Some((text.clone(), *status)),
            _ => None,
        })
        .collect();
    assert_eq!(api.len(), 1);
    assert!(api[0].0.starts_with("API Error: 400"), "{}", api[0].0);
    assert!(api[0].0.contains("previous_message_id"));
    assert_eq!(api[0].1, Some(400));
    assert!(!events
        .iter()
        .any(|evt| matches!(evt, AgentEvent::AssistantText(_))));

    // An ordinary assistant text line is unchanged.
    let mut state = ParseState::default();
    let plain = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi there"}]}}"#;
    let events = parse_line(AgentKind::Claude, plain, &mut state);
    assert!(matches!(&events[..], [AgentEvent::AssistantText(t)] if t == "hi there"));
    let not_flagged = r#"{"type":"assistant","is_api_error_message":false,"message":{"content":[{"type":"text","text":"API Error: 529 in a quote"}]}}"#;
    let events = parse_line(AgentKind::Claude, not_flagged, &mut state);
    assert!(matches!(&events[..], [AgentEvent::AssistantText(_)]));
}

/// Paperclip PR 14a: a bare `{type:"error", message}` event keeps its
/// message (it used to read "codex error").
#[test]
fn codex_error_event_reads_top_level_message() {
    let mut st = ParseState::default();
    let line = r#"{"type":"error","message":"We're currently experiencing high demand"}"#;
    match &parse_codex(line, &mut st)[..] {
        [AgentEvent::Result {
            text,
            is_error,
            detail,
            ..
        }] => {
            assert!(*is_error);
            assert_eq!(
                text.as_deref(),
                Some("We're currently experiencing high demand")
            );
            assert_eq!(
                detail.errors,
                vec!["We're currently experiencing high demand".to_string()]
            );
        }
        other => panic!("got {other:?}"),
    }
    // `error.message` still wins, and the recorded failure shape still parses.
    let failed = result_detail(
        AgentKind::Codex,
        include_str!("../fixtures/usage/codex-failed.jsonl"),
    );
    assert_eq!(failed.errors, vec!["Account unavailable".to_string()]);
    let bare = parse_codex(r#"{"type":"error"}"#, &mut st);
    assert!(matches!(&bare[..], [AgentEvent::Result { text: Some(t), .. }] if t == "codex error"));
}

/// Paperclip PR 14a, changed 2026-10-02: effort reaches each CLI in its
/// own form, every agent offers every level, and a level a model can't
/// take maps to the nearest one or is left out; it never fails the run.
#[test]
fn effort_maps_to_each_cli_and_never_fails_a_run() {
    let with_effort = |base: RunSpec, effort: &str| RunSpec {
        extras: RunExtras {
            effort: Some(effort.to_string()),
            ..RunExtras::default()
        },
        ..base
    };
    let claude = rendered(&test_command(
        AgentKind::Claude,
        &with_effort(spec("acceptEdits", None, false), "high"),
    ));
    assert!(claude.contains("--effort high"), "{claude}");

    for base in [
        spec("acceptEdits", None, true),
        spec("acceptEdits", Some("thread-1"), true),
    ] {
        let codex = rendered(&test_command(AgentKind::Codex, &with_effort(base, "high")));
        let at = codex
            .find("model_reasoning_effort=\"high\"")
            .expect("the override");
        assert!(codex[..at].ends_with("-c "), "{codex}");
        assert!(
            at < codex.find(" exec").expect("exec"),
            "before exec: {codex}"
        );
    }
    let grok = rendered(&test_command(
        AgentKind::Grok,
        &with_effort(spec("acceptEdits", None, false), "medium"),
    ));
    assert!(grok.contains("--reasoning-effort medium"), "{grok}");
    // No effort: no flag.
    assert!(!rendered(&test_command(
        AgentKind::Claude,
        &spec("acceptEdits", None, false)
    ))
    .contains("--effort"));
    assert!(!rendered(&test_command(
        AgentKind::Codex,
        &spec("acceptEdits", None, false)
    ))
    .contains("model_reasoning_effort"));
    assert!(!rendered(&test_command(
        AgentKind::Grok,
        &spec("acceptEdits", None, false)
    ))
    .contains("--reasoning-effort"));

    // What used to be refused now runs: an unknown level is left out, a
    // level the agent lacks maps, and Antigravity takes none without a
    // known variant (never an `--effort` that would conflict).
    for (kind, level) in [
        (AgentKind::Antigravity, "high"),
        (AgentKind::Claude, "turbo"),
        (AgentKind::Codex, "max"),
        (AgentKind::Grok, "max"),
    ] {
        let run = with_effort(spec("acceptEdits", None, false), level);
        validate_run_spec(kind, &run).unwrap_or_else(|e| panic!("{kind:?} {level}: {e}"));
        let cmd = test_command(kind, &run);
        assert!(!is_unavailable_command(&cmd), "{kind:?} with {level} runs");
    }
    let agy = rendered(&test_command(
        AgentKind::Antigravity,
        &with_effort(spec("acceptEdits", None, false), "high"),
    ));
    assert!(!agy.contains("--effort"), "{agy}");
    assert!(rendered(&test_command(
        AgentKind::Claude,
        &with_effort(spec("acceptEdits", None, false), "turbo")
    ))
    .contains("--permission-mode"));
    // Every agent advertises every level.
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Antigravity,
        AgentKind::Grok,
    ] {
        assert_eq!(
            capabilities(kind).effort_levels,
            vec!["low", "medium", "high", "xhigh", "max"],
            "{kind:?}"
        );
    }
    let wire = serde_json::to_value(capabilities(AgentKind::Codex)).unwrap();
    assert_eq!(wire["effortLevels"][0], json!("low"));
}

fn args_of(cmd: &Command) -> Vec<String> {
    crate::secret_fds::expand_append(
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
    )
}

const EVERY_AGENT: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Grok,
    AgentKind::Antigravity,
];

/// The flags each agent puts a resume id behind.
fn resume_flags(args: &[String]) -> Vec<&String> {
    args.iter()
        .filter(|a| matches!(a.as_str(), "--resume" | "resume" | "--conversation"))
        .collect()
}

/// Review finding A·R10: a resume id sits right after a flag, so one that
/// parses as a flag must never reach any agent's argv. Before the fix
/// every builder passed any non-empty id through.
#[test]
fn a_flag_shaped_resume_id_never_reaches_any_agent() {
    for bad in ["-x", "--dangerously-skip-permissions"] {
        for kind in EVERY_AGENT {
            let args = args_of(&test_command(kind, &spec("acceptEdits", Some(bad), true)));
            assert!(
                !args.iter().any(|a| a == bad),
                "{kind:?} passed {bad}: {args:?}"
            );
            assert!(
                resume_flags(&args).is_empty(),
                "{kind:?} resumed {bad}: {args:?}"
            );
        }
    }
    // The control: a real id still resumes on every agent.
    let good = "01a0df79-39e3-7560-86f9-3b598ba5ff05";
    for kind in EVERY_AGENT {
        let args = args_of(&test_command(kind, &spec("acceptEdits", Some(good), true)));
        let at = args.iter().position(|a| a == good).unwrap_or(0);
        assert!(
            at > 0 && resume_flags(&args[at - 1..at]).len() == 1,
            "{kind:?} did not resume {good}: {args:?}"
        );
    }
}

#[test]
fn resume_ids_are_a_bounded_safe_charset() {
    for ok in [
        "01a0df79-39e3-7560-86f9-3b598ba5ff05",
        "sid-1",
        "th_u.2",
        "a",
    ] {
        assert!(is_safe_resume_id(ok), "{ok}");
    }
    let long = "a".repeat(MAX_RESUME_ID_LEN + 1);
    for bad in [
        "",
        "-x",
        "--dangerously-skip-permissions",
        "-",
        "sid 1",
        "sid;rm",
        "../sid",
        "sid\n",
        "s\u{e9}ance",
        long.as_str(),
    ] {
        assert!(!is_safe_resume_id(bad), "{bad:?}");
    }
    assert!(is_safe_resume_id(&"a".repeat(MAX_RESUME_ID_LEN)));
}

fn with_scratch(mut run: RunSpec, scratch: crate::scratch::ScratchSpec) -> RunSpec {
    run.extras.scratch = Some(scratch);
    run
}

const SCRATCH_DIR: &str = "/private/tmp/hollow-run/run-1";

fn assert_scratch_env(cmd: &Command) {
    for key in ["TMPDIR", "TEMP", "TMP"] {
        assert_eq!(
            env_value(cmd, key).as_deref(),
            Some("/private/tmp/hollow-run/run-1/"),
            "{key}"
        );
    }
    assert_eq!(
        env_value(cmd, crate::scratch::SCRATCH_ENV).as_deref(),
        Some(SCRATCH_DIR)
    );
}

/// Build programme PR B7: a build workspace's environment reaches every
/// agent's command.
#[test]
fn extra_env_reaches_every_agent_command() {
    let mut run = spec("acceptEdits", None, true);
    run.extras.env = vec![
        ("CARGO_TARGET_DIR".into(), "/cache/app-1".into()),
        ("PORT".into(), "1612".into()),
    ];
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Antigravity,
        AgentKind::Grok,
    ] {
        let cmd = test_command(kind, &run);
        if is_unavailable_command(&cmd) {
            continue;
        }
        assert_eq!(
            env_value(&cmd, "CARGO_TARGET_DIR").as_deref(),
            Some("/cache/app-1"),
            "{kind:?}"
        );
        assert_eq!(env_value(&cmd, "PORT").as_deref(), Some("1612"), "{kind:?}");
    }
}

/// A Dock-launched Hollow has only the system PATH, where `codex` (a
/// `#!/usr/bin/env node` script) exits 127. Every agent command gets the
/// user's tool folders; a build workspace's own PATH still wins.
#[test]
fn every_agent_command_gets_the_tool_path_unless_extras_set_one() {
    let run = spec("acceptEdits", None, true);
    let mut own = run.clone();
    own.extras.env = vec![("PATH".into(), "/tools:/usr/bin".into())];
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Antigravity,
        AgentKind::Grok,
    ] {
        let cmd = test_command(kind, &run);
        if is_unavailable_command(&cmd) {
            continue;
        }
        let path = env_value(&cmd, "PATH").unwrap_or_default();
        assert!(path.contains(":/opt/homebrew/bin:"), "{kind:?} {path}");
        assert!(path.split(':').any(|d| d == "/usr/bin"), "{kind:?} {path}");
        let cmd = test_command(kind, &own);
        assert_eq!(
            env_value(&cmd, "PATH").as_deref(),
            Some("/tools:/usr/bin"),
            "{kind:?}"
        );
    }
}

/// A build workspace's vault folder rides beside the scratch folder, both
/// before `--permission-mode`, which ends the variadic `--add-dir` list.
#[test]
fn extra_dirs_are_added_before_permission_mode() {
    use crate::scratch::ScratchSpec;
    let mut run = with_scratch(
        spec("acceptEdits", None, true),
        ScratchSpec::Dir(PathBuf::from(SCRATCH_DIR)),
    );
    run.extras.extra_dirs = vec![PathBuf::from("/vault/workspaces/w-1")];
    for kind in [AgentKind::Claude, AgentKind::Antigravity] {
        let args = args_of(&test_command(kind, &run));
        let dirs: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--add-dir")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(dirs.len(), 2, "{kind:?} {args:?}");
        assert_eq!(args[dirs[1] + 1], "/vault/workspaces/w-1");
        if kind == AgentKind::Claude {
            assert_eq!(args[dirs[1] + 2], "--permission-mode", "{args:?}");
        }
    }
}

/// Paperclip PR 15a: Claude gets `--add-dir <dir>` right before
/// `--permission-mode` (which ends the variadic list) and the scratch
/// environment; the appended prompt names only the variable.
#[test]
fn claude_scratch_sets_env_and_add_dir_before_permission_mode() {
    use crate::scratch::{ScratchSpec, SCRATCH_GUIDANCE};
    for base in [
        spec("acceptEdits", None, true),
        spec("acceptEdits", None, false),
        spec("plan", Some("sid-1"), true),
    ] {
        let mut run = with_scratch(base, ScratchSpec::Dir(PathBuf::from(SCRATCH_DIR)));
        run.append_system_prompt = Some(format!("brief\n\n{SCRATCH_GUIDANCE}"));
        let cmd = test_command(AgentKind::Claude, &run);
        let args = args_of(&cmd);
        let at = args
            .iter()
            .position(|a| a == "--add-dir")
            .expect("--add-dir");
        assert_eq!(args[at + 1], SCRATCH_DIR);
        assert_eq!(args[at + 2], "--permission-mode", "{args:?}");
        assert_eq!(args.iter().filter(|a| *a == "--add-dir").count(), 1);
        assert_scratch_env(&cmd);
        let appended = &args[args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap()
            + 1];
        assert!(appended.contains(SCRATCH_GUIDANCE));
        assert!(
            !appended.contains("hollow-run"),
            "the per-run path stays out of the prompt"
        );
        assert!(
            !args[1].contains("hollow-run"),
            "and out of the user prompt"
        );
    }
}

#[test]
fn antigravity_scratch_passes_add_dir_before_sandbox_flags() {
    use crate::scratch::ScratchSpec;
    for (perm, flag) in [
        ("acceptEdits", "--sandbox"),
        ("plan", "--sandbox"),
        ("bypassPermissions", "--dangerously-skip-permissions"),
    ] {
        let run = with_scratch(
            spec(perm, None, false),
            ScratchSpec::Dir(PathBuf::from(SCRATCH_DIR)),
        );
        let cmd = test_command(AgentKind::Antigravity, &run);
        let args = args_of(&cmd);
        let at = args
            .iter()
            .position(|a| a == "--add-dir")
            .expect("--add-dir");
        assert_eq!(args[at + 1], SCRATCH_DIR);
        assert!(
            at < args.iter().position(|a| a == flag).expect(flag),
            "{perm}: {args:?}"
        );
        assert_scratch_env(&cmd);
    }
}

/// Codex's `workspace-write` sandbox treats `$TMPDIR` as a writable root
/// (live probe, codex-cli 0.153.4), and `exec resume` takes no sandbox
/// flags, so Codex gets the environment only, on both forms.
#[test]
fn codex_scratch_sets_env_only_on_exec_and_resume() {
    use crate::scratch::ScratchSpec;
    for base in [
        spec("acceptEdits", None, false),
        spec("acceptEdits", Some("thread-1"), true),
    ] {
        let run = with_scratch(base, ScratchSpec::Dir(PathBuf::from(SCRATCH_DIR)));
        let cmd = test_command(AgentKind::Codex, &run);
        let args = args_of(&cmd);
        assert!(!args.iter().any(|a| a == "--add-dir"), "{args:?}");
        assert_scratch_env(&cmd);
    }
}

/// No scratch leaves the inherited environment alone; an unavailable
/// folder sets only the sentinel; the `/bin/sh` stand-in gets nothing.
#[test]
fn no_scratch_and_stand_ins_leave_tmpdir_alone() {
    use crate::scratch::{ScratchSpec, SCRATCH_ENV, UNAVAILABLE_PATH};
    for kind in AgentKind::all() {
        let cmd = test_command(kind, &spec("acceptEdits", None, false));
        for key in ["TMPDIR", "TEMP", "TMP", SCRATCH_ENV] {
            assert_eq!(env_value(&cmd, key), None, "{kind:?} {key}");
        }
        assert!(!args_of(&cmd).iter().any(|a| a == "--add-dir"));

        let unavailable = test_command(
            kind,
            &with_scratch(spec("acceptEdits", None, false), ScratchSpec::Unavailable),
        );
        assert_eq!(
            env_value(&unavailable, SCRATCH_ENV).as_deref(),
            Some(UNAVAILABLE_PATH),
            "{kind:?}"
        );
        for key in ["TMPDIR", "TEMP", "TMP"] {
            assert_eq!(env_value(&unavailable, key), None, "{kind:?} {key}");
        }
        assert!(!args_of(&unavailable).iter().any(|a| a == "--add-dir"));
    }
    let run = with_scratch(
        spec("acceptEdits", None, false),
        ScratchSpec::Dir(PathBuf::from(SCRATCH_DIR)),
    );
    let stand_in = build_with_codex_servers(
        AgentKind::Codex,
        &run,
        Err("configuration unavailable".into()),
    );
    assert!(is_unavailable_command(&stand_in));
    assert_eq!(env_value(&stand_in, "TMPDIR"), None);
    assert_eq!(env_value(&stand_in, SCRATCH_ENV), None);
    let mut refused = run.clone();
    refused.allowed_tools = vec!["Read".into()];
    let refused = test_command(AgentKind::Antigravity, &refused);
    assert!(is_unavailable_command(&refused));
    assert_eq!(env_value(&refused, SCRATCH_ENV), None);
}

/// Wave 3 review (G2): the one-line reference replaces the appended
/// prompt only when the command really resumes. Codex, Antigravity
/// and Grok pass the resume id only on an interactive spec, so a
/// non-interactive spec with a resume id and `configured_held` runs a
/// fresh session and must get the configured half whole.
#[test]
fn the_configured_half_is_held_back_only_when_the_provider_resumes() {
    const APPENDED: &str = "Hollow's appended prompt for this test.";
    for kind in [AgentKind::Codex, AgentKind::Antigravity, AgentKind::Grok] {
        for interactive in [true, false] {
            let mut run = spec("acceptEdits", Some("native-1"), interactive);
            run.append_system_prompt = Some(APPENDED.into());
            run.extras.configured_held = true;
            let mut args = args_of(&test_command(kind, &run));
            // Q-R33: Codex's prompt is on stdin.
            args.extend(take_last_stdin_prompt());
            let resumes = args.iter().any(|a| a == "native-1");
            assert_eq!(resumes, interactive, "{kind:?} resumes only interactively");
            let prompt = contextual_prompt(kind, &run);
            assert!(
                args.contains(&prompt),
                "{kind:?}: the prompt is the argument"
            );
            assert_eq!(
                prompt.contains(APPENDED),
                !resumes,
                "{kind:?} interactive={interactive}: a fresh session gets the configured half"
            );
            assert_eq!(prompt.contains(CONFIGURED_HELD), resumes, "{kind:?}");
        }
    }
}

/// B03: the login shell's folders sit after Hollow's tool folders and
/// before the inherited and system ones, each folder once.
#[test]
fn the_shell_path_comes_after_hollows_folders_and_before_the_system() {
    let home = std::path::Path::new("/h");
    let shell = vec![
        "/h/google-cloud-sdk/bin".to_string(),
        "/opt/homebrew/bin".to_string(),
        "/usr/bin".to_string(),
    ];
    let tagged = cli_path_dirs_with(&shell, Some("/usr/bin:/bin:/usr/sbin:/sbin"), home);
    let path = crate::shell_path::join(&tagged);
    let dirs: Vec<&str> = path.split(':').collect();
    let at = |d: &str| dirs.iter().position(|x| *x == d).unwrap();
    assert_eq!(
        &dirs[..4],
        [
            "/h/.cargo/bin",
            "/h/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin"
        ]
    );
    assert!(at("/h/google-cloud-sdk/bin") > at("/usr/local/bin"));
    assert!(at("/h/google-cloud-sdk/bin") < at("/usr/bin"));
    let mut unique = dirs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), dirs.len(), "{path}");
    for system in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        assert!(dirs.contains(&system), "{path}");
    }
}

/// B03: until the login shell answers (never, in tests), agents get
/// exactly the PATH they got before.
#[test]
fn without_the_shell_path_cli_path_is_unchanged() {
    assert!(crate::shell_path::folders().is_none());
    let home = dirs::home_dir().unwrap_or_default();
    assert_eq!(
        cli_path(),
        crate::self_install::tool_path(std::env::var("PATH").ok().as_deref(), &home)
    );
}

/// Steering PR 3: a Codex app-server turn runs in each mode exactly as
/// `codex exec` does (build_codex), with the thread taking what exec takes
/// as flags.
#[test]
fn codex_app_server_policy_matches_exec_modes() {
    let p = |approval, reviewer, sandbox| CodexPolicy {
        approval_policy: approval,
        approvals_reviewer: reviewer,
        sandbox,
    };
    assert_eq!(
        codex_app_server_policy("plan"),
        p("never", None, "read-only")
    );
    for perm in ["default", "acceptEdits", ""] {
        assert_eq!(
            codex_app_server_policy(perm),
            p("never", None, "workspace-write"),
            "{perm}"
        );
    }
    assert_eq!(
        codex_app_server_policy("auto"),
        p("on-request", Some("auto_review"), "workspace-write")
    );
    assert_eq!(
        codex_app_server_policy("bypassPermissions"),
        p("never", None, "danger-full-access")
    );
    // Auto's writable roots and network (2026-11-12) ride the one mode
    // whose thread runs `workspace-write` with the automatic reviewer.
    for perm in [
        "plan",
        "default",
        "acceptEdits",
        "",
        "auto",
        "bypassPermissions",
    ] {
        let policy = codex_app_server_policy(perm);
        // The cwd of `spec` is no worktree, so auto gets the sandbox form;
        // the edit modes get the nested-`.git` profile alone (N22).
        let overrides = codex_auto_sandbox_overrides(&spec(perm, None, true));
        let edit = matches!(perm, "default" | "acceptEdits" | "");
        assert_eq!(
            !overrides.is_empty(),
            policy.approvals_reviewer.is_some() || edit,
            "{perm}"
        );
        if edit {
            assert!(codex_auto_uses_profile(&overrides), "{perm}");
        } else if !overrides.is_empty() {
            assert_eq!(policy.sandbox, "workspace-write");
        }
    }
}

#[test]
fn codex_app_server_keeps_execs_top_level_flags() {
    // A build chat: a linked worktree with a vault folder beside it.
    let root = tempfile::tempdir().unwrap();
    let (wt, _) = fake_linked_worktree(root.path(), "flags-wt");
    for perm in [
        "plan",
        "default",
        "acceptEdits",
        "auto",
        "bypassPermissions",
    ] {
        let mut run = spec(perm, None, true);
        run.cwd = wt.clone();
        run.extras.extra_dirs = vec![root.path().to_path_buf()];
        run.extras.effort = Some("low".into());
        run.extras.env = vec![(CODEX_COMPACT_LIMIT_ENV.into(), "90000".into())];
        let mut exec = Command::new("codex");
        build_codex(&mut exec, &run);
        let exec = args_of(&exec);
        let top = &exec[..exec.iter().position(|a| a == "exec").unwrap()];
        let mut server = Command::new("codex");
        build_codex_app_server(&mut server, &run);
        let server = args_of(&server);
        assert_eq!(
            server.last().map(String::as_str),
            Some("app-server"),
            "{perm}"
        );
        assert_eq!(&server[..server.len() - 1], top, "{perm}");
        assert!(top.iter().any(|a| a.starts_with("model_reasoning_effort=")));
        assert!(top
            .iter()
            .any(|a| a == "model_auto_compact_token_limit=90000"));
        // Auto in a linked worktree: the `hollow_auto` permission profile
        // (roots, network, `commondir` read-only) in place of the
        // `sandbox_workspace_write` keys; every other mode neither.
        assert_eq!(
            top.iter()
                .any(|a| a == "permissions.hollow_auto.network.enabled=true"),
            perm == "auto",
            "{perm}"
        );
        assert_eq!(
            top.iter()
                .any(|a| a.starts_with("permissions.hollow_auto.filesystem=")),
            matches!(perm, "auto" | "default" | "acceptEdits"),
            "{perm}"
        );
        assert!(
            !top.iter()
                .any(|a| a.starts_with("sandbox_workspace_write.")),
            "{perm}"
        );
    }
    // A resumed thread takes its sandbox in thread/resume, not in argv.
    let mut server = Command::new("codex");
    build_codex_app_server(&mut server, &spec("plan", Some("t-1"), true));
    let server = args_of(&server);
    assert!(!server.iter().any(|a| a.starts_with("sandbox_mode=")));
    assert!(!server.iter().any(|a| a == "t-1" || a == "hello"));
}

#[test]
fn codex_app_server_start_follows_the_spec() {
    let mut run = spec("plan", Some("t-1"), true);
    run.model = Some("gpt-6-luna".into());
    run.append_system_prompt = Some("context".into());
    let start = codex_app_server_start(&run, false);
    assert_eq!(start.prompt, contextual_prompt(AgentKind::Codex, &run));
    assert!(start.prompt.ends_with("hello"));
    assert_eq!(start.resume.as_deref(), Some("t-1"));
    assert_eq!(start.cwd, "/tmp");
    assert_eq!(start.model.as_deref(), Some("gpt-6-luna"));
    assert_eq!(start.approval_policy, "never");
    assert_eq!(start.sandbox, "read-only");
    assert!(!start.ephemeral);
    assert!(start.config.is_empty(), "effort rides argv, as in exec");

    // A one-shot never resumes and leaves no session behind.
    let mut run = spec("auto", Some("t-1"), false);
    run.model = Some(String::new());
    let start = codex_app_server_start(&run, false);
    assert_eq!(start.resume, None);
    assert_eq!(start.model, None);
    assert!(start.ephemeral);
    assert_eq!(start.approvals_reviewer.as_deref(), Some("auto_review"));
    // No worktree git folder granted: the sandbox as before.
    assert_eq!(start.sandbox, "workspace-write");

    // Auto in a linked worktree runs under the permission profile, which a
    // thread `sandbox` would override: none is named.
    let root = tempfile::tempdir().unwrap();
    let (wt, _) = fake_linked_worktree(root.path(), "start-wt");
    let mut run = spec("auto", None, true);
    run.cwd = wt;
    let mut server = Command::new("codex");
    build_codex_app_server(&mut server, &run);
    assert!(codex_command_uses_profile(&server));
    assert_eq!(codex_app_server_start(&run, true).sandbox, "");
    let mut run = spec("acceptEdits", None, true);
    run.cwd = std::env::temp_dir();
    assert_eq!(
        codex_app_server_start(&run, false).sandbox,
        "workspace-write"
    );
    // An edit mode's profile (N22) empties the thread sandbox like auto's;
    // a profile decision never empties plan's or bypass's.
    assert_eq!(codex_app_server_start(&run, true).sandbox, "");
    for perm in ["plan", "bypassPermissions"] {
        let run = spec(perm, None, true);
        assert_ne!(codex_app_server_start(&run, true).sandbox, "", "{perm}");
    }
}

/// Decide the profile once (2026-11-13): the app-server's thread reads the
/// decision from the built process argv, never from a second worktree check.
/// The worktree layout flips between building the process and starting the
/// thread (its `.git` file rewritten, then restored), in both directions;
/// the thread always agrees with the process, so no run ends up with no
/// profile and no sandbox.
#[test]
fn codex_app_server_thread_follows_the_process_profile_decision() {
    let root = tempfile::tempdir().unwrap();
    let (wt, _) = fake_linked_worktree(root.path(), "decide-once-wt");
    let mut run = spec("auto", None, true);
    run.cwd = wt.clone();
    let dot_git = wt.join(".git");
    let real = std::fs::read_to_string(&dot_git).unwrap();
    // The process is built while the layout holds: it carries the profile.
    let mut server = Command::new("codex");
    build_codex_app_server(&mut server, &run);
    assert!(codex_command_uses_profile(&server));
    // Another writer breaks the layout before the thread starts: a second
    // worktree check would now say "no profile" (the old route).
    std::fs::write(&dot_git, "gitdir: /nowhere/else\n").unwrap();
    assert!(!codex_auto_uses_profile(&codex_auto_sandbox_overrides(
        &run
    )));
    // The thread follows the process: the profile applies, no `sandbox`.
    let start = codex_app_server_start(&run, codex_command_uses_profile(&server));
    assert_eq!(start.sandbox, "");

    // The other way: built while the layout is broken (no profile on argv),
    // the layout restored before the thread starts. The thread names its
    // sandbox, so it is never left with neither.
    let mut server = Command::new("codex");
    build_codex_app_server(&mut server, &run);
    let argv = args_of(&server);
    assert!(!codex_command_uses_profile(&server), "{argv:?}");
    assert!(!argv.iter().any(|a| a.starts_with("default_permissions=")));
    std::fs::write(&dot_git, &real).unwrap();
    assert!(codex_auto_uses_profile(&codex_auto_sandbox_overrides(&run)));
    let start = codex_app_server_start(&run, codex_command_uses_profile(&server));
    assert_eq!(start.sandbox, "workspace-write");

    // Every pairing: an empty thread sandbox only ever goes with a process
    // that carries the profile.
    for perm in [
        "plan",
        "default",
        "acceptEdits",
        "auto",
        "bypassPermissions",
    ] {
        for profile in [false, true] {
            let sandbox = codex_app_thread_sandbox(perm, profile);
            assert!(
                !sandbox.is_empty()
                    || (profile && matches!(perm, "auto" | "acceptEdits" | "default")),
                "{perm} {profile}"
            );
        }
    }
    // A command with no `-c default_permissions=` (or the text elsewhere,
    // not as a `-c` value) reads as no profile.
    let mut odd = Command::new("codex");
    odd.arg("default_permissions=\"hollow_auto\"");
    assert!(!codex_command_uses_profile(&odd));
}

/// Mid-turn steering (2026-10-21): a steerable Claude chat turn reads its
/// messages from stdin, so its argv has no prompt and asks for stream-json
/// input; every other run (a one-shot, another agent, a chat turn without
/// the flag) keeps its prompt on the command line.
#[test]
fn a_steerable_claude_turn_takes_its_prompt_on_stdin() {
    let mut live = spec("acceptEdits", Some("old-session"), true);
    live.prompt = "PROMPT-ON-STDIN".into();
    live.extras.live_input = true;
    let args = args_of(&test_command(AgentKind::Claude, &live));
    assert_eq!(&args[..3], ["-p", "--input-format", "stream-json"]);
    assert!(
        !args.iter().any(|a| a.contains("PROMPT-ON-STDIN")),
        "{args:?}"
    );
    assert!(args.windows(2).any(|w| w == ["--resume", "old-session"]));
    assert!(args
        .windows(2)
        .any(|w| w == ["--output-format", "stream-json"]));
    assert!(live_claude_input(&live));

    let mut oneshot = live.clone();
    oneshot.interactive = false;
    assert!(!live_claude_input(&oneshot));
    // Q-R33: every other local run reads its prompt from stdin too.
    let args = args_of(&test_command(AgentKind::Claude, &oneshot));
    assert_eq!(&args[..2], ["-p", "--output-format"]);
    assert!(!args.iter().any(|a| a.contains("PROMPT-ON-STDIN")));
    assert_eq!(take_last_stdin_prompt().as_deref(), Some("PROMPT-ON-STDIN"));

    let mut plain = live.clone();
    plain.extras.live_input = false;
    let args = args_of(&test_command(AgentKind::Claude, &plain));
    assert_eq!(&args[..2], ["-p", "--output-format"]);
    assert!(!args.iter().any(|a| a.contains("PROMPT-ON-STDIN")));
    assert_eq!(
        stdin_prompt(AgentKind::Claude, &plain).as_deref(),
        Some("PROMPT-ON-STDIN")
    );
    // Grok shares Claude's stream format but has no live input channel; its
    // one-shot prompt is a file, the run's stdin (R5b).
    let args = args_of(&test_command(AgentKind::Grok, &live));
    assert!(
        !args.iter().any(|a| a.contains("PROMPT-ON-STDIN")),
        "{args:?}"
    );
    assert!(args
        .windows(2)
        .any(|w| w == ["--prompt-file", "/dev/stdin"]));
    assert!(take_last_stdin_prompt().is_some_and(|p| p.contains("PROMPT-ON-STDIN")));
    assert!(!args.iter().any(|a| a == "--input-format"));
}

/// Steering PR 3: a steerable Codex chat turn runs `codex app-server` (the
/// prompt goes in `turn/start`, never on argv; the run's MCP override stays
/// in front), while every other Codex run (a one-shot helper, a workflow
/// or teammate, a chat turn without the flag) still runs `codex exec`.
#[test]
fn a_steerable_codex_turn_runs_app_server_and_every_other_run_exec() {
    let mut live = spec("acceptEdits", Some("t-1"), true);
    live.prompt = "PROMPT-IN-TURN-START".into();
    live.extras.live_input = true;
    assert!(live_codex_input(&live));
    let args = args_of(&test_command(AgentKind::Codex, &live));
    assert_eq!(
        args.last().map(String::as_str),
        Some("app-server"),
        "{args:?}"
    );
    assert!(!args.iter().any(|a| a == "exec"), "{args:?}");
    assert!(
        !args
            .iter()
            .any(|a| a.contains("PROMPT-IN-TURN-START") || a == "t-1"),
        "{args:?}"
    );
    assert!(
        args.iter().any(|a| a.starts_with("mcp_servers=")),
        "{args:?}"
    );

    let mut oneshot = live.clone();
    oneshot.interactive = false;
    assert!(!live_codex_input(&oneshot));
    let args = args_of(&test_command(AgentKind::Codex, &oneshot));
    assert!(args.iter().any(|a| a == "exec"), "{args:?}");
    assert!(!args.iter().any(|a| a == "app-server"), "{args:?}");
    // Q-R33: `exec -` reads the prompt from stdin.
    assert_eq!(args.last().map(String::as_str), Some("-"), "{args:?}");
    assert!(!args.iter().any(|a| a.contains("PROMPT-IN-TURN-START")));
    assert!(take_last_stdin_prompt()
        .unwrap()
        .ends_with("PROMPT-IN-TURN-START"));

    let mut plain = live.clone();
    plain.extras.live_input = false;
    let args = args_of(&test_command(AgentKind::Codex, &plain));
    assert!(args.windows(2).any(|w| w == ["resume", "t-1"]), "{args:?}");
    assert!(!args.iter().any(|a| a == "app-server"), "{args:?}");
}
#[test]
fn codex_live_todo_snapshots_are_public_progress_not_tools() {
    let mut state = super::ParseState::default();
    for kind in ["item.started", "item.updated", "item.completed"] {
        let line = serde_json::json!({"type":kind,"item":{"id":"plan-1","type":"todo_list","items":[{"text":"Check routes","completed":true},{"text":"Review","completed":false}]}}).to_string();
        let events = super::parse_line(super::AgentKind::Codex, &line, &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            super::AgentEvent::Plan {
                completed: 1,
                total: 2
            }
        ));
    }
}

/// Codex auto for builds (2026-11-12): a fake linked worktree, as
/// `git worktree add` lays one out (`<wt>/.git` names
/// `<repo>/.git/worktrees/<name>`, whose `commondir` is `../..` and whose
/// `gitdir` points back at `<wt>/.git`; the common folder has `HEAD`,
/// `objects`, `refs` and `logs`), with no git needed. Returns the
/// worktree and the roots auto grants for it (its git folder, then the
/// common folder's `objects`, `refs` and `logs`), canonical.
fn fake_linked_worktree(root: &Path, name: &str) -> (PathBuf, Vec<PathBuf>) {
    let root = &root.canonicalize().unwrap();
    let common = root.join("repo").join(".git");
    let gitdir = common.join("worktrees").join(name);
    std::fs::create_dir_all(&gitdir).unwrap();
    for sub in ["objects", "refs/heads", "logs"] {
        std::fs::create_dir_all(common.join(sub)).unwrap();
    }
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
    let wt = root.join(name);
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    std::fs::write(
        gitdir.join("gitdir"),
        format!("{}\n", wt.join(".git").display()),
    )
    .unwrap();
    let common = common.canonicalize().unwrap();
    (
        wt.canonicalize().unwrap(),
        vec![
            gitdir.canonicalize().unwrap(),
            common.join("objects"),
            common.join("refs"),
            common.join("logs"),
        ],
    )
}

fn path_strs(paths: &[PathBuf]) -> Vec<&str> {
    paths.iter().map(|p| p.to_str().unwrap()).collect()
}

/// The `-c` values on a built command's argv, in order.
fn config_values(args: &[String]) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == "-c")
        .map(|w| w[1].clone())
        .collect()
}

/// The `sandbox_workspace_write.*` `-c` values, which only auto emits.
fn sandbox_write_values(args: &[String]) -> Vec<String> {
    config_values(args)
        .into_iter()
        .filter(|v| v.starts_with("sandbox_workspace_write."))
        .collect()
}

/// A `-c key=value` as Codex reads it: the value parsed as TOML.
fn toml_value(value: &str) -> (String, toml::Value) {
    let (key, raw) = value.split_once('=').unwrap();
    let doc: toml::Table = toml::from_str(&format!("v = {raw}")).unwrap();
    (key.to_string(), doc["v"].clone())
}

/// Only auto gets writable roots and network, on exec (fresh and
/// resumed) and on the app-server alike; plan, default, acceptEdits and
/// bypass get no `sandbox_workspace_write` key at all.
#[test]
fn codex_auto_adds_writable_roots_and_network_and_no_other_mode_does() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "feature-wt");
    let vault = root.path().join("vault ws");
    std::fs::create_dir_all(&vault).unwrap();
    let vault = vault.canonicalize().unwrap();
    for perm in [
        "plan",
        "default",
        "acceptEdits",
        "auto",
        "bypassPermissions",
    ] {
        for resume in [None, Some("th-1")] {
            let mut run = spec(perm, resume, true);
            run.cwd = wt.clone();
            run.extras.extra_dirs = vec![vault.clone()];
            let mut exec = Command::new("codex");
            build_codex(&mut exec, &run);
            let mut server = Command::new("codex");
            build_codex_app_server(&mut server, &run);
            for (what, args) in [("exec", args_of(&exec)), ("app-server", args_of(&server))] {
                let values = sandbox_write_values(&args);
                assert!(values.is_empty(), "{perm} {what} {resume:?}: {args:?}");
                if matches!(perm, "plan" | "bypassPermissions") {
                    assert!(!args.iter().any(|a| a.starts_with("default_permissions=")));
                    continue;
                }
                if perm != "auto" {
                    // N22: default and acceptEdits carry the nested-`.git`
                    // deny and nothing else: no roots, no network, no `-s`.
                    assert!(
                        !args
                            .iter()
                            .any(|a| a == "-s" || a.starts_with("sandbox_mode=")),
                        "{perm} {what} {resume:?}: {args:?}"
                    );
                    let cfg = config_values(&args);
                    let fs_value = cfg
                        .iter()
                        .find(|v| v.starts_with("permissions.hollow_auto.filesystem="))
                        .unwrap_or_else(|| panic!("{perm} {what}: no profile {args:?}"));
                    let fs = toml_value(fs_value).1;
                    let fs = fs.as_table().unwrap();
                    let mut got: Vec<String> = fs.keys().cloned().collect();
                    got.sort();
                    let mut want = vec![
                        format!("{}/*/.git", wt.display()),
                        format!("{}/*/**/.git", wt.display()),
                    ];
                    want.sort();
                    assert_eq!(got, want, "{perm} {what} {resume:?}");
                    assert!(fs.values().all(|v| v.as_str() == Some("deny")));
                    assert!(cfg
                        .iter()
                        .any(|v| v == "permissions.hollow_auto.network.enabled=false"));
                    continue;
                }
                // Auto in a linked worktree runs under the `hollow_auto`
                // profile instead of `workspace-write` (2026-11-12): no
                // `sandbox_workspace_write` key, no `-s`, no `sandbox_mode`.
                assert!(
                    !args
                        .iter()
                        .any(|a| a == "-s" || a.starts_with("sandbox_mode=")),
                    "{what} {resume:?}: {args:?}"
                );
                let cfg = config_values(&args);
                let find = |key: &str| {
                    cfg.iter()
                        .find(|v| v.starts_with(&format!("{key}=")))
                        .unwrap_or_else(|| panic!("{key} missing: {args:?}"))
                        .clone()
                };
                assert_eq!(
                    toml_value(&find("default_permissions")).1,
                    toml::Value::String("hollow_auto".into())
                );
                assert_eq!(
                    toml_value(&find("permissions.hollow_auto.extends")).1,
                    toml::Value::String(":workspace".into())
                );
                assert_eq!(
                    toml_value(&find("permissions.hollow_auto.network.enabled")).1,
                    toml::Value::Boolean(true)
                );
                let fs_value = find("permissions.hollow_auto.filesystem");
                let fs = toml_value(&fs_value).1;
                let fs = fs.as_table().unwrap();
                let denied: Vec<&str> = fs
                    .iter()
                    .filter(|(_, v)| v.as_str() == Some("deny"))
                    .map(|(k, _)| k.as_str())
                    .collect();
                let mut want_deny = vec![
                    format!("{}/*/.git", wt.display()),
                    format!("{}/*/**/.git", wt.display()),
                ];
                want_deny.sort();
                let mut got_deny: Vec<String> = denied.iter().map(|s| s.to_string()).collect();
                got_deny.sort();
                assert_eq!(got_deny, want_deny, "{what} {resume:?}");
                let mut want_write = vec![vault.to_str().unwrap()];
                want_write.extend(path_strs(&git_roots));
                let write: Vec<&str> = fs
                    .iter()
                    .filter(|(_, v)| v.as_str() == Some("write"))
                    .map(|(k, _)| k.as_str())
                    .collect();
                let mut want_sorted = want_write.clone();
                want_sorted.sort();
                let mut got_sorted = write.clone();
                got_sorted.sort();
                assert_eq!(got_sorted, want_sorted, "{what} {resume:?}");
                // The two files a rewrite of which would redirect the
                // person's own git are read-only inside the granted folder.
                let read: Vec<&str> = fs
                    .iter()
                    .filter(|(_, v)| v.as_str() == Some("read"))
                    .map(|(k, _)| k.as_str())
                    .collect();
                let gitdir = &git_roots[0];
                let mut want_read = vec![
                    gitdir.join("commondir").to_str().unwrap().to_string(),
                    gitdir.join("gitdir").to_str().unwrap().to_string(),
                    // Q-R6: never a config the turn can write, extension or not.
                    gitdir.join("config.worktree").to_str().unwrap().to_string(),
                ];
                want_read.sort();
                let mut got_read: Vec<String> = read.iter().map(|s| s.to_string()).collect();
                got_read.sort();
                assert_eq!(got_read, want_read, "{what} {resume:?}");
                // Global options: before the subcommand, so `exec resume`
                // (no `--add-dir`) gets them too.
                let sub = args
                    .iter()
                    .position(|a| a == "exec" || a == "app-server")
                    .unwrap();
                let at = args.iter().position(|a| *a == fs_value).unwrap();
                assert!(at < sub, "{what}: {args:?}");
            }
        }
    }
}

/// Roots are TOML strings Codex reads back exactly, whatever the path
/// holds (spaces, an apostrophe, a backslash, brackets); a relative or
/// repeated folder is dropped (Codex refuses a relative root), and so is
/// one with a double quote or a newline (Codex's seatbelt profile breaks on
/// it, see `codex_auto_writable_roots`); a folder that isn't a linked
/// worktree adds no git folder.
#[test]
fn codex_auto_roots_survive_awkward_paths_and_skip_what_codex_refuses() {
    let root = tempfile::tempdir().unwrap();
    let awkward = root.path().join("Jo's [build] dir \\ with spaces");
    std::fs::create_dir_all(&awkward).unwrap();
    let awkward = awkward.canonicalize().unwrap();
    let quoted = root.path().join("a \"quoted\" dir");
    let newline = root.path().join("line\nbreak");
    for dir in [&quoted, &newline] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let plain = root.path().join("plain");
    std::fs::create_dir_all(plain.join(".git")).unwrap(); // a main checkout
    let mut run = spec("auto", None, true);
    run.cwd = plain.clone();
    run.extras.extra_dirs = vec![
        awkward.clone(),
        PathBuf::from("relative/folder"),
        quoted.clone(),
        newline.clone(),
        awkward.clone(),
    ];
    assert_eq!(codex_auto_writable_roots(&run), [awkward.to_str().unwrap()]);
    let values = codex_auto_sandbox_overrides(&run);
    let (_, roots) = toml_value(&values[0]);
    assert_eq!(
        roots.as_array().unwrap()[0].as_str(),
        Some(awkward.to_str().unwrap())
    );

    // No extra folder and no worktree: network alone.
    let mut bare = spec("auto", None, true);
    bare.cwd = plain;
    assert_eq!(
        codex_auto_sandbox_overrides(&bare),
        ["sandbox_workspace_write.network_access=true"]
    );
    // A `.git` file that names no worktree (a submodule's has no
    // `commondir`) or is malformed adds nothing.
    let sub = root.path().join("sub");
    let modules = root.path().join("super/.git/modules/sub");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(".git"), format!("gitdir: {}\n", modules.display())).unwrap();
    bare.cwd = sub.clone();
    assert!(codex_auto_writable_roots(&bare).is_empty());
    std::fs::write(sub.join(".git"), "not a pointer\n").unwrap();
    assert!(codex_auto_writable_roots(&bare).is_empty());
    // A relative `gitdir:` resolves against the worktree.
    let (_, git_roots) = fake_linked_worktree(root.path(), "rel-wt");
    let rel = root.path().join("rel-wt");
    std::fs::write(rel.join(".git"), "gitdir: ../repo/.git/worktrees/rel-wt\n").unwrap();
    bare.cwd = rel;
    assert_eq!(codex_auto_writable_roots(&bare), path_strs(&git_roots));
}

/// Adversarial review 2026-11-12 (finding 1a): `<cwd>/.git` and every file
/// it leads to can be written by an earlier auto turn, so each forged or
/// odd layout grants nothing; only the real layout grants exactly its
/// git folder plus the common folder's `objects`, `refs` and `logs`.
#[test]
fn a_forged_or_odd_dot_git_grants_codex_auto_nothing() {
    let grants = |cwd: &Path, extra: &[PathBuf]| -> Vec<PathBuf> {
        let mut run = spec("auto", None, true);
        run.cwd = cwd.to_path_buf();
        run.extras.extra_dirs = extra.to_vec();
        codex_auto_writable_roots(&run)
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| !extra.contains(p))
            .collect()
    };
    let write = |path: &Path, text: &str| std::fs::write(path, text).unwrap();
    let home = dirs::home_dir().unwrap().canonicalize().unwrap();

    // The real layout: exactly the git folder, objects, refs and logs.
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt");
    assert_eq!(grants(&wt, &[]), git_roots);
    let common = git_roots[1].parent().unwrap().to_path_buf();
    let gitdir = git_roots[0].clone();
    assert!(
        !grants(&wt, &[]).contains(&common),
        "never the common folder"
    );

    // 1. The plain-folder attack: `.git` names a folder the agent made
    // inside cwd, whose commondir is the home folder.
    let plain = tempfile::tempdir().unwrap();
    let cwd = plain.path().canonicalize().unwrap();
    std::fs::create_dir_all(cwd.join("x")).unwrap();
    write(&cwd.join(".git"), "gitdir: x\n");
    write(&cwd.join("x/commondir"), &format!("{}\n", home.display()));
    write(
        &cwd.join("x/gitdir"),
        &format!("{}\n", cwd.join(".git").display()),
    );
    assert!(grants(&cwd, &[]).is_empty());
    // The same with a `worktrees/<name>` shape, still inside cwd.
    let inside = cwd.join("r/.git/worktrees/w");
    std::fs::create_dir_all(&inside).unwrap();
    for sub in ["objects", "refs"] {
        std::fs::create_dir_all(cwd.join("r/.git").join(sub)).unwrap();
    }
    write(&cwd.join("r/.git/HEAD"), "ref: refs/heads/main\n");
    write(&inside.join("commondir"), "../..\n");
    write(
        &inside.join("gitdir"),
        &format!("{}\n", cwd.join(".git").display()),
    );
    write(
        &cwd.join(".git"),
        &format!("gitdir: {}\n", inside.display()),
    );
    assert!(grants(&cwd, &[]).is_empty(), "a git folder inside cwd");

    // 2. Outside cwd, but `commondir` names the home folder or another
    // repository instead of the folder the layout says.
    let elsewhere = tempfile::tempdir().unwrap();
    let (other_wt, other_roots) = fake_linked_worktree(elsewhere.path(), "other");
    let other_common = other_roots[1].parent().unwrap().to_path_buf();
    for named in [home.clone(), other_common.clone()] {
        write(&gitdir.join("commondir"), &format!("{}\n", named.display()));
        assert!(grants(&wt, &[]).is_empty(), "{}", named.display());
    }
    write(&gitdir.join("commondir"), "../..\n");
    assert_eq!(grants(&wt, &[]), git_roots);

    // 3. A `gitdir:` naming another repository's real worktree: its
    // back-pointer names that worktree, not this cwd.
    write(
        &wt.join(".git"),
        &format!("gitdir: {}\n", other_roots[0].display()),
    );
    assert!(grants(&wt, &[]).is_empty());
    assert_eq!(grants(&other_wt, &[]), other_roots);
    // A back-pointer mismatch in this repository's own folder.
    write(&wt.join(".git"), &format!("gitdir: {}\n", gitdir.display()));
    write(
        &gitdir.join("gitdir"),
        &format!("{}\n", other_wt.join(".git").display()),
    );
    assert!(grants(&wt, &[]).is_empty(), "back-pointer mismatch");
    write(
        &gitdir.join("gitdir"),
        &format!("{}\n", wt.join(".git").display()),
    );
    assert_eq!(grants(&wt, &[]), git_roots);

    // 4. A symlinked `<cwd>/.git` (to a real worktree's pointer file).
    let linked = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(wt.join(".git"), linked.path().join(".git")).unwrap();
    assert!(grants(linked.path(), &[]).is_empty());

    // 5. A common folder that isn't a real git folder, or whose config
    // turns on per-worktree config (a writable file would then be config).
    std::fs::rename(common.join("HEAD"), common.join("HEAD.off")).unwrap();
    assert!(grants(&wt, &[]).is_empty(), "no HEAD");
    std::fs::rename(common.join("HEAD.off"), common.join("HEAD")).unwrap();
    write(
        &common.join("config"),
        "[extensions]\n\tworktreeConfig = true\n",
    );
    assert!(grants(&wt, &[]).is_empty(), "worktreeConfig");
    write(&common.join("config"), "[core]\n\tbare = false\n");
    assert_eq!(grants(&wt, &[]), git_roots);

    // 6. A repository around Hollow's own folder (the vault): nothing.
    let vault_folder = common.parent().unwrap().join("vault/ws");
    std::fs::create_dir_all(&vault_folder).unwrap();
    assert!(grants(&wt, &[vault_folder]).is_empty());

    // 7. `objects` symlinked out of the common folder.
    std::fs::rename(common.join("objects"), common.join("objects.real")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), common.join("objects")).unwrap();
    assert!(grants(&wt, &[]).is_empty(), "symlinked objects");
}

/// Delta review 2026-11-12: an auto turn may write its worktree git folder,
/// so it can point `commondir` at a folder whose config runs a program
/// (`core.fsmonitor`). Once armed (as `build_with_codex_servers` does for a
/// local auto run), the guard puts `commondir` and `gitdir` back byte for
/// byte (a rewrite, a symlink or a folder in their place), and Hollow's own
/// git in that worktree is pinned to the recorded folders and never runs
/// the payload, even before the guard's next pass. Without the arm, plain
/// git runs it: the test's own control.
#[test]
fn a_tampered_worktree_commondir_is_put_back_and_hollows_git_ignores_it() {
    let git = |dir: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        out.status.success()
    };
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]));
    assert!(git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]));
    let wt = root.join("wt");
    assert!(git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()]
    ));
    let common = repo.join(".git");
    let gitdir = common.join("worktrees/wt");
    let commondir = std::fs::read(gitdir.join("commondir")).unwrap();
    let back = std::fs::read(gitdir.join("gitdir")).unwrap();

    // A decoy common folder whose config runs a payload.
    let evil = root.join("evil");
    let pwned = root.join("PWNED");
    std::fs::create_dir_all(evil.join("refs/heads")).unwrap();
    std::fs::create_dir_all(evil.join("objects")).unwrap();
    std::fs::copy(common.join("HEAD"), evil.join("HEAD")).unwrap();
    std::fs::write(
        evil.join("config"),
        format!(
            "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"touch '{}'; false\"\n",
            pwned.display()
        ),
    )
    .unwrap();
    let tamper = || {
        std::fs::write(gitdir.join("commondir"), format!("{}\n", evil.display())).unwrap();
    };

    // Control: unguarded, plain git in the tampered worktree runs it.
    tamper();
    git(&wt, &["status", "--porcelain"]);
    assert!(pwned.exists(), "the fixture's payload must be live");
    std::fs::remove_file(&pwned).unwrap();
    std::fs::write(gitdir.join("commondir"), &commondir).unwrap();
    assert_eq!(restore_codex_worktree_git(&wt), None, "not armed yet");

    arm_codex_worktree_guard(&wt, &[]);
    let pinned = restore_codex_worktree_git(&wt.join("sub"))
        .or_else(|| restore_codex_worktree_git(&wt))
        .expect("armed");
    assert_eq!(pinned, (gitdir.clone(), common.clone()));

    // Hollow's git (`build_workspaces::git_command`) while tampered.
    tamper();
    let mut cmd = crate::build_workspaces::git_command(&wt);
    cmd.args(["status", "--porcelain"]);
    let out = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move { cmd.output().await })
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!pwned.exists(), "Hollow's git must not run the payload");
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), commondir);

    // The guard's pass: a rewrite, a symlink and a folder put back.
    tamper();
    std::fs::remove_file(gitdir.join("gitdir")).unwrap();
    std::os::unix::fs::symlink(&evil, gitdir.join("gitdir")).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 2);
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), commondir);
    assert_eq!(std::fs::read(gitdir.join("gitdir")).unwrap(), back);
    std::fs::remove_file(gitdir.join("commondir")).unwrap();
    std::fs::create_dir_all(gitdir.join("commondir/x")).unwrap();
    assert!(guard_pass(Some(&gitdir)) >= 1);
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), commondir);
    // Unguarded plain git is safe again too.
    git(&wt, &["status", "--porcelain"]);
    assert!(!pwned.exists());
    // A removed worktree is forgotten.
    assert!(git(
        &repo,
        &["worktree", "remove", "--force", wt.to_str().unwrap()]
    ));
    guard_pass(Some(&gitdir));
    assert!(!WORKTREE_GIT_SNAPSHOTS.lock().unwrap().contains_key(&gitdir));
}

/// Final review 2026-11-12, finding 1: `commondir` naming a symlink the
/// agent can repoint resolves to the right folder today and to a decoy
/// later, so its text must be git's own (`../..` or the common folder):
/// anything else grants nothing, and an auto spawn (the arm) writes git's
/// own text back when the layout holds without it, then guards that.
#[test]
fn a_commondir_through_a_symlink_is_refused_then_repaired_never_snapshotted() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt");
    let gitdir = git_roots[0].clone();
    let common = gitdir.parent().unwrap().parent().unwrap().to_path_buf();
    let link = root.path().canonicalize().unwrap().join("link");
    std::os::unix::fs::symlink(&common, &link).unwrap();
    let roots = |wt: &Path| {
        let mut run = spec("auto", None, true);
        run.cwd = wt.to_path_buf();
        codex_auto_writable_roots(&run)
    };
    assert_eq!(roots(&wt), path_strs(&git_roots));
    // The indirection resolves to the real folder, and is still refused.
    std::fs::write(gitdir.join("commondir"), format!("{}\n", link.display())).unwrap();
    assert!(roots(&wt).is_empty(), "a commondir through a symlink");
    // Git's other spelling (the folder itself) is fine.
    std::fs::write(gitdir.join("commondir"), format!("{}\n", common.display())).unwrap();
    assert_eq!(roots(&wt), path_strs(&git_roots));
    // The back-pointer through a symlink is refused the same way.
    let wt_link = root.path().canonicalize().unwrap().join("wt-link");
    std::os::unix::fs::symlink(&wt, &wt_link).unwrap();
    std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        gitdir.join("gitdir"),
        format!("{}\n", wt_link.join(".git").display()),
    )
    .unwrap();
    assert!(roots(&wt).is_empty(), "a back-pointer through a symlink");

    // The arm repairs both to git's own text and snapshots that, even with
    // the link repointed at a decoy: the layout, not `commondir`, names
    // the common folder.
    std::fs::write(gitdir.join("commondir"), format!("{}\n", link.display())).unwrap();
    let decoy = root.path().join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&decoy, &link).unwrap();
    arm_codex_worktree_guard(&wt, &[]);
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), b"../..\n");
    assert_eq!(
        std::fs::read(gitdir.join("gitdir")).unwrap(),
        format!("{}\n", wt.join(".git").display()).into_bytes()
    );
    assert_eq!(roots(&wt), path_strs(&git_roots));
    let snapshot = WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap()
        .get(&gitdir)
        .cloned()
        .expect("armed");
    assert_eq!(snapshot.commondir, b"../..\n");
    assert_eq!(snapshot.common, common);
    forget_codex_worktree_guard(&wt);
}

/// Final review finding 2: the guard runs unsandboxed, so it must never
/// write through a symlink the agent plants in place of `commondir` or
/// `gitdir` (`ln -sf ~/.ssh/authorized_keys <gitdir>/commondir`): the
/// outside file stays as it was and the entry becomes git's own file.
#[test]
fn the_guard_never_writes_through_a_planted_symlink() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt");
    let gitdir = git_roots[0].clone();
    arm_codex_worktree_guard(&wt, &[]);
    let outside = root.path().join("authorized_keys");
    std::fs::write(&outside, "ssh-ed25519 KEEP-ME\n").unwrap();
    for name in ["commondir", "gitdir"] {
        std::fs::remove_file(gitdir.join(name)).unwrap();
        std::os::unix::fs::symlink(&outside, gitdir.join(name)).unwrap();
    }
    assert_eq!(guard_pass(Some(&gitdir)), 2);
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "ssh-ed25519 KEEP-ME\n"
    );
    for name in ["commondir", "gitdir"] {
        let meta = std::fs::symlink_metadata(gitdir.join(name)).unwrap();
        assert!(meta.file_type().is_file(), "{name} is a plain file again");
    }
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), b"../..\n");
    // A symlink to a folder, and no temp file left behind.
    std::fs::remove_file(gitdir.join("commondir")).unwrap();
    std::os::unix::fs::symlink(root.path(), gitdir.join("commondir")).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 1);
    assert!(root.path().join("authorized_keys").is_file());
    let leftovers = std::fs::read_dir(&gitdir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".hollow-guard-")
        })
        .count();
    assert_eq!(leftovers, 0);
    // Snapshot reads never follow either: a symlinked `commondir` reads
    // as nothing, so the layout isn't granted until it is git's own again.
    std::fs::remove_file(gitdir.join("commondir")).unwrap();
    std::os::unix::fs::symlink(
        gitdir.parent().unwrap().parent().unwrap(),
        gitdir.join("commondir"),
    )
    .unwrap();
    assert!(checked_linked_worktree(&wt, &[]).is_none());
    forget_codex_worktree_guard(&wt);
}

/// Final review finding 3: before Hollow's own `git worktree remove` or
/// `prune`, the worktree's snapshot is dropped, so the guard can't
/// recreate files git is deleting (`build_workspaces::remove_for_unless`).
#[test]
fn forgetting_a_worktree_drops_its_guard() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt-a");
    let (wt_b, roots_b) = fake_linked_worktree(root.path(), "wt-b");
    arm_codex_worktree_guard(&wt, &[]);
    arm_codex_worktree_guard(&wt_b, &[]);
    let armed = |gitdir: &PathBuf| WORKTREE_GIT_SNAPSHOTS.lock().unwrap().contains_key(gitdir);
    assert!(armed(&git_roots[0]) && armed(&roots_b[0]));
    // Remove: that worktree only.
    forget_codex_worktree_guard(&wt);
    assert!(!armed(&git_roots[0]) && armed(&roots_b[0]));
    assert_eq!(restore_codex_worktree_git(&wt), None);
    // Prune from the repository: only worktrees whose folder is gone.
    let repo = root.path().canonicalize().unwrap().join("repo");
    forget_codex_worktree_guard(&repo);
    assert!(armed(&roots_b[0]), "a live sibling stays guarded");
    std::fs::remove_dir_all(&wt_b).unwrap();
    forget_codex_worktree_guard(&repo);
    assert!(!armed(&roots_b[0]));
}

/// Review 2026-11-12, item 1: a turn that, while no guard ran, broke BOTH
/// `commondir` (to a decoy whose config runs a payload) and `gitdir`
/// (garbage) defeats a repair that needs the back-pointer. With no
/// snapshot, Hollow's own git is still pinned from `.git` alone and runs
/// no payload; the next auto spawn repairs both files from `.git` alone;
/// a worktree whose layout doesn't hold gets no git from Hollow at all.
#[test]
fn both_git_files_broken_hollows_git_runs_no_payload_and_the_spawn_repairs_them() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap()
            .status
            .success()
    };
    let hollow_git = |dir: &Path| {
        let mut cmd = crate::build_workspaces::git_command(dir);
        cmd.args(["status", "--porcelain"]);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move { cmd.output().await })
            .unwrap()
    };
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]));
    assert!(git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]));
    let wt = root.join("wt");
    assert!(git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()]
    ));
    let common = repo.join(".git");
    let gitdir = common.join("worktrees/wt");
    let evil = root.join("evil");
    let pwned = root.join("PWNED");
    std::fs::create_dir_all(evil.join("refs/heads")).unwrap();
    std::fs::create_dir_all(evil.join("objects")).unwrap();
    std::fs::copy(common.join("HEAD"), evil.join("HEAD")).unwrap();
    std::fs::write(
        evil.join("config"),
        format!(
            "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"touch '{}'; false\"\n",
            pwned.display()
        ),
    )
    .unwrap();
    std::fs::write(gitdir.join("commondir"), format!("{}\n", evil.display())).unwrap();
    std::fs::write(gitdir.join("gitdir"), "garbage\n").unwrap();
    assert_eq!(restore_codex_worktree_git(&wt), None, "no snapshot");
    assert!(
        worktree_layout(&wt, &[]).is_none(),
        "the back-pointer is broken"
    );

    // Control: plain git runs the payload.
    git(&wt, &["status", "--porcelain"]);
    assert!(pwned.exists(), "the fixture's payload must be live");
    std::fs::remove_file(&pwned).unwrap();

    // Hollow's git, pinned from `.git` alone.
    assert_eq!(
        worktree_git_pins(&wt),
        WorktreeGitPins::Pinned(gitdir.clone(), common.clone(), wt.clone())
    );
    let out = hollow_git(&wt);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!pwned.exists(), "Hollow's git must not run the payload");

    // The next auto spawn repairs both files and grants.
    arm_codex_worktree_guard(&wt, &[]);
    assert_eq!(std::fs::read(gitdir.join("commondir")).unwrap(), b"../..\n");
    assert_eq!(
        std::fs::read(gitdir.join("gitdir")).unwrap(),
        format!("{}\n", wt.join(".git").display()).into_bytes()
    );
    assert!(checked_linked_worktree(&wt, &[]).is_some());
    git(&wt, &["status", "--porcelain"]);
    assert!(!pwned.exists(), "plain git is safe again");
    forget_codex_worktree_guard(&wt);

    // A plain checkout is git as before; a worktree whose common folder
    // isn't a real git folder gets none (git fails, nothing is read).
    assert_eq!(worktree_git_pins(&repo), WorktreeGitPins::NotLinked);
    std::fs::rename(common.join("HEAD"), common.join("HEAD.off")).unwrap();
    assert_eq!(worktree_git_pins(&wt), WorktreeGitPins::Refused);
    assert!(!hollow_git(&wt).status.success());
    std::fs::rename(common.join("HEAD.off"), common.join("HEAD")).unwrap();
}

/// Review item 2: a pass skips a snapshot `forget` dropped after the pass
/// listed it (the check is under the lock, right before the write).
#[test]
fn a_guard_pass_never_restores_a_forgotten_worktree() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt");
    let gitdir = git_roots[0].clone();
    arm_codex_worktree_guard(&wt, &[]);
    forget_codex_worktree_guard(&wt);
    std::fs::remove_file(gitdir.join("commondir")).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 0);
    assert!(
        !gitdir.join("commondir").exists(),
        "a forgotten worktree is left to git"
    );
}

/// Review 2026-11-12 (index route): an auto turn can write the worktree's
/// index, add a submodule entry and give it a `.git` naming a decoy whose
/// config runs a `core.fsmonitor` program; plain `git status` then starts a
/// child git there and runs it. Hollow's git (`build_workspaces::git_raw`,
/// what `build_changes` and `worktree_hygiene` run) never does, and the
/// commit check still sees a staged submodule.
#[test]
fn a_planted_submodule_in_the_index_runs_nothing_from_hollows_git() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap()
    };
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]).status.success());
    assert!(git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"])
        .status
        .success());
    let wt = root.join("wt");
    let wt_arg = wt.to_str().unwrap();
    assert!(git(&repo, &["worktree", "add", "-q", "-b", "feat", wt_arg])
        .status
        .success());
    let sha = String::from_utf8(git(&wt, &["rev-parse", "HEAD"]).stdout).unwrap();
    let evil = root.join("evil");
    let pwned = root.join("PWNED");
    std::fs::create_dir_all(evil.join("refs/heads")).unwrap();
    std::fs::create_dir_all(evil.join("objects")).unwrap();
    std::fs::copy(repo.join(".git/HEAD"), evil.join("HEAD")).unwrap();
    std::fs::write(
        evil.join("config"),
        format!(
            "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"touch '{}'; false\"\n[filter \"x\"]\n\tclean = \"touch '{}'; cat\"\n",
            pwned.display(),
            pwned.display()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(wt.join("sub")).unwrap();
    std::fs::write(wt.join("sub/.gitattributes"), "* filter=x\n").unwrap();
    std::fs::write(wt.join("sub/file"), "hi\n").unwrap();
    let cacheinfo = format!("160000,{},sub", sha.trim());
    assert!(
        git(&wt, &["update-index", "--add", "--cacheinfo", &cacheinfo])
            .status
            .success()
    );
    std::fs::create_dir_all(wt.join("sub")).unwrap();
    std::fs::write(wt.join("sub/.git"), format!("gitdir: {}\n", evil.display())).unwrap();

    // Control: plain git runs the payload.
    git(&wt, &["status", "--porcelain", "--untracked-files=no"]);
    assert!(pwned.exists(), "the fixture's payload must be live");
    std::fs::remove_file(&pwned).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let hollow = |args: &'static [&'static str]| {
        let wt = wt.clone();
        rt.block_on(async move {
            crate::build_workspaces::git_raw(&wt, args, Duration::from_secs(30), 1 << 20)
                .await
                .unwrap()
        })
    };
    for args in [
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ][..],
        &["status", "--porcelain", "-unormal"][..],
        &["diff", "--stat"][..],
    ] {
        let out = hollow(args);
        assert_eq!(out.code, Some(0), "{args:?}: {}", out.stderr);
        assert!(
            !pwned.exists(),
            "Hollow's {args:?} must not run the payload"
        );
    }
    // The commit check still sees the staged submodule, and runs nothing.
    let staged = hollow(&["diff", "--cached", "--quiet", "--ignore-submodules=none"]);
    assert_eq!(staged.code, Some(1), "a staged submodule counts");
    assert!(!pwned.exists());
    // The Commit path (`build_changes`: add -A, then commit) runs nothing
    // either, and commits the planted entry as it is.
    let add = hollow(&["add", "-A"]);
    assert_eq!(add.code, Some(0), "{}", add.stderr);
    assert!(!pwned.exists(), "Hollow's add -A must not run the payload");
    let commit = hollow(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=t",
        "commit",
        "-q",
        "-m",
        "one",
    ]);
    assert_eq!(commit.code, Some(0), "{}", commit.stderr);
    assert!(!pwned.exists(), "Hollow's commit must not run the payload");
}

/// A commit whose only change is a real submodule's new commit still goes
/// through Hollow's flags (`diff.ignoreSubmodules=all` doesn't stop `git
/// commit` committing the index).
#[test]
fn a_submodule_only_commit_goes_through_hollows_git() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(["-c", "protocol.file.allow=always"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .output()
            .unwrap()
            .status
            .success()
    };
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let (inner, outer) = (root.join("inner"), root.join("outer"));
    for dir in [&inner, &outer] {
        std::fs::create_dir_all(dir).unwrap();
        assert!(git(dir, &["init", "-q", "-b", "main"]));
    }
    assert!(git(&inner, &["commit", "-q", "--allow-empty", "-m", "i1"]));
    assert!(git(&outer, &["submodule", "-q", "add", "../inner", "s"]));
    assert!(git(&outer, &["commit", "-q", "-m", "add"]));
    assert!(git(
        &outer.join("s"),
        &["commit", "-q", "--allow-empty", "-m", "i2"]
    ));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hollow = |args: &'static [&'static str]| {
        let outer = outer.clone();
        rt.block_on(async move {
            crate::build_workspaces::git_raw(&outer, args, Duration::from_secs(30), 1 << 20)
                .await
                .unwrap()
        })
    };
    assert_eq!(hollow(&["add", "-A"]).code, Some(0));
    let staged = hollow(&["diff", "--cached", "--quiet", "--ignore-submodules=none"]);
    assert_eq!(staged.code, Some(1), "the bump is staged");
    let commit = hollow(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=t",
        "commit",
        "-q",
        "-m",
        "bump",
    ]);
    assert_eq!(commit.code, Some(0), "{}", commit.stderr);
    let last = hollow(&["log", "-1", "--format=%s"]);
    assert_eq!(String::from_utf8_lossy(&last.stdout).trim(), "bump");
}

/// Review items 2 and 4: the repair's sibling exception reads a relative
/// back-pointer against the git folder; `worktreeConfig` is read as the
/// `[extensions]` key, not the word anywhere.
#[test]
fn the_sibling_exception_reads_a_relative_back_pointer_and_worktree_config_is_a_key() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "wt");
    let gitdir = git_roots[0].clone();
    // Another live worktree whose `.git` claims this git folder, named by
    // a relative back-pointer: left alone, nothing granted.
    let sibling = root.path().canonicalize().unwrap().join("sibling");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .unwrap();
    std::fs::write(gitdir.join("gitdir"), "../../../../sibling/.git\n").unwrap();
    assert_eq!(
        gitdir
            .join("../../../../sibling/.git")
            .canonicalize()
            .unwrap(),
        sibling.join(".git")
    );
    arm_codex_worktree_guard(&wt, &[]);
    assert_eq!(
        std::fs::read(gitdir.join("gitdir")).unwrap(),
        b"../../../../sibling/.git\n",
        "a sibling's claim is not rewritten"
    );
    assert!(checked_linked_worktree(&wt, &[]).is_none());
    // Without the claim, the same relative pointer is repaired.
    std::fs::remove_file(sibling.join(".git")).unwrap();
    arm_codex_worktree_guard(&wt, &[]);
    assert!(checked_linked_worktree(&wt, &[]).is_some());
    forget_codex_worktree_guard(&wt);

    for (config, on) in [
        ("[extensions]\n\tworktreeConfig = true\n", true),
        ("[Extensions]\n\tWORKTREECONFIG\n", true),
        ("[extensions] worktreeconfig = yes\n", true),
        ("[extensions]\n\tworktreeConfig = false\n", false),
        ("[core]\n\tworktreeConfig = true\n", false),
        ("[remote \"worktreeconfig\"]\n\turl = x\n", false),
        (
            "[extensions]\n\tobjectFormat = sha1 # worktreeConfig = true\n",
            false,
        ),
    ] {
        assert_eq!(worktree_config_on(config), on, "{config:?}");
    }
}

/// Q-R6 (2026-11-13): the layout check reads `<common>/config` as text and
/// doesn't follow `[include]`/`[includeIf]`. That is git's own rule for
/// `extensions.*` (the repository format is read without includes), so an
/// included `worktreeConfig` doesn't make `config.worktree` a config. Real
/// git; the profile also keeps `config.worktree` read-only regardless.
#[test]
fn an_included_worktree_config_extension_is_not_read_by_git() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let repo = root_path.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]).status.success());
    assert!(git(&repo, &["commit", "-q", "--allow-empty", "-m", "i"])
        .status
        .success());
    let wt = root_path.join("wt");
    assert!(git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()])
        .status
        .success());
    let common = repo.join(".git");
    let gitdir = common.join("worktrees/wt");
    std::fs::write(
        gitdir.join("config.worktree"),
        "[core]\n\thooksPath = /tmp/hollow-planted-hooks\n",
    )
    .unwrap();
    let hooks_path = || {
        String::from_utf8_lossy(&git(&wt, &["config", "--get", "core.hooksPath"]).stdout)
            .trim()
            .to_string()
    };
    assert!(git(&repo, &["config", "core.repositoryformatversion", "1"])
        .status
        .success());
    let base = std::fs::read_to_string(common.join("config")).unwrap();
    let included = root_path.join("included.cfg");
    std::fs::write(&included, "[extensions]\n\tworktreeConfig = true\n").unwrap();
    for section in ["[include]", "[includeIf \"gitdir:/\"]"] {
        let config = format!("{base}{section}\n\tpath = {}\n", included.display());
        std::fs::write(common.join("config"), &config).unwrap();
        assert!(!worktree_config_on(&config), "{section}");
        assert_eq!(
            hooks_path(),
            "",
            "{section}: git ignores the included extension"
        );
        assert!(dot_structure(&wt).is_some(), "{section}");
    }
    // Control: the extension in the common config itself is read by git,
    // and refused by the layout check.
    std::fs::write(
        common.join("config"),
        format!("{base}[extensions]\n\tworktreeConfig = true\n"),
    )
    .unwrap();
    assert_eq!(hooks_path(), "/tmp/hollow-planted-hooks");
    assert!(dot_structure(&wt).is_none());
}

/// A remote auto run (the Mac mini runner) writes the runner's copy of the
/// vault folder, and nothing of this Mac's reaches its argv: not the local
/// extra folders, not the local worktree's git folders.
#[test]
fn a_remote_codex_auto_run_gets_only_the_runners_paths() {
    let root = tempfile::tempdir().unwrap();
    let (wt, git_roots) = fake_linked_worktree(root.path(), "remote-wt");
    let local_vault = root.path().join("vault-ws");
    std::fs::create_dir_all(&local_vault).unwrap();
    let runner_folder = "/Users/runner/hollow-remote/folders/ws-1";
    let mut run = spec("auto", None, true);
    run.cwd = wt.clone();
    run.extras.extra_dirs = vec![local_vault.clone()];
    run.extras.remote = Some(crate::remote_session::RemoteTarget {
        host: "runner-host".into(),
        home: "/Users/runner".into(),
        cwd: "/Users/runner/hollow-remote/sessions/s-1".into(),
        git_worktree: true,
        folder: Some(crate::remote_folder::FolderSync {
            local: local_vault.clone(),
            dir: runner_folder.into(),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(codex_auto_writable_roots(&run), [runner_folder]);
    // As `build_with_codex_servers` hands it to `build_codex`.
    let mut host = run.clone();
    host.extras.extra_dirs = remote_extra_dirs(run.extras.remote.as_ref().unwrap());
    for args in [
        {
            let mut c = Command::new("codex");
            build_codex(&mut c, &host);
            args_of(&c)
        },
        {
            let mut c = Command::new("codex");
            build_codex_app_server(&mut c, &host);
            args_of(&c)
        },
    ] {
        let values = sandbox_write_values(&args);
        assert_eq!(
            values,
            [
                format!("sandbox_workspace_write.writable_roots=[\"{runner_folder}\"]"),
                "sandbox_workspace_write.network_access=true".to_string(),
            ]
        );
        let joined = args.join(" ");
        for here in git_roots.iter().chain([&local_vault]) {
            assert!(!joined.contains(here.to_str().unwrap()), "{joined}");
        }
    }
    // A runner session that isn't a git worktree (Phase 3, the vault folder
    // is the cwd itself) has no extra folder there.
    run.extras.remote.as_mut().unwrap().git_worktree = false;
    assert!(codex_auto_writable_roots(&run).is_empty());
}

/// The live sandbox fixture (2026-11-12): the real `codex sandbox`
/// (seatbelt) with the exact sandbox `-c` values `build_codex` emits for an
/// auto build chat lets the agent write its worktree and its vault folder,
/// commit and branch in the linked worktree and reach a loopback TCP
/// listener; it still refuses a write outside the roots and the shared
/// repository's hooks and config, and the same command under
/// acceptEdits's argv refuses the vault folder, the commit and the
/// network. The fixture alone adds `exclude_slash_tmp` and
/// `exclude_tmpdir_env_var` (its temp folders sit under `/tmp`, which
/// `workspace-write` would otherwise allow). Skips itself, saying why, when
/// `codex` or seatbelt isn't on this machine (it runs on the Mac).
#[test]
fn codex_auto_sandbox_lets_a_build_write_commit_and_connect_live() {
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").exists() {
        eprintln!("SKIPPED codex_auto_sandbox_lets_a_build_write_commit_and_connect_live: no seatbelt (macOS only)");
        return;
    }
    let codex = ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"]
        .into_iter()
        .map(PathBuf::from)
        .chain(
            std::env::var_os("PATH")
                .map(|p| {
                    std::env::split_paths(&p)
                        .map(|d| d.join("codex"))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        )
        .find(|p| p.is_file());
    let Some(codex) = codex else {
        eprintln!("SKIPPED codex_auto_sandbox_lets_a_build_write_commit_and_connect_live: no codex CLI found");
        return;
    };
    let git = |dir: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            // The person's config may sign commits (the runner's does),
            // which needs the temp folder this fixture shuts.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let repo = root_path.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = root_path.join("build wt");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    let common = repo.join(".git");
    // Git names the folder itself (`build-wt`); read it as git wrote it.
    let pointer = std::fs::read_to_string(wt.join(".git")).unwrap();
    let gitdir = PathBuf::from(pointer.trim().strip_prefix("gitdir: ").unwrap());
    assert!(gitdir.starts_with(common.join("worktrees")));
    let vault = root_path.join("Jo's vault ws folder");
    std::fs::create_dir_all(&vault).unwrap();
    // Dropped from the roots: with it Codex's profile wouldn't load at all.
    let quoted = root_path.join("a \"quoted\" folder");
    std::fs::create_dir_all(&quoted).unwrap();
    let outside = root_path.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    // Stands in for the writable temp folder a real run has (`/tmp`,
    // `$TMPDIR`), which the fixture otherwise shuts: used only by the
    // move-in case below.
    let scratch = root_path.join("writable-temp");
    std::fs::create_dir_all(&scratch).unwrap();
    let home = root_path.join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // The sandbox settings of the run as Hollow builds it: `-s` and every
    // `sandbox_workspace_write.*` `-c` value, verbatim from the argv.
    let sandbox_args = |perm: &str| -> Vec<String> {
        let mut run = spec(perm, None, true);
        run.cwd = wt.clone();
        run.extras.extra_dirs = vec![vault.clone(), quoted.clone()];
        let mut cmd = Command::new("codex");
        build_codex(&mut cmd, &run);
        let args = args_of(&cmd);
        // Auto in a worktree names a permission profile and no `-s`;
        // the other modes name their sandbox.
        let profile = config_values(&args)
            .iter()
            .any(|v| v.starts_with("default_permissions="));
        if profile {
            // The fixture's temp folders sit under `/tmp`, which `:workspace`
            // would also allow: shut it here, as the legacy flags did.
            let mut out = vec!["-P".to_string(), "hollow_auto".to_string()];
            for value in config_values(&args)
                .into_iter()
                .filter(|v| v.starts_with("permissions."))
            {
                out.push("-c".into());
                out.push(match value.strip_prefix("permissions.hollow_auto.filesystem={") {
                    Some(rest) => format!(
                        "permissions.hollow_auto.filesystem={{\":slash_tmp\"=\"read\",\":tmpdir\"=\"read\",{}=\"write\",{rest}",
                        serde_json::to_string(scratch.to_str().unwrap()).unwrap()
                    ),
                    None => value,
                });
            }
            return out;
        }
        let mode = args
            .windows(2)
            .find(|w| w[0] == "-s")
            .map(|w| w[1].clone())
            .expect("a fresh run names its sandbox");
        let mut out = vec!["-c".to_string(), format!("sandbox_mode=\"{mode}\"")];
        for value in sandbox_write_values(&args) {
            out.push("-c".into());
            out.push(value);
        }
        out.extend(
            [
                "-c",
                "sandbox_workspace_write.exclude_slash_tmp=true",
                "-c",
                "sandbox_workspace_write.exclude_tmpdir_env_var=true",
            ]
            .map(String::from),
        );
        out
    };
    let script = format!(
        "cd \"$WT\" || exit 9; \
         touch \"$WT/in-wt\" && echo WT_OK; \
         touch \"$VAULT/in-vault\" && echo VAULT_OK; \
         touch \"$OUTSIDE/no\" && echo OUTSIDE_OK; \
         echo \"$PERM\" > f && git add f && git -c user.email=t@example.com -c user.name=t commit -q -m \"$PERM\" && echo COMMIT_OK; \
         git branch \"side-$PERM\" && echo BRANCH_OK; \
         echo x > \"$COMMON/hooks/post-commit\" && echo HOOK_OK; \
         git config core.fsmonitor /bin/false && echo CONFIG_OK; \
         /usr/bin/nc -z -w 3 127.0.0.1 {port} && echo NET_OK; \
         printf '/tmp/evil\\n' > \"$GITDIR/commondir\" && echo COMMONDIR_WRITE_OK; \
         printf 'x\\n' > \"$GITDIR/gitdir\" && echo GITDIRFILE_WRITE_OK; \
         mkdir -p \"$WT/sub/deep\" && git init -q \"$WT/sub/deep\" 2>/dev/null && echo NESTED_GIT_OK; \
         mkdir -p \"$WT/sub2\" && echo 'gitdir: /tmp' > \"$WT/sub2/.git\" 2>/dev/null && echo NESTED_DOTGIT_FILE_OK; \
         git init -q \"$SCRATCH/d\" 2>/dev/null && mv \"$SCRATCH/d\" \"$WT/moved-in\" 2>/dev/null && echo MOVED_IN_GIT_OK; true"
    );
    let run_in_sandbox = |perm: &str| -> String {
        let out = std::process::Command::new(&codex)
            .arg("sandbox")
            .args(sandbox_args(perm))
            .args(["--", "/bin/sh", "-c", &script])
            .current_dir(&wt)
            .env("CODEX_HOME", &home)
            .env("WT", &wt)
            .env("VAULT", &vault)
            .env("OUTSIDE", &outside)
            .env("SCRATCH", &scratch)
            .env("PERM", perm)
            .env("COMMON", &common)
            .env("GITDIR", &gitdir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        format!(
            "{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // The roots: the vault folder, the worktree's git folder and only the
    // common folder's objects, refs and logs (review finding 1b).
    let mut run = spec("auto", None, true);
    run.cwd = wt.clone();
    run.extras.extra_dirs = vec![vault.clone(), quoted.clone()];
    let want: Vec<String> = [
        vault.clone(),
        gitdir.clone(),
        common.join("objects"),
        common.join("refs"),
        common.join("logs"),
    ]
    .iter()
    .map(|p| p.to_str().unwrap().to_string())
    .collect();
    assert_eq!(codex_auto_writable_roots(&run), want);

    // As `build_with_codex_servers` does before a local auto run.
    let commondir_before = std::fs::read(gitdir.join("commondir")).unwrap();
    arm_codex_worktree_guard(&wt, &[vault.clone(), quoted.clone()]);

    let auto = run_in_sandbox("auto");
    let lines: Vec<&str> = auto.lines().collect();
    // The profile makes the worktree git folder's `commondir` and `gitdir`
    // read-only inside the granted folder (2026-11-12): the rewrite is
    // refused, so there is nothing for the 200 ms guard to put back. Commits
    // still work (COMMIT_OK below: the index stays writable).
    assert!(!lines.contains(&"COMMONDIR_WRITE_OK"), "{auto}");
    assert!(!lines.contains(&"GITDIRFILE_WRITE_OK"), "{auto}");
    // A decoy repository for a planted submodule entry can't be made.
    assert!(!lines.contains(&"NESTED_GIT_OK"), "{auto}");
    assert!(!lines.contains(&"NESTED_DOTGIT_FILE_OK"), "{auto}");
    assert!(!wt.join("sub/deep/.git").exists() && !wt.join("sub2/.git").exists());
    // KNOWN GAP (N22 review, 2026-11-15): the deny checks the created or
    // renamed path only, so a repository built in a writable temp folder and
    // moved in passes. Pinned as it is; when the temp folders are made
    // read-only except the run's scratch folder this flips, and so must this
    // line (the index guard and `harden_common_submodule_ignore` still apply).
    assert!(
        lines.contains(&"MOVED_IN_GIT_OK"),
        "known gap closed? flip this assertion: {auto}"
    );
    assert!(wt.join("moved-in/.git").exists());
    std::fs::remove_dir_all(wt.join("moved-in")).unwrap();
    assert_eq!(
        std::fs::read(gitdir.join("commondir")).unwrap(),
        commondir_before
    );
    assert_eq!(guard_pass(Some(&gitdir)), 0, "nothing to put back");
    for want in ["WT_OK", "VAULT_OK", "COMMIT_OK", "BRANCH_OK", "NET_OK"] {
        assert!(lines.contains(&want), "auto should allow {want}:\n{auto}");
    }
    for refused in ["OUTSIDE_OK", "HOOK_OK", "CONFIG_OK"] {
        assert!(
            !lines.contains(&refused),
            "auto must refuse {refused} (outside its roots):\n{auto}"
        );
    }
    assert!(!common.join("hooks/post-commit").exists());
    assert!(vault.join("in-vault").exists());
    let log = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(&wt)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "auto");

    let edits = run_in_sandbox("acceptEdits");
    let lines: Vec<&str> = edits.lines().collect();
    assert!(
        lines.contains(&"WT_OK"),
        "acceptEdits still writes its cwd:\n{edits}"
    );
    for refused in [
        "VAULT_OK",
        "OUTSIDE_OK",
        "COMMIT_OK",
        "BRANCH_OK",
        "HOOK_OK",
        "CONFIG_OK",
        "NET_OK",
        "COMMONDIR_WRITE_OK",
        "GITDIRFILE_WRITE_OK",
        // N22: no decoy repository in acceptEdits either.
        "NESTED_GIT_OK",
        "NESTED_DOTGIT_FILE_OK",
    ] {
        assert!(
            !lines.contains(&refused),
            "acceptEdits must refuse {refused}:\n{edits}"
        );
    }
    drop(listener);
}

/// A forged or odd worktree grants nothing and so selects no profile: the
/// run keeps the `workspace-write` sandbox and its `-s` (2026-11-12).
#[test]
fn no_profile_without_a_checked_worktree_git_folder() {
    let root = tempfile::tempdir().unwrap();
    let (wt, _) = fake_linked_worktree(root.path(), "profile-wt");
    let mut run = spec("auto", None, true);
    run.cwd = wt.clone();
    assert!(codex_auto_uses_profile(&codex_auto_sandbox_overrides(&run)));
    // A forged back-pointer: nothing is granted.
    let gitdir = std::fs::read_to_string(wt.join(".git")).unwrap();
    let gitdir = PathBuf::from(gitdir.trim().strip_prefix("gitdir: ").unwrap());
    std::fs::write(gitdir.join("gitdir"), "/somewhere/else/.git\n").unwrap();
    let values = codex_auto_sandbox_overrides(&run);
    assert!(!codex_auto_uses_profile(&values), "{values:?}");
    // A glob character in a granted path (Codex would read the key as a
    // glob and refuse the profile): the sandbox, with the roots, as before.
    let (globbed, _) = fake_linked_worktree(root.path(), "profile [glob]");
    let mut run_glob = spec("auto", None, true);
    run_glob.cwd = globbed;
    let values = codex_auto_sandbox_overrides(&run_glob);
    assert!(!codex_auto_uses_profile(&values), "{values:?}");
    assert!(values
        .iter()
        .any(|v| v.starts_with("sandbox_workspace_write.writable_roots=")));
    // A remote run never gets one (no folder of this Mac reaches the runner).
    let (wt2, _) = fake_linked_worktree(root.path(), "profile-wt-2");
    let mut remote = spec("auto", None, true);
    remote.cwd = wt2;
    remote.extras.remote = Some(crate::remote_session::RemoteTarget {
        host: "runner-host".into(),
        home: "/Users/runner".into(),
        cwd: "/Users/runner/hollow-remote/sessions/s-1".into(),
        git_worktree: true,
        ..Default::default()
    });
    assert!(!codex_auto_uses_profile(&codex_auto_sandbox_overrides(
        &remote
    )));
    // Plan and bypass never; the edit modes get the nested-`.git` deny
    // alone (N22), local only.
    for perm in ["plan", "bypassPermissions"] {
        let mut other = spec(perm, None, true);
        other.cwd = wt.clone();
        assert!(!codex_auto_uses_profile(&codex_auto_sandbox_overrides(
            &other
        )));
    }
    for perm in ["default", "acceptEdits"] {
        let mut edit = spec(perm, None, true);
        edit.cwd = wt.clone();
        assert!(codex_auto_uses_profile(&codex_auto_sandbox_overrides(
            &edit
        )));
        edit.extras.remote = remote.extras.remote.clone();
        assert!(!codex_auto_uses_profile(&codex_auto_sandbox_overrides(
            &edit
        )));
    }
}

/// CX5b, the org clone's write block per agent and mode, as built. Claude:
/// the `Edit` and `Bash` deny rules in every mode. Codex auto in a checked
/// worktree: the profile names the clone `read`; auto never gets the clone
/// (or a folder in it) as a writable root. Codex full access: no sandbox,
/// so nothing holds (`context_layers::ORG_BLOCK_LIMITS`). Plan: read-only.
#[test]
fn the_org_clone_write_block_per_agent_and_mode() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().canonicalize().unwrap();
    let org = base.join("org");
    std::fs::create_dir_all(org.join("glossary")).unwrap();
    crate::context_layers::set_test_org_root(Some(org.clone()));
    let org_text = org.to_str().unwrap().to_string();

    for perm in [
        "plan",
        "default",
        "acceptEdits",
        "auto",
        "bypassPermissions",
    ] {
        let claude = rendered(&test_command(AgentKind::Claude, &spec(perm, None, false)));
        let deny = claude.find("--disallowed-tools").expect("a deny list");
        assert!(
            claude[deny..].contains(&format!("Edit(/{org_text}/**)")),
            "{perm}: {claude}"
        );
        assert!(
            claude[deny..].contains(&format!("Bash(*{org_text}*)")),
            "{perm}: {claude}"
        );
    }

    let (wt, _) = fake_linked_worktree(&base, "org-wt");
    let mut auto = spec("auto", None, true);
    auto.cwd = wt.clone();
    auto.extras.extra_dirs = vec![org.clone(), org.join("glossary")];
    let values = codex_auto_sandbox_overrides(&auto);
    assert!(codex_auto_uses_profile(&values), "{values:?}");
    let filesystem = values
        .iter()
        .find(|v| v.contains(".filesystem="))
        .expect("the profile's folders");
    assert!(
        filesystem.contains(&format!(
            "{}=\"read\"",
            serde_json::to_string(&org_text).unwrap()
        )),
        "{filesystem}"
    );
    assert!(
        !filesystem.contains(&format!("{org_text}\"=\"write\"")),
        "{filesystem}"
    );
    assert!(
        !filesystem.contains(&format!("{org_text}/glossary")),
        "{filesystem}"
    );
    assert!(codex_auto_writable_roots(&auto)
        .iter()
        .all(|r| !Path::new(r).starts_with(&org)));

    // Full access: the bypass flag and no sandbox at all, so no org entry.
    let mut bypass = spec("bypassPermissions", None, true);
    bypass.cwd = wt.clone();
    let mut cmd = Command::new("codex");
    build_codex(&mut cmd, &bypass);
    let args = args_of(&cmd);
    assert!(args
        .iter()
        .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    assert!(!args.iter().any(|a| a.contains(&org_text)), "{args:?}");
    assert!(crate::context_layers::ORG_BLOCK_LIMITS.contains("Codex in full access (no sandbox)"));
    // Plan: the read-only sandbox.
    let mut plan = spec("plan", None, true);
    plan.cwd = wt;
    let mut cmd = Command::new("codex");
    build_codex(&mut cmd, &plan);
    let args = args_of(&cmd);
    assert!(
        args.windows(2).any(|w| w[0] == "-s" && w[1] == "read-only"),
        "{args:?}"
    );
    crate::context_layers::set_test_org_root(None);
}

/// The planted-submodule route (2026-11-12): a `160000` entry in the index
/// whose folder holds a repository with `core.fsmonitor` made the person's
/// own `git status` run it. `diff.ignoreSubmodules=dirty` in the common
/// config (written at every auto spawn; read-only to the turn) stops git
/// entering it, for the commands a person runs in a worktree. Real git.
#[test]
fn arming_the_guard_sets_ignore_submodules_and_plain_git_no_longer_runs_a_planted_fsmonitor() {
    let git = |dir: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let git_nolock = |dir: &Path, args: &[&str]| {
        String::from_utf8_lossy(
            &std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap()
                .stdout,
        )
        .into_owned()
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let repo = root_path.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = root_path.join("wt");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    let common = repo.join(".git");
    let config_of = || {
        std::process::Command::new("git")
            .args(["config", "--file"])
            .arg(common.join("config"))
            .args(["--get", "diff.ignoreSubmodules"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap()
    };
    assert_eq!(config_of(), "");

    // What a hostile turn leaves: a decoy repository in the worktree and a
    // gitlink for it in the worktree's index.
    let decoy = wt.join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    git(&decoy, &["init", "-q", "."]);
    git(&decoy, &["commit", "-q", "--allow-empty", "-m", "d"]);
    let head = git(&decoy, &["rev-parse", "HEAD"]);
    let ran = root_path.join("RAN");
    git(
        &decoy,
        &[
            "config",
            "core.fsmonitor",
            &format!("touch {}; true", ran.display()),
        ],
    );
    let cacheinfo = format!("160000,{head},decoy");
    let plant = || git(&wt, &["update-index", "--add", "--cacheinfo", &cacheinfo]);
    plant();
    git(&wt, &["status", "--short"]);
    assert!(
        ran.exists(),
        "control: plain git status runs the planted fsmonitor"
    );
    std::fs::remove_file(&ran).unwrap();
    git(&wt, &["update-index", "--force-remove", "decoy"]);

    // Armed: the config is set, and an index that already holds a planted
    // entry is cleaned at once.
    plant();
    arm_codex_worktree_guard(&wt, &[]);
    assert_eq!(config_of(), "dirty");
    assert!(
        !git(&wt, &["ls-files", "-s"]).contains("160000"),
        "arming removes an entry planted while no guard ran"
    );
    let gitdir = std::fs::read_to_string(wt.join(".git")).unwrap();
    let gitdir = PathBuf::from(gitdir.trim().strip_prefix("gitdir: ").unwrap());

    // Planted during a turn: the config covers what the guard hasn't yet
    // removed, for the commands that read `diff.ignoreSubmodules`.
    plant();
    for args in [
        &["status"][..],
        &["status", "-sb"],
        &["diff"],
        &["diff", "--cached"],
        &["log", "-p", "-1"],
        &["stash", "list"],
        &["checkout", "-q", "-b", "nb"],
        &["switch", "-q", "-"],
        &["commit", "-q", "--allow-empty", "-m", "x"],
    ] {
        git(&wt, args);
        assert!(!ran.exists(), "git {args:?} ran the planted fsmonitor");
    }
    // `add -A` doesn't read that setting (control: it runs the payload), so
    // the guard's removal is what stops it.
    git(&wt, &["add", "-A"]);
    assert!(ran.exists(), "control: add -A enters the planted submodule");
    std::fs::remove_file(&ran).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 1, "the guard removes the entry");
    assert!(!git(&wt, &["ls-files", "-s"]).contains("160000"));
    // `add -A` stages the untracked decoy as a new embedded repository (a
    // gitlink again): it starts nothing, and the next pass removes it.
    git(&wt, &["add", "-A"]);
    assert!(!ran.exists(), "nothing runs once the entry is gone");
    assert_eq!(guard_pass(Some(&gitdir)), 1);
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "y"]);
    assert!(!ran.exists());
    // A scan of an unchanged index runs no git and removes nothing.
    assert_eq!(guard_pass(Some(&gitdir)), 0);
    // Review 2026-11-12, finding 1: a planted entry with the index's size and
    // modified time put back is still found (the stamp is ctime and inode),
    // and a symlinked index is replaced.
    let index = gitdir.join("index");
    let mtime = std::fs::metadata(&index).unwrap().modified().unwrap();
    plant();
    let file = std::fs::File::options().write(true).open(&index).unwrap();
    file.set_modified(mtime).unwrap();
    drop(file);
    assert_eq!(guard_pass(Some(&gitdir)), 1, "same mtime, new entry");
    // Round 4: a failed removal is throttled, never remembered as clean: with
    // the git folder write-protected the removal fails, and once it is
    // writable again the same unchanged index is retried after the delay.
    plant();
    let mode = std::fs::metadata(&gitdir).unwrap().permissions();
    std::fs::set_permissions(&gitdir, std::os::unix::fs::PermissionsExt::from_mode(0o500)).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 0, "removal fails read-only");
    std::fs::set_permissions(&gitdir, mode).unwrap();
    assert_eq!(
        guard_pass(Some(&gitdir)),
        0,
        "throttled within the retry delay"
    );
    {
        let snaps = WORKTREE_GIT_SNAPSHOTS.lock().unwrap();
        let snap = snaps.get(&gitdir).unwrap();
        let mut failed = snap.index_failed.lock().unwrap();
        let (stamp, _) = failed
            .take()
            .expect("the failure was recorded, not stamped clean");
        *failed = Some((stamp, std::time::Instant::now()));
    }
    assert_eq!(guard_pass(Some(&gitdir)), 1, "retried after the delay");
    // Round 3: an index over 64 KB (many tracked files) is scrubbed whole.
    let bulk = wt.join("bulk");
    std::fs::create_dir_all(&bulk).unwrap();
    for n in 0..1500 {
        std::fs::write(bulk.join(format!("file-with-a-long-name-{n:05}.txt")), "x").unwrap();
    }
    git(&wt, &["add", "bulk"]);
    assert!(std::fs::metadata(gitdir.join("index")).unwrap().len() > 64 * 1024);
    guard_pass(Some(&gitdir));
    plant();
    assert_eq!(guard_pass(Some(&gitdir)), 1, "big index scrubbed");
    assert!(!git(&wt, &["ls-files", "-s"]).contains("160000"));
    // A FIFO as the index is replaced, never opened by git.
    std::fs::remove_file(gitdir.join("index")).unwrap();
    let fifo = std::ffi::CString::new(gitdir.join("index").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(guard_pass(Some(&gitdir)), 1, "a FIFO index goes");
    git(&wt, &["reset", "-q"]);
    // Round 2: a turn holding `index.lock` doesn't stop the removal.
    plant();
    std::fs::write(gitdir.join("index.lock"), b"").unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 1, "removed despite index.lock");
    assert!(!git_nolock(&wt, &["ls-files", "-s"]).contains("160000"));
    std::fs::remove_file(gitdir.join("index.lock")).unwrap();
    let clean = root_path.join("clean-index");
    std::fs::copy(&index, &clean).unwrap();
    std::fs::remove_file(&index).unwrap();
    std::os::unix::fs::symlink(&clean, &index).unwrap();
    assert_eq!(guard_pass(Some(&gitdir)), 1, "a symlinked index goes");
    assert!(!std::fs::symlink_metadata(&index).is_ok_and(|m| m.file_type().is_symlink()));
    git(&wt, &["status", "--short"]);

    // A person's own `all` or `dirty` stays; any other value is replaced; an
    // unrelated key is left alone.
    for (before, after) in [
        ("all", "all"),
        ("DIRTY", "DIRTY"),
        ("none", "dirty"),
        ("untracked", "dirty"),
    ] {
        git(&repo, &["config", "diff.ignoreSubmodules", before]);
        harden_common_submodule_ignore(&common);
        assert_eq!(config_of(), after, "from {before}");
    }
}

/// Q-R5 (2026-11-13): a split index keeps its entries in a writable
/// `sharedindex.<hash>`, and git reads that file without checking its
/// checksum, so a patched shared index (its trailer set to the hash `index`
/// links) adds a gitlink while `index` stays byte for byte the same. The
/// guard's stamp covers the shared files, so it scans again and removes it.
#[test]
fn a_gitlink_patched_into_a_split_shared_index_is_removed() {
    let git = |dir: &Path, envs: &[(&str, &Path)], args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .envs(envs.iter().map(|(k, v)| (*k, *v)))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let repo = root_path.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &[], &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a"), "a").unwrap();
    git(&repo, &[], &["add", "a"]);
    git(&repo, &[], &["commit", "-q", "-m", "init"]);
    let wt = root_path.join("wt");
    git(
        &repo,
        &[],
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    let head = git(&wt, &[], &["rev-parse", "HEAD"]);
    arm_codex_worktree_guard(&wt, &[]);
    let gitdir = std::fs::read_to_string(wt.join(".git")).unwrap();
    let gitdir = PathBuf::from(gitdir.trim().strip_prefix("gitdir: ").unwrap());
    let shared_files = || -> Vec<PathBuf> {
        std::fs::read_dir(&gitdir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("sharedindex.")
            })
            .collect()
    };

    // The turn splits the index (no config needed); the guard scans it clean.
    git(&wt, &[], &["update-index", "--split-index"]);
    let linked = shared_files();
    assert_eq!(linked.len(), 1);
    let linked = linked[0].clone();
    guard_pass(Some(&gitdir));
    assert_eq!(guard_pass(Some(&gitdir)), 0, "clean and unchanged");

    // A shared index holding a gitlink, made from a copy of the index.
    let copy = root_path.join("copy-index");
    std::fs::copy(gitdir.join("index"), &copy).unwrap();
    git(
        &wt,
        &[("GIT_INDEX_FILE", &copy)],
        &[
            "-c",
            "splitIndex.maxPercentChange=0",
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},decoy"),
        ],
    );
    let payload = shared_files()
        .into_iter()
        .find(|p| *p != linked)
        .expect("a second shared index");
    let mut bytes = std::fs::read(&payload).unwrap();
    std::fs::remove_file(&payload).unwrap();
    // Its trailer becomes the hash `index` links; nothing else changes.
    let hash = linked
        .file_name()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("sharedindex.")
        .to_string();
    let trailer: Vec<u8> = (0..hash.len() / 2)
        .map(|i| u8::from_str_radix(&hash[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let at = bytes.len() - trailer.len();
    bytes[at..].copy_from_slice(&trailer);
    // Settle the shared-file listing the guard saw (the payload is gone).
    guard_pass(Some(&gitdir));
    let index_before = std::fs::read(gitdir.join("index")).unwrap();
    std::fs::write(&linked, &bytes).unwrap();
    assert_eq!(std::fs::read(gitdir.join("index")).unwrap(), index_before);
    assert!(
        git(&wt, &[], &["ls-files", "-s"]).contains("160000"),
        "control: git reads the patched shared index"
    );
    assert_eq!(
        guard_pass(Some(&gitdir)),
        1,
        "the shared index changed, so the guard scans and removes the entry"
    );
    assert!(!git(&wt, &[], &["ls-files", "-s"]).contains("160000"));
    git(&wt, &[], &["status", "--short"]);
}

/// Review 2026-11-12: an inherited mode never hands Grok an unsandboxed
/// `auto` or Antigravity its unavailable one. Q-R3 round 2: from an unknown
/// agent a mode reads the most restrictive way (what every agent that runs
/// it can do), so `default` and `acceptEdits` read only with no commands
/// (Antigravity's accept-edits), `auto` as Claude's and Codex's (all 4
/// agents x 5 modes, plus an unknown spelling).
#[test]
fn an_inherited_mode_never_gives_grok_or_antigravity_auto() {
    use super::{auto_is_unsandboxed, inherited_mode, unavailable_mode, AgentKind};
    let want = |kind: AgentKind, mode: &str| -> &str {
        match (kind, mode) {
            (_, "plan") => "plan",
            (_, "bypassPermissions") => "bypassPermissions",
            (AgentKind::Codex, "default" | "acceptEdits") => "plan",
            (AgentKind::Antigravity, "acceptEdits") => "acceptEdits",
            (_, "default" | "acceptEdits") => "default",
            (AgentKind::Claude | AgentKind::Codex | AgentKind::Antigravity, "auto") => "auto",
            (_, "auto") => "acceptEdits",
            _ => unreachable!("{mode}"),
        }
    };
    for kind in AgentKind::all() {
        for mode in crate::app_mcp::PERMISSION_MODES {
            assert_eq!(
                inherited_mode(None, kind, mode),
                want(kind, mode),
                "{kind:?} {mode}"
            );
            assert!(unavailable_mode(kind, inherited_mode(None, kind, mode)).is_none());
            assert_eq!(
                auto_is_unsandboxed(kind, mode),
                kind == AgentKind::Grok && mode == "auto",
                "{kind:?} {mode}"
            );
        }
        assert_eq!(
            inherited_mode(None, kind, "root"),
            want(kind, "acceptEdits")
        );
    }
    assert_eq!(
        super::grok_sandbox("auto"),
        "off",
        "why Grok auto counts as bypass"
    );
}

/// Q-R3 (2026-11-13, round 2): an inherited mode never gives the child agent
/// more than the parent's mode could do on either axis (`mode_capability`:
/// reach and commands), whatever the names say. Claude's headless `default`
/// reads only and its `acceptEdits` runs no commands, so a Codex child runs
/// read-only (`plan`) from either; plan stays plan everywhere; Codex's
/// read-only gives Claude plan; Auto keeps its handling (Claude and Codex
/// keep it, Grok and Antigravity get acceptEdits).
#[test]
fn an_inherited_mode_never_gains_capability_across_agents() {
    use super::{inherited_mode, mode_capability, unavailable_mode, AgentKind, ModeReach};
    let (claude, codex, grok, agy) = (
        Some(AgentKind::Claude),
        Some(AgentKind::Codex),
        Some(AgentKind::Grok),
        Some(AgentKind::Antigravity),
    );
    // The finding: Claude default to Codex.
    assert_eq!(inherited_mode(claude, AgentKind::Codex, "default"), "plan");
    // Claude plan to Codex (and to every agent).
    for kind in AgentKind::all() {
        assert_eq!(inherited_mode(claude, kind, "plan"), "plan", "{kind:?}");
    }
    // Codex read-only to Claude.
    assert_eq!(inherited_mode(codex, AgentKind::Claude, "plan"), "plan");
    // Auto, unchanged: kept on Claude and Codex, acceptEdits on Grok
    // (sandbox off); Antigravity keeps its own.
    for from in [claude, codex, None] {
        assert_eq!(inherited_mode(from, AgentKind::Claude, "auto"), "auto");
        assert_eq!(inherited_mode(from, AgentKind::Codex, "auto"), "auto");
        assert_eq!(inherited_mode(from, AgentKind::Grok, "auto"), "acceptEdits");
        assert_eq!(
            inherited_mode(from, AgentKind::Antigravity, "auto"),
            "auto",
            "Antigravity's own auto is back (2026-11-20)"
        );
    }
    // Commands are their own axis: Claude acceptEdits edits but runs no
    // commands, Codex can't edit without them (round 2, review item 3).
    assert_eq!(
        inherited_mode(claude, AgentKind::Codex, "acceptEdits"),
        "plan"
    );
    assert_eq!(
        inherited_mode(claude, AgentKind::Grok, "acceptEdits"),
        "default"
    );
    assert_eq!(
        inherited_mode(claude, AgentKind::Antigravity, "acceptEdits"),
        "acceptEdits"
    );
    assert_eq!(
        inherited_mode(codex, AgentKind::Claude, "default"),
        "acceptEdits"
    );
    assert_eq!(
        inherited_mode(grok, AgentKind::Codex, "acceptEdits"),
        "acceptEdits"
    );
    assert_eq!(
        inherited_mode(claude, AgentKind::Grok, "default"),
        "default"
    );
    assert_eq!(
        inherited_mode(agy, AgentKind::Claude, "acceptEdits"),
        "default"
    );
    assert_eq!(inherited_mode(agy, AgentKind::Codex, "acceptEdits"), "plan");
    // Grok's auto is unchecked, but a child never goes above auto's Hollow
    // layer: Claude auto, not bypass. A Codex auto parent's Grok: acceptEdits.
    assert_eq!(inherited_mode(grok, AgentKind::Claude, "auto"), "auto");
    assert_eq!(
        inherited_mode(codex, AgentKind::Grok, "auto"),
        "acceptEdits"
    );
    // The same agent keeps its mode.
    for kind in AgentKind::all() {
        for mode in crate::app_mcp::PERMISSION_MODES {
            if unavailable_mode(kind, mode).is_none() {
                assert_eq!(
                    inherited_mode(Some(kind), kind, mode),
                    mode,
                    "{kind:?} {mode}"
                );
            }
        }
    }
    // The property, every pair: the child can do nothing the parent can't
    // on either axis, runs a mode it can, and never above the parent's
    // Hollow layer (cards: plan, then default and acceptEdits, then auto,
    // then bypass). From an unknown agent it is within every agent's
    // reading of the mode.
    let layer = |m: &str| match m {
        "plan" => 0,
        "auto" => 2,
        "bypassPermissions" => 3,
        _ => 1,
    };
    for kind in AgentKind::all() {
        for mode in crate::app_mcp::PERMISSION_MODES {
            let unknown = inherited_mode(None, kind, mode);
            for from in AgentKind::all() {
                if unavailable_mode(from, mode).is_some() {
                    continue;
                }
                let parent = mode_capability(from, mode);
                assert!(
                    mode_capability(kind, unknown).within(parent),
                    "unknown {mode} to {kind:?}: {unknown}"
                );
                let got = inherited_mode(Some(from), kind, mode);
                let case = format!("{from:?} {mode} to {kind:?}: {got}");
                assert!(mode_capability(kind, got).within(parent), "{case}");
                assert!(unavailable_mode(kind, got).is_none(), "{case}");
                assert!(layer(got) <= layer(mode), "{case}");
            }
        }
    }
    let claude_edits = mode_capability(AgentKind::Claude, "acceptEdits");
    let codex_edits = mode_capability(AgentKind::Codex, "acceptEdits");
    assert_eq!(claude_edits.reach, ModeReach::Folder);
    assert_eq!(codex_edits.reach, ModeReach::Folder);
    assert!(!claude_edits.within(codex_edits) || !codex_edits.within(claude_edits));
    assert!(
        claude_edits.within(codex_edits),
        "Codex's does all Claude's does"
    );
    assert!(!codex_edits.within(claude_edits), "but runs commands too");
    assert_eq!(
        mode_capability(AgentKind::Grok, "auto").reach,
        ModeReach::Unchecked
    );
}

/// Q-R33: no local Claude or Codex run carries its prompt on argv; it is on
/// the command's stdin, an unlinked file that reads back whole. A runner run
/// (its leader ships the argv over ssh stdin) and Grok keep the argument.
#[test]
fn every_local_run_takes_its_prompt_on_stdin_not_argv() {
    use std::io::Read;
    for kind in [AgentKind::Claude, AgentKind::Codex] {
        for (interactive, resume) in [(false, None), (true, None), (true, Some("s-1"))] {
            let mut run = spec("acceptEdits", resume, interactive);
            run.prompt = "SECRET-PROMPT-TEXT calendar: lunch".into();
            let cmd = test_command(kind, &run);
            let args = args_of(&cmd);
            assert!(!is_unavailable_command(&cmd), "{kind:?}");
            assert!(
                !args.iter().any(|a| a.contains("SECRET-PROMPT-TEXT")),
                "{kind:?} {args:?}"
            );
            let carried = take_last_stdin_prompt().expect("prompt on stdin");
            assert!(
                carried.contains("SECRET-PROMPT-TEXT calendar: lunch"),
                "{kind:?}"
            );
            if kind == AgentKind::Codex {
                assert_eq!(args.last().map(String::as_str), Some("-"), "{args:?}");
            }
        }
    }
    let mut file = prompt_stdin_file("line one\nline two").unwrap();
    let mut back = String::new();
    file.read_to_string(&mut back).unwrap();
    assert_eq!(back, "line one\nline two");
    // R5b: Grok reads `--prompt-file /dev/stdin`; Antigravity reads one
    // stream-json user message on stdin with `-p=` (R5 residual, probed
    // 2026-11-17).
    let mut run = spec("acceptEdits", None, false);
    run.prompt = "SECRET-PROMPT-TEXT".into();
    assert!(stdin_prompt(AgentKind::Grok, &run).is_some());
    let agy_line = stdin_prompt(AgentKind::Antigravity, &run).expect("agy prompt on stdin");
    let agy_msg: serde_json::Value = serde_json::from_str(agy_line.trim_end()).unwrap();
    assert_eq!(agy_msg["event"], "user");
    assert!(agy_msg["message"]["content"]
        .as_str()
        .is_some_and(|t| t.contains("SECRET-PROMPT-TEXT")));
    assert!(agy_line.ends_with('\n'), "one NDJSON line");
    let cmd = test_command(AgentKind::Grok, &run);
    let args = args_of(&cmd);
    assert!(
        !args.iter().any(|a| a.contains("SECRET-PROMPT-TEXT")),
        "{args:?}"
    );
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--prompt-file" && w[1] == "/dev/stdin"),
        "{args:?}"
    );
    assert!(!args.iter().any(|a| a == "-p"), "{args:?}");
    assert!(take_last_stdin_prompt()
        .expect("Grok's prompt on stdin")
        .contains("SECRET-PROMPT-TEXT"));
    let agy = args_of(&test_command(AgentKind::Antigravity, &run));
    assert!(
        !agy.iter().any(|a| a.contains("SECRET-PROMPT-TEXT")),
        "{agy:?}"
    );
    assert!(agy.iter().any(|a| a == "-p="), "{agy:?}");
    assert!(
        agy.windows(2)
            .any(|w| w[0] == "--input-format" && w[1] == "stream-json"),
        "{agy:?}"
    );
}

/// R5b (Q-R33): the appended system prompt rides a private descriptor for a
/// local Claude run, not argv; a runner run withholds it for the leader's
/// header. The account token stays in its environment (Claude ignores the
/// token descriptor when a login exists), so it is still set as before.
#[test]
fn claude_appended_prompt_travels_on_a_descriptor() {
    let mut run = spec("acceptEdits", None, false);
    run.append_system_prompt = Some("APPENDED-CONTEXT calendar: lunch".into());
    let cmd = test_command(AgentKind::Claude, &run);
    let raw: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        !raw.iter().any(|a| a.contains("APPENDED-CONTEXT")),
        "{raw:?}"
    );
    assert!(
        !raw.iter().any(|a| a == "--append-system-prompt"),
        "{raw:?}"
    );
    let handed = crate::secret_fds::passed();
    assert_eq!(handed.len(), 1, "{handed:?}");
    assert!(handed[0].0 >= 10, "above the standard and low descriptors");
    assert!(
        raw.windows(2)
            .any(|w| w[0] == "--append-system-prompt-file"
                && w[1] == format!("/dev/fd/{}", handed[0].0)),
        "{raw:?}"
    );
    assert!(handed[0].1.starts_with("APPENDED-CONTEXT calendar: lunch"));
    // The test helper reads it back as the inline flag.
    assert!(args_of(&cmd).iter().any(|a| a == "--append-system-prompt"));
}

/// R5b: the descriptors really reach the child, and what the child can be
/// asked about (`ps` argv, the environment) holds neither text. `/bin/sh`
/// reads both files by number, as Claude Code does.
#[tokio::test]
async fn private_descriptors_reach_the_child_and_not_argv_or_env() {
    let mut probe = Command::new("/bin/sh");
    let fds = crate::secret_fds::pass(&mut probe, &["line one\nAPPEND-SECRET", "TOKEN-SECRET-123"])
        .unwrap();
    assert_eq!(fds.len(), 2);
    assert_ne!(fds[0], fds[1]);
    assert!(fds.iter().all(|fd| *fd >= 10));
    probe
        .arg("-c")
        .arg(format!(
            "cat /dev/fd/{}; echo '|'; cat <&{}; echo '|'; env",
            fds[0], fds[1]
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let argv: Vec<String> = probe
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(!argv.iter().any(|a| a.contains("SECRET")), "{argv:?}");
    let out = probe.output().await.unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.starts_with("line one\nAPPEND-SECRET|\nTOKEN-SECRET-123|\n"),
        "{text}"
    );
    let env = text.rsplit("|\n").next().unwrap();
    assert!(
        !env.contains("SECRET"),
        "the environment holds neither: {env}"
    );
}

/// R5b: the descriptor `secret_fds::pass` opens is Hollow's own file and is
/// closed again when the command is dropped (a descriptor left open per run
/// would leak). `fcntl(F_GETFD)` gives EBADF once it is closed; if another
/// thread already reused the number, the file behind it is no longer ours.
#[test]
fn a_passed_descriptor_is_closed_when_the_command_is_dropped() {
    fn identity(fd: i32) -> Option<(u64, u64)> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        (unsafe { libc::fstat(fd, &mut st) } == 0).then_some((st.st_dev as u64, st.st_ino as u64))
    }
    let mut cmd = Command::new("/bin/echo");
    let fds = crate::secret_fds::pass(&mut cmd, &["SECRET"]).unwrap();
    let fd = fds[0];
    assert!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0,
        "open while the command lives"
    );
    let ours = identity(fd).expect("our file");
    drop(cmd);
    let closed = unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::EBADF);
    assert!(
        closed || identity(fd) != Some(ours),
        "descriptor {fd} still ours after the drop"
    );
}

/// R5b proof against the real thing: start the installed Claude Code (its
/// request goes to a local listener that never answers, so no usage is
/// spent), ask `ps -E` for its argv and environment, and find neither the
/// prompt nor the appended system prompt (the account token stays in env, so
/// it is not asserted), while a
/// control variable set the same way IS shown (so the check can fail).
/// `cargo test --lib ps_shows_no_prompt -- --ignored`
#[tokio::test]
#[ignore = "starts the installed Claude Code against a local stand-in endpoint"]
async fn ps_shows_no_prompt_for_a_real_claude_run() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = std::thread::spawn(move || {
        let mut conns = Vec::new();
        while let Ok((conn, _)) = listener.accept() {
            conns.push(conn);
            if conns.len() > 50 {
                break;
            }
        }
    });
    let config_dir = tempfile::tempdir().unwrap();
    let mut run = spec("acceptEdits", None, false);
    run.prompt = "PS-PROMPT-SECRET please summarise".into();
    run.append_system_prompt = Some("PS-APPEND-SECRET context".into());
    run.extras.env = vec![
        (
            "ANTHROPIC_BASE_URL".into(),
            format!("http://127.0.0.1:{port}"),
        ),
        (
            "CLAUDE_CONFIG_DIR".into(),
            config_dir.path().display().to_string(),
        ),
        (
            crate::claude_accounts::TOKEN_ENV.into(),
            "sk-ant-oat01-PS-TOKEN-SECRET".into(),
        ),
        ("HOLLOW_PS_CONTROL".into(), "PS-CONTROL-VISIBLE".into()),
    ];
    let mut cmd = build_command(AgentKind::Claude, &run);
    let mut child = cmd.spawn().expect("claude starts");
    let pid = child.id().unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "claude is still running"
    );
    let ps = std::process::Command::new("ps")
        .args(["-E", "-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .unwrap();
    let shown = String::from_utf8_lossy(&ps.stdout).into_owned();
    let _ = child.kill().await;
    drop(held);
    assert!(
        shown.contains("PS-CONTROL-VISIBLE"),
        "ps -E shows env: {shown}"
    );
    for secret in ["PS-PROMPT-SECRET", "PS-APPEND-SECRET"] {
        assert!(
            !shown.contains(secret),
            "{secret} visible to ps -E: {shown}"
        );
    }
}

/// R5b: the same `ps -E` check for Codex and Grok (no account, so each
/// exits at its own sign-in or network error: `ps -E` is sampled every 10 ms
/// while it lives, and at least one sample must show the control variable).
/// Antigravity is not in it (its prompt is a stream-json stdin message, see [`stdin_prompt`]).
/// `cargo test --lib ps_shows_no_prompt_for_codex -- --ignored`
#[tokio::test]
#[ignore = "starts the installed Codex and Grok CLIs with an empty HOME"]
async fn ps_shows_no_prompt_for_codex_and_grok_runs() {
    for kind in [AgentKind::Codex, AgentKind::Grok] {
        let home = tempfile::tempdir().unwrap();
        let mut run = spec("acceptEdits", None, false);
        run.prompt = "PS-PROMPT-SECRET please summarise".into();
        run.append_system_prompt = Some("PS-APPEND-SECRET context".into());
        run.extras.env = vec![
            ("HOME".into(), home.path().display().to_string()),
            (
                "CODEX_HOME".into(),
                home.path().join("codex").display().to_string(),
            ),
            ("HOLLOW_PS_CONTROL".into(), "PS-CONTROL-VISIBLE".into()),
        ];
        let mut cmd = build_command(kind, &run);
        let mut child = cmd.spawn().expect("starts");
        let pid = child.id().unwrap().to_string();
        let mut seen_control = false;
        let mut samples = 0;
        for _ in 0..300 {
            let ps = std::process::Command::new("ps")
                .args(["-E", "-ww", "-p", &pid, "-o", "command="])
                .output()
                .unwrap();
            let shown = String::from_utf8_lossy(&ps.stdout).into_owned();
            if shown.trim().is_empty() {
                break;
            }
            samples += 1;
            seen_control |= shown.contains("PS-CONTROL-VISIBLE");
            for secret in ["PS-PROMPT-SECRET", "PS-APPEND-SECRET"] {
                assert!(
                    !shown.contains(secret),
                    "{kind:?}: {secret} visible: {shown}"
                );
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = child.kill().await;
        assert!(
            seen_control,
            "{kind:?}: {samples} samples never showed the control variable"
        );
    }
}

/// CX4b: the one `--settings` of a Claude run carries `autoMemoryDirectory`
/// beside the self-hosted settings, and is absent when neither applies.
#[test]
fn claude_settings_merges_the_memory_dir_into_the_one_settings_value() {
    use std::path::Path;
    assert_eq!(super::claude_settings(None, None), None);
    assert_eq!(
        super::claude_settings(Some("{\"env\":{}}".into()), None).as_deref(),
        Some("{\"env\":{}}")
    );
    let dir = Path::new("/c/memory/-Users-a-b");
    let only: serde_json::Value =
        serde_json::from_str(&super::claude_settings(None, Some(dir)).unwrap()).unwrap();
    assert_eq!(
        only,
        serde_json::json!({"autoMemoryDirectory": "/c/memory/-Users-a-b"})
    );
    let both: serde_json::Value = serde_json::from_str(
        &super::claude_settings(Some("{\"env\":{\"A\":\"1\"}}".into()), Some(dir)).unwrap(),
    )
    .unwrap();
    assert_eq!(both["env"]["A"], "1");
    assert_eq!(both["autoMemoryDirectory"], "/c/memory/-Users-a-b");
    // A path with a quote stays valid JSON.
    let odd = super::claude_settings(None, Some(Path::new("/c/a\"b"))).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&odd).is_ok());
    // No personal root in tests: a plain local run carries no --settings.
    let cmd = build_command(AgentKind::Claude, &spec("acceptEdits", None, false));
    assert!(!cmd.as_std().get_args().any(|a| a == "--settings"));
}

/// CX4b review: a base that isn't a JSON object is returned as it was, with
/// no memory key (the pins are never dropped).
#[test]
fn claude_settings_keeps_an_unparseable_base_unchanged() {
    let dir = Path::new("/c/memory/-x");
    for bad in ["not json", "[1,2]", "\"s\"", "null"] {
        assert_eq!(
            super::claude_settings(Some(bad.into()), Some(dir)).as_deref(),
            Some(bad)
        );
    }
}

// AC1 (2026-11-17): the adapter capability flags. One assertion per agent per
// flag, so flipping a flag is a visible one-line change in this table.

/// Independent literals (not read from `agent::flags`), so the wire is
/// checked against what each agent should say:
/// (agent, steer, interrupt, fork, rollback, queue, pendingSurvivesRestart).
const AC1_TABLE: [(AgentKind, [bool; 6]); 4] = [
    (AgentKind::Claude, [true, true, false, false, true, true]),
    (AgentKind::Codex, [true, true, false, false, true, true]),
    (AgentKind::Grok, [false, true, false, false, true, true]),
    (
        AgentKind::Antigravity,
        [false, true, false, false, true, true],
    ),
];

#[test]
fn every_agent_states_every_ac1_flag() {
    // The table covers every agent, so a new AgentKind fails here until its row exists.
    let listed: Vec<_> = AC1_TABLE.iter().map(|(kind, _)| *kind).collect();
    for kind in AgentKind::all() {
        assert!(
            listed.contains(&kind),
            "{kind:?} has no AC1 row in the test table"
        );
    }
    for (kind, expected) in AC1_TABLE {
        let caps = capabilities(kind);
        let actual = [
            caps.steer,
            caps.interrupt,
            caps.fork,
            caps.rollback,
            caps.queue,
            caps.pending_survives_restart,
        ];
        assert_eq!(actual, expected, "{kind:?}");
        for (feature, want) in Feature::ALL.into_iter().zip(expected) {
            assert_eq!(supports(kind, feature), want, "{kind:?} {}", feature.key());
        }
    }
}

macro_rules! ac1_flag_test {
    ($name:ident, $kind:expr, $feature:expr, $want:expr) => {
        #[test]
        fn $name() {
            assert_eq!(supports($kind, $feature), $want);
            let wire = serde_json::to_value(capabilities($kind)).unwrap();
            assert_eq!(wire[$feature.key()], json!($want));
            assert_eq!(
                wire["unsupportedReasons"].get($feature.key()).is_some(),
                !$want
            );
            assert_eq!(require($kind, $feature).is_ok(), $want);
        }
    };
}

ac1_flag_test!(claude_steer, AgentKind::Claude, Feature::Steer, true);
ac1_flag_test!(
    claude_interrupt,
    AgentKind::Claude,
    Feature::Interrupt,
    true
);
ac1_flag_test!(claude_fork, AgentKind::Claude, Feature::Fork, false);
ac1_flag_test!(claude_rollback, AgentKind::Claude, Feature::Rollback, false);
ac1_flag_test!(claude_queue, AgentKind::Claude, Feature::Queue, true);
ac1_flag_test!(
    claude_pending_survives_restart,
    AgentKind::Claude,
    Feature::PendingSurvivesRestart,
    true
);
ac1_flag_test!(codex_steer, AgentKind::Codex, Feature::Steer, true);
ac1_flag_test!(codex_interrupt, AgentKind::Codex, Feature::Interrupt, true);
ac1_flag_test!(codex_fork, AgentKind::Codex, Feature::Fork, false);
ac1_flag_test!(codex_rollback, AgentKind::Codex, Feature::Rollback, false);
ac1_flag_test!(codex_queue, AgentKind::Codex, Feature::Queue, true);
ac1_flag_test!(
    codex_pending_survives_restart,
    AgentKind::Codex,
    Feature::PendingSurvivesRestart,
    true
);
ac1_flag_test!(grok_steer, AgentKind::Grok, Feature::Steer, false);
ac1_flag_test!(grok_interrupt, AgentKind::Grok, Feature::Interrupt, true);
ac1_flag_test!(grok_fork, AgentKind::Grok, Feature::Fork, false);
ac1_flag_test!(grok_rollback, AgentKind::Grok, Feature::Rollback, false);
ac1_flag_test!(grok_queue, AgentKind::Grok, Feature::Queue, true);
ac1_flag_test!(
    grok_pending_survives_restart,
    AgentKind::Grok,
    Feature::PendingSurvivesRestart,
    true
);
ac1_flag_test!(
    antigravity_steer,
    AgentKind::Antigravity,
    Feature::Steer,
    false
);
ac1_flag_test!(
    antigravity_interrupt,
    AgentKind::Antigravity,
    Feature::Interrupt,
    true
);
ac1_flag_test!(
    antigravity_fork,
    AgentKind::Antigravity,
    Feature::Fork,
    false
);
ac1_flag_test!(
    antigravity_rollback,
    AgentKind::Antigravity,
    Feature::Rollback,
    false
);
ac1_flag_test!(
    antigravity_queue,
    AgentKind::Antigravity,
    Feature::Queue,
    true
);
ac1_flag_test!(
    antigravity_pending_survives_restart,
    AgentKind::Antigravity,
    Feature::PendingSurvivesRestart,
    true
);

#[test]
fn a_false_flag_refuses_with_a_plain_reason_naming_feature_and_agent() {
    let none = Flags {
        steer: false,
        interrupt: false,
        fork: false,
        rollback: false,
        queue: false,
        pending_survives_restart: false,
    };
    let words = [
        (Feature::Steer, "mid-turn"),
        (Feature::Interrupt, "stop"),
        (Feature::Fork, "branch"),
        (Feature::Rollback, "rewind"),
        (Feature::Queue, "hold a message"),
        (Feature::PendingSurvivesRestart, "restarts"),
    ];
    for (feature, word) in words {
        // A hypothetical agent with every flag false: each feature is refused.
        let reason =
            reason_in(none, "Newbie", feature).unwrap_or_else(|| panic!("{}", feature.key()));
        assert!(reason.contains("Newbie"), "{reason}");
        assert!(reason.contains(word), "{reason}");
        assert!(reason.ends_with('.'), "{reason}");
        assert!(
            !reason.contains('\u{2014}') && !reason.contains('\u{2013}'),
            "{reason}"
        );
    }
    // The real table's reasons: the same text on the wire as from `require`.
    for kind in AgentKind::all() {
        let wire = capabilities(kind).unsupported_reasons;
        for feature in Feature::ALL {
            assert_eq!(
                require(kind, feature).err(),
                wire.get(feature.key()).cloned(),
                "{kind:?} {}",
                feature.key()
            );
        }
    }
    let grok = require(AgentKind::Grok, Feature::Steer).unwrap_err();
    assert!(grok.contains("Grok") && grok.contains("mid-turn"), "{grok}");
    assert!(capabilities(AgentKind::Claude)
        .unsupported_reasons
        .contains_key("fork"));
    assert!(!capabilities(AgentKind::Claude)
        .unsupported_reasons
        .contains_key("steer"));
}

#[test]
fn require_is_pure_and_refuses_before_any_side_effect() {
    // A refusal is a returned reason from a function that takes only the
    // agent and the feature: it has nothing to write. Calling it twice agrees.
    let first = require(AgentKind::Antigravity, Feature::Fork);
    assert_eq!(first, require(AgentKind::Antigravity, Feature::Fork));
    assert!(first.unwrap_err().contains("Antigravity"));
}

#[test]
fn h8_native_resume_adds_the_fork_flag_only_while_resuming() {
    let id = "11111111-2222-4333-8444-555555555555";
    let run = |resume: Option<&str>, fork: bool| {
        let mut s = spec("plan", resume, true);
        s.extras.fork_resume = fork;
        args_of(&test_command(AgentKind::Claude, &s))
    };
    let forked = run(Some(id), true);
    assert!(
        forked.windows(2).any(|w| w == ["--resume", id]),
        "{forked:?}"
    );
    assert!(forked.iter().any(|a| a == "--fork-session"));
    // An ordinary resume never forks.
    assert!(!run(Some(id), false).iter().any(|a| a == "--fork-session"));
    // A fresh retry (no resume id) never carries the flag on its own.
    assert!(!run(None, true).iter().any(|a| a == "--fork-session"));
    // An unsafe id is dropped by the backstop, flag and all.
    assert!(!run(Some("--bad"), true)
        .iter()
        .any(|a| a == "--fork-session"));
}

/// 2026-11-20: Hollow's servers reach an Antigravity chat turn over ACP only.
/// A one-shot (automation, teammate) runs `agy -p`, which takes none: it
/// carries no owned MCP, and a config handed to it is refused rather than
/// dropped quietly. The chat's servers go in `session/new`, never argv.
#[test]
fn antigravity_servers_are_for_acp_chats_only() {
    let mut chat = spec("acceptEdits", None, true);
    chat.extras.antigravity_acp = true;
    let mut oneshot = spec("acceptEdits", None, false);
    let config = r#"{"mcpServers":{"hollow_app":{"type":"http","url":"http://127.0.0.1:7/mcp","headers":{"Authorization":"Bearer x"}}}}"#;
    chat.mcp_config = Some(config.into());
    oneshot.mcp_config = Some(config.into());
    assert!(capabilities(AgentKind::Antigravity).owned_mcp);
    assert!(owned_mcp_for(AgentKind::Antigravity, &chat));
    assert!(!owned_mcp_for(AgentKind::Antigravity, &oneshot));
    assert!(validate_run_spec(AgentKind::Antigravity, &chat).is_ok());
    assert!(validate_run_spec(AgentKind::Antigravity, &oneshot)
        .unwrap_err()
        .contains("does not support Hollow-owned connectors"));
    // Claude and Codex are unchanged; Grok still has none.
    assert!(owned_mcp_for(AgentKind::Claude, &oneshot));
    assert!(!capabilities(AgentKind::Grok).owned_mcp);
    let start = antigravity_acp_start(&chat);
    assert_eq!(start.mcp_servers.len(), 1);
    assert_eq!(start.mcp_servers[0]["name"], "hollow_app");
    let argv = args_of(&test_command(AgentKind::Antigravity, &chat)).join(" ");
    assert!(
        !argv.contains("Bearer") && !argv.contains("127.0.0.1"),
        "{argv}"
    );
    // The chat's policy names tools, a one-shot's does not.
    assert_ne!(
        run_policy(AgentKind::Antigravity, &chat),
        run_policy(AgentKind::Antigravity, &oneshot)
    );
}
