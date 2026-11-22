//! Core startup (M1 PR 2): the services Hollow's core runs, started the same
//! way and in the same order by both hosts, the GUI's Tauri `.setup` and
//! `hollow serve` (`serve.rs`), which runs the core with no window server.
//!
//! The GUI's setup calls the three phases with its window, tray, menu and
//! shortcut steps in between ([`process_services`], then
//! [`account_services`], then [`vault_services`]); serve calls them back to
//! back. Each phase takes a [`Core`], so a service started here never sees an
//! `AppHandle` it did not ask for: a service not moved onto the core yet
//! starts only where the core's `gui()` is the GUI's handle, and serve logs that it
//! skipped it ([`SKIPPED_IN_SERVE`]).
//!
//! The boot sweep (`process_group::orphans::start_at_boot`) and the quit
//! signal watcher stay with each host: the sweep must come first, and each
//! host exits its own way.

use crate::commands::AppState;
use crate::core_host::Core;
use crate::{
    agent_cli, agent_models, app_broker, approvals, automated_history, automation_in_chat, cfo,
    claude_accounts, claude_quota, commands, companion_bridge, companion_inbox, context_layers,
    directory_connections, event_triggers, heartbeat, instance, main_agent, notifications, nudges,
    phone_access, process_group, run_pump, schedule_windows, scratch, self_hosted_jobs,
    self_install, services, session_store, shell_path, sync_engine, task_broker, tracker,
    tracker_sync, usage_ledger, vault_embed_index, vault_hygiene, vault_search_index, vault_watch,
    vault_workflows, workflow_permissions, workflows,
};
use std::collections::HashSet;

/// The services serve does not start yet, and why (logged once at boot).
pub(crate) const SKIPPED_IN_SERVE: &[(&str, &str)] = &[
    (
        "schedule windows",
        "its jobs (weekly update, Friday status, gardener, as-of snapshot, friction) take the GUI's handle, so they run only in the app for now",
    ),
    (
        "chat move trigger",
        "serve is the runner chats move to; moving its own chats away when it sleeps runs only in the app",
    ),
];

/// The app state from config.json (`config::locate_and_load`). The GUI
/// boots into setup mode when this fails; serve refuses to start.
pub(crate) fn load_state() -> Result<AppState, String> {
    crate::config::locate_and_load().map(|(config, path)| AppState::new(config, path))
}

/// Phase 1, right after the boot sweep and the quit watcher: process-wide
/// services that need no vault (the crash reaper, the login shell's PATH,
/// the org context pull, the effort catalogs) and, when configured, the
/// approvals socket.
pub(crate) fn process_services(core: &Core, configured: bool) {
    process_group::reaper::spawn();
    // M7 (PER-21): the one-time freeze of the runner defaults, before the
    // run queue or a turn reads them: a setup from before M7 keeps the runner
    // for unattended work and Auto routing. One small file read (and one
    // write, once); errors are logged and retried next launch, never fatal.
    crate::agent_settings::freeze_runner_defaults_at_launch();
    // The login shell's PATH for agent CLIs (B03), on its own thread:
    // nothing waits for it, and agents use Hollow's own PATH until then.
    shell_path::start();
    // The org context clone stays current on its own thread (CX5).
    context_layers::spawn_pull_timer();
    // Effort maps per model (`effort::resolve`); learn the catalogs it
    // needs in the background.
    tauri::async_runtime::spawn(agent_models::warm_effort_catalogs());
    if configured {
        // Inline chat approvals. Binds the per-process Unix socket the
        // confirm-gated MCP subprocesses (m365-mcp / computer-mcp)
        // route their Allow/Deny requests through, so approval renders
        // as a card in the chat instead of a native dialog. Non-fatal
        // if it can't bind — clients fall back to the native dialog.
        // Serve answers every request at once with a refusal (no window
        // shows the card), so a confirm-gated tool fails closed.
        approvals::start(core.clone());
    }
}

/// Phase 2: the agent accounts and this process's own housekeeping.
pub(crate) fn account_services(boot_at: chrono::DateTime<chrono::Utc>) {
    // Mark this process as the core for the Settings, Usage plan card
    // (Paperclip PR 8c). Not vault-gated: the card does not use the
    // vault, and the MCP subcommands dispatch in main.rs before run(),
    // so they never get here and can never read the Keychain.
    claude_quota::register_gui();
    // Several Claude accounts: the core chooses one per Claude run and
    // probes token accounts in the background while spreading usage.
    claude_accounts::register_gui();
    claude_accounts::start_refresher();
    // Agent CLI versions: check about daily, update per Settings.
    // Not in a review copy: the CLIs are shared with the everyday app.
    if instance::may_change_shared_setup() {
        agent_cli::start_worker();
    }
    // Install from Hollow (Build PR B5): the result the detached
    // installer helper left, until the user acknowledges it.
    self_install::load_result_at_startup();
    // A finished app build survives a restart (an install approved
    // after it still finds it; nothing installs by itself).
    self_install::restore_finished_build();

    // Per-run scratch space (Paperclip PR 15a): remove folders under
    // `scratch::base()` left by Hollow processes that are gone, off
    // the main thread. Not vault-gated (scratch lives outside the
    // vault); folders of a live process, this one's included, stay.
    std::thread::spawn(move || {
        scratch::sweep_orphans(boot_at);
    });
}

/// Phase 3: everything that runs on the vault, the run queue first among
/// them. A no-op without a vault on disk (setup mode).
pub(crate) fn vault_services(
    core: &Core,
    configured: bool,
    boot_at: chrono::DateTime<chrono::Utc>,
    stopped_leftovers: HashSet<String>,
) {
    // Workflows scheduler. Polls vault/workflows/*.md every 60s and
    // fires any cron-scheduled workflow whose nextRunAt is in the
    // past. Spawns a background tokio task — no-op if the vault
    // path can't be resolved (config validation should have caught
    // that earlier, but be defensive).
    if let Some(state) = core.try_state::<AppState>() {
        let vault_path = state.config.vault_path.clone();
        if vault_path.exists() {
            // Let AppHandle-free code (permission refusals, post-run
            // flags) notify.
            notifications::register(core.clone());
            // The personal context root's quiet-timer sync (CX6a),
            // on its own thread; idle until a remote is set.
            if configured {
                sync_engine::spawn_timer(vault_path.clone());
                // V4 starts no timer (N15): binaries are fetched on demand.
                // F3: "Connect this Mac's context", checked a minute in.
                crate::mac_context::start(vault_path.clone());
                // PX3: offers to promote the three safety memories to
                // Guardrails (a card each, once), a couple of minutes in.
                crate::context_pins::start(vault_path.clone());
            }
            // Detached one-shots (assists, extractors, titles) record
            // their usage rows in this vault's store whatever their
            // cwd (Paperclip PR 6). Before anything can run one.
            usage_ledger::set_root(&vault_path);
            // Task tools (Paperclip PR 12): the persisted switch,
            // then the broker socket `hollow tasks-mcp` forwards
            // writes to. Before the run-queue pump starts below, so
            // an automation admitted before the webview boots
            // already sees the user's choice.
            if configured {
                task_broker::load_settings();
                tracker_sync::seams::register_running(&state.config);
                tracker::work::register_running(&state.config);
                task_broker::start(core.clone(), vault_path.clone());
                // Companion apps (T0): signed device ops from
                // `.hollow-sync/inbox/`, applied as the person's edits.
                companion_inbox::start(core.clone(), vault_path.clone());
                // Companion apps (W1): the same ops from the backend
                // (`hollow-companiond`), when one is set in Settings.
                companion_bridge::start(core.clone(), vault_path.clone());
                // Hollow app tools (PR A1): the same, for `hollow app-mcp`.
                // Serve answers each call with a refusal for now (the
                // actions read the GUI's managed state).
                app_broker::load_settings();
                app_broker::start(core.clone(), vault_path.clone());
                // Computer control's see-and-act tools (Cua CU1): serves
                // nothing until an engine is pinned.
                crate::computer_broker::start(vault_path.clone());
                // Services that outlive a turn (Build PR B6): stop what a
                // previous Hollow left running, then serve the broker.
                services::start(core.clone(), vault_path.clone());
                crate::shared_mcp_host::start(core.clone(), vault_path.clone());
            }
            // Workflow permission stamps: trust on first use, BEFORE
            // reconciliation, the scheduler, the watcher and any IPC,
            // so interrupted pre-upgrade runs aren't flagged as
            // awaiting approval and then silently trusted.
            match workflow_permissions::initialize(&vault_path) {
                Ok(0) => {}
                Ok(count) => {
                    eprintln!("[workflows] trusted {count} existing workflow(s) on first use")
                }
                Err(error) => {
                    // Every automation is refused until this is fixed;
                    // say so (notification + health), not just stderr.
                    eprintln!("[workflows] permission stamps unavailable: {error}");
                    workflow_permissions::report_unavailable(&vault_path, &error);
                }
            }
            // Remove Hollow's own retired state files (old migration
            // markers, dead caches) before anything reads the vault.
            vault_hygiene::retire_legacy_state(&vault_path);
            directory_connections::start(core.clone(), vault_path.clone());
            // Exactly one Hollow process owns a vault's run queue
            // (Paperclip PR 7): the one holding the flock beside its
            // session database. Only the owner marks another
            // process's in-flight work interrupted (the usage ledger
            // below, stuck workflow statuses, the queue itself) and
            // runs the enqueuers; a second process (a dev build
            // beside the installed app) leaves all of it alone.
            let queue = run_pump::claim_ownership(&vault_path);
            // Paperclip PR 18: a run another live Hollow process
            // holds the lease for (its lease lock is held) is spared
            // by both reconciles below: its manifest stays running,
            // its lease stays, and its usage rows stay `running`.
            // Computed once so the two agree. A legacy lease, or a
            // new one whose lock is free, is reconciled as before.
            let spare = automated_history::foreign_live_owner(&vault_path);
            // A crash can leave an automated-history baseline marked
            // `running`. Finalize it against the startup vault before
            // any scheduler starts a new automated run, preserving the
            // interrupted change set and keeping restore semantics sound.
            // Gated on the queue owner lock, taken just above: a
            // second Hollow process must never finalize the owner's
            // live run as interrupted and delete its lease file (that
            // let a second run take the vault mid-run). Skipped only
            // when another process provably owns the vault; a vault
            // whose lock path cannot be resolved still reconciles.
            if queue != run_pump::QueueState::OwnedElsewhere {
                match automated_history::reconcile_interrupted_sparing(
                    &vault_path,
                    spare.as_deref(),
                ) {
                    Ok(0) => {}
                    Ok(count) => {
                        eprintln!("[history] reconciled {count} interrupted run(s)")
                    }
                    Err(error) => {
                        eprintln!("[history] interrupted-run reconciliation failed: {error}")
                    }
                }
            }
            // The usage ledger's counterpart: rows a crash left
            // `running` become `interrupted` (partial when a
            // checkpoint recorded tokens), and rows past retention
            // are pruned, before any scheduler starts a new run.
            if queue.owned() {
                match usage_ledger::reconcile_interrupted(&vault_path, spare.as_deref()) {
                    Ok(0) => {}
                    Ok(count) => eprintln!("[usage] marked {count} interrupted run(s)"),
                    Err(error) => crate::health::record(
                        &vault_path,
                        "usage",
                        crate::health::Outcome::Failed,
                        format!(
                            "Usage ledger startup check failed: {}",
                            crate::claude_runner::truncate(
                                &crate::claude_runner::redact(&error),
                                160
                            )
                        ),
                    ),
                }
            }
            // Make sure every automation changed outside the editor
            // has its single live review (and notify once) before the
            // queue's claim can refuse it.
            workflow_permissions::sweep(&vault_path, true);
            // Same class of damage one layer up: `last_status` is only
            // cleared by `finalize`, so a workflow whose process died
            // claims to be running forever and shows a spinner that
            // never resolves. Nothing is live this early in setup, so
            // the live-slug list is empty by construction. The heal
            // leaves `next_run_at` due, so an interrupted occurrence
            // re-runs once within its grace (and is skipped past it).
            if queue.owned() {
                let healed = vault_workflows::heal_orphan_running(&vault_path, &[]);
                if !healed.is_empty() {
                    crate::health::record(
                        &vault_path,
                        "workflows",
                        crate::health::Outcome::Partial,
                        format!(
                            "{} routine{} stopped before finishing",
                            healed.len(),
                            if healed.len() == 1 { "" } else { "s" }
                        ),
                    );
                }
            }
            // The run-queue pump: recovers the queue (interrupting
            // this process's predecessor's claimed/running rows and
            // its queued Run now rows, deleting PR 2's
            // `.workflow-deferrals.json`) before its first claim,
            // then drains it one run at a time. After PR 5's
            // initialize and sweep, so no claim verifies before trust
            // on first use.
            // COO rebuild L7: the long-work line follows the saved
            // reserve before the pump's first claim (review L5).
            cfo::settings::publish_at_startup(&vault_path);
            run_pump::start(core.clone(), vault_path.clone(), boot_at, stopped_leftovers);
            // Existing explicit schedules remain; a new workspace never auto-enables routines.
            // The scheduler and the heartbeat timer only enqueue, and
            // only the queue's owner runs them.
            if queue.owned() {
                workflows::start_scheduler(core.clone(), vault_path.clone());
                event_triggers::start(state.config.clone());
                automation_in_chat::start(state.config.clone());
            } else {
                eprintln!(
                    "[run queue] {queue:?}: the scheduler and heartbeat run in the owning process"
                );
            }
            tracker_sync::start(core.clone(), state.config.clone());
            // FTS5 search-index acceleration (a derived prefilter cache,
            // DB kept OUTSIDE the vault under the app data dir). Engaged
            // core-side only; best-effort — search falls back to the linear
            // scan if the index can't open. Runs a full sweep on a copy of
            // the path so a fresh/stale DB is brought current at launch.
            {
                let vp = vault_path.clone();
                std::thread::spawn(move || vault_search_index::engage(&vp));
            }
            // Chat search index integrity check (PER-23): a damaged
            // full-text index is rebuilt from the stored chats.
            session_store::check_search_index_soon(&vault_path);
            // Local semantic-search backfill worker (candle embeddings,
            // on-device). Its own thread because it loads the model +
            // embeds the corpus; dormant until the user enables the
            // feature in Settings (default OFF), so this costs nothing
            // unless opted in. Embeds OFF the vault write path.
            {
                let vp = vault_path.clone();
                std::thread::spawn(move || vault_embed_index::run_semantic_worker(&vp));
            }
            // Live vault updates. Watches the vault root recursively and
            // emits a debounced `vault-changed` event so the UI reflects
            // edits made outside the app (Obsidian, the embedded Claude, a
            // text editor) without a manual reload. Non-fatal if it can't
            // arm — the app still works, just without live refresh.
            vault_watch::start(core.clone(), state.config.clone());
            // Proactive nudges. Deterministic vault scans every few
            // minutes (meeting soon, email aging, frogs, cold
            // projects) feeding the notification center. Settings,
            // Notifications toggles it via set_nudges_enabled.
            nudges::start_timer(core.clone(), vault_path.clone());
            // The phone chat API (2026-11-04): its live buffer always,
            // its server on 127.0.0.1:47680 once Settings turned it
            // on. Queue owner only (a phone's send is a turn this
            // process runs), and never a candidate build (the
            // everyday app owns the port).
            if queue.owned() && instance::background_agents_allowed() {
                phone_access::start(core.clone(), vault_path.clone());
            }
            // M5 PR2: when this Mac sleeps, its running chats move to the
            // paired runner (`chat_move_trigger`); open moves are retried
            // every minute and at wake. Queue owner only (it stops this
            // process's turns), never a candidate build.
            // App only: serve is the runner, and a runner that sleeps
            // must never move its chats away (`SKIPPED_IN_SERVE`).
            if crate::chat_move_trigger::starts_here(
                core.gui().is_some(),
                queue.owned(),
                instance::current().is_candidate(),
                configured,
            ) {
                crate::chat_move_trigger::start();
            }
            // About you builder: "Keep learning" re-reads what the
            // user's agents know at most every 12 h, when it changed.
            // Queue owner only, so two Hollow processes never both scan.
            // A candidate build only when asked (instance.rs).
            if queue.owned() && instance::background_agents_allowed() {
                commands::profile_builder_api::start_background(core.clone(), vault_path.clone());
            }
            // Silent agentic heartbeat. Periodically (gated on a cheap
            // change-check + work-hours + budget) wakes Claude to
            // review the vault and act/surface autonomously. Settings,
            // Automation toggles it via set_heartbeat_enabled.
            if queue.owned() {
                heartbeat::start_timer(core.clone(), vault_path);
            }
            // Self-hosted models Q10: background jobs that wait for
            // an asleep self-hosted model drain here (a look every
            // minute and after the model answers). Queue owner only,
            // so two Hollow processes never both wake it.
            if queue.owned() {
                self_hosted_jobs::start(core.clone());
            }
            // The Main agent's check-ins (main agent M3): a token-free
            // gate every minute; queue owner only, and a candidate
            // build only when asked.
            if queue.owned() && instance::background_agents_allowed() {
                main_agent::start_timer(core.clone());
            }
            // M3a: the app asks paired Macs hourly whether they run a
            // runner COO, so the dispatcher stays on while one does (it
            // starts nothing in serve, which never dispatches). Same gate
            // as the check-ins: no candidate build or second process opens
            // tunnels or reads the Keychain.
            if queue.owned() && instance::background_agents_allowed() {
                crate::dispatcher_coo::start_refresh(core.clone());
            }
            // Scheduler v2 (F1): window jobs run once per
            // occurrence. Same gate as the check-ins. Its jobs still take
            // the GUI's handle (M1 PR 3), so serve skips it.
            if queue.owned() && instance::background_agents_allowed() {
                if let Some(app) = core.gui() {
                    schedule_windows::start_timer(app.clone());
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "core_boot_tests.rs"]
mod tests;
