// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Hidden subcommands: run as an MCP stdio server (spawned by chat/
    // workflow `claude` runs via --mcp-config). Must not boot the GUI.
    let command = std::env::args().nth(1);
    // Each runs background-only, out of the Dock, before it touches anything
    // (PER-34: a vault-mcp reading a PDF through PDFKit showed a second Hollow).
    hollow_lib::enter_background_only(command.as_deref());
    match command.as_deref() {
        Some("codex-relay") => return hollow_lib::run_codex_relay(),
        Some("team-mcp") => return hollow_lib::run_team_mcp(),
        Some("claude-defer-hook") => return hollow_lib::run_claude_defer_hook(),
        Some("computer-mcp") => return hollow_lib::run_computer_mcp(),
        Some("computer") => return hollow_lib::run_computer_cli(),
        Some("vault-mcp") => return hollow_lib::run_vault_mcp(),
        // Code context (CC2): the owned MCP server and its CLI twin.
        Some("code-context-mcp") => return hollow_lib::run_code_context_mcp(),
        Some("code-context") => return hollow_lib::run_code_context_cli(),
        Some("questions-mcp") => return hollow_lib::run_questions_mcp(),
        Some("tasks-mcp") => return hollow_lib::run_tasks_mcp(),
        Some("services-mcp") => return hollow_lib::run_services_mcp(),
        Some("app-mcp") => return hollow_lib::run_app_mcp(),
        // Built-in connectors (connectors library, Transport::Builtin).
        Some("m365-mcp") => return hollow_lib::run_m365_mcp(),
        // A remote connector on Hollow's own sign-in (Transport::Http, oauth).
        Some("remote-mcp") => return hollow_lib::run_remote_mcp(),
        // The one-time Notion-to-vault importer and tracker rekey; reads
        // Notion and the tracker only (tracker_import).
        Some("tracker-import") => return hollow_lib::run_tracker_import(),
        Some("companion-fixtures") => return hollow_lib::run_companion_fixtures(),
        Some("companion-apply") => return hollow_lib::run_companion_apply(),
        Some("companion-bridge-once") => return hollow_lib::run_companion_bridge_once(),
        // Hidden proof harness: embed a probe with the bundled model and exit.
        // Run against the built .app binary to verify the signed-bundle path.
        Some("embed-selftest") => return hollow_lib::run_embed_selftest(),
        // A Codex run's stdio MCP server, read from the run's private
        // config file (merged plan slice 6).
        Some("mcp-launch") => return hollow_lib::run_mcp_launch(),
        // The Jev study runner (agent effectiveness E3): refuses to run
        // unless --endpoint is a loopback fake or --typesafe is explicit.
        Some("jev-study") => return hollow_lib::run_jev_study(),
        // Crash reaper (A·R8): started by the GUI, ends its runs when it exits.
        Some("reaper") => return hollow_lib::run_reaper(),
        // Hollow's core with no window server (M1 PR 2, serve.rs): what a
        // LaunchDaemon runs on a Mac with nobody logged in.
        Some("serve") => return hollow_lib::run_serve(),
        // Move Hollow home: the dry run, and the helper that runs or rolls
        // back a staged move after the app quits (move_home.rs).
        Some("move-home") => return hollow_lib::run_move_home(),
        // The installer inside the app (update_install): run detached from
        // a staged update's own binary; never boots the GUI.
        Some("install-update") => return hollow_lib::run_install_update(),
        _ => {}
    }
    // Not a command: only a plain launch may start the app. `--version` and
    // `--help` answer, any other command or option is refused, and a launch
    // while the app is already running hands off to it (launch.rs).
    if let Some(code) = hollow_lib::gate_gui_launch() {
        std::process::exit(code);
    }
    // An interrupted Move Hollow home is undone before any store opens.
    if let Some(code) = hollow_lib::move_home_launch_guard() {
        std::process::exit(code);
    }
    hollow_lib::run()
}
