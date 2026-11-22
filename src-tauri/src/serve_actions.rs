//! Hollow app tools in `hollow serve` (M1 PR 5).
//!
//! Every action handler reads the app's managed state through
//! `app_actions::Host::app_state`, which hands out a `tauri::State`, and Tauri
//! hands those out only through a `Manager`. Serve has no Tauri app, so it
//! keeps its [`AppState`] in a [`StateShell`]: a mock-runtime app (Tauri's
//! `test` feature, the recipe the action tests use) that is nothing but a
//! container for the state. It has no window, no webview and no event loop,
//! and it needs no window server (the tests run it on a remote runner with
//! no login session). Its path resolver and identifier are mock values, so the
//! shell is never exposed beyond its state.
//!
//! What serve runs, and what it refuses (see `docs/HEADLESS_SERVE.md`):
//! - Read and Write actions in the first set ([`screen`]): the reads of the
//!   areas the COO works in, and the writes the unattended COO may make,
//!   plus a few more. Anything else says it isn't on this runner yet.
//! - Confirm and External actions fail closed in every permission mode
//!   (bypass, an opted-in automation, an unattended COO's safe class):
//!   [`ServeAsker`] honors no standing approval and its [`FailClosed`]
//!   route shows no card. M2 adds a [`CardRoute`] that shows the card on the
//!   person's Mac; the broker and the actions don't change.
//! - Task tracker (Linear) writes are refused until cards reach the person.
//! - The [`SERVE_REFUSED`] ids need a window, the login Keychain or a GUI
//!   login session; an action that asks for the window anyway gets
//!   [`needs_window`]'s wording ([`reword`]).
//!
//! Every outcome, refusals included, is audited like the app's
//! (`app_broker::audit`).

use crate::app_actions::{self, Action, Caller, Card, Host, Tier};
use crate::approvals::Verdict;
use crate::commands::AppState;
use crate::core_host::Core;
use serde_json::Value;
use tauri::test::MockRuntime;
use tauri::Manager;

/// The shell's handle: the managed state's only way out.
type Shell = tauri::AppHandle<MockRuntime>;

/// Serve's [`AppState`], held by a mock-runtime app so action handlers get
/// their `tauri::State`. One per serve process, alive as long as its host.
pub(crate) struct StateShell {
    handle: Shell,
    /// The app itself (not `Sync`, so behind a lock nothing takes).
    _app: std::sync::Mutex<tauri::App<MockRuntime>>,
}

impl StateShell {
    pub(crate) fn new(state: AppState) -> Result<Self, String> {
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|e| format!("cannot hold the app state for Hollow app tools: {e}"))?;
        Ok(Self {
            handle: app.handle().clone(),
            _app: std::sync::Mutex::new(app),
        })
    }

    pub(crate) fn state(&self) -> &AppState {
        self.handle.state::<AppState>().inner()
    }

    /// The host action handlers run on, with serve's `core`.
    pub(crate) fn action_host(&self, core: Core) -> ServeActionHost {
        ServeActionHost {
            shell: self.handle.clone(),
            core,
        }
    }
}

/// What a handler runs on in serve: the shell's state, no GUI handle, and
/// serve's [`Core`] for a handler that calls a core service
/// (`sessions.stop`, `notifications.create_notification`).
#[derive(Clone)]
pub(crate) struct ServeActionHost {
    shell: Shell,
    core: Core,
}

impl Host for ServeActionHost {
    fn app_state(&self) -> tauri::State<'_, AppState> {
        self.shell.state::<AppState>()
    }

    fn app(&self) -> Option<&tauri::AppHandle> {
        None
    }

    fn boxed(&self) -> Box<dyn Host> {
        Box::new(self.clone())
    }

    fn core(&self) -> Option<Core> {
        Some(self.core.clone())
    }
}

// ── Cards ────────────────────────────────────────────────────────────────

/// Where serve sends a card the person must answer. PR 5 has only
/// [`FailClosed`]; M2 adds a route to the person's Mac (or a Needs-you
/// item) and changes nothing else.
pub(crate) trait CardRoute: Send + Sync {
    /// Whether a person can answer a card through this route now.
    fn live(&self) -> bool;
    /// Show the card and wait for the answer (called only when [`live`]).
    ///
    /// [`live`]: CardRoute::live
    fn ask(&self, card: &Card, caller: &Caller) -> Verdict;
    /// What the call hears when nobody can answer.
    fn refusal(&self, action: &Action) -> String;
}

/// PR 5's route: no card ever shows, so every call that needs one is refused.
pub(crate) struct FailClosed;

impl CardRoute for FailClosed {
    fn live(&self) -> bool {
        false
    }

    fn ask(&self, _card: &Card, _caller: &Caller) -> Verdict {
        Verdict::Cancelled
    }

    fn refusal(&self, action: &Action) -> String {
        format!("{} needs the person's Allow on a card, and this Hollow runs without a window (hollow serve); nothing ran and no card was shown. Put the request in your reply or a suggestion for the person, or do it later from Hollow on their Mac.", action.id)
    }
}

/// The broker's asker in serve: no standing approval (a chat's bypass or
/// auto mode, an automation's stamped tools, an unattended COO's safe class)
/// stands in for a card; the card goes to [`CardRoute`].
pub(crate) struct ServeAsker {
    route: Box<dyn CardRoute>,
    answered: std::cell::Cell<Option<Verdict>>,
}

impl ServeAsker {
    pub(crate) fn new(route: Box<dyn CardRoute>) -> Self {
        Self {
            route,
            answered: std::cell::Cell::new(None),
        }
    }

    /// PR 5's asker: every card refused.
    pub(crate) fn fail_closed() -> Self {
        Self::new(Box::new(FailClosed))
    }
}

impl crate::app_broker::Asker for ServeAsker {
    fn ask(&self, card: &Card, caller: &Caller) -> Verdict {
        let verdict = self.route.ask(card, caller);
        self.answered.set(Some(verdict));
        verdict
    }

    fn can_show(&self) -> bool {
        self.route.live()
    }

    fn standing_approvals(&self) -> bool {
        false
    }

    fn refusal(&self, action: &Action) -> String {
        self.route.refusal(action)
    }

    fn answered(&self) -> Option<Verdict> {
        self.answered.get()
    }
}

// ── The broker in serve ─────────────────────────────────────────────────

/// Serve's [`Policy`](crate::app_broker::Policy): [`screen`], then
/// [`reword`].
pub(crate) const SERVE_POLICY: crate::app_broker::Policy =
    crate::app_broker::Policy { screen, reword };

/// One admitted request in serve: the broker's own checks, run and audit
/// (`app_broker::dispatch`) on serve's action host, with
/// [`SERVE_POLICY`] and [`ServeAsker::fail_closed`].
pub(crate) fn dispatch(
    host: Box<dyn Host>,
    vault: &std::path::Path,
    req: &crate::app_broker::AppRequest,
    enabled: bool,
) -> Result<Value, String> {
    let make = || host.boxed();
    let asker = ServeAsker::fail_closed();
    crate::app_broker::dispatch(&make, vault, req, enabled, &asker, &SERVE_POLICY)
}

// ── What runs ────────────────────────────────────────────────────────────

/// Actions serve never runs, and why: they need Hollow's window or webview,
/// the login Keychain, a browser or Finder in a login session, or they quit
/// or replace the running Hollow. Checked before anything else.
pub(crate) const SERVE_REFUSED: &[(&str, &str)] = &[
    ("ui.read_tab", WINDOW),
    ("ui.inspect_tab", WINDOW),
    ("ui.navigate", WINDOW),
    ("ui.chat_preview_open", WINDOW),
    ("ui.open_markdown_preview", WINDOW),
    ("ui.vault_open_in_obsidian", FINDER),
    ("ui.revoke_tab", WINDOW),
    ("ui.list_tab_grants", WINDOW),
    ("ui.reveal_themes_folder", FINDER),
    ("ui.vault_reveal_in_finder", FINDER),
    ("system.dictation_status", WINDOW),
    ("system.quit_hollow", QUITS),
    ("system.setup_restart", QUITS),
    ("system.move_hollow_home", QUITS),
    ("system.rollback_move_home", QUITS),
    ("settings.app.run_on_startup_status", WINDOW),
    ("settings.save_custom_icon", WINDOW),
    ("settings.reset_custom_icon", WINDOW),
    ("settings.select_app_icon_preset", WINDOW),
    ("vault.save_preview_to_vault", WINDOW),
    ("canvases.export_canvas", WINDOW),
    ("build.install_built_app", QUITS),
    ("settings.connectors.m365_sign_in", SIGN_IN),
    ("settings.connectors.plaud_mcp_sign_in", SIGN_IN),
    ("settings.connectors.remote_connector_sign_in", SIGN_IN),
    ("settings.accounts.provider_sign_in", SIGN_IN),
    ("recordings.plaud_login", SIGN_IN),
    ("settings.accounts.refresh_claude_accounts", KEYCHAIN),
    ("tracker.use_keychain_key", KEYCHAIN),
    ("usage.claude_quota", KEYCHAIN),
];

const WINDOW: &str = "needs Hollow's window";
const FINDER: &str = "opens Finder or another app in a login session";
const QUITS: &str = "quits, restarts or replaces the running Hollow";
const SIGN_IN: &str = "opens a browser sign-in";
const KEYCHAIN: &str = "reads the login Keychain";

/// Areas whose reads run in serve.
const READ_AREAS: &[&str] = &[
    "sessions",
    "tasks",
    "workspaces",
    "vault",
    "automations",
    "notifications",
    "history",
    "usage",
    "goals",
    "commitments",
    "interactions",
    "review",
    "learning",
    "agenda",
    "tracker",
];

/// Reads outside [`READ_AREAS`] that run too.
const READ_IDS: &[&str] = &[
    "settings.get",
    "settings.get_main_agent_settings",
    "build.overview",
];

/// The writes that run in serve: every write the unattended COO may make
/// (`main_agent::unattended::SAFE`) and the ones its tools ask for.
pub(crate) const WRITE_IDS: &[&str] = &[
    "sessions.start",
    "sessions.send",
    "sessions.stop",
    "sessions.manage",
    "sessions.handover",
    "sessions.steer_session",
    "sessions.wake_me",
    "sessions.cancel_wake",
    "sessions.suggestion_outcome",
    "sessions.cancel_held_send",
    "sessions.submit_result",
    "tasks.create_task",
    "tasks.update_task",
    "tasks.set_task_status",
    "tasks.set_task_priority",
    "tasks.restore_task",
    "tasks.create_project",
    "tasks.update_project",
    "tasks.set_project_status",
    "workspaces.create_workspace",
    "workspaces.update_workspace",
    "workspaces.set_result_review",
    "workspaces.triage_mark",
    "vault.vault_create_note",
    "vault.vault_write_note",
    "vault.vault_save_note_cas",
    "vault.vault_give_up_write",
    "vault.answer_question",
    "vault.save_answer",
    "notifications.create_notification",
    "learning.submit_learning_proposal",
    "agenda.prepare_meeting",
];

/// Whether serve runs `action` at all, before the broker's own checks. A
/// Confirm or External action passes here and is refused by its card
/// ([`ServeAsker`]), so its refusal says what it needs.
pub(crate) fn screen(action: &Action) -> Result<(), String> {
    if let Some((_, why)) = SERVE_REFUSED.iter().find(|(id, _)| *id == action.id) {
        return Err(format!("{} {why}, and this Hollow runs without one (hollow serve); nothing ran. Put it in your reply for the person to do in Hollow on their Mac.", action.id));
    }
    let tracker_write =
        action.area == "tracker" || app_actions::TRACKER_WRITES.contains(&action.id);
    if tracker_write && action.tier == Tier::Write {
        return Err(format!("{} writes to the task tracker, which this Hollow (hollow serve) doesn't do yet; nothing ran. Put it in your reply for the person to do from Hollow on their Mac.", action.id));
    }
    let runs = match action.tier {
        Tier::Read => READ_AREAS.contains(&action.area) || READ_IDS.contains(&action.id),
        Tier::Write => WRITE_IDS.contains(&action.id),
        Tier::Confirm | Tier::External => true,
    };
    if runs {
        Ok(())
    } else {
        Err(format!("{} isn't available on this runner yet (hollow serve runs a first set of Hollow app tools); nothing ran. Put it in your reply for the person to do in Hollow on their Mac.", action.id))
    }
}

/// The refusal for an action that asked for Hollow's window.
pub(crate) fn needs_window(id: &str) -> String {
    format!("{id} needs Hollow's window, and this Hollow runs without one (hollow serve); nothing ran. Put it in your reply for the person to do in Hollow on their Mac.")
}

/// A handler's "needs the running app" (`Cx::app` and its kin) in serve's
/// words: the app's wording would send the agent looking for an app that
/// isn't there.
pub(crate) fn reword(id: &str, outcome: Result<Value, String>) -> Result<Value, String> {
    outcome.map_err(|error| {
        let window = [
            "needs the running app",
            "need the running app",
            "Hollow's window is unavailable",
        ]
        .iter()
        .any(|says| error.contains(says));
        if window {
            needs_window(id)
        } else {
            error
        }
    })
}

#[cfg(test)]
#[path = "serve_actions_tests.rs"]
mod tests;
