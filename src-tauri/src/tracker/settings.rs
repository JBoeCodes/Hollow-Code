//! The tracker's config block and API key.
//!
//! **Config** (`config.json`, top-level `tracker`; written by Settings,
//! Sources, Task tracker):
//!
//! ```json
//! "tracker": {
//!   "provider": "linear",
//!   "pollSeconds": 120,
//!   "linear": { "teamId": "<uuid>", "teamKey": "ENG", "transport": "graphql" }
//! }
//! ```
//!
//! The provider's own block (named by `provider`) holds the shared team
//! Hollow may touch, an optional personal team (`personalTeamId`,
//! `personalTeamKey`: Personal, key HOL, 2026-11-12) and the transport. Loading is tolerant: a missing block means
//! the tracker is off, and a malformed value is ignored with a note in
//! [`TrackerConfig::problems`] instead of failing the whole config. Old
//! `notion.*` keys still load and are ignored. There is no default team: without one the
//! tracker stays off, so nothing can ever read a team nobody chose.
//!
//! **Key.** Hollow's one Keychain bundle (`secret_store`) under
//! `tracker:<provider>`. A provider may name its own Keychain item (Linear:
//! `hollow-linear` / `default`); [`copy_keychain_item`] copies it into
//! the bundle once, on the person's click, so later builds never prompt for
//! a second item. The key never reaches a log, an error or the frontend.

use super::linear;
use serde::Deserialize;
use std::collections::BTreeMap;

/// What Hollow knows about a provider before talking to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    /// The provider's own Keychain item (service, account), if it has one.
    pub keychain_item: Option<(&'static str, &'static str)>,
    pub transports: &'static [&'static str],
    /// The only team id the provider's adapter accepts, and why another is
    /// refused (none: any team).
    pub team_pin: Option<(&'static str, &'static str)>,
    /// The one optional second team (the personal team), by id.
    pub personal_team_pin: Option<&'static str>,
}

/// Every provider with an adapter. A second provider is one more row and its
/// adapter file.
pub const PROVIDERS: &[ProviderInfo] = &[ProviderInfo {
    id: linear::PROVIDER,
    display_name: linear::DISPLAY_NAME,
    keychain_item: Some(linear::KEYCHAIN_ITEM),
    transports: linear::TRANSPORTS,
    team_pin: Some((linear::TEAM_ID, linear::TEAM_ONLY)),
    personal_team_pin: Some(linear::PERSONAL_TEAM_ID),
}];

impl ProviderInfo {
    /// Whether the provider's adapter may touch `team` (its pinned shared
    /// team or its pinned personal one; any team when it pins none).
    pub fn allows(&self, team: &str) -> bool {
        match self.team_pin {
            None => true,
            Some((pin, _)) => team == pin || self.personal_team_pin == Some(team),
        }
    }
}

pub fn provider(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

pub const DEFAULT_POLL_SECONDS: u64 = 120;
/// The interval setting's range: 1 to 60 minutes.
pub const MIN_POLL_SECONDS: u64 = 60;
pub const MAX_POLL_SECONDS: u64 = 3600;

/// Settings' "Track agent work" (`trackAgentWork`, two-Macs T3a): off,
/// a dry run (Hollow records in its audit what it would file and sends
/// nothing; the default) or on. The older `true` and `false` read as on
/// and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentWork {
    Off,
    #[default]
    DryRun,
    On,
}

impl AgentWork {
    /// The setting's values, as written in config.json.
    pub const VALUES: [&'static str; 3] = ["off", "dryRun", "on"];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentWork::Off => "off",
            AgentWork::DryRun => "dryRun",
            AgentWork::On => "on",
        }
    }

    /// A stored value: one of [`Self::VALUES`] or a boolean.
    pub fn from_value(value: &serde_json::Value) -> Option<AgentWork> {
        match value {
            serde_json::Value::Bool(true) => Some(AgentWork::On),
            serde_json::Value::Bool(false) => Some(AgentWork::Off),
            serde_json::Value::String(text) => match text.trim() {
                "off" => Some(AgentWork::Off),
                "dryRun" | "dry_run" => Some(AgentWork::DryRun),
                "on" => Some(AgentWork::On),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The `tracker` block as loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerConfig {
    /// The chosen provider's id; None = the tracker is off.
    pub provider: Option<String>,
    /// The pull pass interval, clamped to 60 to 3600 seconds.
    pub poll_seconds: u64,
    /// The one team Hollow may read and write (the provider's id for it).
    pub team_id: Option<String>,
    /// The team's short key ("ENG"), for display and issue keys.
    pub team_key: Option<String>,
    /// The optional second team: the person's own (`personalTeamId` in the
    /// provider block), mirrored beside the shared one. Only the provider's
    /// pinned personal team is accepted.
    pub personal_team_id: Option<String>,
    pub personal_team_key: Option<String>,
    /// The provider's transport (`graphql` unless set).
    pub transport: String,
    /// `trackAgentWork`: delegated agent work writes its Started and Outcome
    /// comments and status moves to the tracker (X1). True only when
    /// [`Self::agent_work`] is on; while not, work links are still recorded
    /// locally and nothing is sent.
    pub track_agent_work: bool,
    /// `trackAgentWork` itself (T3a): off, a dry run (the default) or on.
    pub agent_work: AgentWork,
    /// `fridayStatus` (A18): the built-in Friday 15:00 job writes each
    /// tracked project's status into Hollow. Off by default.
    pub friday_status: bool,
    /// `postStatusUpdates` (A18): a Friday status also goes to the tracker
    /// as a project status update, after the Hollow note. Off by default.
    pub post_status_updates: bool,
    /// Values that were ignored while loading, for Settings to show.
    pub problems: Vec<String>,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        TrackerConfig {
            provider: None,
            poll_seconds: DEFAULT_POLL_SECONDS,
            team_id: None,
            team_key: None,
            personal_team_id: None,
            personal_team_key: None,
            transport: "graphql".into(),
            track_agent_work: false,
            agent_work: AgentWork::DryRun,
            friday_status: false,
            post_status_updates: false,
            problems: Vec::new(),
        }
    }
}

/// `#[serde(deserialize_with)]` for `Config::tracker`: never fails.
pub fn tolerant<'de, D>(deserializer: D) -> Result<TrackerConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer).unwrap_or(serde_json::Value::Null);
    Ok(TrackerConfig::from_value(&value))
}

fn text(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl TrackerConfig {
    pub fn from_value(value: &serde_json::Value) -> TrackerConfig {
        let mut config = TrackerConfig::default();
        let Some(block) = value.as_object() else {
            if !value.is_null() {
                config
                    .problems
                    .push("The tracker settings aren't an object; the tracker is off.".into());
            }
            return config;
        };
        match block.get("provider") {
            None | Some(serde_json::Value::Null) => {}
            Some(raw) => match text(Some(raw)).map(|p| p.to_ascii_lowercase()) {
                Some(id) if provider(&id).is_some() => config.provider = Some(id),
                Some(id) => config.problems.push(format!(
                    "Hollow has no adapter for the task tracker \"{id}\"; the tracker is off."
                )),
                None if raw.as_str().is_some() => {}
                None => config
                    .problems
                    .push("The tracker provider isn't text; the tracker is off.".into()),
            },
        }
        match block.get("pollSeconds") {
            None | Some(serde_json::Value::Null) => {}
            Some(raw) => match raw.as_u64() {
                Some(secs) => {
                    config.poll_seconds = secs.clamp(MIN_POLL_SECONDS, MAX_POLL_SECONDS);
                }
                None => config.problems.push(format!(
                    "The sync interval isn't a whole number of seconds; using {DEFAULT_POLL_SECONDS}."
                )),
            },
        }
        match block.get("trackAgentWork") {
            None | Some(serde_json::Value::Null) => {}
            Some(raw) => match AgentWork::from_value(raw) {
                Some(mode) => config.agent_work = mode,
                None => {
                    config.agent_work = AgentWork::Off;
                    config
                        .problems
                        .push("Track agent work isn't off, dryRun or on; it stays off.".into());
                }
            },
        }
        config.track_agent_work = config.agent_work == AgentWork::On;
        for (key, slot, words) in [
            (
                "fridayStatus",
                &mut config.friday_status,
                "The Friday status",
            ),
            (
                "postStatusUpdates",
                &mut config.post_status_updates,
                "Post status updates",
            ),
        ] {
            match block.get(key) {
                None | Some(serde_json::Value::Null) => {}
                Some(raw) => match raw.as_bool() {
                    Some(on) => *slot = on,
                    None => config
                        .problems
                        .push(format!("{words} isn't true or false; it stays off.")),
                },
            }
        }
        let Some(id) = config.provider.clone() else {
            return config;
        };
        let Some(own) = block.get(&id) else {
            return config;
        };
        let Some(own) = own.as_object() else {
            config.problems.push(format!(
                "The {id} settings aren't an object; no team is set."
            ));
            return config;
        };
        match text(own.get("teamId")) {
            Some(team) if looks_like_id(&team) => config.team_id = Some(team),
            Some(_) => config
                .problems
                .push("The team id isn't an id Hollow recognises; no team is set.".into()),
            None => {}
        }
        match text(own.get("teamKey")) {
            Some(key) if looks_like_team_key(&key) => config.team_key = Some(key),
            Some(_) => config
                .problems
                .push("The team key isn't a short code like AI; it is ignored.".into()),
            None => {}
        }
        match text(own.get("personalTeamId")) {
            Some(team) if looks_like_id(&team) => config.personal_team_id = Some(team),
            Some(_) => config.problems.push(
                "The personal team id isn't an id Hollow recognises; no personal team is set."
                    .into(),
            ),
            None => {}
        }
        match text(own.get("personalTeamKey")) {
            Some(key) if looks_like_team_key(&key) => config.personal_team_key = Some(key),
            Some(_) => config
                .problems
                .push("The personal team key isn't a short code like HOL; it is ignored.".into()),
            None => {}
        }
        if let Some(transport) = text(own.get("transport")).map(|t| t.to_ascii_lowercase()) {
            let known = provider(&id).is_some_and(|p| p.transports.contains(&transport.as_str()));
            if known {
                config.transport = transport;
            } else {
                config.problems.push(format!(
                    "The transport \"{transport}\" isn't one Hollow offers; using graphql."
                ));
            }
        }
        config
    }

    /// The teams this config syncs, the shared one first: its own id, then
    /// the personal team's when one is set.
    pub fn team_ids(&self) -> Vec<String> {
        self.team_id
            .iter()
            .chain(self.personal_team_id.iter())
            .cloned()
            .collect()
    }

    pub fn provider_info(&self) -> Option<&'static ProviderInfo> {
        self.provider.as_deref().and_then(provider)
    }

    /// Why the tracker can't run yet, in the order a person fixes them; None
    /// when it can.
    pub fn missing(&self, key: KeyState) -> Option<String> {
        if self.provider_info().is_none() {
            return Some("Choose a task tracker.".into());
        }
        if self.team_id.is_none() {
            return Some("Choose the team Hollow may use.".into());
        }
        if let (Some((pin, why)), Some(team)) = (
            self.provider_info().and_then(|p| p.team_pin),
            self.team_id.as_deref(),
        ) {
            if team != pin {
                return Some(why.into());
            }
        }
        if let (Some(personal), Some(pin)) = (
            self.personal_team_id.as_deref(),
            self.provider_info().map(|p| p.personal_team_pin),
        ) {
            if pin != Some(personal) {
                return Some(
                    self.provider_info()
                        .and_then(|p| p.team_pin)
                        .map_or("Hollow works only with its two teams.", |(_, why)| why)
                        .into(),
                );
            }
        }
        match key {
            KeyState::Saved => None,
            KeyState::Missing => Some("Save an API key.".into()),
            KeyState::Unreadable => {
                Some("Hollow can't read its saved credentials (Keychain access).".into())
            }
        }
    }
}

/// A provider's id for a team: a UUID (Linear) or another opaque id of
/// letters, digits and dashes.
pub fn looks_like_id(value: &str) -> bool {
    (8..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A team's short key: 1 to 10 capital letters or digits, starting with a letter.
pub fn looks_like_team_key(value: &str) -> bool {
    (1..=10).contains(&value.len())
        && value.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// A change from Settings. `None` leaves a value alone; an empty provider
/// turns the tracker off (its provider block stays, for turning it back on).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerSettingsUpdate {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub poll_seconds: Option<u64>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team_key: Option<String>,
    /// The personal team (Personal). An empty value turns it off.
    #[serde(default)]
    pub personal_team_id: Option<String>,
    #[serde(default)]
    pub personal_team_key: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    /// Settings, Task tracker, "Track agent work in the tracker".
    #[serde(default)]
    pub track_agent_work: Option<bool>,
    /// Settings, Task tracker, "Write a Friday status at 3 PM" (A18).
    #[serde(default)]
    pub friday_status: Option<bool>,
    /// Settings, Task tracker, "Also post it to the tracker" (A18).
    #[serde(default)]
    pub post_status_updates: Option<bool>,
}

/// `tracker.configure` (T6b): the whole tracker block in one step, so an
/// agent can run the cutover without a person filling Settings. Only team
/// Engineering, for every provider; the key is never an argument.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerConfigure {
    /// Defaults to Linear, the one provider with an adapter.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub team_id: String,
    /// Defaults to `ENG`.
    #[serde(default)]
    pub team_key: Option<String>,
    #[serde(default)]
    pub poll_seconds: Option<u64>,
    #[serde(default)]
    pub track_agent_work: Option<bool>,
    /// Also copy the provider's own Keychain item into Hollow's bundle.
    #[serde(default)]
    pub copy_keychain_key: bool,
}

/// The one team `tracker.configure` accepts, and its key.
pub const CONFIGURE_TEAM: (&str, &str) = ("11111111-1111-4111-8111-111111111111", "ENG");

impl TrackerConfigure {
    /// The Settings update this stands for, or why it is refused (before
    /// anything is written).
    pub fn update(&self) -> Result<TrackerSettingsUpdate, String> {
        let (team, key) = CONFIGURE_TEAM;
        let refused =
            || format!("tracker.configure sets up team {key} ({team}) only; nothing was changed.");
        if self.team_id.trim() != team {
            return Err(refused());
        }
        if self
            .team_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|k| !k.is_empty() && k != key)
        {
            return Err(refused());
        }
        let provider = self
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(linear::PROVIDER);
        Ok(TrackerSettingsUpdate {
            provider: Some(provider.to_string()),
            poll_seconds: self.poll_seconds,
            team_id: Some(team.to_string()),
            team_key: Some(key.to_string()),
            personal_team_id: None,
            personal_team_key: None,
            transport: None,
            track_agent_work: self.track_agent_work,
            friday_status: None,
            post_status_updates: None,
        })
    }
}

/// Settings key `tracker.trackAgentWork` (T3a): write `mode` into the
/// config file's tracker block (created when missing), other keys kept.
pub fn set_agent_work(map: &mut BTreeMap<String, serde_json::Value>, mode: AgentWork) {
    let block = map
        .entry("tracker".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !block.is_object() {
        *block = serde_json::json!({});
    }
    if let Some(block) = block.as_object_mut() {
        block.insert("trackAgentWork".into(), serde_json::json!(mode.as_str()));
    }
}

/// Apply `update` to the config file's map, refusing an invalid value before
/// anything changes. Keys Hollow doesn't model are kept.
pub fn apply_update(
    map: &mut BTreeMap<String, serde_json::Value>,
    update: &TrackerSettingsUpdate,
) -> Result<(), String> {
    let current = TrackerConfig::from_value(map.get("tracker").unwrap_or(&serde_json::Value::Null));
    let provider_id = match update.provider.as_deref().map(str::trim) {
        Some("") => None,
        Some(id) => {
            let id = id.to_ascii_lowercase();
            if provider(&id).is_none() {
                return Err(format!(
                    "Hollow has no adapter for the task tracker \"{id}\"."
                ));
            }
            Some(id)
        }
        None => current.provider.clone(),
    };
    if let Some(secs) = update.poll_seconds {
        if !(MIN_POLL_SECONDS..=MAX_POLL_SECONDS).contains(&secs) {
            return Err("The sync interval must be between 1 and 60 minutes.".into());
        }
    }
    let wants_provider_values = update.team_id.is_some()
        || update.team_key.is_some()
        || update.personal_team_id.is_some()
        || update.personal_team_key.is_some()
        || update.transport.is_some();
    if wants_provider_values && provider_id.is_none() {
        return Err("Choose a task tracker before its team or transport.".into());
    }
    if let Some(team) = update.team_id.as_deref().map(str::trim) {
        if !team.is_empty() && !looks_like_id(team) {
            return Err("That team id isn't an id Hollow recognises.".into());
        }
        let pin = provider_id
            .as_deref()
            .and_then(provider)
            .and_then(|p| p.team_pin);
        if let Some((_, why)) = pin.filter(|(pin, _)| !team.is_empty() && team != *pin) {
            return Err(why.into());
        }
    }
    if let Some(key) = update.team_key.as_deref().map(str::trim) {
        if !key.is_empty() && !looks_like_team_key(key) {
            return Err(
                "A team key is a short code of capital letters and digits, like AI.".into(),
            );
        }
    }
    if let Some(team) = update.personal_team_id.as_deref().map(str::trim) {
        let pin = provider_id
            .as_deref()
            .and_then(provider)
            .and_then(|p| p.personal_team_pin);
        if !team.is_empty() && pin != Some(team) {
            return Err(
                "Hollow's personal team is Personal only; no other team can be added.".into(),
            );
        }
    }
    if let Some(key) = update.personal_team_key.as_deref().map(str::trim) {
        if !key.is_empty() && !looks_like_team_key(key) {
            return Err(
                "A team key is a short code of capital letters and digits, like HOL.".into(),
            );
        }
    }
    let transport = update
        .transport
        .as_deref()
        .map(|t| t.trim().to_ascii_lowercase());
    if let (Some(t), Some(id)) = (transport.as_deref(), provider_id.as_deref()) {
        if !provider(id).is_some_and(|p| p.transports.contains(&t)) {
            return Err(format!(
                "\"{t}\" isn't a transport Hollow offers for this tracker."
            ));
        }
    }

    let block = map
        .entry("tracker".into())
        .or_insert_with(|| serde_json::json!({}));
    if !block.is_object() {
        *block = serde_json::json!({});
    }
    let block = block.as_object_mut().expect("object assigned above");
    match (&update.provider, &provider_id) {
        (Some(_), Some(id)) => {
            block.insert("provider".into(), serde_json::json!(id));
        }
        (Some(_), None) => {
            block.remove("provider");
        }
        (None, _) => {}
    }
    if let Some(secs) = update.poll_seconds {
        block.insert("pollSeconds".into(), serde_json::json!(secs));
    }
    // The boolean is the old on/off switch: `true` turns it on; `false`
    // turns on into off but leaves a dry run alone (T3a), so a form that
    // reads `trackAgentWork: false` from the status and saves it unchanged
    // never ends the dry run. `tracker.trackAgentWork` sets all three.
    match update.track_agent_work {
        Some(true) => {
            block.insert("trackAgentWork".into(), serde_json::json!(true));
        }
        Some(false) if current.agent_work != AgentWork::DryRun => {
            block.insert("trackAgentWork".into(), serde_json::json!(false));
        }
        _ => {}
    }
    if let Some(on) = update.friday_status {
        block.insert("fridayStatus".into(), serde_json::json!(on));
    }
    if let Some(on) = update.post_status_updates {
        block.insert("postStatusUpdates".into(), serde_json::json!(on));
    }
    if let (true, Some(id)) = (wants_provider_values, provider_id.as_deref()) {
        let own = block
            .entry(id.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if !own.is_object() {
            *own = serde_json::json!({});
        }
        let own = own.as_object_mut().expect("object assigned above");
        for (key, value) in [
            ("teamId", update.team_id.as_deref().map(str::trim)),
            ("teamKey", update.team_key.as_deref().map(str::trim)),
            (
                "personalTeamId",
                update.personal_team_id.as_deref().map(str::trim),
            ),
            (
                "personalTeamKey",
                update.personal_team_key.as_deref().map(str::trim),
            ),
            ("transport", transport.as_deref()),
        ] {
            match value {
                Some("") => {
                    own.remove(key);
                }
                Some(value) => {
                    own.insert(key.into(), serde_json::json!(value));
                }
                None => {}
            }
        }
    }
    Ok(())
}

// ── The API key ─────────────────────────────────────────────────────────────

/// Whether a key is saved, without its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerKeyState"))]
#[serde(rename_all = "camelCase")]
pub enum KeyState {
    Missing,
    Saved,
    /// The bundle couldn't be read (Keychain access denied or locked).
    Unreadable,
}

/// The bundle key for a provider's API key.
pub fn key_name(provider: &str) -> String {
    format!("tracker:{provider}")
}

/// The saved key, for the adapter. Never log or return it to the frontend.
pub fn saved_key(provider: &str) -> Result<Option<String>, crate::secret_store::StoreError> {
    crate::secret_store::get(&key_name(provider))
}

pub fn key_state(provider: &str) -> KeyState {
    match crate::secret_store::present(&key_name(provider)) {
        Ok(true) => KeyState::Saved,
        Ok(false) => KeyState::Missing,
        Err(_) => KeyState::Unreadable,
    }
}

/// A key as typed or pasted: trimmed, one token, no control characters.
fn clean_key(raw: &str) -> Result<&str, String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err("Paste an API key first.".into());
    }
    if key.len() > 512 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like an API key (one line, no spaces).".into());
    }
    Ok(key)
}

pub fn save_key(provider: &str, raw: &str) -> Result<(), String> {
    let key = clean_key(raw)?;
    crate::secret_store::set(&key_name(provider), key).map_err(|e| e.to_string())
}

pub fn clear_key(provider: &str) -> Result<(), String> {
    crate::secret_store::delete(&key_name(provider)).map_err(|e| e.to_string())
}

/// Copy the provider's own Keychain item into Hollow's bundle (one macOS
/// prompt, the first time). Ok(false) when the provider names no item or the
/// item doesn't exist.
pub fn copy_keychain_item(provider_id: &str) -> Result<bool, String> {
    let Some(item) = provider(provider_id).and_then(|p| p.keychain_item) else {
        return Ok(false);
    };
    copy_item(provider_id, item)
}

fn copy_item(provider_id: &str, (service, account): (&str, &str)) -> Result<bool, String> {
    match crate::secret_store::read_item(service, account).map_err(|e| e.to_string())? {
        None => Ok(false),
        Some(value) => {
            // A malformed item gets its own words; a failed save keeps the
            // store's (neither ever carries the value).
            let key = clean_key(&value)
                .map_err(|_| "The Keychain item doesn't hold a usable API key.".to_string())?;
            crate::secret_store::set(&key_name(provider_id), key).map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn load(value: serde_json::Value) -> TrackerConfig {
        TrackerConfig::from_value(&value)
    }

    #[test]
    fn a_full_block_loads_and_a_missing_one_means_off() {
        let config = load(json!({
            "provider": "linear", "pollSeconds": 300,
            "linear": {"teamId": "11111111-1111-4111-8111-111111111111", "teamKey": "ENG", "transport": "graphql"}
        }));
        assert_eq!(config.provider.as_deref(), Some("linear"));
        assert_eq!(config.poll_seconds, 300);
        assert_eq!(
            config.team_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(config.team_key.as_deref(), Some("ENG"));
        assert_eq!(config.transport, "graphql");
        assert!(config.problems.is_empty());
        assert_eq!(config.missing(KeyState::Saved), None);
        assert_eq!(
            config.missing(KeyState::Missing).as_deref(),
            Some("Save an API key.")
        );

        // X1: agent work tracking sends nothing unless set, and a bad value
        // stays off. T3a: unset is a dry run; true and false read as on, off.
        assert!(!config.track_agent_work);
        assert_eq!(config.agent_work, AgentWork::DryRun);
        assert!(load(json!({"trackAgentWork": true})).track_agent_work);
        assert_eq!(
            load(json!({"trackAgentWork": false})).agent_work,
            AgentWork::Off
        );
        let on = load(json!({"trackAgentWork": "on"}));
        assert!(on.track_agent_work && on.agent_work == AgentWork::On);
        let dry = load(json!({"trackAgentWork": "dryRun"}));
        assert!(!dry.track_agent_work && dry.agent_work == AgentWork::DryRun);
        let bad = load(json!({"trackAgentWork": "yes"}));
        assert!(!bad.track_agent_work && bad.problems.len() == 1);
        assert_eq!(bad.agent_work, AgentWork::Off);
        let mut map = BTreeMap::new();
        set_agent_work(&mut map, AgentWork::On);
        assert_eq!(map["tracker"], json!({"trackAgentWork": "on"}));
        // An unchanged form save (the status's false) keeps a dry run; true
        // turns it on, and false then turns it off.
        let mut map = BTreeMap::new();
        set_agent_work(&mut map, AgentWork::DryRun);
        let save = |map: &mut BTreeMap<String, serde_json::Value>, on: bool| {
            apply_update(
                map,
                &TrackerSettingsUpdate {
                    track_agent_work: Some(on),
                    ..Default::default()
                },
            )
            .unwrap();
            TrackerConfig::from_value(&map["tracker"]).agent_work
        };
        assert_eq!(save(&mut map, false), AgentWork::DryRun);
        assert_eq!(save(&mut map, true), AgentWork::On);
        assert_eq!(save(&mut map, false), AgentWork::Off);
        // tracker.configure goes through the same update.
        let mut map = BTreeMap::new();
        let configure = TrackerConfigure {
            team_id: CONFIGURE_TEAM.0.into(),
            track_agent_work: Some(false),
            ..Default::default()
        };
        apply_update(&mut map, &configure.update().unwrap()).unwrap();
        assert_eq!(
            TrackerConfig::from_value(&map["tracker"]).agent_work,
            AgentWork::DryRun
        );
        // A18: the Friday status and its tracker copy are off unless set.
        assert!(!config.friday_status && !config.post_status_updates);
        let on = load(json!({"fridayStatus": true, "postStatusUpdates": true}));
        assert!(on.friday_status && on.post_status_updates);
        let bad = load(json!({"fridayStatus": 1, "postStatusUpdates": "on"}));
        assert!(!bad.friday_status && !bad.post_status_updates && bad.problems.len() == 2);

        let off = load(serde_json::Value::Null);
        assert_eq!(off, TrackerConfig::default());
        assert_eq!(
            off.missing(KeyState::Saved).as_deref(),
            Some("Choose a task tracker.")
        );
    }

    #[test]
    fn malformed_values_are_ignored_with_a_note_never_an_error() {
        let config = load(json!({
            "provider": "jira", "pollSeconds": "fast", "jira": {"teamId": "x"}
        }));
        assert_eq!(config.provider, None, "an unknown provider is off");
        assert_eq!(config.poll_seconds, DEFAULT_POLL_SECONDS);
        assert_eq!(config.problems.len(), 2, "{:?}", config.problems);

        let config = load(json!({
            "provider": "LINEAR", "pollSeconds": 5,
            "linear": {"teamId": "not an id!", "teamKey": "lowercase", "transport": "smoke-signals"}
        }));
        assert_eq!(config.provider.as_deref(), Some("linear"));
        assert_eq!(config.poll_seconds, MIN_POLL_SECONDS, "clamped up");
        assert_eq!(config.team_id, None, "no team means the tracker can't run");
        assert_eq!(config.team_key, None);
        assert_eq!(config.transport, "graphql");
        assert_eq!(config.problems.len(), 3, "{:?}", config.problems);
        assert_eq!(
            config.missing(KeyState::Saved).as_deref(),
            Some("Choose the team Hollow may use.")
        );

        assert_eq!(
            load(json!({"pollSeconds": 99999})).poll_seconds,
            MAX_POLL_SECONDS
        );
        assert_eq!(load(json!("linear")).problems.len(), 1);
        assert_eq!(load(json!({"provider": 7})).problems.len(), 1);
        assert_eq!(
            load(json!({"provider": "linear", "linear": []}))
                .problems
                .len(),
            1
        );
    }

    #[test]
    fn the_whole_config_still_loads_with_a_broken_tracker_block() {
        let config: crate::config::Config = serde_json::from_value(json!({
            "vaultPath": "/vault",
            "notion": {"twoWaySync": true, "taskDataSourceId": "t"},
            "tracker": {"provider": ["linear"], "pollSeconds": -1, "linear": "ENG"}
        }))
        .unwrap();
        assert_eq!(config.tracker.provider, None);
        assert!(!config.tracker.problems.is_empty());

        let config: crate::config::Config =
            serde_json::from_value(json!({"vaultPath": "/vault"})).unwrap();
        assert_eq!(config.tracker, TrackerConfig::default());
    }

    #[test]
    fn settings_updates_validate_first_and_keep_unknown_keys() {
        let mut map: BTreeMap<String, serde_json::Value> = serde_json::from_value(json!({
            "vaultPath": "/vault",
            "tracker": {"custom": 1, "linear": {"keep": true}}
        }))
        .unwrap();
        let update = TrackerSettingsUpdate {
            provider: Some("linear".into()),
            poll_seconds: Some(600),
            team_id: Some(" 11111111-1111-4111-8111-111111111111 ".into()),
            team_key: Some("ENG".into()),
            personal_team_id: None,
            personal_team_key: None,
            transport: None,
            track_agent_work: Some(true),
            friday_status: Some(true),
            post_status_updates: Some(false),
        };
        apply_update(&mut map, &update).unwrap();
        assert_eq!(
            map["tracker"],
            json!({"custom": 1, "provider": "linear", "pollSeconds": 600, "trackAgentWork": true,
                   "fridayStatus": true, "postStatusUpdates": false,
                   "linear": {"keep": true, "teamId": "11111111-1111-4111-8111-111111111111", "teamKey": "ENG"}})
        );
        let before = map.clone();
        for bad in [
            TrackerSettingsUpdate {
                provider: Some("jira".into()),
                ..Default::default()
            },
            TrackerSettingsUpdate {
                poll_seconds: Some(30),
                ..Default::default()
            },
            TrackerSettingsUpdate {
                team_id: Some("bad id".into()),
                ..Default::default()
            },
            TrackerSettingsUpdate {
                team_key: Some("eng".into()),
                ..Default::default()
            },
            TrackerSettingsUpdate {
                transport: Some("fax".into()),
                ..Default::default()
            },
        ] {
            assert!(apply_update(&mut map, &bad).is_err(), "{bad:?}");
            assert_eq!(map, before, "a refused update changes nothing");
        }
        // Turning it off keeps the provider's block for later.
        apply_update(
            &mut map,
            &TrackerSettingsUpdate {
                provider: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(map["tracker"].get("provider").is_none());
        assert_eq!(map["tracker"]["linear"]["teamKey"], "ENG");
        // With no provider, a team can't be set.
        assert!(apply_update(
            &mut map,
            &TrackerSettingsUpdate {
                team_key: Some("ENG".into()),
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn keys_are_cleaned_saved_and_cleared_without_being_echoed() {
        let provider = "test-tracker-save";
        assert_eq!(key_state(provider), KeyState::Missing);
        assert!(save_key(provider, "   ").is_err());
        let refused = save_key(provider, "lin_api_has space").unwrap_err();
        assert!(!refused.contains("lin_api"), "{refused}");
        save_key(provider, "  lin_api_abc123\n").unwrap();
        assert_eq!(
            saved_key(provider).unwrap().as_deref(),
            Some("lin_api_abc123")
        );
        assert_eq!(key_state(provider), KeyState::Saved);
        clear_key(provider).unwrap();
        assert_eq!(key_state(provider), KeyState::Missing);
    }

    #[test]
    fn the_keychain_item_is_copied_once_into_the_bundle() {
        let provider = "test-tracker-copy";
        let item = ("hollow-test-tracker-item", "team");
        assert_eq!(copy_item(provider, item), Ok(false), "no item yet");
        crate::secret_store::tests::seed_item(item.0, item.1, "lin_api_from_keychain");
        assert_eq!(copy_item(provider, item), Ok(true));
        assert_eq!(
            saved_key(provider).unwrap().as_deref(),
            Some("lin_api_from_keychain")
        );
        crate::secret_store::tests::seed_item(item.0, item.1, "two words");
        let refused = copy_item(provider, item).unwrap_err();
        assert!(!refused.contains("two words"), "{refused}");
        assert_eq!(
            saved_key(provider).unwrap().as_deref(),
            Some("lin_api_from_keychain")
        );
        // Linear's item is the one Alex created on 2026-11-07.
        assert_eq!(provider_item("linear"), Some(("hollow-linear", "default")));
        assert_eq!(copy_keychain_item("unknown"), Ok(false));
    }

    fn provider_item(id: &str) -> Option<(&'static str, &'static str)> {
        provider(id).and_then(|p| p.keychain_item)
    }

    /// Review fix 2: Hollow works only with team Engineering, for people
    /// and agents alike: Settings refuses another team, the status names
    /// why, and the adapter refuses to be built for it.
    #[test]
    fn another_team_than_engineering_is_refused() {
        let other = "8d8e6207-0000-4000-8000-000000000000";
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let update = TrackerSettingsUpdate {
            provider: Some("linear".into()),
            team_id: Some(other.into()),
            ..Default::default()
        };
        assert_eq!(
            apply_update(&mut map, &update).unwrap_err(),
            linear::TEAM_ONLY
        );
        assert!(map.is_empty(), "nothing was written");
        let config = TrackerConfig::from_value(
            &json!({"provider": "linear", "linear": {"teamId": other, "teamKey": "LP"}}),
        );
        assert_eq!(
            config.missing(KeyState::Saved).as_deref(),
            Some(linear::TEAM_ONLY)
        );
        let refused = linear::LinearAdapter::from_config_with_key(&config, "fixture-key-only")
            .err()
            .unwrap();
        assert_eq!(
            refused,
            crate::tracker::TrackerError::NotConfigured(linear::TEAM_ONLY.into())
        );
        let ai = TrackerConfig::from_value(
            &json!({"provider": "linear", "linear": {"teamId": linear::TEAM_ID, "teamKey": "ENG"}}),
        );
        assert!(ai.missing(KeyState::Saved).is_none());
        assert!(linear::LinearAdapter::from_config_with_key(&ai, "fixture-key-only").is_ok());
    }

    #[test]
    fn the_personal_team_is_hollow_j_only_and_every_other_team_stays_refused() {
        let hol = linear::PERSONAL_TEAM_ID;
        let lp = "11111111-2222-3333-4444-555555555555";
        let pro = "99999999-8888-7777-6666-555555555555";
        // A config with both teams loads, and lists the shared team first.
        let both = TrackerConfig::from_value(&json!({"provider": "linear", "linear": {
            "teamId": linear::TEAM_ID, "teamKey": "ENG", "personalTeamId": hol, "personalTeamKey": "PER"}}));
        assert_eq!(
            both.team_ids(),
            vec![linear::TEAM_ID.to_string(), hol.to_string()]
        );
        assert!(both.missing(KeyState::Saved).is_none());
        // Today's config (no personal team) is unchanged.
        let ai = TrackerConfig::from_value(
            &json!({"provider": "linear", "linear": {"teamId": linear::TEAM_ID, "teamKey": "ENG"}}),
        );
        assert_eq!(ai.team_ids(), vec![linear::TEAM_ID.to_string()]);
        // Any other team as the personal team (LP, PRO, by id) fails setup.
        for other in [lp, pro] {
            let bad = TrackerConfig::from_value(&json!({"provider": "linear", "linear": {
                "teamId": linear::TEAM_ID, "personalTeamId": other, "personalTeamKey": "LP"}}));
            assert!(bad.missing(KeyState::Saved).is_some(), "{other}");
            let mut map = std::collections::BTreeMap::new();
            let update = TrackerSettingsUpdate {
                provider: Some("linear".into()),
                personal_team_id: Some(other.into()),
                ..Default::default()
            };
            assert!(apply_update(&mut map, &update).is_err(), "{other}");
            assert!(map.is_empty(), "nothing was written");
        }
        // Adding Personal writes the personal keys and keeps the rest; an
        // empty value removes it.
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "tracker".to_string(),
            json!({"provider": "linear", "linear": {"teamId": linear::TEAM_ID, "teamKey": "ENG"}}),
        );
        let update = TrackerSettingsUpdate {
            personal_team_id: Some(hol.into()),
            personal_team_key: Some("PER".into()),
            ..Default::default()
        };
        apply_update(&mut map, &update).unwrap();
        assert_eq!(map["tracker"]["linear"]["personalTeamId"], hol);
        assert_eq!(map["tracker"]["linear"]["teamId"], linear::TEAM_ID);
        let off = TrackerSettingsUpdate {
            personal_team_id: Some(String::new()),
            personal_team_key: Some(String::new()),
            ..Default::default()
        };
        apply_update(&mut map, &off).unwrap();
        assert!(map["tracker"]["linear"].get("personalTeamId").is_none());
        // The adapter only scopes to the two teams, by id.
        let adapter = linear::LinearAdapter::new(linear::TEAM_ID, "fixture-key-only").unwrap();
        assert!(adapter.scoped_to(hol).is_ok());
        for other in [lp, pro] {
            assert!(adapter.scoped_to(other).is_err(), "{other}");
            assert!(
                linear::LinearAdapter::new(other, "fixture-key-only").is_err(),
                "{other}"
            );
        }
        assert_eq!(linear::allowed_team("per"), Some(hol));
        assert_eq!(linear::allowed_team("LP"), None);
        assert_eq!(linear::allowed_team("PRO"), None);
        assert_eq!(linear::allowed_team(lp), None);
    }
}
