mod account_pool;
mod acp_client;
mod activation;
mod activity_ledger;
mod agent;
mod agent_cli;
mod agent_models;
mod agent_out;
// AG9: chat-event to AG-UI translator; tests only until an HTTP route needs it.
#[cfg(test)]
mod ag_ui;
mod agent_sessions;
mod agent_settings;
mod app_actions;
mod app_broker;
mod app_icon_presets;
mod app_mcp;
mod app_sessions;
mod approval_terms;
mod approval_uses;
mod approvals;
mod artifacts;
mod ask_hollow;
mod asof;
mod audit;
mod automated_history;
mod automation_in_chat;
// Helper subcommands run background-only, out of the Dock (PER-34).
mod background_only;
mod backup_health;
mod brief_read;
mod browser_pane;
mod budgets;
mod build_changes;
mod build_slots;
mod build_workspaces;
mod builtin_connectors;
mod canvas_answers;
mod canvas_bridge_log;
mod canvas_export;
mod canvas_fence;
mod canvas_items;
mod capability_contract;
#[cfg(test)] // build.rs uses it; the lib only tests it.
mod cargo_target_rule;
mod chat;
mod chat_chain;
mod chat_context;
// AG1: the chat stream grammar checker; tests only until a runtime caller exists.
mod chat_branch;
#[cfg(test)]
mod chat_grammar;
// AG2: the typed `chat-event` schema (`ChatEventPayload` is generated from it);
// the builders in `chat.rs` write through it, the conformance check is tests only.
mod chat_event;
mod chat_limits;
mod chat_move;
// Lane M (2026-11-17): owed continuations and told move notices kept across a restart.
mod chat_move_owed;
mod chat_move_send;
mod chat_move_trigger;
mod chat_offline;
mod chat_pause;
mod chat_resume;
mod chat_rows;
mod chat_title;
mod child_status;
mod child_verdict;
mod custom_icons;
mod remote_folder;
mod remote_session;
mod remote_status;
mod subagents;
// Remote runners (2026-11-03): GitHub and Google Cloud on a runner turn.
mod onepassword;
mod runner_access;
mod runner_owner;
// Lane M (2026-11-17): routed COO turns kept across a restart.
mod routed_store;
// M3b (2026-11-15): the runner COO on `hollow serve`.
mod runner_coo;
// Remote runners 3 (2026-11-03): Settings, Remote runner, Set up.
mod runner_setup;
// Repo features R5: the citation check on Run records.
mod cite_check;
// COO rebuild L7: the CFO (agent, model, effort and account per turn).
mod cfo;
mod claude_accounts;
mod claude_defer;
mod claude_quota;
mod claude_runner;
// Code context (CC2): the code index and the hollow_code_context tools.
mod code_context;
mod code_context_mcp;
mod code_index;
// Code mode's sandboxed script runner (CM1a; served through hollow_app by CM1c).
mod codemode;
// Steering PR 3: Codex turns over `codex app-server` (not wired in yet).
mod codex_app_server;
mod codex_inventory;
mod codex_probe;
mod codex_rollout;
mod codex_transform;
mod commands;
mod companion_bridge;
mod companion_docs;
mod companion_inbox;
mod computer_activity;
mod computer_broker;
mod computer_cli;
mod computer_commands;
mod computer_doctor;
mod computer_keys;
mod computer_mcp;
mod computer_thismac;
mod config;
mod conflict_copies;
mod connector_hygiene;
mod connectors;
mod context_budget;
mod context_compact;
mod context_compiler;
mod context_gauge;
mod context_layers;
mod context_org_digest;
mod context_pins;
mod context_propose;
mod context_rank;
mod context_review;
mod context_roots;
#[cfg(test)]
mod context_roots_tests;
mod context_scan;
mod context_stale;
mod switch_bundle;
// Settings, Context (CX17): the layer rows' reads and person-only writes.
mod context_settings;
mod context_use;
#[cfg(test)]
mod coo_parity_tests_g1;
#[cfg(test)]
mod coo_parity_tests_g2;
#[cfg(test)]
mod coo_parity_tests_g3;
#[cfg(test)]
mod coo_parity_tests_g4;
#[cfg(test)]
mod coo_parity_tests_g5;
#[cfg(test)]
mod coo_parity_tests_g6;
#[cfg(test)]
mod coo_parity_tests_g7;
mod core_boot;
mod core_host;
mod delegation;
mod descendants;
mod diagnostics_bundle;
mod dictation;
mod digest;
mod disk_guard;
mod dispatcher_coo;
// PDF and Office text for connected projects, sources and OneDrive (repo features R14a).
mod doc_text;
mod effort;
mod embed_chunk;
mod embed_model;
mod eval_candidates;
mod event_triggers;
mod evidence;
mod extract;
mod file_stamp;
mod first_connect;
// H6: the first-run setup sheet's progress in config.json.
mod first_run;
mod flock;
mod folder_abstracts;
mod freeze_watchdog;
mod fresh_brief;
mod friction;
mod frontmatter;
mod gardener;
mod git_activity;
mod git_card;
mod github_pages;
mod goal_context;
mod health;
mod heartbeat;
mod hollow_databases;
mod hollow_repo;
mod host_scope;
mod ingest;
mod install_brings;
mod install_ready;
mod install_targets;
mod instance;
mod interaction_schema;
mod interactions;
mod interrupt;
mod jev;
mod jev_decide;
mod jev_study;
mod launch;
mod learning_capture;
mod pr_card;
// Leak checks (CX10): the flag-or-block switch, the flags ledger, the
// pre-push context check and the monthly audit.
mod leak_checks;
mod lease_lock;
mod lesson_memories;
mod lifecycle;
mod linear_link;
mod lock_order;
// "Connect this Mac's context" (two-Macs F3): the offer per Mac pair and Connect.
mod m365_auth;
mod m365_mcp;
mod mac_context;
mod main_agent;
mod main_nav;
mod mcp_attach;
mod mcp_envelope;
mod md_open;
mod media;
mod meeting_prep;
mod meeting_recap;
mod memory_guard;
mod memory_homes;
mod memory_maintenance;
mod memory_provenance;
mod memory_report;
// PX6: auto-memory entries sorted into Home, Personal, Project or Stale.
mod memory_sort;
mod mode_contract;
pub mod move_home;
mod move_home_icloud;
mod move_home_plan;
mod move_home_rekey;
mod move_home_run;
#[cfg(test)]
mod move_home_tests;
mod needs_you;
mod note_lifecycle;
mod notifications;
mod office_preview;
mod open_guard;
mod open_in;
mod overnight_summary;
mod pdf_preview;
mod people_dossier;
mod port_owners;
mod prep_pack;
mod private_memory;
mod process_capture;
mod process_skill;
mod provider_accounts;
mod quote_check;
mod records;
mod skill_learning;
mod today_brief;
mod triage;
mod vault_answer;
mod webkit_capture;
mod weekly_review;
// Save a Result as Markdown with its sources, the person's own (overnight F6).
mod result_export;
mod settings_keys;
// Settings export/import, the person's own (overnight F7).
mod next_step;
mod night_queue;
mod nudges;
mod operator;
mod page_cursor;
mod period;
mod phone_access;
// M2 PR 2: the peer link's MacBook side (paired Macs, the SSH tunnel, the client).
mod peer_client;
// PL1: the link's reconnect rules (jitter, one retry owner, idempotent commands).
mod peer_link;
mod peer_machines;
mod peer_tunnel;
mod peer_usage;
mod plan_rows;
mod plaud;
mod plaud_mcp_auth;
#[cfg(test)]
mod prep_bench;
mod prep_deadline;
mod prep_timing;
mod proc_info;
mod process_group;
mod profile_builder;
mod project_agents;
mod project_card;
mod project_icons;
mod project_intake;
mod project_next_move;
mod project_portfolio;
mod project_scripts;
mod project_workers;
mod settings_transfer;
// Repo features R14b: the BM25 content index for connected project files.
mod project_text_index;
mod question_mcp;
mod quick_add;
// Replay bench (S07): HEAD's resume plan over the recorded turn sequences,
// config-changed resets recorded → replayed, on OF17's snapshot; aggregates
// only, `#[ignore]`d real run.
#[cfg(test)]
mod replay_bench;
// Real-vault replay probes (OF17): merged pure functions over a copy of the
// workspace docs and a Sessions DB snapshot, aggregates only, `#[ignore]`d.
#[cfg(test)]
mod replay_probes;
// A chat turn's recall started at admission, taken within a cutoff (J3a).
mod recall_prefetch;
mod recall_trace;
mod repo_diff;
mod repo_trust;
mod restart_resume;
mod result_kind;
// The host retrieval broker (agent effectiveness F3): automatic retrieval,
// search_vault and the vault search app actions share it.
mod retrieval_broker;
// Real-vault known-item retrieval eval (OF01a, OB12's gate), `#[ignore]`d.
#[cfg(test)]
mod retrieval_eval;
mod review_queue;
mod run_activity;
// Broker peer checks and the private per-run MCP config file (merged plan
// slice 6, reviews A·S5 / B·F11).
mod run_auth;
mod run_effects;
mod run_guard;
mod run_manifest;
mod run_permit;
mod run_pump;
mod run_queue;
// M4: an automation's owner Mac (`runs_on`).
mod runs_on;
mod runs_on_claim;
mod scan_rules;
mod schedule_windows;
mod scratch;
mod secret_fds;
mod secret_store;
mod self_hosted;
mod self_hosted_forwarder;
mod self_hosted_jobs;
mod self_hosted_registry;
mod self_hosted_wake;
mod self_install;
mod serve;
mod serve_actions;
mod serve_control;
mod shell_path;
mod stable_layers;
mod stakeholder_sim;
mod steering;
mod step_spine;
// Receipt states and the live install-result read (slice 6, review B·F02);
// its own file so the fix in self_install.rs can be reverted without losing the tests.
mod agenda;
mod app_menu;
mod artifact_sessions;
mod calendar_source;
mod directory_connections;
mod inline_ai;
mod logo_options;
mod public_page;
mod recordings;
mod remote_auth;
mod remote_image;
mod remote_mcp;
mod tab_grants;
mod terminal;
mod untrusted;
// The install card's diff and the approval bound to a build receipt, Bypass
// included (slice 6, review A·S4/B·F12).
#[cfg(test)]
mod perf_hot_paths;
#[cfg(test)]
mod recall_probe;
#[cfg(test)]
mod self_install_approval_tests;
#[cfg(test)]
mod self_install_receipt_tests;
mod services;
mod services_mcp;
mod session_import;
mod session_store;
mod sessions;
mod setup;
mod setup_hook;
mod side_chat;
#[cfg(test)]
mod turn_prep_probe;
// Hollow's own writes to WORKSPACE.md, PLAN.md and SOURCES.md, and the
// lost-update safety net for parallel chats (concurrency PR 2, 2026-10-20).
#[cfg(unix)]
mod shared_codex;
mod shared_docs;
mod shared_mcp;
mod shared_mcp_host;
mod shortcuts;
// The confirm-tier audit: every action through the broker in every mode
// (overnight E4).
#[cfg(test)]
mod tier_audit_tests;
// The broker's attached-source chunks and on-demand reader (agent
// effectiveness F3 part 2).
mod run_attribution;
mod run_ledger;
mod skill_import;
mod skill_index;
mod skill_lint;
mod skill_portability;
mod skill_provenance;
mod skills;
mod source_evidence;
// Sources of truth (H4a): who owns each field; owner switches.
#[cfg(test)]
mod source_pins;
mod sources;
mod state_file;
// Context sync engine (CX6a): the personal root's git sync, its own thread.
mod sync_engine;
// Write-once binaries between the Macs over the runner's ssh (V4), its own thread.
mod sync_files;
// Turn checkpoints with a safe restore in a chat's own worktree (CP1).
mod task_broker;
mod task_state;
mod tasks_mcp;
mod team_mcp;
mod this_mac;
mod turn_checkpoints;
// Tests that must not straddle local midnight (overnight D2).
#[cfg(test)]
mod test_clock;
#[cfg(test)]
mod test_http;
// Earlier test runs' per-process folders in $TMPDIR (overnight S6).
#[cfg(test)]
mod test_temp;
mod themes;
mod tracker;
mod tracker_import;
mod tracker_sync;
mod trigger_evaluator;
mod trigger_scanner;
mod update_client;
mod update_install;
// Wire enums that exist only to generate `src/generated/ipc.ts` (ts-rs);
// Rust never constructs them.
#[cfg(test)]
#[allow(dead_code)]
mod ts_bindings;
mod turn_feedback;
mod turn_request;
mod types;
mod usage;
mod usage_ledger;
mod usage_prices;
mod value;
mod vault;
#[cfg(test)]
mod vault_bench;
mod vault_clash;
mod vault_commitments;
mod vault_embed_index;
mod vault_goals;
mod vault_hygiene;
mod vault_index;
mod vault_learning;
mod vault_mcp;
mod vault_people;
mod vault_projects;
mod vault_search;
#[cfg(test)]
mod vault_search_eval;
mod vault_search_index;
mod vault_tasks;
mod vault_watch;
mod vault_workflows;
mod visual_prompt;
mod wake_me;
mod window_state;
mod work_origin;
mod workflow_hygiene;
mod workflow_permissions;
mod workflow_precheck;
mod workflow_retry;
mod workflows;
mod workspace_activity;
mod workspace_aliases;
mod workspace_canvases;
mod workspace_connections;
mod workspace_merge;
mod workspace_pins;
mod workspace_reentry;
mod workspace_results;
mod workspace_reviews;
mod workspace_shape;
mod workspaces;
mod worktree_hygiene;
mod xlsx_cells;

use commands::AppState;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use tauri_plugin_global_shortcut::ShortcutState;

fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        match lifecycle::main_window_state(app) {
            Some(state) if state.ui_available() => {
                if window.hide().is_ok() {
                    lifecycle::set_in_tray(true);
                }
            }
            _ => {
                let _ = window.unminimize();
                if window.show().is_ok() {
                    lifecycle::set_in_tray(false);
                }
                let _ = window.set_focus();
            }
        }
    }
}

/// A system-wide key or tray item that lands in the main window: bring it
/// forward and run the webview's shared action `id` there (`app-menu`, so
/// `src/actionRun.ts` runs it like its palette row). `main-agent` is the
/// Main agent (main agent M1, the top bar's orb); `quick-ingest` is Import a
/// source, the Add sheet set to a Library source; `quick-capture` is the Add
/// sheet set to a task (S5 removed both separate overlay windows).
fn show_main_with(app: &AppHandle, id: &str) {
    use tauri::Emitter;
    // The Add sheet steps back here once something is saved (S5 review).
    if id == "quick-capture" || id == "quick-ingest" {
        let state = lifecycle::main_window_state(app);
        lifecycle::note_came_from(state.and_then(|s| lifecycle::came_from(&s)));
    }
    lifecycle::bring_main_forward(app);
    let _ = app.emit(app_menu::MENU_EVENT, id.to_string());
}

/// Run as the teamwork MCP stdio server (the hidden `team-mcp` subcommand,
/// spawned by chat/workflow `claude` runs via `--mcp-config`). Never boots
/// the Tauri GUI — it just speaks JSON-RPC on stdin/stdout.
pub fn run_team_mcp() {
    team_mcp::serve();
}

/// Run as the computer-control MCP stdio server (the hidden `computer-mcp`
/// subcommand, spawned by chat `claude` runs via `--mcp-config`). Same
/// shape as `run_team_mcp` — JSON-RPC on stdin/stdout, no GUI.
pub fn run_computer_mcp() {
    computer_mcp::serve();
}

/// The `computer` CLI twin (CU4): `hollow computer call <tool> '<json>'`, the
/// shell path to the same broker for agents with no per-run MCP flag.
pub fn run_computer_cli() {
    computer_cli::run_cli();
}

/// Run as the ask-your-vault MCP stdio server (the hidden `vault-mcp`
/// subcommand, spawned by chat `claude` runs via `--mcp-config`). Same shape
/// as the others — JSON-RPC on stdin/stdout, no GUI; read-only `search_vault`.
pub fn run_vault_mcp() {
    vault_mcp::serve();
}

/// Run as the code-context MCP stdio server (the hidden `code-context-mcp`
/// subcommand, CC2): read-only callers, impact and reads of one checkout.
pub fn run_code_context_mcp() {
    code_context_mcp::serve();
}

/// The `code-context` CLI twin (CC2): `hollow code-context <tool> '<json>'`
/// prints the same JSON the MCP server returns. Never reads stdin.
pub fn run_code_context_cli() {
    code_context_mcp::run_cli();
}

/// Run as the question-card MCP stdio server (the hidden `questions-mcp`
/// subcommand, spawned by chat runs via `--mcp-config`, Paperclip PR 13).
/// Stateless: it validates `ask_user_questions` requests and answers
/// "Posted"; the GUI records the card from the completed tool event.
pub fn run_questions_mcp() {
    question_mcp::serve();
}

/// Run as the task-tools MCP stdio server (the hidden `tasks-mcp`
/// subcommand, spawned by chat and workflow runs via `--mcp-config`,
/// Paperclip PR 12). Reads the vault root from `HOLLOW_VAULT_ROOT`; every
/// write is forwarded to the GUI's task broker over its socket.
pub fn run_tasks_mcp() {
    tasks_mcp::serve();
}

/// Run as the Hollow app MCP stdio server (the hidden `app-mcp` subcommand,
/// PR A1). Stateless: every call is forwarded to the GUI's app broker.
pub fn run_app_mcp() {
    app_mcp::serve();
}

/// `hollow claude-defer-hook`: the PreToolUse hook of an ask-first Claude
/// chat turn (AG4b): defers a gated tool call until the person has answered
/// its approval card.
pub fn run_claude_defer_hook() {
    claude_defer::run_hook();
}

/// `hollow m365-mcp`: the built-in Microsoft 365 connector (calendar and
/// files, read-only), on Hollow's own Microsoft sign-in.
pub fn run_m365_mcp() {
    m365_mcp::serve();
}

/// `hollow remote-mcp`: a remote connector on Hollow's own sign-in, bridged
/// to stdio (five asks B2).
pub fn run_remote_mcp() {
    remote_mcp::serve();
}

/// `main.rs` calls this first, with the first argument: a hidden subcommand
/// sets its activation policy to Prohibited, so a helper never shows a
/// second Hollow in the Dock (PER-34, `background_only`). A plain launch is
/// left alone. Returns whether the policy was set.
pub fn enter_background_only(first: Option<&str>) -> bool {
    background_only::enter(first)
}

/// What `main` asks once no command matched (launch.rs): `Some(code)` to
/// exit with it, `None` to start the app. `--version` and `--help` answer,
/// an unknown command or option is refused, and a second launch hands off to
/// the running app.
pub fn gate_gui_launch() -> Option<i32> {
    launch::gate(
        &std::env::args_os()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    )
}

/// `hollow services-mcp` (Build PR B6): forwards service calls to the GUI.
pub fn run_services_mcp() {
    services_mcp::serve();
}

/// `hollow companion-fixtures <dir>`: write the companion apps' golden
/// fixtures (`docs/COMPANION_PROTOCOL.md`), generated by the real code.
pub fn run_companion_fixtures() {
    let Some(out) = std::env::args().nth(2) else {
        eprintln!("usage: hollow companion-fixtures <dir>");
        std::process::exit(2);
    };
    let commit = app_menu::build_identity().commit;
    match companion_inbox::export_fixtures(std::path::Path::new(&out), &commit) {
        Ok(path) => println!("{}", path.display()),
        Err(error) => {
            eprintln!("COMPANION_FIXTURES_FAIL {error}");
            std::process::exit(1);
        }
    }
}

/// `hollow companion-apply <vault> <store> [CODE]` (debug builds only): one
/// applier pass over a temporary vault, for the companion apps' end-to-end
/// tests. Refuses any vault outside the temp folder.
pub fn run_companion_apply() {
    #[cfg(debug_assertions)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.len() < 4 {
            eprintln!("usage: hollow companion-apply <temp vault> <store dir> [pairing code]");
            std::process::exit(2);
        }
        match companion_inbox::apply_once_cli(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
            args.get(4).map(String::as_str),
        ) {
            Ok(summary) => println!("{}", serde_json::to_string(&summary).unwrap_or_default()),
            Err(error) => {
                eprintln!("COMPANION_APPLY_FAIL {error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(debug_assertions))]
    {
        eprintln!("companion-apply exists only in debug builds");
        std::process::exit(2);
    }
}

/// `hollow companion-bridge-once <vault> <store> <url> <token> [CODE]` (debug
/// builds only): one backend bridge pass over a temporary vault, for the
/// companion apps' end-to-end tests. Refuses any vault outside the temp folder.
pub fn run_companion_bridge_once() {
    #[cfg(debug_assertions)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.len() < 6 {
            eprintln!(
                "usage: hollow companion-bridge-once <temp vault> <store dir> <backend url> <applier token> [pairing code]"
            );
            std::process::exit(2);
        }
        match companion_bridge::bridge_once_cli(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
            &args[4],
            &args[5],
            args.get(6).map(String::as_str),
        ) {
            Ok(summary) => println!("{summary}"),
            Err(error) => {
                eprintln!("COMPANION_BRIDGE_FAIL {error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(debug_assertions))]
    {
        eprintln!("companion-bridge-once exists only in debug builds");
        std::process::exit(2);
    }
}

/// `hollow tracker-import --config <file> --vault <dir> <mode> …`: the
/// one-time Notion-to-vault importer and tracker rekey (`tracker_import`).
/// Reads Notion and the tracker only; never boots the GUI.
pub fn run_tracker_import() {
    std::process::exit(tracker_import::main(std::env::args().skip(2).collect()));
}

/// `hollow serve`: Hollow's core with no window (M1 PR 2, `serve.rs`): the
/// run queue, schedulers, COO, phone API and agent tool sockets on a plain
/// Tokio runtime, never Tauri's event loop. Exits with the status it prints.
pub fn run_serve() {
    std::process::exit(serve::main(std::env::args().skip(2).collect()));
}

/// `Hollow move-home ...`: the Move Hollow home dry run (default, read-only),
/// and the detached helper that runs or rolls back a staged move while the
/// app is closed (`move_home.rs`). Never boots the GUI.
pub fn run_move_home() {
    std::process::exit(move_home::cli_main(std::env::args().skip(2).collect()));
}

/// `hollow install-update ...`: the installer inside the app (updates U4,
/// `update_install`): swap in a staged, verified update, reopen it, go back
/// after two failed starts; `--go-back` and `--status`. Never boots the GUI.
pub fn run_install_update() {
    std::process::exit(update_install::cli::main(
        std::env::args().skip(2).collect(),
    ));
}

/// Before a plain GUI launch: roll back a move whose helper died mid-run
/// (`move_home::launch_guard`); `Some(code)` means exit instead.
pub fn move_home_launch_guard() -> Option<i32> {
    move_home::launch_guard()
}

/// `hollow mcp-launch <run config file> <server> <sha256>`: how a Codex run
/// starts a stdio MCP server without its command, arguments or environment
/// (proofs, connector secrets) on Codex's argv (`run_auth`, merged plan
/// slice 6). Refuses a file whose digest no longer matches.
/// Execs the server; never boots the GUI.
pub fn run_mcp_launch() {
    std::process::exit(run_auth::launch_main());
}

/// Private relay child, without booting a second GUI.
pub fn run_codex_relay() {
    #[cfg(unix)]
    {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|runtime| runtime.block_on(shared_codex::run_relay()));
        std::process::exit(if matches!(result, Ok(Ok(()))) { 0 } else { 1 });
    }
    #[cfg(not(unix))]
    std::process::exit(2);
}

/// `hollow jev-study <fixture-dir> <out-dir> (--endpoint <loopback> |
/// --typesafe) …`: the Jev study runner (agent effectiveness E3,
/// `jev_study`). Refuses to run without an explicit target; never boots the
/// GUI.
pub fn run_jev_study() {
    std::process::exit(jev_study::main(std::env::args().skip(2).collect()));
}

/// `hollow reaper <hollow pid> <run ledger path>`: the crash reaper the GUI
/// starts at setup (`process_group::reaper`, A·R8). It waits for that Hollow
/// to exit, however it exits, then ends the runs it left. Never boots the
/// GUI.
pub fn run_reaper() {
    let mut args = std::env::args().skip(2);
    let parent = args.next().and_then(|arg| arg.parse::<i32>().ok());
    match (parent, args.next()) {
        (Some(parent), Some(ledger)) if parent > 1 => {
            process_group::reaper::run(parent, ledger.into());
        }
        _ => {
            eprintln!("usage: hollow reaper <hollow pid> <run ledger path>");
            std::process::exit(2);
        }
    }
}

/// Everything a quit stops, once per process: every live agent run (R2: its
/// ledger entries, SIGTERM → grace → SIGKILL, at most
/// `orphans::QUIT_BUDGET`) alongside the services (Build PR B6). Both
/// `RunEvent::Exit` and a termination signal (A·R8) get here; the second
/// caller waits for the first to finish.
pub(crate) fn stop_children_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once_force(|_| {
        let runs = std::thread::spawn(process_group::orphans::settle_at_quit);
        services::stop_all_on_exit();
        // The terminal drawer's shells (parity 7.2): each one's whole group.
        terminal::stop_all_on_exit();
        // Self-hosted models' loopback forwarders (Q2): no listener outlives
        // the app.
        self_hosted_forwarder::stop_all();
        let _ = runs.join();
    });
}

/// SIGTERM, SIGINT and SIGHUP would end this process before `RunEvent::Exit`
/// could settle its runs (A·R8): take them over, settle once
/// (`stop_children_once`), then exit through Tauri, with a hard exit if the
/// event loop does not get there in time.
#[cfg(unix)]
fn settle_on_quit_signals(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let signals = match process_group::orphans::QuitSignals::install() {
            Ok(signals) => signals,
            Err(error) => {
                eprintln!("[orphans] cannot watch quit signals: {error}");
                return;
            }
        };
        let exit = move |signal: i32| {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                std::process::exit(128 + signal);
            });
            handle.exit(128 + signal);
        };
        process_group::orphans::settle_on_signal(signals, stop_children_once, exit).await;
    });
}

/// Hidden `embed-selftest` subcommand: prove the bundled embedding model loads
/// and runs from INSIDE the signed `.app` (the one thing `cargo test` cannot
/// reach — it loads from the dev path). Resolves the model from the running
/// bundle's `Resources`, embeds a probe, checks dimension + L2-norm + a
/// similar-beats-dissimilar ranking, prints `EMBED_SELFTEST_OK`, and exits.
/// Never boots the GUI.
pub fn run_embed_selftest() {
    let dir = embed_model::default_model_dir();
    match embed_selftest_inner(&dir) {
        Ok((dim, norm)) => {
            println!(
                "EMBED_SELFTEST_OK dim={dim} norm={norm:.4} model_dir={}",
                dir.display()
            );
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("EMBED_SELFTEST_FAIL {e} model_dir={}", dir.display());
            std::process::exit(1);
        }
    }
}

fn embed_selftest_inner(dir: &std::path::Path) -> Result<(usize, f32), String> {
    let v = embed_model::embed_passage(dir, "hello world")?;
    let dim = v.len();
    if dim != 384 {
        return Err(format!("expected 384 dims, got {dim}"));
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if (norm - 1.0).abs() >= 1e-3 {
        return Err(format!("not L2-normalized: norm={norm}"));
    }
    // Ranking sanity (catches a wrong pooling choice the norm check cannot).
    let q = embed_model::embed_query(dir, "How do I reset my password?")?;
    let near = embed_model::embed_passage(dir, "Steps to recover or reset your account password.")?;
    let far = embed_model::embed_passage(dir, "The chef seared the scallops in brown butter.")?;
    let dot = |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() };
    let s_near = dot(&q, &near);
    let s_far = dot(&q, &far);
    if s_near <= s_far {
        return Err(format!(
            "semantic ranking wrong: near={s_near:.4} far={s_far:.4}"
        ));
    }
    Ok((dim, norm))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load config eagerly. A missing/invalid config.json is no longer fatal:
    // the app boots in *setup mode* (placeholder config, `configured: false`)
    // and the frontend renders the Onboarding wizard instead of the shell.
    // `connect_directories` writes config.json and the app restarts to pick it up.
    let app_state = match core_boot::load_state() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("config.json not loaded — booting into setup mode: {e}");
            AppState::unconfigured(e)
        }
    };
    let configured = app_state.configured;
    // What this launch is and which folders it resolved, for the test bed
    // (merged plan step 0): `{app data}/instance.json`.
    instance::write_manifest(
        &app_state.config_path,
        configured.then_some(app_state.config.vault_path.as_path()),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        // Nothing navigates the main window away from the app: the guard
        // blocks web/mail links like any other foreign URL and never opens
        // them itself. Clicked links are opened by the frontend (linkGuard.ts,
        // the artifact frames' nav bridge), see main_nav.rs.
        .plugin(main_nav::plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Frontier F8: reopen at the size and place the user left the window.
        // Only `main`; the overlays and previews position themselves.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .with_denylist(&["md-preview"])
                .build(),
        )
        // The updater client (`update_client`): its check and download
        // (minisign signature) only, built at runtime with the channel,
        // token and public key; Hollow's own installer does the swap. No
        // capability grants its commands to the webview.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Launch-at-login. `LaunchAgent` writes a per-user
        // ~/Library/LaunchAgents plist (no admin rights). The `--hidden` arg
        // is appended to argv whenever the OS auto-launches Hollow, so the
        // setup show-thread below leaves `main` hidden — the app comes up in
        // the menu-bar tray, the way a background runner should.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        // Sandboxed artifact documents (chat ```html fences, workflow briefs,
        // preview-pane .html). Served off a dedicated scheme so the document
        // gets ITS OWN CSP header instead of inheriting the app CSP — which
        // is what kept artifact inline <script>s dead in installed builds.
        // See artifacts.rs for the security model.
        .register_uri_scheme_protocol("artifact", |_ctx, request| artifacts::serve(&request))
        // Local video (inline video, 2026-10-17): opaque tokens from
        // `media_grant`, 2 MiB Range chunks read off the main thread, `main`
        // webview only. See media.rs for the security model.
        .register_asynchronous_uri_scheme_protocol("hollow-media", |ctx, request, responder| {
            media::serve(ctx.webview_label(), request, responder)
        })
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // Which keys these are is a setting (shortcuts.rs).
                    match shortcuts::global_for(shortcut).as_deref() {
                        Some("quick-capture") => show_main_with(app, "quick-capture"),
                        Some("quick-ingest") => show_main_with(app, "quick-ingest"),
                        Some("main-agent") => show_main_with(app, "main-agent"),
                        _ => {}
                    }
                })
                .build(),
        )
        .manage(app_state)
        .manage(md_open::MdOpenState::default())
        .invoke_handler(tauri::generate_handler![
            commands::workspaces_api::list_workspaces,
            commands::workspaces_api::workspace_icons,
            shortcuts::get_shortcut_settings,
            shortcuts::set_shortcut_settings,
            shortcuts::suspend_shortcuts,
            app_menu::set_menu_enabled,
            connectors::list_connectors,
            connectors::save_connector,
            connectors::check_connectors,
            m365_auth::m365_sign_in,
            m365_auth::m365_sign_out,
            m365_auth::m365_status,
            plaud_mcp_auth::plaud_mcp_sign_in,
            plaud_mcp_auth::plaud_mcp_sign_out,
            connectors::delete_connector,
            connectors::import_connector,
            connectors::connector_use_hollow_sign_in,
            connectors::remote_connector_sign_in,
            connectors::remote_connector_sign_out,
            jev::jev_status,
            jev::save_jev_settings,
            jev::remove_jev_key,
            jev::test_jev_key,
            jev_decide::jev_decisions_report,
            schedule_windows::automation_windows,
            schedule_windows::set_automation_windows,
            schedule_windows::set_automation_window,
            digest::digest_weekly,
            gardener::gardener_scan,
            workspace_connections::workspace_connections,
            extract::extract_run,
            extract::extract_status,
            records::records_decisions,
            records::records_people,
            people_dossier::records_refresh_people,
            asof::history_read_as_of,
            asof::workspace_metrics_daily,
            records::records_propose_decision,
            records::records_import_brief_decisions,
            records::records_person,
            records::records_supersede_decision,
            records::records_set_decision_status,
            commands::workspaces_api::read_work_file,
            commands::workspaces_api::plan_task_candidates,
            commands::workspaces_api::toggle_plan_row,
            commands::workspaces_api::project_portfolio,
            commands::workspaces_api::set_project_portfolio,
            commands::workspaces_api::workspace_activity,
            commands::workspaces_api::create_workspace,
            commands::workspaces_api::ensure_quick_chats_workspace,
            commands::workspaces_api::ensure_main_agent_workspace,
            commands::workspaces_api::create_main_standing_orders,
            main_agent::main_agent_children,
            main_agent::main_agent_status,
            ask_hollow::ask_hollow_plan,
            ask_hollow::ask_hollow_run,
            vault_answer::vault_answer_ask,
            vault_answer::vault_answer_save,
            weekly_review::weekly_review,
            weekly_review::weekly_review_take,
            weekly_review::weekly_review_save,
            today_brief::today_brief_cached,
            today_brief::today_brief,
            prep_pack::prep_pack,
            meeting_recap::meeting_recap,
            triage::triage_arrivals,
            triage::triage_suggest,
            triage::triage_mark,
            dictation::dictation_status,
            dictation::dictation_authorize,
            dictation::dictation_start,
            dictation::dictation_stop,
            dictation::open_dictation_settings,
            main_agent::main_agent_activity,
            main_agent::get_main_agent_settings,
            main_agent::set_main_agent_settings,
            chat_limits::get_chat_limits,
            chat_limits::set_chat_limits,
            commands::workspaces_api::read_workspace,
            commands::workspaces_api::workspace_sessions,
            commands::results_api::workspace_results,
            commands::results_api::set_result_review,
            commands::results_api::result_snapshot,
            commands::results_api::workspace_digests,
            commands::results_api::result_preview,
            commands::results_api::list_all_results,
            commands::results_api::list_all_results_peek,
            commands::agenda_api::agenda_events,
            commands::agenda_api::calendar_source_status,
            commands::agenda_api::set_calendar_source,
            commands::agenda_api::open_calendar_settings,
            commands::agenda_api::list_focus_blocks,
            commands::agenda_api::save_focus_block,
            commands::agenda_api::delete_focus_block,
            commands::recordings_api::list_recordings,
            commands::recordings_api::read_recording,
            commands::recordings_api::recording_audio_url,
            commands::recordings_api::set_recording_link,
            commands::recordings_api::keep_recording_transcript,
            commands::canvases_api::list_canvases,
            commands::canvases_api::record_canvas_bridge_refusal,
            commands::canvases_api::read_canvas,
            commands::canvases_api::canvas_item_source,
            commands::canvases_api::canvas_plan_items,
            commands::canvases_api::read_canvas_revision,
            commands::canvases_api::record_canvas,
            commands::canvases_api::save_canvas_state,
            workspace_reviews::list_annotation_sets,
            workspace_reviews::read_annotation_set,
            workspace_reviews::save_annotation_set,
            workspace_reviews::add_annotation_capture,
            commands::workspaces_api::update_workspace,
            commands::workspaces_api::set_department,
            commands::workspaces_api::set_workspace_canvas_theme,
            commands::workspaces_api::trash_workspace,
            commands::workspaces_api::restore_workspace,
            commands::workspaces_api::trashed_workspaces,
            commands::workspaces_api::preview_workspace_merge,
            commands::workspaces_api::archive_shared_docs,
            commands::workspaces_api::merge_workspace,
            commands::workspaces_api::undo_workspace_merge,
            commands::workspaces_api::create_build_workspace,
            commands::workspaces_api::remove_build_worktree,
            commands::workspaces_api::build_repo_candidates,
            commands::workspaces_api::quick_chat_scratch,
            commands::workspaces_api::adopt_chat_scratch,
            commands::workspaces_api::clone_repository,
            commands::workspaces_api::list_project_scripts,
            commands::workspaces_api::save_project_scripts,
            commands::workspaces_api::run_project_script,
            commands::build_api::build_status,
            commands::build_api::build_rail_states,
            commands::build_api::build_diff,
            commands::build_api::build_commit,
            commands::build_api::build_push,
            commands::build_api::build_open_pr,
            commands::build_api::build_set_runs_on,
            commands::build_api::remote_runner,
            commands::build_api::runner_status,
            commands::workspaces_api::workspace_runs_on,
            commands::workspaces_api::workspace_set_runs_on,
            commands::runner_api::runner_access_get,
            commands::runner_api::runner_access_set,
            commands::runner_api::runner_github_status,
            commands::runner_api::runner_github_save_token,
            commands::runner_api::runner_github_clear_token,
            commands::runner_api::runner_github_set_source,
            commands::onepassword_api::onepassword_status,
            commands::onepassword_api::onepassword_check,
            commands::onepassword_api::onepassword_set_enabled,
            commands::onepassword_api::onepassword_set_reference,
            commands::onepassword_api::onepassword_clear_reference,
            commands::runner_api::runner_set,
            commands::runner_api::runner_check_host,
            commands::runner_api::runner_record_owner,
            commands::runner_api::runner_setup_plan,
            commands::runner_api::runner_setup_check,
            commands::runner_api::runner_setup_apply,
            commands::runner_api::runner_cloud_estimate,
            commands::runner_api::runner_cloud_create,
            commands::build_api::build_update_from_base,
            commands::build_api::app_build_status,
            commands::build_api::app_build_brings,
            commands::build_api::start_app_build,
            commands::build_api::cancel_app_build,
            commands::build_api::self_install_result,
            commands::build_api::self_install_receipt,
            lifecycle::exit_running_work,
            lifecycle::resolve_quit,
            lifecycle::pending_quit_request,
            lifecycle::after_summoned_capture,
            commands::build_api::install_targets,
            commands::build_api::prune_install_targets,
            commands::build_api::worktree_hygiene,
            commands::build_api::clean_worktrees,
            commands::build_api::ack_self_install_result,
            update_client::ipc::update_get_state,
            update_client::ipc::update_get_settings,
            update_client::ipc::update_set_channel,
            update_client::ipc::update_set_overnight,
            update_client::ipc::update_check_now,
            update_client::ipc::update_connect,
            update_client::ipc::update_confirm_connect,
            update_client::ipc::update_discard_connect,
            update_client::ipc::update_disconnect,
            update_client::ipc::update_dismiss,
            update_client::ipc::update_install_now,
            update_client::ipc::update_install_when_idle,
            update_client::ipc::update_cancel_wait,
            update_client::ipc::update_last_result,
            update_client::ipc::update_ack_last_result,
            update_client::publish::update_publish_status,
            update_client::publish::update_publish_main,
            update_client::publish::update_withdraw_main,
            update_client::publish::update_set_publisher,
            update_client::ipc::update_offer_local_build,
            commands::build_api::list_services,
            browser_pane::browser_open,
            browser_pane::browser_show,
            browser_pane::browser_go,
            browser_pane::browser_url,
            browser_pane::browser_snapshot,
            canvas_export::canvas_export,
            canvas_export::canvas_measure,
            canvas_export::canvas_capture_job,
            canvas_export::canvas_capture_ready,
            canvas_export::canvas_capture_failed,
            browser_pane::browser_inspect,
            browser_pane::browser_page_text,
            browser_pane::browser_grant_tab,
            browser_pane::browser_revoke_tab,
            browser_pane::browser_tab_grants,
            browser_pane::browser_close,
            commands::build_api::stop_service,
            commands::build_api::service_log,
            commands::workspaces_api::attach_workspace_source,
            commands::workspaces_api::preview_source_refresh,
            commands::workspaces_api::apply_source_refresh,
            commands::context_scan_api::context_scan_sources,
            commands::context_scan_api::start_context_scan,
            commands::context_scan_api::context_scan_status,
            commands::context_scan_api::list_context_scan,
            commands::context_scan_api::cancel_context_scan,
            commands::context_review_api::context_review_list,
            commands::context_review_api::context_review_accept,
            commands::context_review_api::context_review_skip,
            commands::context_review_api::context_review_unskip,
            first_run::first_run_status,
            first_run::first_run_set_step,
            first_run::first_run_reopen,
            commands::session_import_api::session_import_sources,
            commands::session_import_api::inspect_session_import,
            commands::session_import_api::session_import_status,
            commands::session_import_api::list_session_import,
            commands::session_import_api::preview_session_import,
            commands::session_import_api::commit_session_import,
            commands::session_import_api::cancel_session_import,
            commands::session_import_api::continue_imported_session,
            commands::session_import_api::native_resume_offer,
            commands::session_import_api::continue_imported_session_native,
            commands::session_import_api::set_session_import_search,
            commands::sessions_api::list_agent_sessions,
            commands::sessions_api::list_interrupted_chats,
            commands::sessions_api::resume_interrupted_chats,
            commands::sessions_api::read_agent_session,
            commands::sessions_api::list_artifact_sessions,
            commands::sessions_api::read_artifact_session_context,
            commands::sessions_api::read_agent_conversation,
            commands::sessions_api::read_tool_result,
            commands::sessions_api::archive_agent_session,
            commands::sessions_api::list_conversation_organization,
            commands::sessions_api::organize_conversation,
            commands::sessions_api::configure_conversation_settlement,
            commands::sessions_api::sweep_conversation_settlement,
            commands::sessions_api::move_agent_session,
            commands::sessions_api::rename_agent_session,
            commands::sessions_api::agent_session_live,
            commands::sessions_api::session_store_status,
            chat_chain::conversation_chain,
            subagents::session_subagents,
            commands::interactions_api::list_pending_interactions,
            commands::interactions_api::list_open_interactions,
            commands::interactions_api::resolve_interaction,
            commands::interactions_api::dismiss_interaction,
            commands::interactions_api::reopen_interaction,
            commands::needs_you_api::list_needs_you,
            commands::needs_you_api::session_target,
            commands::needs_you_api::search_sessions,
            commands::needs_you_api::conflict_copies,
            commands::needs_you_api::conflict_copy_pair,
            commands::needs_you_api::resolve_conflict_copy,
            commands::needs_you_api::dismiss_conflict_copy,
            commands::needs_you_api::undismiss_conflict_copy,
            commands::needs_you_api::answer_lesson_memory,
            commands::needs_you_api::restore_lesson_memory,
            commands::feedback_api::record_turn_feedback,
            commands::feedback_api::list_feedback,
            commands::feedback_api::export_feedback,
            commands::feedback_api::clear_feedback,
            commands::feedback_api::file_eval_candidate,
            commands::tasks_tools_api::set_tasks_mcp_enabled,
            commands::tasks_tools_api::tasks_mcp_enabled,
            commands::app_bridge_api::app_ui_respond,
            commands::app_bridge_api::take_pending_ui_settings,
            commands::app_bridge_api::ack_pending_ui_settings,
            commands::app_tools_api::set_app_mcp_enabled,
            commands::app_tools_api::app_mcp_enabled,
            // The task tracker (overhaul A2): Settings' Task tracker and the
            // Tasks page read these; the writes answer "not yet" until T3.
            commands::tracker_api::tracker_status,
            commands::tracker_api::tracker_view,
            commands::tracker_api::tracker_issue,
            commands::tracker_api::tracker_issue_history,
            commands::tracker_api::tracker_issue_work,
            commands::tracker_api::tracker_team_meta,
            commands::tracker_api::tracker_link_setup,
            commands::tracker_api::take_linear_links,
            commands::tracker_api::tracker_sync,
            commands::tracker_api::edit_tracker_item,
            commands::tracker_api::create_tracker_task,
            commands::tracker_api::create_tracker_project,
            commands::tracker_api::archive_tracker_task,
            commands::tracker_api::comment_tracker_issue,
            commands::tracker_api::write_friday_status,
            commands::tracker_api::attach_canvas_to_issue,
            commands::tracker_api::set_tracker_key,
            commands::tracker_api::clear_tracker_key,
            commands::tracker_api::use_keychain_tracker_key,
            commands::tracker_api::set_tracker_settings,
            // Sources of truth (H4a): Settings, Sources' rows and owner switches.
            commands::sources_api::sources_status,
            commands::sources_api::sources_set,
            commands::sources_api::sources_migration_plan,
            commands::sources_api::sources_migration_apply,
            commands::sources_api::sources_migration_rollback,
            commands::sources_api::sources_migration_status,
            // Event triggers (COO rebuild F2).
            commands::event_triggers_api::event_triggers_status,
            commands::event_triggers_api::set_event_trigger,
            commands::usage_api::usage_summary,
            commands::usage_api::ledger_query,
            commands::usage_api::job_timing,
            // Repo features R6: the usage ring's Context section.
            commands::usage_api::conversation_context_use,
            commands::usage_api::run_cost,
            commands::usage_api::price_table,
            commands::usage_api::set_price,
            value::summary,
            value::monthly_report,
            value::set_baseline,
            value::get_baselines,
            stakeholder_sim::personas_list,
            stakeholder_sim::personas_draft,
            stakeholder_sim::pressure_tests,
            step_spine::run_steps,
            step_spine::retry_run_from_step,
            commands::claude_quota_api::claude_quota,
            claude_accounts::claude_accounts,
            claude_accounts::add_claude_account,
            claude_accounts::update_claude_account,
            claude_accounts::remove_claude_account,
            claude_accounts::set_claude_account_routing,
            claude_accounts::set_claude_account_failover,
            claude_accounts::headroom,
            cfo::cfo_settings,
            cfo::cfo_save_settings,
            cfo::cfo_approve_extra_usage,
            cfo::cfo_view,
            claude_accounts::refresh_claude_accounts,
            provider_accounts::provider_accounts,
            provider_accounts::provider_sign_in,
            provider_accounts::provider_sign_out,
            commands::self_hosted_api::self_hosted_models,
            commands::self_hosted_api::save_self_hosted_model,
            commands::self_hosted_api::remove_self_hosted_model,
            commands::self_hosted_api::set_self_hosted_model_enabled,
            commands::self_hosted_api::test_self_hosted_model,
            commands::self_hosted_api::wake_self_hosted_model,
            commands::self_hosted_api::self_hosted_wake_status,
            commands::self_hosted_api::self_hosted_jobs,
            commands::self_hosted_api::set_self_hosted_job,
            commands::self_hosted_api::run_self_hosted_jobs_now,
            commands::self_hosted_api::run_self_hosted_job_as_now,
            commands::self_hosted_api::retry_self_hosted_job,
            commands::self_hosted_api::discard_self_hosted_job,
            commands::budgets_api::get_budgets,
            commands::budgets_api::save_budget,
            commands::budgets_api::delete_budget,
            commands::budgets_api::resume_budget,
            commands::run_queue_api::list_run_queue,
            commands::run_queue_api::cancel_queued_run,
            commands::repo_receipts_api::read_history_repo_changes,
            commands::system::operator_context,
            commands::system::mark_operator_profile_reviewed,
            commands::system::operator_context_switch,
            commands::system::set_operator_context_enabled,
            commands::profile_builder_api::profile_builder_view,
            commands::profile_builder_api::profile_builder_scan,
            commands::profile_builder_api::profile_builder_accept,
            commands::profile_builder_api::profile_builder_accept_all,
            commands::profile_builder_api::profile_builder_dismiss,
            commands::profile_builder_api::profile_builder_answer,
            commands::profile_builder_api::profile_builder_skip_question,
            commands::profile_builder_api::profile_builder_set_auto_learn,
            commands::system::list_people,
            commands::list_sessions,
            commands::read_session,
            commands::generate_session_title,
            commands::vault_tree,
            commands::vault_search,
            commands::semantic_search,
            commands::semantic_search_status,
            commands::set_semantic_search_enabled,
            commands::vault_items,
            commands::vault_note,
            commands::vault_write_note,
            commands::vault_give_up_write,
            commands::vault_create_note,
            commands::vault_open_in_obsidian,
            commands::obsidian_available,
            open_in::open_in_targets,
            open_in::open_in_open,
            commands::vault_move_note,
            commands::vault_delete_note,
            commands::vault_reveal_in_finder,
            commands::vault_notes_list,
            commands::vault_graph,
            commands::vault_backlinks,
            commands::list_history_runs,
            commands::list_note_history,
            commands::get_history_run,
            commands::read_history_version,
            commands::restore_history_change,
            commands::restore_history_run,
            commands::history_storage_status,
            commands::clear_automated_history,
            commands::list_evidence_receipts,
            commands::get_evidence_receipt,
            commands::run_context_manifest,
            commands::list_goals,
            commands::create_goal,
            commands::update_goal,
            commands::trash_goal,
            commands::restore_goal,
            commands::goal_alignment,
            commands::suggest_project_next_move,
            commands::list_review_inbox,
            commands::resolve_review_item,
            companion_inbox::companion_status,
            companion_inbox::companion_pairing_start,
            companion_inbox::companion_pairing_cancel,
            companion_inbox::companion_revoke_device,
            companion_bridge::companion_backend_status,
            companion_bridge::companion_backend_set,
            companion_bridge::companion_backend_repush,
            companion_bridge::companion_backend_claim,
            companion_bridge::companion_backend_test,
            companion_bridge::companion_scope_get,
            companion_bridge::companion_scope_set,
            phone_access::phone_access_status,
            phone_access::phone_access_set_enabled,
            phone_access::phone_access_pairing_start,
            phone_access::phone_access_pairing_cancel,
            phone_access::phone_access_machine_pairing_start,
            phone_access::phone_access_machine_pairing_cancel,
            phone_access::phone_access_approvals_start,
            phone_access::phone_access_approvals_cancel,
            phone_access::phone_access_approvals_off,
            phone_access::phone_access_approvals_wait,
            commands::peer_api::peer_machines,
            commands::peer_api::peer_pair,
            commands::chat_move_api::peer_forget,
            commands::chat_move_api::chat_move_to_runner,
            commands::chat_move_api::chat_move_keep_here,
            commands::chat_move_api::chat_continue_offline,
            commands::chat_move_api::chat_move_homes,
            commands::chat_move_api::chat_offline_rows,
            commands::chat_move_api::chat_move_release_here,
            commands::peer_api::peer_reorder_hosts,
            commands::peer_api::peer_cancel_connect,
            commands::peer_api::peer_pair_cancel,
            commands::peer_api::peer_sessions,
            commands::peer_api::peer_detail,
            commands::peer_api::peer_watch,
            commands::peer_api::peer_unwatch,
            commands::peer_api::peer_send,
            commands::peer_api::peer_start_coo,
            commands::peer_api::peer_stop,
            commands::peer_api::peer_answer,
            phone_access::phone_access_revoke,
            phone_access::phone_access_publish,
            phone_access::phone_access_unpublish,
            phone_access::phone_access_new_chat_mode,
            phone_access::phone_access_chat_modes,
            phone_access::phone_push_import_key,
            phone_access::phone_push_remove_key,
            phone_access::phone_push_test,
            phone_access::lid::phone_lid_status,
            phone_access::lid::phone_lid_set,
            commands::snooze_review_item,
            commands::wake_review_item,
            commands::activation_inventory,
            commands::run_activation_scan,
            commands::list_commitments,
            commands::create_commitment,
            commands::update_commitment,
            commands::set_commitment_status,
            commands::trash_commitment,
            commands::restore_commitment,
            commands::list_commitment_proposals,
            commands::submit_commitment_proposal,
            commands::resolve_commitment_proposal,
            commands::list_learning_proposals,
            commands::list_private_memories,
            commands::promote_private_memory,
            commands::list_skill_lessons,
            commands::resolve_skill_lesson,
            commands::undo_skill_lesson,
            commands::submit_learning_proposal,
            commands::get_learning_settings,
            commands::set_learning_suggestions,
            commands::resolve_learning_proposal,
            commands::resolve_learning_proposals_batch,
            commands::undo_learning_batch,
            commands::undo_review_reject,
            commands::list_learning_context,
            commands::memory_maintenance,
            commands::edit_memory_maintenance,
            commands::supersede_learning_context,
            commands::recall_learning_context,
            commands::tasks,
            commands::projects,
            commands::dashboard,
            commands::git_activity,
            git_card::git_card_status,
            git_card::git_card_branches,
            git_card::git_card_switch,
            git_card::git_card_new_worktree,
            git_card::git_card_suggest_message,
            git_card::git_card_commit_push,
            git_card::git_card_diff,
            git_card::git_card_range_files,
            pr_card::pr_card_view,
            github_pages::github_list,
            github_pages::github_detail,
            github_pages::github_triage,
            terminal::terminal_open,
            terminal::terminal_gate,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            terminal::terminal_adopt_chat,
            commands::set_task_status,
            commands::set_task_priority,
            commands::set_project_status,
            commands::create_task,
            commands::parse_quick_add,
            commands::update_task,
            commands::delete_task,
            commands::restore_task,
            commands::migrate_task_priorities,
            commands::create_project,
            commands::update_project,
            commands::task_assist,
            inline_ai::inline_ai,
            inline_ai::inline_ai_cancel,
            commands::project_assist,
            commands::list_task_filters,
            commands::save_task_filter,
            commands::delete_task_filter,
            commands::chat_preflight,
            commands::chat_send,
            commands::chat_efforts_list,
            commands::chat_effort_set,
            commands::chat_efforts_import,
            commands::chat_steer,
            commands::chat_steer_support,
            commands::chat_cancel_held,
            commands::chat_ack_held_failed,
            commands::agent_available,
            commands::agent_descriptors,
            commands::codex_helper_status,
            commands::agent_models,
            commands::agent_cli_status,
            commands::update_agent_cli,
            commands::restore_agent_cli,
            commands::cancel_agent_cli_update,
            commands::set_agent_cli_policy,
            commands::get_agent_settings,
            commands::get_extraction_settings,
            commands::set_extraction_settings,
            commands::set_chat_retitle,
            commands::sessions_api::regenerate_chat_title,
            commands::set_agent_settings,
            commands::chat_cancel,
            commands::chat_pause,
            commands::chat_resume_paused,
            commands::chat_pause_all,
            commands::chat_resume_all,
            commands::chat_paused_list,
            commands::chat_set_resume_at,
            commands::chat_save_attachment,
            commands::chat_locate_file,
            commands::chat_find_file,
            commands::chat_preview_file,
            commands::pdf_preview_page,
            commands::office_preview,
            commands::chat_preview_open,
            commands::media_grant,
            commands::media_release,
            commands::media_poster,
            commands::chat_discover,
            commands::claude_admin,
            commands::delete_session,
            commands::list_skills,
            commands::skill_portability_table,
            commands::read_skill,
            commands::write_user_skill,
            commands::delete_user_skill,
            commands::preview_skill_import,
            commands::commit_skill_import,
            commands::discard_skill_import,
            commands::skill_provenance,
            commands::stage_skill_update,
            commands::set_skill_note,
            commands::plaud_connected,
            commands::plaud_login,
            commands::plaud_logout,
            commands::plaud_ping,
            commands::clear_debug_logs,
            commands::system::diagnostics,
            commands::system::save_diagnostics,
            settings_transfer::settings_export,
            settings_transfer::settings_import_preview,
            settings_transfer::settings_import_apply,
            result_export::result_export_markdown,
            commands::system::disk_guard,
            commands::system::agent_tool_path,
            commands::system::set_disk_threshold,
            commands::system::disk_threshold,
            commands::system::overnight_summary,
            // Repo features R7b: Hollow's databases and the old-index cleanup.
            commands::system::hollow_databases,
            commands::system::stale_indexes,
            commands::system::remove_stale_indexes,
            // Move Hollow home: dry run, start (then quit), status, rollback.
            commands::move_home::move_home_plan,
            commands::move_home::move_home_start,
            commands::move_home::move_home_status,
            commands::move_home::move_home_rollback,
            commands::awareness::backup_health,
            commands::approvals_api::approval_respond,
            commands::ingest_run,
            commands::ingest_cancel,
            commands::list_custom_themes,
            commands::read_custom_theme,
            commands::write_custom_theme,
            commands::delete_custom_theme,
            commands::reveal_themes_folder,
            commands::themes_dir_path,
            commands::custom_icons_api::read_custom_icon,
            commands::custom_icons_api::prepare_custom_icon,
            commands::custom_icons_api::save_custom_icon,
            commands::custom_icons_api::reset_custom_icon,
            commands::custom_icons_api::select_app_icon_preset,
            logo_options::generate_logo_options,
            logo_options::cancel_logo_options,
            commands::list_workflows,
            commands::read_workflow,
            commands::list_workflow_outputs,
            commands::write_workflow,
            commands::delete_workflow,
            commands::run_workflow,
            commands::cancel_workflow,
            commands::list_notifications,
            commands::unread_notification_count,
            commands::mark_notification_read,
            commands::mark_all_notifications_read,
            commands::mark_notifications_read,
            commands::delete_notification,
            commands::delete_notifications,
            commands::clear_notifications,
            commands::create_notification,
            commands::set_notify_native_enabled,
            commands::announce_chat_native,
            commands::set_dock_badge,
            commands::system::set_foreground_notification_scope,
            commands::set_minimize_to_tray,
            commands::set_window_vibrancy,
            commands::set_run_on_startup,
            commands::run_on_startup_status,
            commands::set_nudges_enabled,
            commands::run_nudge_scan,
            commands::set_heartbeat_enabled,
            commands::set_heartbeat_drafts_enabled,
            commands::set_heartbeat_config,
            commands::run_heartbeat_now,
            commands::artifact_put,
            remote_image::fetch_remote_image,
            commands::set_teamwork_enabled,
            commands::set_agent_role,
            commands::set_computer_control_enabled,
            computer_commands::computer_status,
            computer_commands::computer_doctor,
            computer_commands::computer_stop_all,
            computer_commands::computer_chat_state,
            computer_commands::computer_chat_frame,
            computer_commands::computer_chat_control,
            computer_commands::computer_denied_apps,
            computer_commands::computer_set_app_allowed,
            computer_commands::host_scope_agent_denied,
            computer_commands::host_scope_remove_agent_deny,
            commands::set_vault_mcp_enabled,
            commands::set_code_context_mcp_enabled,
            commands::system::list_activity_ledger,
            commands::list_heartbeat_tasks,
            commands::resolve_heartbeat_task,
            commands::vault_save_note_cas,
            commands::setup_status,
            commands::create_profile_notes,
            commands::setup_restart,
            commands::directories_status,
            sync_engine::context_sync_status,
            sync_engine::context_sync_now,
            sync_engine::set_context_sync_enabled,
            sync_engine::configure_personal_sync,
            context_settings::context_layers_status,
            context_settings::context_org_access_check,
            context_settings::context_personal_local_only,
            context_settings::set_context_root,
            context_settings::set_context_machine_slug,
            mac_context::mac_context_offer,
            mac_context::mac_context_answer,
            mac_context::mac_context_connect,
            leak_checks::context_check_status,
            leak_checks::context_install_check,
            leak_checks::context_remove_check,
            sync_engine::configure_vault_sync,
            sync_files::file_copy_status,
            sync_files::configure_file_copy,
            turn_checkpoints::turn_checkpoints,
            turn_checkpoints::restore_turn_checkpoint,
            chat_branch::branch_chat,
            chat_branch::chat_branch_info,
            chat_branch::chat_branch_handoff,
            side_chat::side_chat_open,
            side_chat::side_chat_list,
            side_chat::side_chat_send,
            sync_files::peer_files,
            sync_files::fetch_peer_file,
            sync_engine::sync_clashes,
            sync_engine::resolve_sync_clash,
            commands::connect_hollow_home,
            commands::refresh_directories,
            commands::open_project_workspace,
            commands::set_directory_exclusion,
            commands::build_identity,
            commands::read_connected_file,
            commands::connected_files,
            commands::connected_index_status,
            commands::set_connected_index_enabled,
            commands::set_workspace_source,
            md_open::take_preview_file,
            md_open::save_preview_to_vault,
            md_open::open_markdown_preview,
            // Updates, People (row U9): the publisher's Mac only; its switch,
            // update_set_publisher, is registered with Publish to Main above.
            update_client::people_admin::update_people_list,
            update_client::people_admin::update_people_add,
            update_client::people_admin::update_people_revoke,
            update_client::people_admin::update_people_set_channels,
            // Updates row U6: Go back.
            update_client::goback_run::update_go_back_plan,
            update_client::goback_run::update_go_back,
            update_client::goback_run::update_go_back_when_idle,
            update_client::goback_run::update_unskip,
        ])
        .setup(move |app| {
            // When this process started: the run queue's startup recovery
            // interrupts queued Run now rows requested before it (Paperclip
            // PR 7), never this boot's own.
            let boot_at = chrono::Utc::now();
            // Runs a previous Hollow left running (R2): install the run
            // ledger (`{app data}/run-ledger.json`), then end every process
            // a DEAD Hollow's entries still name (exact run tag, or leader
            // pid + start time), SIGTERM → 3 s → SIGKILL. First, and
            // synchronous, so nothing below (history and usage
            // reconciliation, the run queue's recovery, the scheduler) runs
            // while an "ended" turn is still merging or building. Not
            // vault-gated: the ledger is app-wide. Costs one file read when
            // nothing is stale. The run ids go to the queue's recovery,
            // which notes their interrupted rows.
            let stopped_leftovers = process_group::orphans::start_at_boot(boot_at);
            // This process's own runs must not outlive it either (A·R8): a
            // termination signal settles them before exiting, and the reaper
            // child sweeps them the moment a crash ends this process.
            #[cfg(unix)]
            settle_on_quit_signals(app.handle().clone());
            // The core's services start through `core_boot`, the same calls
            // `hollow serve` makes (M1 PR 2); the window, tray, menu and
            // shortcut steps stay here, between its phases.
            let core = core_host::Core::from(app.handle());
            core_boot::process_services(&core, configured);
            // Evidence when the UI hangs (OC10).
            freeze_watchdog::start(app.handle().clone());
            // In setup mode the overlays have no vault to write into — leave
            // their global shortcuts unregistered until the wizard finishes
            // (the post-setup restart re-runs this with `configured: true`).
            // Registered below with the other shortcuts, once the menu and
            // tray exist (Settings, Keyboard shortcuts can change or turn
            // them off).
            if configured && !instance::is_review_launch() {
                shortcuts::allow_globals();
            }

            // If launch-at-login is already enabled, make sure its LaunchAgent
            // plist carries the bundle association so macOS Login Items shows
            // "Hollow" + the app icon (not the bare "hollow" binary). Idempotent
            // and self-gating — no-ops when the agent isn't installed — so it
            // also retro-fixes installs enabled before this key existed. It
            // also repoints a login item whose program was deleted (enabled
            // from a since-removed dev build) at this installed bundle.
            if !instance::is_review_launch() {
                commands::brand_login_item(app.handle());
            }

            core_boot::account_services(boot_at);
            // Updates (row U5): check about 45 s after launch, then every
            // 30 minutes; a review copy gets an inert client.
            update_client::start(app.handle());

            // Native menu bar (audit H14): standard Edit items plus Hollow's
            // own actions, each forwarded to the webview as `app-menu`.
            #[cfg(target_os = "macos")]
            {
                let menu = app_menu::build(app.handle())?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| app_menu::handle(app, event.id.as_ref()));
            }

            // Embedded provenance names the binary in Mission Control and
            // the Window menu, even though main's custom title bar hides it.
            let identity = app_menu::build_identity();
            let identity_title = identity.short_text();
            if let Some(window) = app.get_window("main") {
                let _ = window.set_title(&identity_title);
            }

            // Tray ids differ from app-menu ids: both callbacks see every event.
            let identity_item = MenuItem::with_id(
                app,
                "tray-build-identity",
                &identity_title,
                false,
                None::<&str>,
            )?;
            let copy_build_item = MenuItem::with_id(
                app,
                "tray-copy-build",
                "Copy Build Identity",
                true,
                None::<&str>,
            )?;
            let open_item = MenuItem::with_id(app, "open", "Open Hollow", true, None::<&str>)?;
            // Brings the window forward and starts a quick chat (the same
            // `app-menu` action as File → New Chat).
            let quick_chat_item =
                MenuItem::with_id(app, "new-quick-chat", "New Chat", true, None::<&str>)?;
            let quick_item = MenuItem::with_id(
                app,
                "quick",
                "Quick Capture Task…",
                true,
                Some("CmdOrCtrl+Alt+Shift+T"),
            )?;
            let ingest_item = MenuItem::with_id(
                app,
                "ingest",
                "Import a source…",
                true,
                Some("CmdOrCtrl+Alt+Shift+I"),
            )?;
            // Stop all computer control (CU8): ends every desktop session
            // and forgets what chats were allowed, from the menu bar.
            let computer_stop_item = MenuItem::with_id(
                app,
                "computer-stop",
                "Stop all computer control",
                true,
                None::<&str>,
            )?;
            let main_agent_item = MenuItem::with_id(
                app,
                "main-agent",
                "COO",
                true,
                Some("CmdOrCtrl+Alt+Shift+H"),
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Hollow", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &identity_item,
                    &copy_build_item,
                    &open_item,
                    &main_agent_item,
                    &quick_chat_item,
                    &quick_item,
                    &ingest_item,
                    &computer_stop_item,
                    &separator,
                    &quit_item,
                ],
            )?;

            // On macOS the menu-bar status item should be a *template* image: a
            // monochrome black-on-transparent glyph that the OS auto-tints to the
            // bar theme (dark glyph on light bars, light glyph on dark bars), like
            // the WiFi/battery/Spotlight icons. We ship the Hollow arch mark as
            // that silhouette (`icons/tray-template.png`). Elsewhere there's no
            // template concept and a pure-black glyph could vanish on a dark panel,
            // so fall back to the full-color app icon.
            #[cfg(target_os = "macos")]
            let (tray_icon, tray_is_template) = (
                tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?,
                true,
            );
            #[cfg(not(target_os = "macos"))]
            let (tray_icon, tray_is_template) = (app.default_window_icon().unwrap().clone(), false);

            let _tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(tray_is_template)
                // A review copy names itself, so its Quit is never mistaken
                // for the everyday app's (plan completion L5, native plan G5).
                .tooltip(instance::tray_tooltip(instance::current(), &identity))
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray-copy-build" => app_menu::handle(app, "copy-build"),
                    "open" => toggle_main_window(app),
                    "new-quick-chat" => app_menu::handle(app, "new-quick-chat"),
                    "main-agent" => show_main_with(app, "main-agent"),
                    "quick" => show_main_with(app, "quick-capture"),
                    "ingest" => show_main_with(app, "quick-ingest"),
                    // Stop all computer control (CU8): works with no window.
                    "computer-stop" => {
                        computer_broker::end_all_sessions();
                        computer_broker::audit_control("", "stop-all-tray");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Restore only after the menu-bar identity exists.
            custom_icons::restore(app.handle());

            // The saved shortcut settings: menu accelerators, the tray's
            // hints and the system-wide quick capture / ingest keys.
            app.manage(shortcuts::TrayItems(vec![
                ("quick", quick_item.clone()),
                ("ingest", ingest_item.clone()),
                ("main-agent", main_agent_item.clone()),
            ]));
            let boot_warnings = shortcuts::apply(app.handle(), &shortcuts::load());
            for warning in &boot_warnings {
                eprintln!("[shortcuts] {warning}");
            }
            // One bell item for every key that didn't start (P1-49), so a
            // taken system-wide key isn't found out only by pressing it.
            if let (Some(notice), Some(state)) = (
                shortcuts::boot_notice(&boot_warnings),
                app.try_state::<AppState>(),
            ) {
                if state.config.vault_path.exists() {
                    let _ = notifications::create(app.handle(), &state.config.vault_path, notice);
                }
            }

            // Show the main window shortly after launch — UNLESS this launch was
            // triggered by opening a markdown file. On macOS the OS hands us the
            // file via `RunEvent::Opened`, which fires just after `setup` and flips
            // `MdOpenState::opened_a_file`; in that case we leave the dashboard
            // hidden and show only the document preview window. The short delay lets
            // that event land. Fallback: if no file arrived, main always shows, so
            // the window is never lost. (main is `visible: false` in tauri.conf.json,
            // so nothing flashes during the wait.)
            // `--hidden` is appended to argv when the OS auto-launches Hollow
            // at login (see the autostart plugin above). In that case keep the
            // dashboard hidden and come up in the tray only — the background
            // runner experience.
            let launched_hidden = std::env::args().any(|a| a == "--hidden");
            // main starts hidden, so it is in the tray until first focused
            // (`--hidden`, or a launch that only opened a document) (S15).
            lifecycle::set_in_tray(true);
            let show_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let opened = show_handle
                    .try_state::<md_open::MdOpenState>()
                    .map(|s| s.opened_a_file.load(std::sync::atomic::Ordering::SeqCst))
                    .unwrap_or(false);
                if !opened && !launched_hidden {
                    lifecycle::bring_main_forward(&show_handle);
                }
            });

            core_boot::vault_services(&core, configured, boot_at, stopped_leftovers);

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                // Any other window is hidden, not closed (the quick overlays
                // that relied on this went in S5; the branch stays for any
                // window not named below). The runtime `md-preview-*` document
                // windows are throwaway, so let them close for real and forget
                // their file mapping. `main` honors the user's minimize-to-tray
                // toggle: when on (default) it hides + keeps the app resident
                // under the tray; when off, closing the window quits the whole
                // app (on macOS a closed window alone leaves the process
                // running, so we explicitly exit — matching the tray Quit item).
                let label = window.label();
                if label.starts_with("md-preview") {
                    if let Some(state) = window.try_state::<md_open::MdOpenState>() {
                        state.forget(label);
                    }
                } else if label == "main" {
                    if lifecycle::minimize_to_tray() {
                        if window.hide().is_ok() {
                            lifecycle::set_in_tray(true);
                        }
                        api.prevent_close();
                    } else {
                        api.prevent_close();
                        window.app_handle().exit(0);
                    }
                } else {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
            WindowEvent::Focused(true) if window.label() == "main" => {
                // Every way back from the tray (tray item, shortcut, menu,
                // dock click) shows and focuses the window (S15).
                lifecycle::set_in_tray(false);
                // A Linear link still waiting for the page (linear_link.rs).
                linear_link::nudge_if_pending(window.app_handle());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            // A preview window can be visible while main is in the tray.
            // Preserve the selected window if main is already visible.
            tauri::RunEvent::Reopen { .. } => {
                if app_handle.get_window("main").is_some_and(|window| {
                    !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false)
                }) {
                    lifecycle::bring_main_forward(app_handle);
                }
            }
            // macOS hands opened documents to the running app here (we're
            // registered for *.md via bundle.fileAssociations) — one event,
            // possibly several urls. Open a preview window per markdown file.
            // Quitting ends every service (Build PR B6), including an
            // install-and-restart's quit, and settles every live agent run
            // (R2), once (`stop_children_once`). A hide to the tray never
            // gets here. Whatever the budget cuts short stays in the run
            // ledger for the reaper or the next launch's sweep.
            tauri::RunEvent::ExitRequested { api, .. } => lifecycle::guard_exit(app_handle, &api),
            tauri::RunEvent::Exit => {
                freeze_watchdog::note_exit();
                stop_children_once()
            }
            tauri::RunEvent::Opened { urls } => {
                for url in urls {
                    // `hollow://updates/connect?token=…` carries a secret:
                    // stored in the Keychain, never logged (update_client).
                    if update_client::link::is_updates_link(url.as_str()) {
                        update_client::link::handle(app_handle, url.as_str());
                        continue;
                    }
                    // A `hollow://` link (Linear's Custom link): a draft in
                    // a chat, never a run (linear_link.rs).
                    if url.scheme() == linear_link::SCHEME {
                        linear_link::handle(app_handle, url.as_str());
                        continue;
                    }
                    if let Ok(path) = url.to_file_path() {
                        if md_open::is_markdown(&path) {
                            md_open::open_preview_window(app_handle, &path);
                        }
                    }
                }
            }
            _ => {}
        });
}
