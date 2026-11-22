//! `core_boot`: the phases both hosts call, their order, and that nothing in
//! them needs the GUI's handle except the one service serve skips.

use super::*;
use crate::core_host::RecordingHost;

/// The part of a source file before its `#[cfg(test)]` module.
fn body(source: &str) -> &str {
    let end = source
        .find(concat!(
            "#[cfg(test)]\n#[path = ",
            "\"core_boot_tests.rs\"]"
        ))
        .or_else(|| source.find(concat!("#[cfg(test)]\n#[path = ", "\"serve_tests.rs\"]")))
        .unwrap_or(source.len());
    &source[..end]
}

/// [`body`] without its comment lines.
fn code(source: &str) -> String {
    body(source)
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Each needle's offset in `source`, which must rise in the order given.
fn assert_in_order(file: &str, source: &str, needles: &[&str]) {
    let mut last = 0usize;
    let mut last_needle = "(start)";
    for needle in needles {
        let at = crate::source_pins::find(&source[last..], needle)
            .map(|i| i + last)
            .unwrap_or_else(|| panic!("{file}: `{needle}` missing, or not after `{last_needle}`"));
        last = at + 1;
        last_needle = needle;
    }
}

#[test]
fn the_vault_phase_keeps_the_apps_startup_order() {
    let src = body(include_str!("core_boot.rs"));
    assert_in_order(
        "core_boot.rs",
        src,
        &[
            concat!("notifications::", "register(core.clone())"),
            concat!("usage_ledger::", "set_root("),
            concat!("task_broker::", "start(core.clone()"),
            concat!("app_broker::", "start(core.clone()"),
            concat!("services::", "start(core.clone()"),
            concat!("workflow_permissions::", "initialize("),
            concat!("directory_connections::", "start(core.clone()"),
            concat!("run_pump::", "claim_ownership("),
            concat!("automated_history::", "reconcile_interrupted_sparing("),
            concat!("usage_ledger::", "reconcile_interrupted("),
            concat!("cfo::settings::", "publish_at_startup("),
            concat!("run_pump::", "start(core.clone()"),
            concat!("workflows::", "start_scheduler(core.clone()"),
            concat!("tracker_sync::", "start(core.clone()"),
            concat!("vault_watch::", "start(core.clone()"),
            concat!("nudges::", "start_timer(core.clone()"),
            concat!("phone_access::", "start(core.clone()"),
            concat!("heartbeat::", "start_timer(core.clone()"),
            concat!("self_hosted_jobs::", "start(core.clone())"),
            concat!("main_agent::", "start_timer(core.clone())"),
            concat!("schedule_windows::", "start_timer(app.clone())"),
        ],
    );
    assert_in_order(
        "core_boot.rs",
        src,
        &[
            concat!("process_group::reaper::", "spawn()"),
            concat!("approvals::", "start(core.clone())"),
            concat!("claude_quota::", "register_gui()"),
            concat!("claude_accounts::", "register_gui()"),
            concat!("scratch::", "sweep_orphans(boot_at)"),
            concat!("if vault_path", ".exists() {"),
        ],
    );
}

/// The GUI's setup and serve call the same phases in the same order, after
/// the boot sweep and the quit watcher.
#[test]
fn both_hosts_boot_through_the_same_phases() {
    let lib = include_str!("lib.rs");
    let setup = &lib[lib.find(concat!(".setup(", "move |app|")).expect("setup")..];
    assert_in_order(
        "lib.rs setup",
        setup,
        &[
            concat!("orphans::", "start_at_boot(boot_at)"),
            concat!("settle_on_quit", "_signals(app.handle().clone())"),
            concat!("core_boot::", "process_services(&core, configured)"),
            concat!("shortcuts::", "allow_globals()"),
            concat!("commands::", "brand_login_item("),
            concat!("core_boot::", "account_services(boot_at)"),
            concat!("TrayIcon", "Builder::with_id("),
            concat!(
                "core_boot::",
                "vault_services(&core, configured, boot_at, stopped_leftovers)"
            ),
        ],
    );
    let serve = body(include_str!("serve.rs"));
    assert_in_order(
        "serve.rs",
        serve,
        &[
            concat!("orphans::", "start_at_boot(boot_at)"),
            concat!("QuitSignals::", "install()"),
            concat!("core_boot::", "process_services(&core, true)"),
            concat!("core_boot::", "account_services(boot_at)"),
            concat!(
                "core_boot::",
                "vault_services(&core, true, boot_at, stopped_leftovers)"
            ),
            concat!("orphans::", "settle_on_signal("),
        ],
    );
    // The queue lock goes only once the runtime is down (PR 2 review).
    assert_in_order(
        "serve.rs main",
        serve,
        &[
            concat!("runtime.", "shutdown_timeout("),
            concat!("run_pump::", "release_", "ownership()"),
            "drop(owner)",
        ],
    );
}

/// Nothing in the phases takes the GUI's handle, except through
/// `core.gui()` for the two services serve skips; serve never builds Tauri's
/// app. (Since M1 PR 5 serve's state sits in a mock-runtime shell, built
/// only in `serve_actions.rs` and pinned there.)
#[test]
fn the_phases_need_no_app_handle_and_serve_no_tauri_app() {
    let src = code(include_str!("core_boot.rs"));
    assert!(!src.contains(concat!("App", "Handle")));
    assert!(!src.contains(concat!("app.", "handle()")));
    assert_eq!(src.matches(concat!("core.", "gui()")).count(), 2);
    assert_eq!(SKIPPED_IN_SERVE.len(), 2);
    let serve = code(include_str!("serve.rs"));
    for forbidden in [
        concat!("tauri::", "Builder"),
        concat!("generate_", "context!"),
        concat!("tauri_plugin", "_"),
        concat!(".run(", "|"),
    ] {
        assert!(!serve.contains(forbidden), "serve.rs has {forbidden}");
    }
}

/// Without managed state, or with a vault that is not on disk (setup mode),
/// the vault phase starts nothing and emits nothing.
#[test]
fn the_vault_phase_without_a_vault_starts_nothing() {
    let bare = RecordingHost::default();
    vault_services(
        &Core::new(bare.clone()),
        true,
        chrono::Utc::now(),
        HashSet::new(),
    );
    assert!(bare.events().is_empty());

    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.path().join("no-such-vault");
    let rec = RecordingHost::with(
        Some(AppState::new(config, dir.path().join("config.json"))),
        None,
    );
    vault_services(
        &Core::new(rec.clone()),
        true,
        chrono::Utc::now(),
        HashSet::new(),
    );
    assert!(rec.events().is_empty());
    assert!(rec.banners().is_empty());
}

/// Every service serve skips names why; the list is what it logs at boot.
#[test]
fn skipped_services_say_why() {
    for (service, why) in SKIPPED_IN_SERVE {
        assert!(
            !service.is_empty() && why.contains("only in the app") && !why.contains("PR "),
            "{service}: {why}"
        );
    }
}

/// M5 PR2 review: `hollow serve` is the runner, so it never starts the chat
/// move trigger (a sleeping runner would move its chats to the MacBook), and
/// neither does a candidate build, even one allowed background work.
#[test]
fn serve_and_candidates_never_start_the_chat_move_trigger() {
    use crate::chat_move_trigger::starts_here;
    assert!(starts_here(true, true, false, true), "the app starts it");
    assert!(
        !starts_here(false, true, false, true),
        "serve has no window"
    );
    assert!(!starts_here(true, true, true, true), "a candidate build");
    assert!(
        !starts_here(true, false, false, true),
        "not the queue owner"
    );
    assert!(!starts_here(true, true, false, false), "setup mode");
    let src = code(include_str!("core_boot.rs"));
    let at = src.find(concat!("chat_move_trigger::", "start()")).unwrap();
    let gate = &src[src[..at]
        .rfind(concat!("chat_move_trigger::", "starts_here("))
        .unwrap()..at];
    assert!(gate.contains(concat!("core.", "gui().is_some()")), "{gate}");
    assert!(gate.contains("is_candidate()"), "{gate}");
    assert!(SKIPPED_IN_SERVE
        .iter()
        .any(|(service, _)| *service == "chat move trigger"));
}

/// M3a round 3: the app's hourly runner COO check opens tunnels and reads
/// the Keychain, so it starts behind the same gate as the check-ins (queue
/// owner, background agents allowed), never in a candidate build or a
/// second process.
#[test]
fn the_runner_coo_check_starts_only_behind_the_background_gate() {
    let src = code(include_str!("core_boot.rs"));
    let call = src
        .find(concat!("dispatcher_coo::", "start_refresh("))
        .expect("started in the vault phase");
    let gate = "if queue.owned() && instance::background_agents_allowed() {";
    let opened = src[..call].rfind(gate).expect("behind the gate");
    assert!(
        !src[opened + gate.len()..call].contains(['{', '}']),
        "the call sits directly inside that gate"
    );
}
