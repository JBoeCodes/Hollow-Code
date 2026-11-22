//! Every setting an agent can read or change through Hollow app tools
//! (PR A4): one row per backend setting and per UI-owned key, with its
//! type, tier, owner and the Settings section it lives in (an id from
//! `src/settingsIndex.ts`; a vitest guard holds the two together, and
//! [`NONE`] names the sections that have no key, with why).
//!
//! **Owners.** A backend key is read and written by the same functions its
//! Settings row calls. A UI key lives in the webview (a store signal and
//! localStorage, pushed to the backend at boot), so it is read and changed
//! only through its store setter, over the webview bridge
//! (`commands::app_bridge_api`, `src/appBridge.ts`): writing its backend
//! mirror directly would be undone at the next launch and never show in
//! Settings.
//!
//! **Tiers.** `Write` keys change at once (recorded as the run's change);
//! `Confirm` keys (what agents may do, spend, secrets, oversight) show the
//! user old → new on a card first. Secrets are never returned (only whether
//! one is set) and show as •••• on the card.

use crate::app_actions::{Cx, Output, Tier};
use serde_json::{json, Value};

/// A setting's value type.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Type {
    Bool,
    Text,
    Choice(&'static [&'static str]),
    /// A whole number in this range, or null for the default.
    Int(i64, i64),
    /// A JSON object whose fields the summary names.
    Object,
    /// A secret string: never returned, •••• on a card.
    Secret,
}

/// Who owns a setting.
#[derive(Clone, Copy)]
pub(crate) enum Owner {
    /// Read and written in the backend by these functions (`set` returns
    /// the value it saved).
    Backend {
        get: fn(Cx) -> Output,
        set: fn(Cx, Value) -> Output,
    },
    /// A webview store key, changed only through its store setter.
    Ui,
}

impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Owner::Backend { .. } => "backend",
            Owner::Ui => "ui",
        })
    }
}

/// One setting.
#[derive(Debug)]
pub(crate) struct Key {
    pub key: &'static str,
    /// The Settings section id (`src/settingsIndex.ts`): one of the
    /// twelve live sections (Settings PR 3, plan section 6), never a part's
    /// or a retired section's id.
    pub section: &'static str,
    pub kind: Type,
    pub tier: Tier,
    pub owner: Owner,
    pub summary: &'static str,
    /// A value an agent may not set (checked before any card).
    pub refuse: Option<fn(&Value) -> Option<&'static str>>,
}

const AGENTS: &[&str] = &["claude", "codex", "antigravity", "grok"];

macro_rules! ui {
    ($key:literal, $section:literal, $kind:expr, $tier:ident, $summary:literal) => {
        Key {
            key: $key,
            section: $section,
            kind: $kind,
            tier: Tier::$tier,
            owner: Owner::Ui,
            summary: $summary,
            refuse: None,
        }
    };
}

macro_rules! backend {
    ($key:literal, $section:literal, $kind:expr, $tier:ident, $get:expr, $set:expr, $summary:literal) => {
        Key {
            key: $key,
            section: $section,
            kind: $kind,
            tier: Tier::$tier,
            owner: Owner::Backend {
                get: $get,
                set: $set,
            },
            summary: $summary,
            refuse: None,
        }
    };
}

/// Every setting.
pub(crate) static KEYS: &[Key] = &[
    // ── Connectors (what agents can reach) ──────────────────────────────
    ui!("agentTools.vaultSearch", "connectors", Type::Bool, Confirm, "Agents can search the vault and projects (hollow_vault)."),
    ui!("agentTools.codeContext", "connectors", Type::Bool, Confirm, "Agents can look up callers and impact in a git checkout (hollow_code_context)."),
    backend!("agentTools.tasks", "connectors", Type::Bool, Confirm, |_| Box::pin(async { Ok(json!(crate::tasks_mcp::enabled())) }), |_, v| Box::pin(async move {
        let on = v.as_bool().unwrap_or(false);
        crate::task_broker::save_settings(on)?;
        Ok(json!(on))
    }), "Agents can read and change tasks (hollow_tasks)."),
    Key {
        refuse: Some(|v| (v.as_bool() != Some(false)).then_some("An agent can only turn Hollow app tools off; turning them on is the user's.")),
        ..backend!("agentTools.app", "connectors", Type::Bool, Confirm, |_| Box::pin(async { Ok(json!(crate::app_mcp::enabled())) }), |_, _| Box::pin(async {
            crate::app_broker::save_settings(false)?;
            Ok(json!(false))
        }), "Agents can use Hollow app tools (this server). An agent may only switch it off.")
    },
    ui!("agentTools.computerControl", "connectors", Type::Bool, Confirm, "Agents can control this Mac's apps and browser (hollow_computer)."),
    ui!("agentTools.teamwork", "connectors", Type::Bool, Confirm, "Agents can delegate to the other agents (hollow_team)."),
    ui!("agentRole.claude", "connectors", Type::Object, Confirm, "Claude's teammate role: {role, brief}, prepended to every task delegated to it."),
    ui!("agentRole.codex", "connectors", Type::Object, Confirm, "Codex's teammate role: {role, brief}."),
    ui!("agentRole.antigravity", "connectors", Type::Object, Confirm, "Antigravity's teammate role: {role, brief}."),
    ui!("agentRole.grok", "connectors", Type::Object, Confirm, "Grok's teammate role: {role, brief}."),
    // ── Personalization: About you ──────────────────────────────────────
    ui!("operator.contextEnabled", "personalization", Type::Bool, Confirm, "Every chat and automation is told the operator profile (me/)."),
    backend!("profile.autoLearn", "personalization", Type::Bool, Confirm, |cx| Box::pin(async move {
        let view = crate::commands::profile_builder_api::view_in(cx.vault()).await?;
        Ok(serde_json::to_value(view).map_err(|e| e.to_string())?["autoLearn"].clone())
    }), |cx, v| Box::pin(async move {
        let on = v.as_bool().unwrap_or(false);
        crate::commands::profile_builder_api::set_auto_learn_in(cx.vault(), on).await?;
        Ok(json!(on))
    }), "About you looks for new facts on its own every half day (a model call each time)."),
    // ── Personalization: Memory ─────────────────────────────────────────
    backend!("learning.suggestions", "personalization",Type::Choice(&["off", "important", "all"]), Confirm, |_| Box::pin(async {
        Ok(json!(crate::vault_learning::settings().suggestions.as_str()))
    }), |_, v| Box::pin(async move {
        let mode = v.as_str().and_then(crate::vault_learning::Suggestions::parse).ok_or("off, important or all")?;
        Ok(json!(crate::vault_learning::set_suggestions(mode)?.suggestions.as_str()))
    }), "Learning suggestions: off (none), important (the default: at most one per exchange, only high impact, one per agent run) or all (up to three per exchange). Accepting stays the person's."),
    // ── Advanced, Search & indexes: context prioritizer (Jev; key in Accounts)
    backend!("jev.enabled", "advanced",Type::Bool, Confirm, |_| jev_get("enabled"), |_, v| jev_set(Some(v), None, None), "Rank connected evidence with Jev (a paid remote service) before each turn."),
    backend!("jev.decisions", "advanced",Type::Bool, Confirm, |_| jev_get("decisionsEnabled"), |_, v| jev_set(None, None, Some(v)), "Jev helps decide duplicates and relevance."),
    backend!("jev.apiKey", "accounts",Type::Secret, Confirm, |_| Box::pin(async {
        let status = serde_json::to_value(crate::jev::jev_status().await?).map_err(|e| e.to_string())?;
        Ok(json!({ "set": status["keyPresent"] }))
    }), |_, v| jev_set(None, Some(v), None), "The Jev (TypeSafe) API key. Never returned."),
    // ── Advanced, Search & indexes: Search by meaning ───────────────────
    ui!("semanticSearchEnabled", "advanced",Type::Bool, Write, "Index the vault on this Mac for meaning-based search."),
    // ── Models (agent updates in Accounts, runner routing in Devices) ───
    backend!("agents.defaultAgent", "models", Type::Choice(AGENTS), Confirm, |_| agent_get("/defaultAgent"), |_, v| agent_set("/defaultAgent", v), "The agent new chats use."),
    backend!("agents.model.claude", "models", Type::Text, Confirm, |_| agent_get("/models/claude"), |_, v| agent_set("/models/claude", v), "Claude's default model."),
    backend!("agents.model.codex", "models", Type::Text, Confirm, |_| agent_get("/models/codex"), |_, v| agent_set("/models/codex", v), "Codex's default model."),
    backend!("agents.model.antigravity", "models", Type::Text, Confirm, |_| agent_get("/models/antigravity"), |_, v| agent_set("/models/antigravity", v), "Antigravity's default model."),
    backend!("agents.model.grok", "models", Type::Text, Confirm, |_| agent_get("/models/grok"), |_, v| agent_set("/models/grok", v), "Grok's default model."),
    backend!("agents.effort.claude", "models", Type::Choice(&crate::effort::LEVELS), Confirm, |_| agent_get("/efforts/claude"), |_, v| agent_set("/efforts/claude", v), "Claude's reasoning effort (mapped to the nearest level each model takes)."),
    backend!("agents.effort.codex", "models", Type::Choice(&crate::effort::LEVELS), Confirm, |_| agent_get("/efforts/codex"), |_, v| agent_set("/efforts/codex", v), "Codex's reasoning effort (mapped to the nearest level each model takes)."),
    backend!("agents.effort.antigravity", "models", Type::Choice(&crate::effort::LEVELS), Confirm, |_| agent_get("/efforts/antigravity"), |_, v| agent_set("/efforts/antigravity", v), "Antigravity's reasoning effort (picks the model's matching variant)."),
    backend!("agents.effort.grok", "models", Type::Choice(&crate::effort::LEVELS), Confirm, |_| agent_get("/efforts/grok"), |_, v| agent_set("/efforts/grok", v), "Grok's reasoning effort (mapped to the nearest level each model takes)."),
    backend!("agents.updates.claude", "accounts", Type::Choice(&["automatic", "ask", "off"]), Confirm, |_| cli_policy_get("claude"), |_, v| cli_policy_set("claude", v), "When Hollow updates Claude Code's CLI: automatic (checks daily, updates when no run uses it; the default), ask (says when one is ready) or off."),
    backend!("agents.updates.codex", "accounts", Type::Choice(&["automatic", "ask", "off"]), Confirm, |_| cli_policy_get("codex"), |_, v| cli_policy_set("codex", v), "When Hollow updates Codex's CLI: automatic (checks daily, updates when no run uses it; the default), ask (says when one is ready) or off."),
    backend!("agents.updates.antigravity", "accounts", Type::Choice(&["automatic", "ask", "off"]), Confirm, |_| cli_policy_get("antigravity"), |_, v| cli_policy_set("antigravity", v), "When Hollow updates Antigravity's CLI: automatic (checks daily, updates when no run uses it; the default), ask (says when one is ready) or off."),
    backend!("agents.updates.grok", "accounts", Type::Choice(&["automatic", "ask", "off"]), Confirm, |_| cli_policy_get("grok"), |_, v| cli_policy_set("grok", v), "When Hollow updates Grok's CLI: automatic (checks daily, updates when no run uses it; the default), ask (says when one is ready) or off."),
    backend!("agents.acp.grok", "advanced", Type::Bool, Confirm, |_| agent_get("/acp/grok"), |_, v| agent_set("/acp/grok", v), "Run Grok's chat turns over the Agent Client Protocol instead of its one-shot command (off by default; only bypass-permissions turns go, every other mode stays on the one-shot command)."),
    backend!("agents.codexNoOutputTimeoutMin", "advanced", Type::Int(0, 1440), Confirm, |_| agent_get("/codexNoOutputTimeoutMin"), |_, v| agent_set("/codexNoOutputTimeoutMin", v), "Minutes Codex may go silent before it is stopped (null switches the watchdog off)."),
    // The Mac mini runner (2026-11-01): absent is this Mac.
    backend!("agents.runsOn", "devices", Type::Choice(&crate::agent_settings::RUNS_ON), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::runs_on_choice(settings.runs_on.as_deref()).unwrap_or_else(|| "this-mac".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/runsOn", v).await?;
        Ok(if saved.is_null() { json!("this-mac") } else { saved })
    }), "Where a build workspace's Claude and Codex turns run when it has no choice of its own: this-mac (the default) or runner (the remote runner in ~/.config/hollow/slot-remote; mac-mini, its old name, means the same; the agent process runs there in a copy of the worktree, everything else stays here). Another workspace's chats move only on that workspace's own choice; the COO and Chats never. Unattended work and worker chats follow agents.unattendedRunsOn instead, and with its default they run here when the runner can't take them."),
    // Overnight N6 (2026-11-02): absent follows Runs on (M7; was the runner).
    backend!("agents.unattendedRunsOn", "devices", Type::Choice(&crate::agent_settings::UNATTENDED_RUNS_ON), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::unattended_runs_on_choice(settings.unattended_runs_on.as_deref()).unwrap_or_else(|| "same".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/unattendedRunsOn", v).await?;
        Ok(if saved.is_null() { json!("same") } else { saved })
    }), "Where unattended work (night-queue items, chats an unattended run starts) and a COO worker batch's chats run when their build workspace has no Runs on of its own: same (the default: follow Runs on, so naming a runner moves nothing) or runner (mac-mini, its old name, means the same: the remote runner when it can take the turn, else this Mac with a notice in the chat). A workspace's own choice, vault workspaces, the COO and Chats are unchanged."),
    // M3a PR2 (2026-11-15): absent is auto.
    backend!("agents.cooDispatcher", "devices", Type::Choice(&crate::agent_settings::COO_DISPATCHER), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::coo_dispatcher_choice(settings.coo_dispatcher.as_deref()).unwrap_or_else(|| "auto".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/cooDispatcher", v).await?;
        Ok(if saved.is_null() { json!("auto") } else { saved })
    }), "What this Mac's COO is when a paired Mac runs a runner COO: auto (the default: the dispatcher, which answers quick questions itself and sends long work to the runner COO or a project's lead there) or off (always the full COO here, as with no runner paired). The next COO turn uses it."),
    // Smart routing (2026-11-03): absent is this-mac (M7; was auto).
    backend!("agents.buildsOn", "devices", Type::Choice(&crate::agent_settings::BUILDS_ON), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::builds_on_choice(settings.builds_on.as_deref()).unwrap_or_else(|| "this-mac".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/buildsOn", v).await?;
        Ok(if saved.is_null() { json!("this-mac") } else { saved })
    }), "Where hollow-slot runs a turn's build and test commands when a runner is set up (~/.config/hollow/slot-remote): this-mac (the default, so naming a runner sends nothing there), auto (each command decides, and its first line says where and why: gates, Rust builds and whole test suites go to the runner, quick checks and anything needing this Mac stay here), or runner (everything that can go; mac-mini, its old name, means the same; a command needing this Mac still stays). Applies to turns that start after the change."),
    // Smart routing 7 (2026-11-03): absent is this-mac.
    backend!("agents.appBuildsOn", "advanced", Type::Choice(&crate::agent_settings::APP_BUILDS_ON), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::app_builds_on_choice(settings.app_builds_on.as_deref()).unwrap_or_else(|| "this-mac".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/appBuildsOn", v).await?;
        Ok(if saved.is_null() { json!("this-mac") } else { saved })
    }), "Where Build app compiles Hollow in a Hollow build workspace: this-mac (the default) or runner (mac-mini, its old name, means the same: the remote runner in ~/.config/hollow/slot-remote compiles it, signed ad hoc there; this Mac signs it again with its own identity, then Install and restart works as usual). Any failure there builds here instead. The runner may link against an older macOS SDK than this Mac's."),
    // Repo ideas 2026-10-27 #8: absent is Wait.
    backend!("agents.whenOut", "models", Type::Choice(&["wait", "claude", "codex", "antigravity", "grok"]), Confirm, |_| Box::pin(async {
        let settings = crate::agent_settings::get_settings()?;
        Ok(json!(crate::agent_settings::when_out_choice(settings.when_out.as_deref()).unwrap_or_else(|| "wait".into())))
    }), |_, v| Box::pin(async move {
        let saved = agent_set("/whenOut", v).await?;
        Ok(if saved.is_null() { json!("wait") } else { saved })
    }), "When an agent runs out of usage: wait (the default: the chat says when it has room again) or an agent to continue the conversation on, only after a turn that changed nothing."),
    // ── Chat ────────────────────────────────────────────────────────────
    backend!("chat.canvases", "chat", Type::Choice(&["off", "helps", "most"]), Confirm, |_| agent_get("/canvases"), |_, v| agent_set("/canvases", v), "How often agents answer with a canvas: off, helps (when it helps; the default, read as null) or most (most replies, more tokens). A chat's own choice in the model menu wins for that chat."),
    // ── Usage ───────────────────────────────────────────────────────────
    ui!("claudeQuotaOptIn", "usage", Type::Bool, Confirm, "Show the Claude plan's usage limits (reads the Claude login from the Keychain and asks Anthropic)."),
    // Automatic rotation has no off switch. Only its early-move threshold is configurable.
    backend!("accounts.claudeCeiling", "usage", Type::Object, Confirm, |_| Box::pin(async {
        let view = crate::claude_accounts::claude_accounts().await?;
        Ok(json!({ "ceilingPercent": view.ceiling_percent }))
    }), |_, v| Box::pin(async move {
        let ceiling = v["ceilingPercent"].as_u64().filter(|n| (50..=100).contains(n))
            .ok_or_else(|| "ceilingPercent must be an integer from 50 to 100".to_string())? as u8;
        let view = crate::claude_accounts::save_routing(crate::claude_accounts::Routing::Spread, None, Some(ceiling)).await?;
        Ok(json!({ "ceilingPercent": view.ceiling_percent }))
    }), "Automatic Claude rotation's early-move threshold: {ceilingPercent: 50-100}. Rotation and quota recovery always remain on; accounts with 98% or more used are never selected."),
    backend!("accounts.claudeFailoverAcrossOrganizations", "usage", Type::Bool, Confirm, |_| Box::pin(async {
        let view = serde_json::to_value(crate::claude_accounts::claude_accounts().await?).map_err(|e| e.to_string())?;
        Ok(view["failoverAcrossOrganizations"].clone())
    }), |_, v| Box::pin(async move {
        let on = v.as_bool().unwrap_or(false);
        let view = crate::claude_accounts::set_claude_account_failover_across_organizations(on).await?;
        Ok(json!(view.failover_across_organizations))
    }), "Failover and long-run routing may move a run to a Claude account of another organisation (off: they stay inside the organisation set on each account)."),
    // ── General: Startup, and Background work: the heartbeat ────────────
    ui!("app.runOnStartup", "general", Type::Bool, Confirm, "Start Hollow at login (a macOS login item)."),
    ui!("app.minimizeToTray", "general", Type::Bool, Write, "Closing the window keeps Hollow running in the menu bar."),
    ui!("heartbeat.enabled", "background", Type::Bool, Confirm, "The heartbeat runs background agent passes (they spend)."),
    ui!("heartbeat.drafts", "background", Type::Bool, Confirm, "The heartbeat may write drafts."),
    ui!("heartbeat.config", "background", Type::Object, Confirm, "{workHoursStart, workHoursEnd, minIntervalMin, dailyBudget}: when and how often the heartbeat runs."),
    // ── General ─────────────────────────────────────────────────────────
    ui!("theme", "general", Type::Text, Write, "system (Match macOS), light, dark, glass-light, glass-dark, or custom:<theme name>."),
    ui!("notifyInApp", "general", Type::Bool, Write, "Show notification toasts in the window."),
    ui!("notifyNative", "general", Type::Bool, Confirm, "Show macOS notification banners (off hides the run-guard and budget alerts too)."),
    ui!("nudgesEnabled", "general", Type::Bool, Write, "Nudge about overdue and stuck work."),
    ui!("sidebarCollapsed", "general", Type::Bool, Write, "The sidebar is collapsed."),
    ui!("chatPanelOpen", "general", Type::Bool, Write, "The chat side panel is open."),
    ui!("sidePaneWidth", "general", Type::Int(240, 1600), Write, "The side pane's width in pixels (null for the default)."),
    ui!("chatContextEnabled", "general", Type::Bool, Write, "Each chat message carries a snapshot of the page the user is on."),
    ui!("railCollapsed", "general", Type::Object, Write, "{ids: [workspace ids]}: workspaces whose conversations are folded in the sidebar."),
    ui!("notifFilter", "general", Type::Choice(&["all", "attention", "routines", "nudges", "agent"]), Write, "The notification center's filter."),
    ui!("paletteRecents", "general", Type::Object, Write, "{ids: []}: the command palette's recent items (only clearing is useful)."),
    // ── Keyboard shortcuts ──────────────────────────────────────────────
    backend!("shortcuts", "shortcuts", Type::Object, Write, |_| Box::pin(async {
        serde_json::to_value(crate::shortcuts::view()).map_err(|e| e.to_string())
    }), |_, v| Box::pin(async move {
        let next = crate::shortcuts::merge(crate::shortcuts::load(), &v)?;
        let view = crate::shortcuts::change(crate::notifications::registered_app(), next)?;
        serde_json::to_value(view).map_err(|e| e.to_string())
    }), "{enabled?, keys?}: enabled false turns every Hollow shortcut off (in the window and system-wide); keys maps a command id (the Settings → Keyboard shortcuts rows: new-conversation, search, go-tasks, quick-capture, …) to a chord like CmdOrCtrl+Shift+K, \"\" for none, or null for its default. Merged into the saved settings."),
    // ── Sources: the task tracker (T3a) ─────────────────────────────────
    backend!("tracker.trackAgentWork", "sources", Type::Choice(&crate::tracker::settings::AgentWork::VALUES), Confirm, |cx| Box::pin(async move {
        Ok(json!(crate::commands::tracker_api::agent_work(&cx.state()).as_str()))
    }), |cx, v| Box::pin(async move {
        Ok(json!(crate::commands::tracker_api::save_agent_work(&cx.state(), v.as_str().unwrap_or(""))?))
    }), "Track agent work: whether a person's chat that did a build, research or a deliverable becomes a tracker issue (team AI for a tracked project, Personal otherwise), as do subagents in tracked projects. off; dryRun (the default: Hollow records in its audit what it would file and sends nothing); on. Quick questions and checks never get one. Applies after a restart; issues already filed stay."),
    // Leak checks (CX10): one switch; unset means leak_checks::DEFAULT_MODE (flag, until CX10b).
    backend!("context.leakChecks", "sources", Type::Choice(&crate::leak_checks::LeakMode::VALUES), Confirm, |_| Box::pin(async {
        Ok(json!(crate::leak_checks::mode().as_str()))
    }), |_, v| Box::pin(async move {
        Ok(json!(crate::leak_checks::set_mode(v.as_str().unwrap_or(""))?.as_str()))
    }), "Leak checks at write time: flag (the default for the first week: a project doc write that names a computer or a person is written and recorded in the leak flags, context.leak_flags) or block (refused, naming the right layer). A secret is refused in both."),
    // Folder abstracts (CX23): off until a measured recall comparison says they help.
    backend!("context.folderAbstracts", "sources", Type::Bool, Confirm, |_| Box::pin(async {
        Ok(json!(crate::folder_abstracts::enabled()))
    }), |_, v| Box::pin(async move {
        Ok(json!(crate::folder_abstracts::set_enabled(v.as_bool().unwrap_or(false))?))
    }), "Folder abstracts: a one-line abstract and short overview per project and org section, built by context.refresh_folder_abstracts (a model call per changed file or folder) and shown on list_workspaces rows. Off by default."),
    // ── Advanced ────────────────────────────────────────────────────────
    ui!("backupManagedElsewhere", "advanced", Type::Bool, Write, "The user backs the vault up another way (the backup check stops warning)."),
];

/// Sections and parts (a former section kept whole, S2) that hold no key
/// of their own, and where their changes go. `settings.get` lists them
/// under `sectionsWithoutKeys` with the section that holds them.
pub(crate) const NONE: &[(&str, &str)] = &[
    ("vault", "the Hollow folder and its source change through system.connect_hollow_home and system.set_workspace_source"),
    ("history", "restores and the clear are history.* actions"),
    ("skills", "skills.* actions"),
    ("updates", "update checks, the channel and the update token are the person's own through Settings, Updates (the update_* commands are user-only; no agent action installs or changes the channel)"),
    ("diffs", "diff view switches are per-machine view preferences set in Settings only (store/viewPrefs.ts)"),
    ("browser", "where web links open is a per-machine view preference set in Settings only (store/viewPrefs.ts)"),
    ("worktrees", "the worktree report is build.worktree_hygiene and the clean-up is build.clean_worktrees"),
    ("archived", "archiving and restoring conversations are the sessions.* organization actions"),
    ("about", "the welcome page reopens through its existing user-only setup commands; no settings key"),
    ("selfhosted", "self-hosted model endpoints and their Test are the user's own (the *self_hosted_model* commands are user-only)"),
];

/// A retired Settings section id and the section that holds it now, the
/// same table as `SETTINGS_SECTION_ALIASES` in `src/settingsIndex.ts` (a
/// vitest guard holds the two together): where Settings opens a retired id.
/// Settings PR 3 (11-12): the seven sections before the twelve are retired
/// ids too. `settings.get` with a retired id reads `LEGACY_SECTIONS` instead,
/// so an older call keeps returning the same keys.
pub(crate) const SECTION_ALIASES: &[(&str, &str)] = &[
    ("interface", "general"),
    ("defaults", "models"),
    ("operator", "personalization"),
    ("automation", "background"),
    ("system", "advanced"),
    ("capture", "connectors"),
    ("cadence", "connectors"),
    ("doctor", "advanced"),
    ("customization", "models"),
    ("agenttools", "connectors"),
    ("teamwork", "connectors"),
    ("computer", "connectors"),
    ("m365write", "connectors"),
    ("startup", "general"),
    ("routines", "background"),
    ("heartbeat", "background"),
    ("appearance", "general"),
    ("notifications", "general"),
    ("paths", "sources"),
    ("debug", "advanced"),
    ("setup", "sources"),
    ("feedback", "usage"),
    ("context", "advanced"),
    ("vaultsearch", "advanced"),
    ("learning", "personalization"),
    ("companions", "devices"),
    ("imports", "connectors"),
    ("skills", "connectors"),
    ("indexes", "advanced"),
    ("vault", "sources"),
    ("history", "advanced"),
    ("selfhosted", "models"),
    ("layers", "sources"),
];

/// Each key's section before Settings PR 3, frozen from main `b0ed05aea`
/// (the contract in `src/settingsLegacy.json`): `settings.get` with an id
/// that is no longer a live section returns exactly the keys it returned
/// then, though PR 3 split those sections up.
pub(crate) const LEGACY_SECTIONS: &[(&str, &[&str])] = &[
    (
        "connectors",
        &[
            "agentTools.vaultSearch",
            "agentTools.tasks",
            "agentTools.app",
            "agentTools.computerControl",
            "agentTools.teamwork",
            "agentRole.claude",
            "agentRole.codex",
            "agentRole.antigravity",
            "agentRole.grok",
            "tracker.trackAgentWork",
        ],
    ),
    (
        "operator",
        &[
            "operator.contextEnabled",
            "profile.autoLearn",
            "learning.suggestions",
            "jev.enabled",
            "jev.decisions",
            "jev.apiKey",
            "semanticSearchEnabled",
        ],
    ),
    (
        "defaults",
        &[
            "agents.defaultAgent",
            "agents.model.claude",
            "agents.model.codex",
            "agents.model.antigravity",
            "agents.model.grok",
            "agents.effort.claude",
            "agents.effort.codex",
            "agents.effort.antigravity",
            "agents.effort.grok",
            "agents.updates.claude",
            "agents.updates.codex",
            "agents.updates.antigravity",
            "agents.updates.grok",
            "agents.codexNoOutputTimeoutMin",
            "agents.runsOn",
            "agents.unattendedRunsOn",
            "agents.buildsOn",
            "agents.appBuildsOn",
            "agents.whenOut",
            "chat.canvases",
            "accounts.claudeCeiling",
            "accounts.claudeFailoverAcrossOrganizations",
        ],
    ),
    ("usage", &["claudeQuotaOptIn"]),
    (
        "automation",
        &[
            "app.runOnStartup",
            "app.minimizeToTray",
            "heartbeat.enabled",
            "heartbeat.drafts",
            "heartbeat.config",
        ],
    ),
    (
        "interface",
        &[
            "theme",
            "notifyInApp",
            "notifyNative",
            "nudgesEnabled",
            "sidebarCollapsed",
            "chatPanelOpen",
            "sidePaneWidth",
            "chatContextEnabled",
            "railCollapsed",
            "notifFilter",
            "paletteRecents",
            "shortcuts",
        ],
    ),
    ("system", &["backupManagedElsewhere"]),
];

/// Keys added after that freeze: no retired id ever returned them, so an old
/// id still returns exactly what it did, and they are read only under their
/// live section.
#[cfg(test)]
pub(crate) const NEWER_KEYS: &[&str] = &[
    "context.leakChecks",
    "agents.cooDispatcher",
    "context.folderAbstracts",
    "agentTools.codeContext",
    "agents.acp.grok",
];

/// The retired ids before Settings PR 3 and the section each opened then
/// (frozen from main `b0ed05aea`, as `LEGACY_SECTIONS`).
pub(crate) const LEGACY_ALIASES: &[(&str, &str)] = &[
    ("capture", "connectors"),
    ("cadence", "connectors"),
    ("doctor", "system"),
    ("customization", "defaults"),
    ("agenttools", "connectors"),
    ("teamwork", "connectors"),
    ("computer", "connectors"),
    ("m365write", "connectors"),
    ("startup", "automation"),
    ("routines", "automation"),
    ("heartbeat", "automation"),
    ("appearance", "interface"),
    ("notifications", "interface"),
    ("paths", "system"),
    ("debug", "system"),
    ("setup", "system"),
    ("feedback", "usage"),
    ("context", "system"),
    ("vaultsearch", "system"),
    ("learning", "operator"),
    ("companions", "connectors"),
    ("imports", "connectors"),
    ("shortcuts", "interface"),
    ("chat", "defaults"),
    ("accounts", "defaults"),
    ("skills", "defaults"),
    ("indexes", "system"),
    ("vault", "system"),
    ("history", "system"),
    ("selfhosted", "defaults"),
];

/// The old section `id` named before Settings PR 3, when `id` is not a live
/// section now (a section of the twelve, or Sources, which has no key).
pub(crate) fn legacy_section(id: &str) -> Option<&'static str> {
    let live = resolve_section(id) == id
        && (KEYS.iter().any(|k| k.section == id) || NONE.iter().any(|(none, _)| *none == id));
    if live {
        return None;
    }
    let old = LEGACY_ALIASES
        .iter()
        .find(|(retired, _)| *retired == id)
        .map_or(id, |(_, old)| *old);
    LEGACY_SECTIONS
        .iter()
        .find(|(section, _)| *section == old)
        .map(|(section, _)| *section)
}

/// The keys an old section held before Settings PR 3.
pub(crate) fn legacy_keys(old: &str) -> &'static [&'static str] {
    LEGACY_SECTIONS
        .iter()
        .find(|(section, _)| *section == old)
        .map_or(&[][..], |(_, keys)| *keys)
}

/// The live section an id names: itself, or the section a retired id is
/// part of now.
pub(crate) fn resolve_section(id: &str) -> &str {
    SECTION_ALIASES
        .iter()
        .find(|(old, _)| *old == id)
        .map_or(id, |(_, live)| live)
}

pub(crate) fn find(key: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.key == key.trim())
}

/// Check a value against a key's type (before any card or bridge call).
pub(crate) fn validate(key: &Key, value: &Value) -> Result<(), String> {
    let fits = match key.kind {
        Type::Bool => value.is_boolean(),
        Type::Text => value
            .as_str()
            .is_some_and(|s| s.chars().count() <= 200 && !s.contains('\n')),
        Type::Choice(values) => value.as_str().is_some_and(|s| values.contains(&s)),
        Type::Int(min, max) => {
            value.is_null() || value.as_i64().is_some_and(|n| (min..=max).contains(&n))
        }
        Type::Object => value.is_object() && value.to_string().len() <= 16 * 1024,
        Type::Secret => value
            .as_str()
            .is_some_and(|s| !s.trim().is_empty() && s.len() <= 4096),
    };
    if !fits {
        let want = match key.kind {
            Type::Bool => "true or false".to_string(),
            Type::Text => "one line of text (at most 200 characters)".to_string(),
            Type::Choice(values) => format!("one of {}", values.join(", ")),
            Type::Int(min, max) => format!("a whole number from {min} to {max}, or null"),
            Type::Object => "an object".to_string(),
            Type::Secret => "a non-empty string".to_string(),
        };
        return Err(format!(
            "{}: the value must be {want}. Nothing changed.",
            key.key
        ));
    }
    if let Some(reason) = key.refuse.and_then(|refuse| refuse(value)) {
        return Err(format!("{} Nothing changed.", reason));
    }
    Ok(())
}

/// A value as a card shows it.
pub(crate) fn shown(key: &Key, value: &Value) -> String {
    match key.kind {
        Type::Secret => "••••".into(),
        _ => match value {
            Value::Null => "(default)".into(),
            Value::String(s) => s.clone(),
            other => crate::app_actions::cut(&other.to_string(), 200),
        },
    }
}

/// The catalog row of a key (never its value).
pub(crate) fn describe(key: &Key) -> Value {
    let kind = match key.kind {
        Type::Bool => json!("bool"),
        Type::Text => json!("text"),
        Type::Choice(values) => json!({ "oneOf": values }),
        Type::Int(min, max) => json!({ "int": [min, max] }),
        Type::Object => json!("object"),
        Type::Secret => json!("secret"),
    };
    json!({
        "key": key.key,
        "section": key.section,
        "type": kind,
        "tier": key.tier.as_str(),
        "owner": format!("{:?}", key.owner),
        "summary": key.summary,
    })
}

// ── Backend getters and setters ─────────────────────────────────────────

fn cli_policy_get(agent: &'static str) -> Output {
    Box::pin(async move {
        let kind = crate::agent::AgentKind::from_opt(agent).ok_or("unknown agent")?;
        Ok(json!(crate::agent_cli::policy(kind).as_str()))
    })
}

fn cli_policy_set(agent: &'static str, value: Value) -> Output {
    Box::pin(async move {
        let text = value.as_str().ok_or("Updates are automatic, ask or off.")?;
        Ok(json!(crate::agent_cli::set_policy(agent, text)?))
    })
}

fn agent_get(pointer: &'static str) -> Output {
    Box::pin(async move {
        let settings = serde_json::to_value(crate::agent_settings::get_settings()?)
            .map_err(|e| e.to_string())?;
        Ok(settings.pointer(pointer).cloned().unwrap_or(Value::Null))
    })
}

/// Read-modify-write of one agent setting through `set_agent_settings`'s
/// own function (normalization included).
fn agent_set(pointer: &'static str, value: Value) -> Output {
    Box::pin(async move {
        let mut settings = serde_json::to_value(crate::agent_settings::get_settings()?)
            .map_err(|e| e.to_string())?;
        let (parent, leaf) = pointer.rsplit_once('/').unwrap_or(("", pointer));
        let leaf = leaf.to_string();
        let target = if parent.is_empty() {
            Some(&mut settings)
        } else {
            settings.pointer_mut(parent)
        };
        let object = match target {
            Some(Value::Object(object)) => object,
            Some(slot) => {
                *slot = json!({});
                slot.as_object_mut().expect("just made an object")
            }
            None => {
                let top = parent.trim_start_matches('/').to_string();
                settings[top.as_str()] = json!({});
                settings[top.as_str()]
                    .as_object_mut()
                    .expect("just made an object")
            }
        };
        match (&value, parent.is_empty()) {
            (Value::Null, false) => {
                object.remove(&leaf);
            }
            _ => {
                object.insert(leaf, value);
            }
        }
        let next = serde_json::from_value(settings).map_err(|e| format!("agent settings: {e}"))?;
        let saved = serde_json::to_value(crate::agent_settings::set_settings(next)?)
            .map_err(|e| e.to_string())?;
        Ok(saved.pointer(pointer).cloned().unwrap_or(Value::Null))
    })
}

fn jev_get(field: &'static str) -> Output {
    Box::pin(async move {
        let status =
            serde_json::to_value(crate::jev::jev_status().await?).map_err(|e| e.to_string())?;
        Ok(status[field].clone())
    })
}

/// `save_jev_settings` with the current values for what isn't changing.
fn jev_set(enabled: Option<Value>, key: Option<Value>, decisions: Option<Value>) -> Output {
    Box::pin(async move {
        let status =
            serde_json::to_value(crate::jev::jev_status().await?).map_err(|e| e.to_string())?;
        let enabled = enabled
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| status["enabled"].as_bool().unwrap_or(false));
        let saved = crate::jev::save_jev_settings(
            enabled,
            key.and_then(|v| v.as_str().map(str::to_string)),
            decisions.and_then(|v| v.as_bool()),
        )
        .await?;
        let saved = serde_json::to_value(saved).map_err(|e| e.to_string())?;
        Ok(
            json!({ "enabled": saved["enabled"], "keySet": saved["keyPresent"], "decisionsEnabled": saved["decisionsEnabled"] }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn keys_are_unique_typed_and_in_live_sections() {
        let sections = include_str!("../../src/settingsIndex.ts");
        let mut seen = BTreeSet::new();
        for key in KEYS {
            assert!(seen.insert(key.key), "duplicate key {}", key.key);
            assert!(
                sections.contains(&format!("id: '{}'", key.section)),
                "{}: no section {}",
                key.key,
                key.section
            );
            assert!(
                matches!(key.tier, Tier::Write | Tier::Confirm),
                "{}: a setting is write or confirm",
                key.key
            );
            assert!(
                key.summary.ends_with('.') || key.summary.ends_with(')'),
                "{}",
                key.key
            );
            if matches!(key.kind, Type::Secret) {
                assert_eq!(key.tier, Tier::Confirm, "{}: a secret always asks", key.key);
            }
        }
        for (section, why) in NONE {
            // A live id, or an old part id kept as an alias (History & recovery
            // moved to Library in Settings PR 4; `history` still resolves).
            assert!(
                sections.contains(&format!("id: '{section}'"))
                    || sections.contains(&format!("\n  {section}: '")),
                "{section}"
            );
            // A part (a retired id, read as its section) or a live section
            // with no key of its own (Sources).
            assert!(
                !KEYS.iter().any(|k| k.section == *section),
                "{section} has keys but is listed as none"
            );
            assert!(!why.is_empty());
        }
        // A key names the live section that shows it, never a part's old id.
        for key in KEYS {
            assert_eq!(
                resolve_section(key.section),
                key.section,
                "{}: {} is a retired id",
                key.key,
                key.section
            );
        }
    }

    #[test]
    fn every_old_id_and_emitted_settings_link_reaches_a_section_with_keys() {
        // `settings.get` filters on `resolve_section(asked)`, so an old id
        // returns exactly the keys of the section that holds it now.
        // Or, for a section with no key of its own (Sources, PR 3), says
        // where its changes go.
        let has_keys = |section: &str| {
            KEYS.iter().any(|k| k.section == section) || NONE.iter().any(|(id, _)| *id == section)
        };
        for (old, live) in SECTION_ALIASES {
            assert_eq!(resolve_section(old), *live, "{old}");
            assert!(has_keys(live), "{old} -> {live} has no keys");
            let asked = KEYS
                .iter()
                .filter(|k| k.section == resolve_section(old))
                .count();
            let held = KEYS.iter().filter(|k| k.section == *live).count();
            assert_eq!(asked, held, "{old}");
        }
        // The `settings:` links Hollow emits (notifications, run guard,
        // shortcuts, disk guard, accounts, heartbeat beat prompt) name a live
        // section or a retired id.
        let emitters = [
            include_str!("run_guard.rs"),
            include_str!("shortcuts.rs"),
            include_str!("disk_guard.rs"),
            include_str!("claude_accounts.rs"),
            include_str!("heartbeat.rs"),
            include_str!("notifications.rs"),
        ];
        let mut links = BTreeSet::new();
        for src in emitters {
            for part in src.split("\"settings:").skip(1) {
                let id: String = part
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if !id.is_empty() {
                    links.insert(id);
                }
            }
        }
        assert!(links.len() >= 4, "found {links:?}");
        for id in &links {
            assert!(
                has_keys(resolve_section(id)),
                "settings:{id} reaches no section with keys"
            );
        }
    }

    #[test]
    fn an_old_id_returns_the_keys_it_returned_before_pr_3() {
        let keys = |id: &str| {
            let mut v: Vec<&str> = legacy_keys(legacy_section(id).unwrap()).to_vec();
            v.sort_unstable();
            v
        };
        let sorted = |mut v: Vec<&'static str>| {
            v.sort_unstable();
            v
        };
        assert_eq!(
            keys("interface"),
            sorted(vec![
                "theme",
                "notifyInApp",
                "notifyNative",
                "nudgesEnabled",
                "sidebarCollapsed",
                "chatPanelOpen",
                "sidePaneWidth",
                "chatContextEnabled",
                "railCollapsed",
                "notifFilter",
                "paletteRecents",
                "shortcuts",
            ])
        );
        assert_eq!(
            keys("defaults"),
            sorted(vec![
                "agents.defaultAgent",
                "agents.model.claude",
                "agents.model.codex",
                "agents.model.antigravity",
                "agents.model.grok",
                "agents.effort.claude",
                "agents.effort.codex",
                "agents.effort.antigravity",
                "agents.effort.grok",
                "agents.updates.claude",
                "agents.updates.codex",
                "agents.updates.antigravity",
                "agents.updates.grok",
                "agents.codexNoOutputTimeoutMin",
                "agents.runsOn",
                "agents.unattendedRunsOn",
                "agents.buildsOn",
                "agents.appBuildsOn",
                "agents.whenOut",
                "chat.canvases",
                "accounts.claudeCeiling",
                "accounts.claudeFailoverAcrossOrganizations",
            ])
        );
        assert_eq!(
            keys("operator"),
            sorted(vec![
                "operator.contextEnabled",
                "profile.autoLearn",
                "learning.suggestions",
                "jev.enabled",
                "jev.decisions",
                "jev.apiKey",
                "semanticSearchEnabled",
            ])
        );
        assert_eq!(
            keys("automation"),
            sorted(vec![
                "app.runOnStartup",
                "app.minimizeToTray",
                "heartbeat.enabled",
                "heartbeat.drafts",
                "heartbeat.config",
            ])
        );
        assert_eq!(keys("system"), vec!["backupManagedElsewhere"]);
        // Retired ids read their old section: `learning` was Knowledge's,
        // `paths` System's, `feedback` Usage's, `selfhosted` Agents'.
        assert_eq!(keys("learning"), keys("operator"));
        assert_eq!(keys("paths"), keys("system"));
        assert_eq!(keys("feedback"), vec!["claudeQuotaOptIn"]);
        assert_eq!(keys("selfhosted"), keys("defaults"));
        // Live sections, old or new, read their section now.
        for live in [
            "general",
            "models",
            "connectors",
            "usage",
            "accounts",
            "chat",
            "shortcuts",
            "sources",
        ] {
            assert_eq!(legacy_section(live), None, "{live}");
        }
        // Every key had exactly one old section, and every old alias names one.
        let mut seen = BTreeSet::new();
        for (_, keys) in LEGACY_SECTIONS {
            for key in *keys {
                assert!(seen.insert(*key), "{key} twice");
                assert!(find(key).is_some(), "{key} is gone");
            }
        }
        for key in NEWER_KEYS {
            assert!(!seen.contains(key), "{key} is newer than the freeze");
            assert!(find(key).is_some(), "{key} is gone");
        }
        assert_eq!(seen.len() + NEWER_KEYS.len(), KEYS.len());
        assert_eq!(LEGACY_ALIASES.len(), 30);
        for (old, section) in LEGACY_ALIASES {
            assert!(
                !legacy_keys(section).is_empty() || *section == "usage",
                "{old}"
            );
        }
    }

    /// R5 (overnight 11-13): `accounts`, `chat` and `shortcuts` were retired
    /// ids before Settings PR 3 (reading Agents' 23 keys, Agents' again and
    /// Interface's 12) and are live sections now, so `settings.get` returns
    /// their own smaller sets. Kept on purpose: nothing in the repo (agent
    /// instructions, prompts, docs, mocks) asks those ids for an older key,
    /// and every old key is still read by name or by its retired section
    /// (`defaults`, `interface`). Documented in docs/reference/surfaces.md.
    #[test]
    fn live_accounts_chat_and_shortcuts_return_their_own_keys() {
        let live = |id: &str| {
            assert_eq!(legacy_section(id), None, "{id} is live");
            let mut v: Vec<&str> = KEYS
                .iter()
                .filter(|k| k.section == resolve_section(id))
                .map(|k| k.key)
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(
            live("accounts"),
            vec![
                "agents.updates.antigravity",
                "agents.updates.claude",
                "agents.updates.codex",
                "agents.updates.grok",
                "jev.apiKey",
            ]
        );
        assert_eq!(live("chat"), vec!["chat.canvases"]);
        assert_eq!(live("shortcuts"), vec!["shortcuts"]);
    }

    #[test]
    fn values_are_checked_before_anything_runs() {
        let theme = find("theme").unwrap();
        assert!(validate(theme, &json!("dark")).is_ok());
        assert!(validate(theme, &json!(3)).is_err());
        let agent = find("agents.defaultAgent").unwrap();
        assert!(validate(agent, &json!("codex")).is_ok());
        assert!(validate(agent, &json!("gpt"))
            .unwrap_err()
            .contains("one of claude"));
        let width = find("sidePaneWidth").unwrap();
        assert!(validate(width, &json!(480)).is_ok() && validate(width, &Value::Null).is_ok());
        assert!(validate(width, &json!(10)).is_err());
        let app = find("agentTools.app").unwrap();
        assert!(validate(app, &json!(false)).is_ok());
        assert!(validate(app, &json!(true))
            .unwrap_err()
            .contains("only turn Hollow app tools off"));
        let key = find("jev.apiKey").unwrap();
        assert_eq!(shown(key, &json!("ts-live-123")), "••••");
        assert!(
            !describe(key).to_string().contains("ts-live"),
            "a description never carries a value"
        );
    }
}
