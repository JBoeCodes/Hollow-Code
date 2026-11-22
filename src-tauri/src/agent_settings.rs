//! App-wide execution preferences, independent of the connected vault.
use crate::agent::{self, AgentKind};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default)]
    pub default_agent: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, String>,
    /// Paperclip PR 14a: reasoning effort per agent (any agent since
    /// 2026-10-02), one of `effort::LEVELS`. Absent means the runtime default.
    /// Omitted from the wire when empty, so a settings payload without it
    /// (older files, older frontends) reads and writes unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub efforts: BTreeMap<String, String>,
    /// Paperclip PR 15b: the Codex no-output watchdog, in minutes. ABSENT
    /// means the default ([`CODEX_NO_OUTPUT_DEFAULT_MIN`]); an explicit
    /// `null` switches it off; 0 reads as the default and values are clamped
    /// to [`CODEX_NO_OUTPUT_MAX_MIN`] ([`normalize`]). The default itself is
    /// omitted from the wire, so older files and payloads read and write
    /// unchanged, while `null` is written and kept.
    #[serde(
        default = "default_codex_no_output_min",
        skip_serializing_if = "is_default_codex_no_output_min"
    )]
    pub codex_no_output_timeout_min: Option<u32>,
    /// Merged plan slice 3 (A·R2): the no-output watchdog of every OTHER
    /// agent (`claude`, `grok`, `antigravity`), minutes per agent id, with
    /// Codex's semantics: an ABSENT entry is the default
    /// ([`NO_OUTPUT_DEFAULT_MIN`]), `null` switches it off, 0 reads as the
    /// default and values clamp to [`NO_OUTPUT_MAX_MIN`]. Codex keeps
    /// `codex_no_output_timeout_min`; a `codex` entry here is moved into it
    /// when saved ([`normalize`]). Default entries are dropped, so an empty
    /// map stays off the wire.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub no_output_timeout_min: BTreeMap<String, Option<u32>>,
    /// Merged plan slice 3 (A·R2): an optional deadline per run, minutes
    /// per agent id (every agent, Codex included). Absent or 0 means none;
    /// values clamp to [`DEADLINE_MAX_MIN`]. A chat turn ends at it; a
    /// workflow run ends at the earlier of it and its fixed hour.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub deadline_min: BTreeMap<String, u32>,
    /// Canvas C3 wave 2 C4: Settings, Chat, Canvases, `off` or `most`
    /// (`visual_prompt::CanvasFrequency`). ABSENT is the default, When it
    /// helps: `helps` is dropped when saved ([`normalize`]), so older files
    /// and payloads read and write unchanged. A chat's own choice in the
    /// model menu wins over it for that chat's turns. An unknown value in a
    /// hand-edited file (or a newer build's, after a rollback) reads as the
    /// default, and saving one stores the default (A16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, as = "Option<crate::ts_bindings::CanvasFrequency>"))]
    pub canvases: Option<String>,
    /// OB22: the context window Hollow's agents auto-compact against, in
    /// tokens, one of [`COMPACT_WINDOWS`]. ABSENT is off, the default
    /// (J07): each CLI keeps its own threshold. When set, Claude runs get
    /// `CLAUDE_CODE_AUTO_COMPACT_WINDOW` and Codex runs `-c
    /// model_auto_compact_token_limit` ([`compaction_env_with`]). Any other
    /// value saves as off ([`normalize`]; [`set_settings`] logs it), and a
    /// hand-edited one is ignored when a run reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub compact_window: Option<u32>,
    /// Repo ideas 2026-10-27 #8: Settings, Models, When an
    /// agent runs out. ABSENT is Wait, the default: a turn whose agent ran
    /// out of usage says when it has room again, and nothing else runs. An
    /// agent id continues such a turn on that agent, only after a turn that
    /// changed nothing (`agent_out::continuation`). `wait`, or an id this
    /// build doesn't know (a newer build's, after a rollback), saves as the
    /// default ([`normalize`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub when_out: Option<String>,
    /// The Mac mini runner (2026-11-01, Phase 2; vault workspaces Phase 3,
    /// 2026-11-02): Settings, Devices, Runs on, where a build
    /// workspace's Claude and Codex turns run when it has no choice of its own
    /// (`build_workspaces`' `runs_on`). A vault workspace moves only on its
    /// own choice (`remote_folder::runs_on_for`), never on this. ABSENT is `this-mac`, the default; `runner` (or the old `mac-mini`, M6) runs them on
    /// the remote runner `hollow-slot --remote` uses
    /// (`crate::remote_session`). Anything else saves as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub runs_on: Option<String>,
    /// Overnight N6 (2026-11-02): Settings, Devices, Overnight
    /// and worker chats, where unattended work (a night-queue item, a turn
    /// an unattended run started) and a Main agent worker batch's chats run
    /// when their build workspace has no Runs on of its own. `runner` (or
    /// the old `mac-mini`): the runner when it can take the turn, else this
    /// Mac with a notice (`remote_session::unattended_runner`). ABSENT, the
    /// default since M7 (PER-21, 2026-11-15; it was the runner before), and
    /// `same` follow Runs on above, so naming a runner moves nothing.
    /// Setups from before M7 keep the runner through
    /// [`freeze_runner_defaults`]. Anything else saves as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub unattended_runs_on: Option<String>,
    /// Smart routing (2026-11-03): Settings, Devices, Builds and tests,
    /// where `hollow-slot` runs a turn's build and test commands when a
    /// runner is set up, the turn's `HOLLOW_SLOT_ROUTE`
    /// ([`crate::build_slots::route_env`]). ABSENT is this Mac, the default
    /// since M7 (PER-21, 2026-11-15; it was `auto` before, and setups from
    /// before M7 keep `auto` through [`freeze_runner_defaults`]). `auto`:
    /// each command decides (`build_slots`' routing); `runner` (or the old
    /// `mac-mini`): everything that can go, a command that needs this Mac
    /// still stays here; `this-mac`. Anything else saves as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub builds_on: Option<String>,
    /// Smart routing 7 (2026-11-03): Settings, Advanced, App builds, where
    /// Build app compiles Hollow. ABSENT is this Mac, the default;
    /// `runner` (or the old `mac-mini`) compiles on the remote runner and signs here
    /// (`self_install::RemoteAppBuild`), falling back to this Mac on any
    /// failure. Anything else saves as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub app_builds_on: Option<String>,
    /// M7 (PER-21, 2026-11-15): the one-time freeze of the runner defaults
    /// ran ([`freeze_runner_defaults`]). Never cleared: every save keeps it
    /// ([`set_settings`]), so the freeze never runs twice.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub runner_defaults_frozen: bool,
    /// M5 PR2 (2026-11-15): When this Mac sleeps, move running chats to the
    /// paired runner (`chat_move_trigger`). ABSENT is off (M5 PR2 review:
    /// the runner may bill another account, so moving the person's chats
    /// while they sleep is their own choice); only `true` turns it on, and
    /// then only while the runner (the paired Mac `slot-remote` names) is
    /// paired and answers. Off, running chats stop with the sleep and resume
    /// after wake, as before. A save that omits it keeps the saved choice
    /// ([`carry_over`]): Settings' control is the UI PR's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub move_on_sleep: Option<bool>,
    /// M3a PR2 (2026-11-15): Settings, Devices, COO on this Mac. ABSENT is
    /// `auto`, the default: this Mac's COO is the dispatcher while a paired
    /// Mac runs a runner COO (`dispatcher_coo::active`). `off`: it stays
    /// the single, full COO even then, its prompt byte for byte the single
    /// COO's. Anything else saves as the default ([`coo_dispatcher_choice`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub coo_dispatcher: Option<String>,
    /// OC1 (2026-11-18): per agent, run its chat turns over the Agent Client
    /// Protocol (`crate::acp_client`) instead of the headless one-shot
    /// command. ABSENT (and `false`) is off, the default. Only `grok` is
    /// kept ([`normalize`]): Antigravity's ACP is unprobed
    /// (`acp_client::selectable`). On, a turn still runs headless in any
    /// mode ACP can't map exactly (`agent::grok_acp_mode`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub acp: BTreeMap<String, bool>,
}

/// The remote runner as a saved choice (M6, PER-20, 2026-11-15): `runner`,
/// since other people's runners aren't Mac minis. Files, clients and tools
/// from before it say [`LEGACY_RUNNER`], which still means exactly the same
/// everywhere a choice is read or accepted ([`is_runner`]). Reading never
/// rewrites a file: a choice is written, as `runner`, only by a save.
pub const RUNNER: &str = "runner";

/// The runner's old name (2026-11-01 to 2026-11-15), still accepted and
/// still what an installed `hollow-slot` and the phone wire (v1.1) know.
pub const LEGACY_RUNNER: &str = "mac-mini";

/// Whether a saved or asked value names the remote runner (`runner`, or the
/// old `mac-mini`), whitespace ignored.
pub fn is_runner(value: &str) -> bool {
    matches!(value.trim(), RUNNER | LEGACY_RUNNER)
}

/// A saved Runs on value as Hollow uses it: the runner (either name) as
/// [`RUNNER`], anything else trimmed and unchanged (so an unknown value
/// still counts as a choice of its own, as before).
pub(crate) fn canonical_place(value: &str) -> String {
    if is_runner(value) {
        RUNNER.to_string()
    } else {
        value.trim().to_string()
    }
}

/// An asked Runs on value (`this-mac`, `runner` or the old `mac-mini`) as it
/// is stored, or `None` when it is neither.
pub(crate) fn place_choice(value: &str) -> Option<&'static str> {
    match value.trim() {
        "this-mac" => Some("this-mac"),
        v if is_runner(v) => Some(RUNNER),
        _ => None,
    }
}

/// Where a turn can run: this Mac, or the remote runner (`mac-mini`, its
/// old name, last: accepted, never written).
pub const RUNS_ON: [&str; 3] = ["this-mac", RUNNER, LEGACY_RUNNER];

/// Overnight N6: where unattended and worker chats run (the same as Runs
/// on, the default since M7; or the runner when it answers).
pub const UNATTENDED_RUNS_ON: [&str; 3] = ["same", RUNNER, LEGACY_RUNNER];

/// A saved Overnight and worker chats choice: `Some("runner")` (saved as
/// `runner` or `mac-mini`), `Some("same")`, or `None` for the default, which
/// follows Runs on like `same` (M7; nothing, or anything else).
pub(crate) fn unattended_runs_on_choice(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        Some(v) if is_runner(v) => Some(RUNNER.to_string()),
        Some("same") => Some("same".to_string()),
        _ => None,
    }
}

/// M3a PR2: the COO on this Mac, a dispatcher when a runner COO is paired
/// (`auto`, the default) or always the full COO (`off`).
pub const COO_DISPATCHER: [&str; 2] = ["auto", "off"];

/// A saved COO on this Mac choice: `Some("off")`, or `None` for `auto`, the
/// default (nothing, `auto`, or anything else).
pub(crate) fn coo_dispatcher_choice(value: Option<&str>) -> Option<String> {
    (value.map(str::trim) == Some("off")).then(|| "off".to_string())
}

/// Whether the person turned the dispatcher off (Settings, Devices, COO on
/// this Mac: Always the full COO). An unreadable file reads as `auto`.
/// Tests set their own (`dispatcher_coo::tests::SETTING_OFF`).
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn coo_dispatcher_off() -> bool {
    get_settings()
        .ok()
        .and_then(|s| coo_dispatcher_choice(s.coo_dispatcher.as_deref()))
        .is_some()
}

/// Smart routing: where a turn's build and test commands run (this Mac, the
/// default since M7; Auto decides for each command).
pub const BUILDS_ON: [&str; 4] = ["this-mac", "auto", RUNNER, LEGACY_RUNNER];

/// Smart routing 7: where Build app compiles Hollow (this Mac, the
/// default, or the remote runner).
pub const APP_BUILDS_ON: [&str; 3] = ["this-mac", RUNNER, LEGACY_RUNNER];

/// A saved App builds choice: `Some("runner")` (saved as `runner` or
/// `mac-mini`), or `None` for this Mac (the default, `this-mac`, nothing,
/// or anything else).
pub(crate) fn app_builds_on_choice(value: Option<&str>) -> Option<String> {
    value.is_some_and(is_runner).then(|| RUNNER.to_string())
}

/// A saved Builds and tests choice: `Some("runner")` (saved as `runner` or
/// `mac-mini`), `Some("auto")` or `Some("this-mac")`, or `None` for this
/// Mac, the default since M7 (nothing, or anything else). `auto` is kept
/// when saved: before M7 it was the default and saved as absent.
pub(crate) fn builds_on_choice(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        Some(v) if is_runner(v) => Some(RUNNER.to_string()),
        Some(v @ ("this-mac" | "auto")) => Some(v.to_string()),
        _ => None,
    }
}

/// A saved Runs on choice: `Some("runner")` (saved as `runner` or
/// `mac-mini`), or `None` for this Mac (the default, `this-mac`, nothing,
/// or anything else).
pub(crate) fn runs_on_choice(value: Option<&str>) -> Option<String> {
    value.is_some_and(is_runner).then(|| RUNNER.to_string())
}

/// Minutes without output or process activity before a run is stopped
/// (the contract's default), for every agent since merged plan slice 3.
pub const NO_OUTPUT_DEFAULT_MIN: u32 = 30;
/// The largest accepted watchdog setting (one day).
pub const NO_OUTPUT_MAX_MIN: u32 = 1440;
/// Codex's names for the same bounds (Paperclip PR 15b).
pub const CODEX_NO_OUTPUT_DEFAULT_MIN: u32 = NO_OUTPUT_DEFAULT_MIN;
pub const CODEX_NO_OUTPUT_MAX_MIN: u32 = NO_OUTPUT_MAX_MIN;
/// The largest accepted per-run deadline (one day).
pub const DEADLINE_MAX_MIN: u32 = 1440;

fn default_codex_no_output_min() -> Option<u32> {
    Some(CODEX_NO_OUTPUT_DEFAULT_MIN)
}

fn is_default_codex_no_output_min(value: &Option<u32>) -> bool {
    *value == Some(CODEX_NO_OUTPUT_DEFAULT_MIN)
}

/// Written by hand: a derived `Default` would switch the watchdog off.
impl Default for AgentSettings {
    fn default() -> Self {
        AgentSettings {
            default_agent: None,
            models: BTreeMap::new(),
            efforts: BTreeMap::new(),
            codex_no_output_timeout_min: default_codex_no_output_min(),
            no_output_timeout_min: BTreeMap::new(),
            deadline_min: BTreeMap::new(),
            canvases: None,
            compact_window: None,
            when_out: None,
            runs_on: None,
            unattended_runs_on: None,
            builds_on: None,
            app_builds_on: None,
            runner_defaults_frozen: false,
            move_on_sleep: None,
            coo_dispatcher: None,
            acp: BTreeMap::new(),
        }
    }
}

/// The saved When an agent runs out choice (repo ideas 10-27 #8): an agent
/// id, or `None` for Wait (the default, `wait`, nothing, or an id this build
/// doesn't know).
pub(crate) fn when_out_choice(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .and_then(AgentKind::from_opt)
        .map(|kind| kind.as_str().to_string())
}

/// The agent a turn that ran out continues on, from the saved settings;
/// `None` is Wait.
pub(crate) fn when_out() -> Option<AgentKind> {
    let settings = get_settings().ok()?;
    AgentKind::from_opt(&when_out_choice(settings.when_out.as_deref())?)
}

/// OB22: the compaction windows Hollow offers, in tokens (300K, 400K,
/// 600K). Anything else saves as off.
pub const COMPACT_WINDOWS: [u32; 3] = [300_000, 400_000, 600_000];

/// The saved Canvases choice (Canvas C3 wave 2 C4): `off` or `most`;
/// `None` for the default (`helps`), nothing, or a choice this build
/// doesn't know (overnight A16).
fn canvases_choice(value: Option<&str>) -> Option<String> {
    use crate::visual_prompt::CanvasFrequency;
    match CanvasFrequency::parse(value?.trim())? {
        CanvasFrequency::Helps => None,
        choice => Some(choice.as_str().to_string()),
    }
}

/// OB22: the saved compaction window, if it is one Hollow offers. A
/// hand-edited other value reads as off, so the CLI keeps its own threshold.
pub(crate) fn compact_window(settings: &AgentSettings) -> Option<u32> {
    settings
        .compact_window
        .filter(|window| COMPACT_WINDOWS.contains(window))
}

/// OB22: the run environment that carries the compaction window to `kind`:
/// Claude's own `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, or for Codex Hollow's
/// carrier [`agent::CODEX_COMPACT_LIMIT_ENV`], which `build_codex` turns
/// into `-c model_auto_compact_token_limit=<n>` and keeps off the child's
/// environment. Launch points chain it onto `RunExtras::env`. Empty when the
/// window is off, and for Antigravity and Grok, which have no such setting.
pub(crate) fn compaction_env_with(
    kind: AgentKind,
    settings: &AgentSettings,
) -> Vec<(String, String)> {
    let Some(window) = compact_window(settings) else {
        return Vec::new();
    };
    let key = match kind {
        AgentKind::Claude => agent::CLAUDE_COMPACT_WINDOW_ENV,
        AgentKind::Codex => agent::CODEX_COMPACT_LIMIT_ENV,
        AgentKind::Antigravity | AgentKind::Grok => return Vec::new(),
    };
    vec![(key.to_string(), window.to_string())]
}

/// [`compaction_env_with`] over the saved settings. An unreadable settings
/// file reads as off (logged): a run never fails over its compaction window.
pub(crate) fn compaction_env(kind: AgentKind) -> Vec<(String, String)> {
    match get_settings() {
        Ok(settings) => compaction_env_with(kind, &settings),
        Err(error) => {
            eprintln!("[agent-settings] {error}; no compaction window for this run");
            Vec::new()
        }
    }
}

/// The watchdog setting as stored: 0 → the default, clamped to the maximum,
/// `None` (off) kept.
fn normalize_codex_no_output(value: Option<u32>) -> Option<u32> {
    value.map(|minutes| match minutes {
        0 => CODEX_NO_OUTPUT_DEFAULT_MIN,
        m => m.min(CODEX_NO_OUTPUT_MAX_MIN),
    })
}

/// A deadline setting as stored: 0 (none) → `None`, clamped to the maximum.
fn normalize_deadline(minutes: u32) -> Option<u32> {
    if minutes == 0 {
        None
    } else {
        Some(minutes.min(DEADLINE_MAX_MIN))
    }
}

fn minutes(value: u32) -> std::time::Duration {
    std::time::Duration::from_secs(u64::from(value) * 60)
}

/// The watchdog and deadline a run of one agent gets (merged plan slice 3,
/// A·R2): the ONE resolution every launch point uses, the same for Claude,
/// Codex, Grok and Antigravity. Chat and workflows pass `stall_after` as
/// `process_group::Watch::stall_after` and `deadline` as
/// `Supervision::deadline` (a workflow keeps the earlier of it and its own
/// hour).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RunLimits {
    pub deadline: Option<std::time::Duration>,
    pub stall_after: Option<std::time::Duration>,
}

/// How long `kind` may run with neither output nor process activity before
/// the watchdog ends it, `None` when switched off (Paperclip PR 15b; every
/// agent since merged plan slice 3). Codex reads its own field, the others
/// their [`AgentSettings::no_output_timeout_min`] entry, with the same
/// semantics.
pub(crate) fn stall_limit_for(
    kind: AgentKind,
    settings: &AgentSettings,
) -> Option<std::time::Duration> {
    let stored = match kind {
        AgentKind::Codex => settings.codex_no_output_timeout_min,
        other => settings
            .no_output_timeout_min
            .get(other.as_str())
            .copied()
            .unwrap_or_else(default_codex_no_output_min),
    };
    normalize_codex_no_output(stored).map(minutes)
}

/// `kind`'s optional per-run deadline (merged plan slice 3), `None` when
/// unset.
pub(crate) fn deadline_for(
    kind: AgentKind,
    settings: &AgentSettings,
) -> Option<std::time::Duration> {
    settings
        .deadline_min
        .get(kind.as_str())
        .copied()
        .and_then(normalize_deadline)
        .map(minutes)
}

/// [`RunLimits`] for `kind` from `settings`.
pub(crate) fn limits_for(kind: AgentKind, settings: &AgentSettings) -> RunLimits {
    RunLimits {
        deadline: deadline_for(kind, settings),
        stall_after: stall_limit_for(kind, settings),
    }
}

/// [`limits_for`] over the saved settings. An unreadable settings file
/// falls back to the defaults (the watchdog on, no deadline) rather than
/// switching the watchdog off.
pub(crate) fn run_limits(kind: AgentKind) -> RunLimits {
    let settings = get_settings().unwrap_or_else(|error| {
        eprintln!("[agent-settings] {error}; the run limits use their defaults");
        AgentSettings::default()
    });
    limits_for(kind, &settings)
}

static SETTINGS_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Hollow's app-data folder for vault-independent preferences
/// (`~/Library/Application Support/com.hollow.app` on macOS): the home of
/// `agent-settings.json` and `budgets.json`. Test builds get a per-process
/// temp folder, so unit tests never read or change the user's live files.
pub(crate) fn app_data_dir() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        Ok(crate::test_temp::process_dir(
            "hollow-agent-settings-tests-",
        ))
    }
    #[cfg(not(test))]
    {
        // A native review build can keep preferences, provider settings and
        // the service registry separate from the installed app. Vault stores
        // are independently keyed by the HOLLOW_CONFIG vault path.
        if let Some(path) = std::env::var_os("HOLLOW_APP_DATA_DIR") {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("HOLLOW_APP_DATA_DIR must be an absolute path".into());
            }
            return Ok(path);
        }
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .map(|root| root.join(crate::instance::current().app_data_name()))
            .ok_or_else(|| "Could not locate Hollow's app data directory.".into())
    }
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("agent-settings.json"))
}

/// Antigravity's default model saved as a level variant (`gemini-4-neon-high`,
/// from when the picker listed every variant) becomes its base model, and
/// its level becomes the agent's default effort unless one is already saved:
/// the model picker no longer sets effort a second time (2026-11-20). Run on
/// every read and save, so an old file migrates without a rewrite.
pub(crate) fn migrate_antigravity_model(settings: &mut AgentSettings) {
    let keys: Vec<String> = settings
        .models
        .keys()
        .filter(|name| AgentKind::from_opt(name) == Some(AgentKind::Antigravity))
        .cloned()
        .collect();
    for key in keys {
        let model = settings.models.get(&key).cloned().unwrap_or_default();
        let (base, level) = crate::effort::split_agy(model.trim());
        let Some(level) = level else {
            continue;
        };
        let level = crate::effort::canonical(level).filter(|l| *l != crate::effort::ULTRACODE);
        let base = base.to_string();
        settings.models.insert(key, base);
        let saved = settings
            .efforts
            .keys()
            .any(|name| AgentKind::from_opt(name) == Some(AgentKind::Antigravity));
        if let (Some(level), false) = (level, saved) {
            settings.efforts.insert(
                AgentKind::Antigravity.as_str().to_string(),
                level.to_string(),
            );
        }
    }
}

fn read_at(path: &Path) -> Result<AgentSettings, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(|mut settings: AgentSettings| {
                migrate_antigravity_model(&mut settings);
                settings
            })
            .map_err(|e| format!("Read agent settings: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgentSettings::default()),
        Err(e) => Err(format!("Read agent settings: {e}")),
    }
}

pub fn get_settings() -> Result<AgentSettings, String> {
    let _guard = SETTINGS_LOCK
        .lock()
        .map_err(|_| "Agent settings lock unavailable")?;
    read_at(&settings_path()?)
}

/// The settings as `set_settings` would save them, or why it refuses them.
/// Pure (no file, no log), so a confirm card can preview the save with it.
pub(crate) fn normalize(mut settings: AgentSettings) -> Result<AgentSettings, String> {
    settings.default_agent = settings
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            AgentKind::from_opt(s)
                .map(|kind| kind.as_str().to_string())
                .ok_or_else(|| format!("Unknown agent '{s}'."))
        })
        .transpose()?;
    migrate_antigravity_model(&mut settings);
    let mut models = BTreeMap::new();
    for (name, model) in settings.models {
        let kind = AgentKind::from_opt(&name).ok_or_else(|| format!("Unknown agent '{name}'."))?;
        // Self-hosted models Q5 review: a default model is used by runs
        // that aren't chats too, which can't run a self-hosted model. One
        // already saved is dropped, never a refusal of the whole save (the
        // composer's Effort row re-saves the whole object);
        // `set_settings` refuses a newly asked one and logs the drop.
        if crate::self_hosted::entry_id(&model).is_some() {
            continue;
        }
        if !model.trim().is_empty() {
            models.insert(kind.as_str().to_string(), model.trim().to_string());
        }
    }
    settings.models = models;
    // Efforts are validated when saved: every agent takes every Hollow
    // level (`effort::LEVELS`; Codex's old `minimal` is stored as `low`),
    // and a run maps it to what its model takes. Only an unknown level is
    // refused.
    let mut efforts = BTreeMap::new();
    for (name, effort) in settings.efforts {
        let kind = AgentKind::from_opt(&name).ok_or_else(|| format!("Unknown agent '{name}'."))?;
        let effort = effort.trim();
        if effort.is_empty() {
            continue;
        }
        let level = crate::effort::canonical(effort)
            .filter(|level| *level != crate::effort::ULTRACODE)
            .ok_or_else(|| {
                if effort == crate::effort::ULTRACODE {
                    return "Ultracode is chosen per chat, beside the permission mode; it can't be an agent's default.".to_string();
                }
                format!("'{effort}' is not an effort level (choose {}).", agent::effort_levels(kind).join(", "))
            })?;
        efforts.insert(kind.as_str().to_string(), level.to_string());
    }
    settings.efforts = efforts;
    // The watchdog of every other agent (merged plan slice 3): keys
    // canonicalized, a `codex` entry moved into Codex's own field, values
    // normalized like Codex's, default entries dropped (absent IS the
    // default), `null` (off) kept.
    let mut no_output = BTreeMap::new();
    for (name, value) in std::mem::take(&mut settings.no_output_timeout_min) {
        let kind = AgentKind::from_opt(&name).ok_or_else(|| format!("Unknown agent '{name}'."))?;
        if kind == AgentKind::Codex {
            settings.codex_no_output_timeout_min = value;
            continue;
        }
        let value = normalize_codex_no_output(value);
        if value != default_codex_no_output_min() {
            no_output.insert(kind.as_str().to_string(), value);
        }
    }
    settings.no_output_timeout_min = no_output;
    settings.codex_no_output_timeout_min =
        normalize_codex_no_output(settings.codex_no_output_timeout_min);
    let mut deadlines = BTreeMap::new();
    for (name, value) in std::mem::take(&mut settings.deadline_min) {
        let kind = AgentKind::from_opt(&name).ok_or_else(|| format!("Unknown agent '{name}'."))?;
        if let Some(value) = normalize_deadline(value) {
            deadlines.insert(kind.as_str().to_string(), value);
        }
    }
    settings.deadline_min = deadlines;
    // Canvases (Canvas C3 wave 2 C4): a known choice, the default dropped.
    // An unknown one (a newer build's choice read back after a rollback)
    // saves as the default rather than failing the save (overnight A16), as
    // the compaction window does: every save path sends the whole object.
    settings.canvases = canvases_choice(settings.canvases.as_deref());
    // OB22: the compaction window, off or one of the offered sizes. Another
    // size saves as off rather than failing the save (OB22 review M1): every
    // save path (Settings, `set_agent_settings`, `change_setting`'s
    // read-modify-write) sends the whole object, so a hand-edited size read
    // back from the file can't be told from an explicit one, and refusing it
    // would block every other agent setting until the file was fixed.
    // Pure: `set_settings` logs the drop, and `set_agent_settings`' card
    // previews with this (OB22 delta review) without logging a save.
    settings.compact_window = compact_window(&settings);
    // When an agent runs out (repo ideas 10-27 #8): an agent id, else Wait
    // (absent). Like Canvases, an unknown value saves as the default rather
    // than failing every other agent setting's save.
    settings.when_out = when_out_choice(settings.when_out.as_deref());
    settings.runs_on = runs_on_choice(settings.runs_on.as_deref());
    settings.unattended_runs_on = unattended_runs_on_choice(settings.unattended_runs_on.as_deref());
    settings.builds_on = builds_on_choice(settings.builds_on.as_deref());
    settings.app_builds_on = app_builds_on_choice(settings.app_builds_on.as_deref());
    settings.coo_dispatcher = coo_dispatcher_choice(settings.coo_dispatcher.as_deref());
    // OC1: the ACP switch, kept only where it is on for an agent whose
    // dialect the switch can select (absent is off).
    settings.acp = std::mem::take(&mut settings.acp)
        .into_iter()
        .filter_map(|(name, on)| AgentKind::from_opt(&name).filter(|_| on))
        .filter(|kind| crate::acp_client::selectable(*kind))
        .map(|kind| (kind.as_str().to_string(), true))
        .collect();
    Ok(settings)
}

/// Self-hosted models Q5 review: the first default model in `asked` that is
/// a self-hosted id and isn't what `saved` already holds for that agent (a
/// new choice, refused); `None` when every such id was already saved
/// (dropped by [`normalize`] with a log line, so the rest still saves).
pub(crate) fn newly_chat_only(
    asked: &BTreeMap<String, String>,
    saved: &BTreeMap<String, String>,
) -> Option<String> {
    asked
        .iter()
        .filter(|(_, model)| crate::self_hosted::entry_id(model).is_some())
        .find(|(name, model)| {
            let key = AgentKind::from_opt(name).map_or(name.as_str(), |kind| kind.as_str());
            saved.get(key).map(|m| m.trim()) != Some(model.trim())
        })
        .map(|(_, model)| model.trim().to_string())
}

pub fn set_settings(settings: AgentSettings) -> Result<AgentSettings, String> {
    let asked = settings.compact_window;
    let canvases = settings.canvases.clone();
    let asked_models = settings.models.clone();
    let settings = normalize(settings)?;
    if let (Some(window), None) = (asked, settings.compact_window) {
        eprintln!(
            "[agent-settings] {window} is not a compaction window (300000, 400000 or 600000); saved as off"
        );
    }
    if let Some(value) = canvases.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if crate::visual_prompt::CanvasFrequency::parse(value).is_none() {
            eprintln!("[agent-settings] '{value}' is not a Canvases choice (off, helps or most); saved as the default");
        }
    }
    let _guard = SETTINGS_LOCK
        .lock()
        .map_err(|_| "Agent settings lock unavailable")?;
    let path = settings_path()?;
    if asked_models
        .values()
        .any(|m| crate::self_hosted::entry_id(m).is_some())
    {
        let saved = read_at(&path).map(|s| s.models).unwrap_or_default();
        if let Some(model) = newly_chat_only(&asked_models, &saved) {
            return Err(crate::self_hosted::not_for_this(&model));
        }
        eprintln!("[agent-settings] a saved self-hosted default model was dropped: only a chat can use one");
    }
    let mut settings = settings;
    if let Ok(prior) = read_at(&path) {
        carry_over(&prior, &mut settings);
    }
    write_at(&path, &settings)?;
    Ok(settings)
}

/// M7 (PER-21, 2026-11-15): naming a runner turns nothing on, so absent
/// Overnight and worker chats and Builds and tests now mean off (this Mac).
/// Setups from before that keep what was in force: once, when the marker is
/// unset and a runner is set up (`runner_set_up`), each of the two that is
/// absent gets its old default (`runner`, `auto`); a saved value is never
/// touched. Without a runner nothing is written but the marker, so a runner
/// named later starts with nothing on. Pure; returns whether it changed
/// anything. Other fields, an old `mac-mini` included, are left as read.
pub(crate) fn freeze_runner_defaults(settings: &mut AgentSettings, runner_set_up: bool) -> bool {
    if settings.runner_defaults_frozen {
        return false;
    }
    if runner_set_up {
        if settings.unattended_runs_on.is_none() {
            settings.unattended_runs_on = Some(RUNNER.into());
        }
        if settings.builds_on.is_none() {
            settings.builds_on = Some("auto".into());
        }
    }
    settings.runner_defaults_frozen = true;
    true
}

/// [`freeze_runner_defaults`] on the saved file, at launch: one read, and
/// one atomic write only the first time. An unreadable file or a failed
/// write is logged and retried at the next launch; it never stops Hollow.
pub fn freeze_runner_defaults_at_launch() {
    let runner_set_up = crate::remote_session::configured_runner().is_some();
    let result = (|| -> Result<bool, String> {
        let _guard = SETTINGS_LOCK
            .lock()
            .map_err(|_| "Agent settings lock unavailable")?;
        freeze_at(&settings_path()?, runner_set_up)
    })();
    match result {
        Ok(true) => eprintln!("[agent-settings] runner defaults frozen (M7)"),
        Ok(false) => {}
        Err(e) => eprintln!("[agent-settings] runner defaults not frozen yet: {e}"),
    }
}

/// [`freeze_runner_defaults_at_launch`] on `path` (the caller holds the
/// settings lock). The file is read and written as [`AgentSettings`] without
/// [`normalize`], so every other value is written back as it was read.
fn freeze_at(path: &Path, runner_set_up: bool) -> Result<bool, String> {
    let mut settings = read_at(path)?;
    if !freeze_runner_defaults(&mut settings, runner_set_up) {
        return Ok(false);
    }
    write_at(path, &settings)?;
    Ok(true)
}

/// What a save keeps from the file it replaces: each old `mac-mini` that
/// still means the runner ([`keep_legacy_runner`]) and the M7 freeze marker,
/// which no save clears (the frontend's payload omits it), so the freeze
/// never runs again over the person's choices.
pub(crate) fn carry_over(prior: &AgentSettings, settings: &mut AgentSettings) {
    keep_legacy_runner(prior, settings);
    settings.runner_defaults_frozen |= prior.runner_defaults_frozen;
    // M5 PR2: Settings' payload has no `moveOnSleep` until its control
    // ships, so a save without it keeps the saved choice.
    if settings.move_on_sleep.is_none() {
        settings.move_on_sleep = prior.move_on_sleep;
    }
}

/// M6 review: a save keeps a field's old `mac-mini` while it still means the
/// runner, so saving an unrelated field (every save sends the whole object)
/// never rewrites it, and a Hollow rolled back to before M6 still reads it.
/// A field the person changes (this Mac, Auto, Follow Runs on, or the runner
/// set from absent) is written as normalized, the runner as `runner`.
/// Overnight and worker chats is kept the same way (since M7 its runner is
/// a saved `runner`, no longer the default).
pub(crate) fn keep_legacy_runner(prior: &AgentSettings, settings: &mut AgentSettings) {
    let legacy = |v: &Option<String>| v.as_deref().map(str::trim) == Some(LEGACY_RUNNER);
    for (before, now) in [
        (&prior.runs_on, &mut settings.runs_on),
        (&prior.unattended_runs_on, &mut settings.unattended_runs_on),
        (&prior.builds_on, &mut settings.builds_on),
        (&prior.app_builds_on, &mut settings.app_builds_on),
    ] {
        if legacy(before) && now.as_deref() == Some(RUNNER) {
            *now = before.clone();
        }
    }
}

fn write_at(path: &Path, settings: &AgentSettings) -> Result<(), String> {
    write_json_at(path, settings, "agent settings")
}

/// Atomically replace `path` with `value` as pretty JSON through
/// [`crate::vault::atomic_write`] (synced temp file, rename, directory sync).
/// `what` names the file in errors.
fn write_json_at(path: &Path, value: &impl Serialize, what: &str) -> Result<(), String> {
    if path.parent().is_none() {
        return Err(format!("Invalid {what} path"));
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("Encode {what}: {e}"))?;
    crate::vault::atomic_write(path, &bytes).map_err(|e| format!("Save {what}: {e}"))
}

// ── Agent tool switches (Paperclip PR 12) ────────────────────────────────
//
// `agent-tools.json` in [`app_data_dir`] holds the Settings "Task tools"
// and "Hollow app tools" switches (`{"tasksTools": bool, "appTools": bool}`,
// the second from PR A1). It is a separate file so the
// agent-settings IPC payload is unchanged, and it lives on the backend (not
// localStorage) because the run-queue pump can admit a scheduled workflow
// before the webview has booted. `task_broker::load_settings` loads it in
// setup, before the pump starts; `set_tasks_mcp_enabled` saves it.

/// File name inside [`app_data_dir`].
pub(crate) const AGENT_TOOLS_FILE: &str = "agent-tools.json";

/// The persisted agent tool switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTools {
    /// Attach the `hollow_tasks` MCP to chats and automations. Default on.
    #[serde(default = "default_on")]
    pub tasks_tools: bool,
    /// Attach the `hollow_app` MCP (PR A1) to chats and automations. Default
    /// on.
    #[serde(default = "default_on")]
    pub app_tools: bool,
}

fn default_on() -> bool {
    true
}

impl Default for AgentTools {
    fn default() -> Self {
        AgentTools {
            tasks_tools: true,
            app_tools: true,
        }
    }
}

fn agent_tools_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(AGENT_TOOLS_FILE))
}

/// The switches at `path`: defaults (everything on) when the file is
/// missing, an error when it is unreadable or malformed.
fn read_agent_tools_at(path: &Path) -> Result<AgentTools, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Read agent tools: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgentTools::default()),
        Err(e) => Err(format!("Read agent tools: {e}")),
    }
}

/// Whether the task tools are on. A missing file means on; an unreadable or
/// malformed one means OFF (logged), so a damaged file can never re-enable
/// tools the user switched off.
pub(crate) fn tasks_tools_enabled() -> bool {
    tasks_tools_enabled_at(agent_tools_path())
}

fn tasks_tools_enabled_at(path: Result<PathBuf, String>) -> bool {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match path.and_then(|path| read_agent_tools_at(&path)) {
        Ok(tools) => tools.tasks_tools,
        Err(error) => {
            eprintln!(
                "[agent-tools] {error}; task tools stay off until the file is fixed or saved again"
            );
            false
        }
    }
}

/// Whether the Hollow app tools are on, read like [`tasks_tools_enabled`]: a
/// missing file means on, a damaged one means off.
pub(crate) fn app_tools_enabled() -> bool {
    app_tools_enabled_at(agent_tools_path())
}

fn app_tools_enabled_at(path: Result<PathBuf, String>) -> bool {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match path.and_then(|path| read_agent_tools_at(&path)) {
        Ok(tools) => tools.app_tools,
        Err(error) => {
            eprintln!("[agent-tools] {error}; Hollow app tools stay off until the file is fixed or saved again");
            false
        }
    }
}

/// Persist the Hollow app tools switch; the task switch keeps its value.
pub(crate) fn set_app_tools_enabled(enabled: bool) -> Result<(), String> {
    set_app_tools_enabled_at(&agent_tools_path()?, enabled)
}

fn set_app_tools_enabled_at(path: &Path, enabled: bool) -> Result<(), String> {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut tools = read_agent_tools_at(path).unwrap_or_default();
    tools.app_tools = enabled;
    write_json_at(path, &tools, "agent tools")
}

/// Persist the task tools switch (atomic replace). Saving rewrites a damaged
/// file with a valid one.
pub(crate) fn set_tasks_tools_enabled(enabled: bool) -> Result<(), String> {
    set_tasks_tools_enabled_at(&agent_tools_path()?, enabled)
}

fn set_tasks_tools_enabled_at(path: &Path, enabled: bool) -> Result<(), String> {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Other switches keep their saved values; a damaged file becomes the
    // defaults plus this switch.
    let mut tools = read_agent_tools_at(path).unwrap_or_default();
    tools.tasks_tools = enabled;
    write_json_at(path, &tools, "agent tools")
}

/// OC1: whether Settings' ACP switch is on for `kind` (unreadable settings
/// read as off).
pub fn acp_enabled(kind: AgentKind) -> bool {
    get_settings().is_ok_and(|s| acp_enabled_with(&s, kind))
}

/// [`acp_enabled`] over given settings.
pub(crate) fn acp_enabled_with(settings: &AgentSettings, kind: AgentKind) -> bool {
    crate::acp_client::selectable(kind) && settings.acp.get(kind.as_str()) == Some(&true)
}

/// The saved reasoning effort for `kind` (Paperclip PR 14a), or `None` for
/// the runtime default. Reads the settings file, so it fails exactly the way
/// [`resolve`] does. The level is returned as stored, NOT re-validated:
/// `agent::validate_run_spec` refuses one that has become unsupported, so the
/// run fails visibly instead of dropping the setting.
pub fn effort(kind: AgentKind) -> Result<Option<String>, String> {
    Ok(effort_with(&get_settings()?, kind))
}

/// [`effort`] over given settings: the trimmed stored value, if non-empty.
/// A hand-edited `ultracode` is dropped: it is chosen per chat (Track U), so
/// it never reaches workflows, the heartbeat or teammates as a default.
pub(crate) fn effort_with(settings: &AgentSettings, kind: AgentKind) -> Option<String> {
    settings
        .efforts
        .get(kind.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != crate::effort::ULTRACODE)
        .map(str::to_string)
}

/// Explicit run selection, then app preference, then first installed adapter.
/// An unavailable or invalid selection is an error, never a provider switch.
pub fn resolve(
    explicit_agent: Option<&str>,
    explicit_model: Option<&str>,
) -> Result<(AgentKind, Option<String>), String> {
    resolve_with(
        &get_settings()?,
        explicit_agent,
        explicit_model,
        &agent::available(),
    )
}

fn resolve_with(
    settings: &AgentSettings,
    explicit_agent: Option<&str>,
    explicit_model: Option<&str>,
    available: &[AgentKind],
) -> Result<(AgentKind, Option<String>), String> {
    let name = explicit_agent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(settings.default_agent.as_deref());
    let kind = match name {
        Some(name) => AgentKind::from_opt(name).ok_or_else(|| format!("Unknown agent '{name}'. Choose an agent in Settings → Models."))?,
        None => *available.first().ok_or_else(|| "No supported agent is installed. Install and sign in to an agent, then select it in Settings → Models.".to_string())?,
    };
    if !available.contains(&kind) {
        return Err(format!("{} is selected but its CLI is unavailable. Install it or choose another agent in Settings → Models.", kind.label()));
    }
    // Some("") deliberately chooses the provider's CLI default; None inherits.
    let model = explicit_model
        .or_else(|| settings.models.get(kind.as_str()).map(String::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok((kind, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_overrides_are_provider_scoped() {
        let settings = AgentSettings {
            default_agent: Some("codex".into()),
            models: BTreeMap::from([
                ("codex".into(), "custom-model".into()),
                ("claude".into(), "sonnet".into()),
            ]),
            efforts: BTreeMap::new(),
            ..Default::default()
        };
        let all = AgentKind::all();
        assert_eq!(
            resolve_with(&settings, None, None, &all).unwrap(),
            (AgentKind::Codex, Some("custom-model".into()))
        );
        assert_eq!(
            resolve_with(&settings, Some("claude"), None, &all).unwrap(),
            (AgentKind::Claude, Some("sonnet".into()))
        );
        assert_eq!(
            resolve_with(&settings, None, Some(""), &all).unwrap(),
            (AgentKind::Codex, None)
        );
        // Explicit IDs are owned by the runtime/account, not a bundled model catalog.
        assert_eq!(
            resolve_with(&settings, None, Some("provider/any-model"), &all).unwrap(),
            (AgentKind::Codex, Some("provider/any-model".into()))
        );
    }
    #[test]
    fn unavailable_and_invalid_selections_never_fallback() {
        let settings = AgentSettings {
            default_agent: Some("codex".into()),
            ..Default::default()
        };
        assert!(resolve_with(&settings, None, None, &[AgentKind::Claude])
            .unwrap_err()
            .contains("Codex is selected"));
        assert!(
            resolve_with(&settings, Some("typo"), None, &AgentKind::all())
                .unwrap_err()
                .contains("Unknown agent")
        );
        assert!(resolve_with(&AgentSettings::default(), None, None, &[])
            .unwrap_err()
            .contains("No supported agent"));
        assert_eq!(
            resolve_with(&AgentSettings::default(), None, None, &[AgentKind::Codex]).unwrap(),
            (AgentKind::Codex, None)
        );
    }
    #[test]
    fn missing_file_defaults_but_corruption_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-settings.json");
        assert_eq!(read_at(&path).unwrap(), AgentSettings::default());
        std::fs::write(&path, "broken").unwrap();
        assert!(read_at(&path).is_err());
    }

    #[test]
    fn settings_round_trip_atomically_outside_a_vault() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app-data/agent-settings.json");
        let first = AgentSettings {
            default_agent: Some("codex".into()),
            models: BTreeMap::from([("codex".into(), "custom-model".into())]),
            efforts: BTreeMap::from([("codex".into(), "high".into())]),
            codex_no_output_timeout_min: Some(12),
            no_output_timeout_min: BTreeMap::from([("claude".into(), None)]),
            deadline_min: BTreeMap::from([("grok".into(), 45)]),
            canvases: Some("most".into()),
            compact_window: Some(400_000),
            when_out: Some("codex".into()),
            runs_on: Some("runner".into()),
            unattended_runs_on: Some("same".into()),
            builds_on: Some("this-mac".into()),
            app_builds_on: Some("runner".into()),
            runner_defaults_frozen: true,
            move_on_sleep: Some(false),
            coo_dispatcher: Some("off".into()),
            acp: BTreeMap::from([("grok".into(), true)]),
        };
        write_at(&path, &first).unwrap();
        assert_eq!(read_at(&path).unwrap(), first);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        let replacement = AgentSettings {
            default_agent: Some("antigravity".into()),
            ..first
        };
        write_at(&path, &replacement).unwrap();
        assert_eq!(read_at(&path).unwrap(), replacement);
    }
    /// M6 (PER-20): settings saved before the runner's neutral name say
    /// `mac-mini` in each field. They read as the runner (each field's
    /// choice, the turn's `HOLLOW_SLOT_ROUTE` in the name an older installed
    /// hollow-slot knows) and reading leaves the file byte for byte. M6
    /// review: a save of an unrelated field keeps each old `mac-mini`; only
    /// a field the person changes is written, the runner as `runner`.
    #[test]
    fn old_mac_mini_settings_read_as_the_runner_and_only_a_change_writes_runner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-settings.json");
        let old = r#"{"defaultAgent":"claude","models":{},"runsOn":"mac-mini","unattendedRunsOn":"mac-mini","buildsOn":"mac-mini","appBuildsOn":"mac-mini"}"#;
        std::fs::write(&path, old).unwrap();
        let read = read_at(&path).unwrap();
        assert_eq!(read_at(&path).unwrap(), read);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            old,
            "reading never rewrites it"
        );
        assert_eq!(
            runs_on_choice(read.runs_on.as_deref()).as_deref(),
            Some(RUNNER)
        );
        assert_eq!(
            builds_on_choice(read.builds_on.as_deref()).as_deref(),
            Some(RUNNER)
        );
        assert_eq!(
            app_builds_on_choice(read.app_builds_on.as_deref()).as_deref(),
            Some(RUNNER)
        );
        assert_eq!(
            unattended_runs_on_choice(read.unattended_runs_on.as_deref()).as_deref(),
            Some(RUNNER)
        );
        assert_eq!(
            crate::build_slots::route_env(read.builds_on.as_deref()).1,
            LEGACY_RUNNER,
            "an older installed hollow-slot reads only mac-mini"
        );
        assert_eq!(
            crate::remote_session::unattended_runner_for(
                AgentKind::Claude,
                Some(None),
                read.unattended_runs_on.as_deref(),
                Some("host".into())
            )
            .as_deref(),
            Some("host")
        );
        // Each field in the new name reads the same.
        for v in ["runner", " runner ", "mac-mini"] {
            assert!(is_runner(v), "{v}");
            assert_eq!(runs_on_choice(Some(v)).as_deref(), Some(RUNNER));
            assert_eq!(builds_on_choice(Some(v)).as_deref(), Some(RUNNER));
            assert_eq!(app_builds_on_choice(Some(v)).as_deref(), Some(RUNNER));
            assert_eq!(unattended_runs_on_choice(Some(v)).as_deref(), Some(RUNNER));
            assert_eq!(place_choice(v), Some(RUNNER));
            assert_eq!(canonical_place(v), RUNNER);
        }
        for v in ["this-mac", "auto", "same", "mac", ""] {
            assert!(!is_runner(v), "{v}");
        }
        assert_eq!(place_choice(" this-mac "), Some("this-mac"));
        assert_eq!(place_choice("moon"), None);
        assert_eq!(canonical_place(" moon "), "moon");
        assert_eq!(runs_on_choice(Some("this-mac")), None);
        assert_eq!(
            builds_on_choice(Some("this-mac")).as_deref(),
            Some("this-mac")
        );
        // Normalized, each runner field is `runner`.
        let normalized = normalize(read.clone()).unwrap();
        assert_eq!(normalized.runs_on.as_deref(), Some(RUNNER));
        assert_eq!(normalized.builds_on.as_deref(), Some(RUNNER));
        assert_eq!(normalized.app_builds_on.as_deref(), Some(RUNNER));
        assert_eq!(normalized.unattended_runs_on.as_deref(), Some(RUNNER));
        // M6 review: a save of an unrelated field (the whole object, as
        // every save sends it) leaves each old `mac-mini` as it was.
        let mut unrelated = normalize(AgentSettings {
            default_agent: Some("codex".into()),
            ..read.clone()
        })
        .unwrap();
        keep_legacy_runner(&read, &mut unrelated);
        assert_eq!(unrelated.default_agent.as_deref(), Some("codex"));
        for field in [
            &unrelated.runs_on,
            &unrelated.unattended_runs_on,
            &unrelated.builds_on,
            &unrelated.app_builds_on,
        ] {
            assert_eq!(field.as_deref(), Some(LEGACY_RUNNER));
        }
        write_at(&path, &unrelated).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"runner\""), "{text}");
        assert_eq!(read_at(&path).unwrap(), unrelated);
        // A real change is written as normalized: away from the runner...
        let prior = read_at(&path).unwrap();
        let mut away = normalize(AgentSettings {
            runs_on: Some("this-mac".into()),
            unattended_runs_on: Some("same".into()),
            builds_on: Some("auto".into()),
            app_builds_on: None,
            ..prior.clone()
        })
        .unwrap();
        keep_legacy_runner(&prior, &mut away);
        assert_eq!(away.runs_on, None);
        assert_eq!(away.unattended_runs_on.as_deref(), Some("same"));
        assert_eq!(away.builds_on.as_deref(), Some("auto"));
        assert_eq!(away.app_builds_on, None);
        write_at(&path, &away).unwrap();
        // ...and back to it from absent or this Mac, as `runner`.
        let prior = read_at(&path).unwrap();
        let mut back = normalize(AgentSettings {
            runs_on: Some(LEGACY_RUNNER.into()),
            unattended_runs_on: Some(RUNNER.into()),
            builds_on: Some(RUNNER.into()),
            app_builds_on: Some(RUNNER.into()),
            ..prior.clone()
        })
        .unwrap();
        keep_legacy_runner(&prior, &mut back);
        assert_eq!(back.runs_on.as_deref(), Some(RUNNER));
        assert_eq!(back.unattended_runs_on.as_deref(), Some(RUNNER));
        assert_eq!(back.builds_on.as_deref(), Some(RUNNER));
        assert_eq!(back.app_builds_on.as_deref(), Some(RUNNER));
        write_at(&path, &back).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mac-mini"), "{text}");
        assert!(text.contains("\"runsOn\": \"runner\""), "{text}");
        // Every Choice list accepts both names (settings_keys validates
        // against them) and names `runner` before `mac-mini`.
        for list in [
            &RUNS_ON[..],
            &UNATTENDED_RUNS_ON[..],
            &BUILDS_ON[..],
            &APP_BUILDS_ON[..],
        ] {
            let at = |v: &str| list.iter().position(|x| *x == v).unwrap();
            assert!(at(RUNNER) < at(LEGACY_RUNNER), "{list:?}");
        }
    }

    /// M3a PR2: COO on this Mac is `auto` (absent) unless saved `off`; an
    /// unknown value and `auto` save as absent, so a file without the field
    /// reads and writes unchanged.
    #[test]
    fn coo_dispatcher_is_auto_unless_saved_off() {
        assert_eq!(coo_dispatcher_choice(None), None);
        assert_eq!(coo_dispatcher_choice(Some("auto")), None);
        assert_eq!(coo_dispatcher_choice(Some("moon")), None);
        assert_eq!(coo_dispatcher_choice(Some(" off ")).as_deref(), Some("off"));
        for (asked, saved) in [
            (Some("off"), Some("off")),
            (Some("auto"), None),
            (Some("moon"), None),
            (None, None),
        ] {
            let normalized = normalize(AgentSettings {
                coo_dispatcher: asked.map(str::to_string),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(normalized.coo_dispatcher.as_deref(), saved, "{asked:?}");
        }
        let wire = serde_json::to_value(AgentSettings::default()).unwrap();
        assert!(wire.get("cooDispatcher").is_none(), "{wire}");
        let off: AgentSettings =
            serde_json::from_str(r#"{"models":{},"cooDispatcher":"off"}"#).unwrap();
        assert_eq!(off.coo_dispatcher.as_deref(), Some("off"));
        assert_eq!(COO_DISPATCHER, ["auto", "off"]);
    }

    /// M7 (PER-21): absent Overnight and worker chats and Builds and tests
    /// mean off; `auto` and `same` are kept when saved.
    #[test]
    fn absent_runner_choices_mean_off_and_auto_is_a_saved_choice() {
        assert_eq!(unattended_runs_on_choice(None), None);
        assert_eq!(unattended_runs_on_choice(Some("moon")), None);
        assert_eq!(
            unattended_runs_on_choice(Some(" same ")).as_deref(),
            Some("same")
        );
        assert_eq!(
            unattended_runs_on_choice(Some("runner")).as_deref(),
            Some(RUNNER)
        );
        assert_eq!(builds_on_choice(None), None);
        assert_eq!(builds_on_choice(Some(" auto ")).as_deref(), Some("auto"));
        let saved = normalize(AgentSettings {
            unattended_runs_on: Some("same".into()),
            builds_on: Some("auto".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(saved.unattended_runs_on.as_deref(), Some("same"));
        assert_eq!(saved.builds_on.as_deref(), Some("auto"));
        let off = normalize(AgentSettings {
            unattended_runs_on: Some("moon".into()),
            builds_on: Some("moon".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!((off.unattended_runs_on, off.builds_on), (None, None));
    }

    /// M7: the one-time freeze. With a runner set up, each of the two that
    /// is absent gets its pre-M7 default and the marker is set in the same
    /// write; a saved value, an old `mac-mini` included, is left as read and
    /// every other byte of the file is unchanged. Without a runner only the
    /// marker is written. A second run, and any later save, change nothing.
    #[test]
    fn the_runner_defaults_freeze_once_and_keep_saved_choices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-settings.json");
        // A file as Hollow wrote it before M7: both absent, an old mac-mini.
        let before = AgentSettings {
            default_agent: Some("claude".into()),
            runs_on: Some(LEGACY_RUNNER.into()),
            app_builds_on: Some(LEGACY_RUNNER.into()),
            ..Default::default()
        };
        write_at(&path, &before).unwrap();
        let old_text = std::fs::read_to_string(&path).unwrap();
        assert!(freeze_at(&path, true).unwrap());
        let frozen = read_at(&path).unwrap();
        assert_eq!(
            frozen,
            AgentSettings {
                unattended_runs_on: Some(RUNNER.into()),
                builds_on: Some("auto".into()),
                runner_defaults_frozen: true,
                ..before.clone()
            }
        );
        // Byte for byte: the two fields and the marker are the only change.
        assert_eq!(
            old_text,
            "{\n  \"defaultAgent\": \"claude\",\n  \"models\": {},\n  \"runsOn\": \"mac-mini\",\n  \"appBuildsOn\": \"mac-mini\"\n}"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"defaultAgent\": \"claude\",\n  \"models\": {},\n  \"runsOn\": \"mac-mini\",\n  \"unattendedRunsOn\": \"runner\",\n  \"buildsOn\": \"auto\",\n  \"appBuildsOn\": \"mac-mini\",\n  \"runnerDefaultsFrozen\": true\n}"
        );
        // Effective: the runner keeps unattended work; Auto routing stays.
        assert_eq!(
            crate::remote_session::unattended_runner_for(
                AgentKind::Claude,
                Some(None),
                frozen.unattended_runs_on.as_deref(),
                Some("host".into())
            )
            .as_deref(),
            Some("host")
        );
        assert_eq!(
            crate::build_slots::route_env(frozen.builds_on.as_deref()).1,
            "auto"
        );
        // Never again: no write at all on a second run, runner or not.
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(!freeze_at(&path, true).unwrap());
        assert!(!freeze_at(&path, false).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);

        // Present values are never overwritten.
        let mut chosen = AgentSettings {
            unattended_runs_on: Some("same".into()),
            builds_on: Some("this-mac".into()),
            ..Default::default()
        };
        assert!(freeze_runner_defaults(&mut chosen, true));
        assert_eq!(chosen.unattended_runs_on.as_deref(), Some("same"));
        assert_eq!(chosen.builds_on.as_deref(), Some("this-mac"));
        assert!(chosen.runner_defaults_frozen);

        // No runner set up: the marker only, so one named later starts off.
        let none = dir.path().join("none.json");
        assert!(freeze_at(&none, false).unwrap());
        let read = read_at(&none).unwrap();
        assert_eq!(
            read,
            AgentSettings {
                runner_defaults_frozen: true,
                ..Default::default()
            }
        );
        assert_eq!(read.unattended_runs_on, None);
        assert_eq!(
            crate::build_slots::route_env(read.builds_on.as_deref()).1,
            "this-mac"
        );
        assert!(
            !freeze_at(&none, true).unwrap(),
            "a runner named later turns nothing on"
        );
        assert_eq!(read_at(&none).unwrap(), read);

        // A save (whose payload omits the marker) keeps it.
        let mut saved = normalize(AgentSettings::default()).unwrap();
        carry_over(&read, &mut saved);
        assert!(saved.runner_defaults_frozen);
    }

    /// Canvas C3 wave 2 C4: Settings, Chat, Canvases saves `off` or
    /// `most`, drops the default (`helps`, absent on the wire), saves an
    /// unknown choice as the default (A16), and a file without the key
    /// reads as the default.
    #[test]
    fn the_canvases_setting_round_trips_and_drops_the_default() {
        let saved = |value: Option<&str>| {
            normalize(AgentSettings {
                canvases: value.map(str::to_string),
                ..Default::default()
            })
        };
        assert_eq!(saved(Some("off")).unwrap().canvases.as_deref(), Some("off"));
        assert_eq!(
            saved(Some(" most ")).unwrap().canvases.as_deref(),
            Some("most")
        );
        assert_eq!(saved(Some("helps")).unwrap().canvases, None);
        assert_eq!(saved(Some("")).unwrap().canvases, None);
        assert_eq!(saved(None).unwrap().canvases, None);
        // A16: an unknown choice (a newer build's, after a rollback) saves
        // as the default instead of blocking every other setting's save.
        assert_eq!(saved(Some("always")).unwrap().canvases, None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-settings.json");
        let off = saved(Some("off")).unwrap();
        write_at(&path, &off).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"canvases\": \"off\""), "{text}");
        assert_eq!(read_at(&path).unwrap(), off);
        write_at(&path, &saved(Some("helps")).unwrap()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("canvases"),
            "the default stays off the wire: {text}"
        );
        std::fs::write(&path, r#"{"defaultAgent":null,"models":{}}"#).unwrap();
        assert_eq!(read_at(&path).unwrap().canvases, None);
        // A newer Hollow's value still reads; a turn treats it as the default.
        std::fs::write(&path, r#"{"models":{},"canvases":"always"}"#).unwrap();
        let newer = read_at(&path).unwrap();
        assert_eq!(
            crate::visual_prompt::resolve(None, newer.canvases.as_deref()),
            crate::visual_prompt::CanvasFrequency::Helps
        );
    }

    /// OB22: the compaction window is off by default (absent on the wire,
    /// and an older file without the key reads as off), saves only one of
    /// the three offered sizes, and reaches Claude as its env var and Codex
    /// as Hollow's carrier for the `-c` override; no other agent gets one.
    #[test]
    fn the_compaction_window_is_off_by_default_and_saves_only_offered_sizes() {
        assert_eq!(AgentSettings::default().compact_window, None);
        let older: AgentSettings =
            serde_json::from_str(r#"{"defaultAgent":null,"models":{}}"#).unwrap();
        assert_eq!(older.compact_window, None);
        let plain = serde_json::to_value(&older).unwrap();
        assert!(plain.get("compactWindow").is_none(), "{plain}");

        let saved = |window: Option<u32>| {
            normalize(AgentSettings {
                compact_window: window,
                ..Default::default()
            })
        };
        for window in COMPACT_WINDOWS {
            let value = saved(Some(window)).unwrap();
            assert_eq!(value.compact_window, Some(window));
            let wire = serde_json::to_value(&value).unwrap();
            assert_eq!(wire["compactWindow"], window, "{wire}");
            let reread: AgentSettings = serde_json::from_value(wire).unwrap();
            assert_eq!(reread, value);
        }
        assert_eq!(saved(None).unwrap().compact_window, None);
        // OB22 review M1: any other size saves as off instead of failing the
        // save, so a hand-edited file never blocks the other settings.
        for odd in [0, 1, 200_000, 350_000, 500_000, 1_000_000] {
            assert_eq!(saved(Some(odd)).unwrap().compact_window, None, "{odd}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-settings.json");
        std::fs::write(
            &path,
            r#"{"defaultAgent":"codex","models":{"claude":"opus"},"compactWindow":500000}"#,
        )
        .unwrap();
        let hand_edited = read_at(&path).unwrap();
        assert_eq!(hand_edited.compact_window, Some(500_000));
        let resaved = normalize(AgentSettings {
            default_agent: Some("claude".into()),
            ..hand_edited
        })
        .unwrap();
        assert_eq!(resaved.default_agent.as_deref(), Some("claude"));
        assert_eq!(
            resaved.models.get("claude").map(String::as_str),
            Some("opus")
        );
        assert_eq!(resaved.compact_window, None);

        // Off: no agent gets anything.
        for kind in AgentKind::all() {
            assert!(compaction_env_with(kind, &AgentSettings::default()).is_empty());
        }
        let on = AgentSettings {
            compact_window: Some(300_000),
            ..Default::default()
        };
        let pair = |key: &str| vec![(key.to_string(), "300000".to_string())];
        assert_eq!(
            compaction_env_with(AgentKind::Claude, &on),
            pair("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        );
        assert_eq!(
            compaction_env_with(AgentKind::Codex, &on),
            pair(agent::CODEX_COMPACT_LIMIT_ENV)
        );
        assert!(compaction_env_with(AgentKind::Antigravity, &on).is_empty());
        assert!(compaction_env_with(AgentKind::Grok, &on).is_empty());
        // A hand-edited size Hollow doesn't offer reads as off.
        let hand = AgentSettings {
            compact_window: Some(123),
            ..Default::default()
        };
        assert_eq!(compact_window(&hand), None);
        assert!(compaction_env_with(AgentKind::Claude, &hand).is_empty());
    }

    #[test]
    fn normalization_preserves_custom_models_and_canonicalizes_aliases() {
        let input = AgentSettings {
            default_agent: Some(" agy ".into()),
            models: BTreeMap::from([("agy".into(), " provider/model:version ".into())]),
            efforts: BTreeMap::new(),
            ..Default::default()
        };
        let value = normalize(input).unwrap();
        assert_eq!(value.default_agent.as_deref(), Some("antigravity"));
        assert_eq!(
            value.models.get("antigravity").unwrap(),
            "provider/model:version"
        );
    }

    /// 2026-11-20: a default saved as a level variant becomes its base and
    /// lends its level to the agent's effort, once; a saved effort stands.
    #[test]
    fn a_saved_antigravity_variant_migrates_to_its_base_and_effort() {
        let migrated = |models: &[(&str, &str)], efforts: &[(&str, &str)]| {
            let mut settings = AgentSettings {
                models: models
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                efforts: efforts
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                ..Default::default()
            };
            migrate_antigravity_model(&mut settings);
            settings
        };
        let value = migrated(&[("antigravity", "gemini-4-neon-high")], &[]);
        assert_eq!(value.models["antigravity"], "gemini-4-neon");
        assert_eq!(value.efforts["antigravity"], "high");
        let kept = migrated(
            &[("agy", "gemini-3.8-flash-low")],
            &[("antigravity", "max")],
        );
        assert_eq!(kept.models["agy"], "gemini-3.8-flash");
        assert_eq!(kept.efforts["antigravity"], "max", "a saved effort stands");
        let plain = migrated(&[("antigravity", "gemini-pro-orchestrator")], &[]);
        assert_eq!(plain.models["antigravity"], "gemini-pro-orchestrator");
        assert!(plain.efforts.is_empty());
        let other = migrated(&[("codex", "gpt-5-high")], &[]);
        assert_eq!(other.models["codex"], "gpt-5-high", "only Antigravity's");
        let saved = normalize(migrated(&[], &[])).unwrap();
        assert!(saved.models.is_empty());
        let mut raw = AgentSettings::default();
        raw.models
            .insert("antigravity".into(), "gemini-4-neon-xhigh".into());
        let saved = normalize(raw).unwrap();
        assert_eq!(saved.models["antigravity"], "gemini-4-neon");
        assert_eq!(saved.efforts["antigravity"], "xhigh");
    }

    #[test]
    fn task_tools_switch_defaults_on_fails_closed_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-data").join(AGENT_TOOLS_FILE);

        // Missing file: on.
        assert!(tasks_tools_enabled_at(Ok(path.clone())));
        // Saving off persists, in the documented shape.
        set_tasks_tools_enabled_at(&path, false).unwrap();
        assert!(!tasks_tools_enabled_at(Ok(path.clone())));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({ "tasksTools": false, "appTools": true })
        );
        set_tasks_tools_enabled_at(&path, true).unwrap();
        assert!(tasks_tools_enabled_at(Ok(path.clone())));

        // A damaged file never re-enables tools the user switched off.
        std::fs::write(&path, "{not json").unwrap();
        assert!(!tasks_tools_enabled_at(Ok(path.clone())));
        // No resolvable directory: off as well.
        assert!(!tasks_tools_enabled_at(Err("no app data".into())));
        // Saving repairs it.
        set_tasks_tools_enabled_at(&path, true).unwrap();
        assert!(tasks_tools_enabled_at(Ok(path.clone())));
        // An empty object means defaults.
        std::fs::write(&path, "{}").unwrap();
        assert!(tasks_tools_enabled_at(Ok(path)));
    }

    /// Paperclip PR 14a, changed 2026-10-02: every agent saves every level;
    /// `minimal` is stored as `low`; only an unknown level or agent is
    /// refused. A stored value is read back unchanged (the run maps it).
    #[test]
    fn efforts_normalize_and_pass_through() {
        let saved = normalize(AgentSettings {
            efforts: BTreeMap::from([
                (" Claude ".into(), " high ".into()),
                ("codex".into(), "".into()),
                ("antigravity".into(), "max".into()),
                ("grok".into(), "xhigh".into()),
            ]),
            ..Default::default()
        })
        .unwrap();
        // Keys are canonicalized, values trimmed, empties dropped.
        assert_eq!(
            saved.efforts,
            BTreeMap::from([
                ("antigravity".into(), "max".into()),
                ("claude".into(), "high".into()),
                ("grok".into(), "xhigh".into())
            ])
        );
        let legacy = normalize(AgentSettings {
            efforts: BTreeMap::from([("codex".into(), "minimal".into())]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(legacy.efforts["codex"], "low");
        for (agent, level) in [
            ("claude", "turbo"),
            ("typo", "low"),
            ("claude", "ultracode"),
            ("codex", "ultracode"),
        ] {
            let refused = normalize(AgentSettings {
                efforts: BTreeMap::from([(agent.to_string(), level.to_string())]),
                ..Default::default()
            });
            assert!(refused.is_err(), "{agent}={level} must be refused");
        }
        // Older files and payloads carry no efforts, and an empty map stays
        // off the wire.
        let legacy: AgentSettings =
            serde_json::from_str(r#"{"defaultAgent":null,"models":{}}"#).unwrap();
        assert!(legacy.efforts.is_empty());
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!({ "defaultAgent": null, "models": {} })
        );
        // Read unvalidated: a hand-edited unsupported level reaches the run.
        let stored = AgentSettings {
            efforts: BTreeMap::from([
                ("claude".into(), " turbo ".into()),
                ("codex".into(), " ".into()),
            ]),
            ..Default::default()
        };
        assert_eq!(
            effort_with(&stored, AgentKind::Claude).as_deref(),
            Some("turbo")
        );
        assert_eq!(effort_with(&stored, AgentKind::Codex), None);
        assert_eq!(effort_with(&stored, AgentKind::Antigravity), None);
        // Ultracode is per chat: a hand-edited default never reaches a run.
        let hand = AgentSettings {
            efforts: BTreeMap::from([("claude".into(), "ultracode".into())]),
            ..Default::default()
        };
        assert_eq!(effort_with(&hand, AgentKind::Claude), None);
    }

    /// Paperclip PR 15b: absent is the 30-minute default, null switches the
    /// watchdog off (and is written back as null), 0 is the default, large
    /// values clamp (every agent is watched since merged plan slice 3, see
    /// `every_agent_gets_the_watchdog_and_an_optional_deadline`).
    #[test]
    fn codex_watchdog_default_null_and_non_positive() {
        use std::time::Duration;
        let absent: AgentSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.codex_no_output_timeout_min, Some(30));
        assert_eq!(
            AgentSettings::default().codex_no_output_timeout_min,
            Some(30)
        );
        // The default stays off the wire, so older payloads are unchanged.
        assert_eq!(
            serde_json::to_value(&absent).unwrap(),
            serde_json::json!({ "defaultAgent": null, "models": {} })
        );

        let off: AgentSettings =
            serde_json::from_str(r#"{"codexNoOutputTimeoutMin":null}"#).unwrap();
        assert_eq!(off.codex_no_output_timeout_min, None);
        assert_eq!(
            serde_json::to_value(&off).unwrap()["codexNoOutputTimeoutMin"],
            serde_json::Value::Null
        );
        let reread: AgentSettings =
            serde_json::from_value(serde_json::to_value(&off).unwrap()).unwrap();
        assert_eq!(
            reread.codex_no_output_timeout_min, None,
            "null survives a round trip"
        );

        let zero = normalize(AgentSettings {
            codex_no_output_timeout_min: Some(0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(zero.codex_no_output_timeout_min, Some(30));
        let huge = normalize(AgentSettings {
            codex_no_output_timeout_min: Some(5000),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(huge.codex_no_output_timeout_min, Some(1440));
        let kept = normalize(off.clone()).unwrap();
        assert_eq!(kept.codex_no_output_timeout_min, None);
        let custom: AgentSettings =
            serde_json::from_str(r#"{"codexNoOutputTimeoutMin":12}"#).unwrap();
        assert_eq!(
            serde_json::to_value(&custom).unwrap()["codexNoOutputTimeoutMin"],
            12
        );

        let defaults = AgentSettings::default();
        assert_eq!(
            stall_limit_for(AgentKind::Codex, &defaults),
            Some(Duration::from_secs(30 * 60))
        );
        assert_eq!(stall_limit_for(AgentKind::Codex, &off), None, "disabled");
        assert_eq!(
            stall_limit_for(AgentKind::Codex, &custom),
            Some(Duration::from_secs(12 * 60))
        );
        // A hand-edited file is normalized when read for a run too.
        let raw_zero = AgentSettings {
            codex_no_output_timeout_min: Some(0),
            ..Default::default()
        };
        assert_eq!(
            stall_limit_for(AgentKind::Codex, &raw_zero),
            Some(Duration::from_secs(30 * 60))
        );
    }

    /// Merged plan slice 3 (A·R2, closes D5b): the watchdog and the
    /// optional deadline resolve through one path for every agent. Before
    /// it, only Codex had a watchdog and no chat turn had a deadline.
    #[test]
    fn every_agent_gets_the_watchdog_and_an_optional_deadline() {
        use std::time::Duration;
        let defaults = AgentSettings::default();
        for kind in AgentKind::all() {
            assert_eq!(
                limits_for(kind, &defaults),
                RunLimits {
                    deadline: None,
                    stall_after: Some(Duration::from_secs(30 * 60)),
                },
                "{} is watched by default and has no deadline",
                kind.as_str()
            );
        }

        // Claude's entry has Codex's semantics: null is off, 0 the default,
        // large values clamp; a `codex` entry moves into Codex's field.
        let saved = normalize(AgentSettings {
            no_output_timeout_min: BTreeMap::from([
                (" Claude ".into(), None),
                ("grok".into(), Some(0)),
                ("agy".into(), Some(5000)),
                ("codex".into(), Some(7)),
            ]),
            deadline_min: BTreeMap::from([
                ("claude".into(), 20),
                ("codex".into(), 0),
                ("grok".into(), 9000),
            ]),
            ..Default::default()
        })
        .unwrap();
        let others = &saved.no_output_timeout_min;
        assert_eq!(others.len(), 2, "the grok default is dropped");
        assert_eq!(others["antigravity"], Some(1440));
        assert_eq!(others["claude"], None, "off is kept");
        assert_eq!(saved.codex_no_output_timeout_min, Some(7));
        assert_eq!(
            saved.deadline_min,
            BTreeMap::from([("claude".into(), 20), ("grok".into(), 1440)]),
            "0 is no deadline"
        );
        assert_eq!(
            limits_for(AgentKind::Claude, &saved),
            RunLimits {
                deadline: Some(Duration::from_secs(20 * 60)),
                stall_after: None,
            }
        );
        assert_eq!(
            limits_for(AgentKind::Grok, &saved),
            RunLimits {
                deadline: Some(Duration::from_secs(1440 * 60)),
                stall_after: Some(Duration::from_secs(30 * 60)),
            }
        );
        assert_eq!(
            limits_for(AgentKind::Codex, &saved).stall_after,
            Some(Duration::from_secs(7 * 60))
        );
        let typo = normalize(AgentSettings {
            deadline_min: BTreeMap::from([("typo".into(), 5)]),
            ..Default::default()
        });
        assert!(typo.is_err());

        // On the wire: off is written as null and survives a round trip;
        // empty maps stay off it, so older payloads are unchanged.
        let wire = serde_json::to_value(&saved).unwrap();
        let written = wire["noOutputTimeoutMin"]
            .as_object()
            .expect("the map is written");
        let off = written.get("claude");
        assert_eq!(off, Some(&serde_json::Value::Null), "off is null");
        assert_eq!(wire["deadlineMin"]["claude"], 20);
        let reread: AgentSettings = serde_json::from_value(wire).unwrap();
        assert_eq!(reread, saved);
        let plain = serde_json::to_value(AgentSettings::default()).unwrap();
        assert!(plain.get("noOutputTimeoutMin").is_none());
        assert!(plain.get("deadlineMin").is_none());
    }

    #[test]
    fn task_tools_file_lives_in_the_app_data_dir() {
        // One settings-dir helper (8a's app_data_dir), per-process in tests.
        let path = agent_tools_path().unwrap();
        assert_eq!(path, app_data_dir().unwrap().join("agent-tools.json"));
        assert!(path.starts_with(std::env::temp_dir()));
    }
}
