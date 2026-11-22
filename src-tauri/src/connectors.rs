//! The connectors library: MCP servers added once in Hollow and shared with
//! every agent that can use them (Claude Code and Codex), instead of being
//! configured separately inside each CLI.
//!
//! Hollow runs Claude with `--strict-mcp-config` and switches every
//! user-configured Codex server off, so an agent only ever sees what Hollow
//! attaches. This library is how a user's own connectors (a mail service, a
//! CRM, an internal API) get attached on purpose: per connector, which
//! agents share it, whether chats and/or automations get it, whether it is
//! always attached or only when the request needs it (the context
//! prioritizer, `context_rank.rs`), and whether its tools run without asking.
//!
//! Storage: `connectors.json` beside `agent-settings.json`
//! (`agent_settings::app_data_dir`), outside the vault, written 0600 through
//! the private config publisher. Environment values and headers can hold
//! secrets: IPC only ever returns their names. Existing definitions can be
//! imported from `~/.claude.json` (`mcpServers`) and `~/.codex/config.toml`
//! (`[mcp_servers.*]`); Hollow never edits those files.
//!
//! **claude.ai connectors** (`Transport::ClaudeAi`): the connectors the user
//! added on claude.ai (Superhuman Mail, Microsoft 365, …) are listed from
//! `GET /v1/mcp_servers` with Claude Code's own login and attached as a
//! `claudeai-proxy` entry, which Claude Code accepts in `--mcp-config` even
//! under `--strict-mcp-config` (verified 2026-09-26 on 2.1.283: Notion came up
//! connected with its 45 tools). Claude only, and the run goes out on Claude
//! Code's own login (`agent.rs` skips account spreading), because the
//! connector belongs to that claude.ai account.
use crate::agent::AgentKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "connectors.json";
const SCHEMA: u32 = 1;
pub(crate) const MAX_CONNECTORS: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    #[default]
    Stdio,
    Http,
    /// A connector added on claude.ai, reached through Anthropic's proxy.
    ClaudeAi,
    /// Built into Hollow (`builtin_connectors.rs`): a hidden `hollow`
    /// subcommand on Hollow's own sign-in.
    Builtin,
}

/// The `anthropic-beta` value Claude Code 2.1.283 sends to list connectors.
const CLAUDEAI_BETA: &str = "mcp-servers-2025-12-04";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Attached to every eligible run.
    #[default]
    Always,
    /// Attached only when the context prioritizer judges the request needs it.
    Relevant,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    /// The MCP server name agents see (`mcp__<id>__<tool>`).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// A claude.ai connector's server id (`mcpsrv_…`); empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_id: String,
    /// An https icon for the UI (claude.ai gives one per connector); empty
    /// means the UI derives one from the URL or name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon_url: String,
    /// Agent ids that share it (`claude`, `codex`).
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default = "yes")]
    pub chat: bool,
    #[serde(default)]
    pub automations: bool,
    #[serde(default)]
    pub mode: Mode,
    /// Preapprove its tools (Claude `--allowed-tools`, Codex approval mode).
    /// Headless agents never prompt, so without this its tools are denied.
    #[serde(default = "yes")]
    pub auto_approve: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// `hollow`, or `claude` / `codex` when imported from that CLI.
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// A remote server Hollow signs in to itself (`remote_auth.rs`) and
    /// serves through `hollow remote-mcp`, so it works on every Claude
    /// account and in Codex. Remote (`Http`) connectors only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub oauth: bool,
    /// The client id of an app the person registered with the server, for a
    /// server that doesn't let Hollow register itself; empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub oauth_client_id: String,
    /// That app's secret, when the server treats it as a confidential client
    /// (a Zoom Marketplace app; overnight 10-11 Y6). Stored like the other
    /// connector secrets, never sent to the UI or an agent; only with a client id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub oauth_client_secret: String,
    /// A claude.ai connector the person keeps over Hollow's own connector
    /// for the same service ("Use Hollow's instead" off, five asks B3): it
    /// isn't replaced, and Hollow's one is left out instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_claudeai: bool,
}

fn yes() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Library {
    schema_version: u32,
    #[serde(default)]
    connectors: Vec<Connector>,
}

/// A connector as the UI sees it: secrets reduced to their names.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub transport: Transport,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub server_id: String,
    pub icon_url: String,
    pub env_keys: Vec<String>,
    pub header_keys: Vec<String>,
    pub agents: Vec<String>,
    pub chat: bool,
    pub automations: bool,
    pub mode: Mode,
    pub auto_approve: bool,
    pub enabled: bool,
    pub origin: String,
    pub updated_at: String,
    /// A built-in connector's sign-in, for Settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builtin: Option<crate::builtin_connectors::BuiltinState>,
    pub oauth: bool,
    pub oauth_client_id: String,
    /// Whether a client secret is saved (never the secret itself).
    pub oauth_client_secret_set: bool,
    /// Hollow's own sign-in for an `oauth` remote connector (no tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<crate::remote_auth::Status>,
    pub keep_claudeai: bool,
}

impl From<&Connector> for ConnectorView {
    fn from(c: &Connector) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            transport: c.transport,
            command: c.command.clone(),
            args: c.args.clone(),
            url: c.url.clone(),
            server_id: c.server_id.clone(),
            icon_url: c.icon_url.clone(),
            env_keys: c.env.keys().cloned().collect(),
            header_keys: c.headers.keys().cloned().collect(),
            agents: c.agents.clone(),
            chat: c.chat,
            automations: c.automations,
            mode: c.mode,
            auto_approve: c.auto_approve && !runs_cua_driver(c),
            enabled: c.enabled,
            origin: c.origin.clone(),
            updated_at: c.updated_at.clone(),
            builtin: None,
            oauth: c.oauth,
            oauth_client_id: c.oauth_client_id.clone(),
            oauth_client_secret_set: !c.oauth_client_secret.is_empty(),
            remote: None,
            keep_claudeai: c.keep_claudeai,
        }
    }
}

/// A server configured inside an agent CLI, offered for import.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub name: String,
    /// `claude`, `codex` or `claudeai`.
    pub source: String,
    pub transport: Transport,
    pub command: String,
    pub url: String,
    pub icon_url: String,
    /// Already in the library under this id.
    pub in_library: bool,
    /// Why it can't be imported (an unsupported transport), if so.
    pub unsupported: Option<String>,
    /// Something to know before using it (a claude.ai connector that still
    /// needs signing in on claude.ai).
    pub note: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub connectors: Vec<ConnectorView>,
    pub discovered: Vec<Discovered>,
    pub problems: Vec<String>,
    /// Why claude.ai connectors could not be listed, if they couldn't.
    pub claudeai_error: Option<String>,
    /// Whose claude.ai connectors these are: Claude Code's own login. A
    /// connector signed in under another claude.ai account or organization
    /// shows "Connected" on claude.ai and still needs signing in here.
    pub claudeai_login: Option<ClaudeAiLogin>,
}

/// The claude.ai account and organization Claude Code's own login belongs
/// to (`~/.claude.json` → `oauthAccount`; no tokens).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAiLogin {
    pub email: String,
    pub organization: Option<String>,
}

impl ClaudeAiLogin {
    /// `jo@x.com in Acme`, or just the email.
    pub fn describe(&self) -> String {
        match &self.organization {
            Some(org) => format!("{} in {org}", self.email),
            None => self.email.clone(),
        }
    }
}

/// Save input. `env`/`headers` map a name to a new value, or to null to keep
/// the stored value (the UI never receives values to send back).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub server_id: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default = "yes")]
    pub chat: bool,
    #[serde(default)]
    pub automations: bool,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "yes")]
    pub auto_approve: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Left out (an agent's `save_connector` that only flips a switch):
    /// the saved connector's value is kept (review B2 #3).
    #[serde(default)]
    pub oauth: Option<bool>,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    /// Left out keeps the saved secret (unless the client id changes);
    /// `""` clears it.
    #[serde(default)]
    pub oauth_client_secret: Option<String>,
    /// "Use Hollow's instead" off; left out keeps the saved choice.
    #[serde(default)]
    pub keep_claudeai: Option<bool>,
}

// ── storage ────────────────────────────────────────────────────────────────

/// Whether two connector URLs are the same address (trimmed). A saved
/// client secret stays only with the exact address it was saved for: the
/// server's metadata decides where the secret is sent at each sign-in, and
/// on a shared host another path may name another authorization server
/// (B2 review).
fn same_address(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

fn library_path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn load_from(dir: &Path) -> Result<Vec<Connector>, String> {
    let path = library_path(dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Connectors library unreadable: {e}")),
    };
    let lib: Library = serde_json::from_str(&text)
        .map_err(|e| format!("Connectors library needs repair ({}): {e}", path.display()))?;
    if lib.schema_version != SCHEMA {
        return Err(format!(
            "Unsupported connectors library version {}",
            lib.schema_version
        ));
    }
    Ok(lib.connectors)
}

/// The library with every built-in connector present: one the user never
/// changed is its default (on, chats, when relevant, Claude and Codex). A
/// stored row for a built-in this build doesn't have (Hollow's own Notion
/// connector, removed 2026-11-10) is left out.
fn with_builtins(mut all: Vec<Connector>) -> Vec<Connector> {
    all.retain(|c| {
        c.transport != Transport::Builtin || crate::builtin_connectors::get(&c.id).is_some()
    });
    for b in crate::builtin_connectors::ALL {
        if !all.iter().any(|c| c.id == b.id) {
            all.push(Connector {
                id: b.id.into(),
                name: b.name.into(),
                description: b.description.into(),
                transport: Transport::Builtin,
                agents: vec!["claude".into(), "codex".into()],
                chat: true,
                automations: false,
                mode: Mode::Relevant,
                auto_approve: b.auto_approve,
                enabled: true,
                origin: "hollow".into(),
                ..Default::default()
            });
        }
    }
    all
}

fn save_to(dir: &Path, connectors: &[Connector]) -> Result<(), String> {
    let value = serde_json::to_value(Library {
        schema_version: SCHEMA,
        connectors: connectors.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    let map: BTreeMap<String, Value> = serde_json::from_value(value).map_err(|e| e.to_string())?;
    // It holds connector secrets: 0600 on every save, not only on the first
    // (a file an older Hollow wrote 0644 is tightened; B2, #537 review).
    crate::setup::publish_private_config(&library_path(dir), &map)
}

/// Every library connector's id and name, built-ins included, for Settings,
/// Sources' knowledge references (H4a). Never a secret.
pub(crate) fn library_names_in(dir: &Path) -> Result<Vec<(String, String)>, String> {
    Ok(with_builtins(load_from(dir)?)
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect())
}

/// Every library connector, built-ins included (CD1's hygiene table reads
/// their command, arguments and environment; secrets stay where they are).
pub(crate) fn library_in(dir: &Path) -> Result<Vec<Connector>, String> {
    Ok(with_builtins(load_from(dir)?))
}

/// [`library_in`] for the app's own library.
pub(crate) fn library() -> Result<Vec<Connector>, String> {
    library_in(&app_dir()?)
}

/// [`library_names_in`] for the app's own library.
pub(crate) fn library_names() -> Result<Vec<(String, String)>, String> {
    library_names_in(&app_dir()?)
}

fn app_dir() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        Err("tests pass an explicit connectors dir".into())
    }
    #[cfg(not(test))]
    {
        crate::agent_settings::app_data_dir()
    }
}

// ── validation ─────────────────────────────────────────────────────────────

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(48).collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && !id.starts_with("hollow")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn valid_env_key(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && k.len() <= 128
}

fn valid_header(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 128
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

fn valid_server_id(id: &str) -> bool {
    id.len() > 7
        && id.len() <= 64
        && id.starts_with("mcpsrv_")
        && id[7..].bytes().all(|b| b.is_ascii_alphanumeric())
}

fn one_line(v: &str) -> bool {
    !v.contains(['\n', '\r', '\0'])
}

fn merge_secrets(
    incoming: &BTreeMap<String, Option<String>>,
    stored: &BTreeMap<String, String>,
    what: &str,
    valid: fn(&str) -> bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (key, value) in incoming {
        let key = key.trim();
        if !valid(key) {
            return Err(format!("Invalid {what} name: {key}"));
        }
        let value = match value {
            Some(v) => v.clone(),
            None => stored
                .get(key)
                .cloned()
                .ok_or_else(|| format!("Enter a value for {key}"))?,
        };
        if !one_line(&value) || value.len() > 8192 {
            return Err(format!("{key} needs a single-line value"));
        }
        out.insert(key.to_string(), value);
    }
    Ok(out)
}

fn validate(
    input: &ConnectorInput,
    existing: Option<&Connector>,
    taken: &[&str],
) -> Result<Connector, String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 60 || !one_line(name) {
        return Err("Give the connector a name (up to 60 characters)".into());
    }
    let id = match existing {
        Some(c) => c.id.clone(),
        None => {
            let id = input
                .id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| slug(name));
            if !valid_id(&id) {
                return Err("Use a name with letters or digits (it becomes the connector's id; ids can't start with \"hollow\")".into());
            }
            if taken.contains(&id.as_str()) {
                return Err(format!("A connector with the id {id} already exists"));
            }
            id
        }
    };
    let description = input.description.trim().to_string();
    if description.chars().count() > 400 || !one_line(&description) {
        return Err("Keep the description to one line (up to 400 characters)".into());
    }
    let mut server_id = String::new();
    let builtin = existing.and_then(|c| crate::builtin_connectors::get(&c.id));
    if (input.transport == Transport::Builtin) != builtin.is_some() {
        return Err(
            "Built-in connectors come with Hollow; they can't be added or turned into another kind"
                .into(),
        );
    }
    let (command, args, url) = match input.transport {
        Transport::Builtin => (String::new(), Vec::new(), String::new()),
        Transport::Stdio => {
            let command = input.command.trim().to_string();
            if command.is_empty() || !one_line(&command) || command.len() > 1024 {
                return Err("Enter the command that starts the connector (for example npx)".into());
            }
            if input.args.len() > 64 || input.args.iter().any(|a| !one_line(a) || a.len() > 2048) {
                return Err("Arguments must be single lines (at most 64)".into());
            }
            (command, input.args.clone(), String::new())
        }
        Transport::Http => {
            let url = input.url.trim().to_string();
            let parsed = reqwest::Url::parse(&url)
                .map_err(|_| "Enter the connector's full URL".to_string())?;
            let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"));
            if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
                return Err("Use an https:// URL (http:// only for localhost)".into());
            }
            (String::new(), Vec::new(), url)
        }
        Transport::ClaudeAi => {
            let url = input.url.trim().to_string();
            if reqwest::Url::parse(&url)
                .map(|u| u.scheme() != "https")
                .unwrap_or(true)
            {
                return Err(
                    "A claude.ai connector needs its https:// URL; import it from claude.ai".into(),
                );
            }
            server_id = match input.server_id.trim() {
                "" => existing.map(|c| c.server_id.clone()).unwrap_or_default(),
                id => id.to_string(),
            };
            if !valid_server_id(&server_id) {
                return Err(
                    "Import claude.ai connectors from the list below; they need their claude.ai id"
                        .into(),
                );
            }
            (String::new(), Vec::new(), url)
        }
    };
    let oauth = input
        .oauth
        .unwrap_or_else(|| existing.is_some_and(|c| c.oauth));
    let oauth_client_id = input
        .oauth_client_id
        .clone()
        .unwrap_or_else(|| {
            existing
                .map(|c| c.oauth_client_id.clone())
                .unwrap_or_default()
        })
        .trim()
        .to_string();
    // The approved mail server only on Hollow's sign-in, where its writes ask
    // on Hollow's card (10-11 B4): never a plain URL connector beside it.
    if input.transport == Transport::Http && !oauth && crate::remote_auth::approved_mail(&url) {
        return Err("Superhuman Mail connects on Hollow's sign-in, where sending and changes ask first: tick Sign in with Hollow".into());
    }
    if oauth {
        if input.transport != Transport::Http {
            return Err("Only a remote (URL) connector can use Hollow's sign-in".into());
        }
        if crate::remote_auth::messaging_host(&url) {
            return Err("Hollow doesn't connect to mail, Slack or Teams".into());
        }
        if crate::remote_auth::anthropic_hosted(&url) {
            return Err(
                "This address only works through claude.ai; Hollow can't sign in to it".into(),
            );
        }
        // The bridge sends only Hollow's token, so a header would be
        // dropped without a word (review B2 #4).
        if !input.headers.is_empty() {
            return Err(
                "A connector on Hollow's sign-in sends no headers of its own; remove them".into(),
            );
        }
        if oauth_client_id.len() > 200 || !oauth_client_id.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("A client id is one line of up to 200 characters, no spaces".into());
        }
    }
    // A saved secret belongs to the client id and the address it was saved
    // with: a new id, or a new URL, without a new secret drops it rather
    // than sending it to someone else (B2, #537 review).
    let oauth_client_secret = match input.oauth_client_secret.as_deref().map(str::trim) {
        Some(s) => s.to_string(),
        None => existing
            .filter(|c| c.oauth_client_id == oauth_client_id && same_address(&c.url, &input.url))
            .map(|c| c.oauth_client_secret.clone())
            .unwrap_or_default(),
    };
    if oauth && !oauth_client_secret.is_empty() {
        // HTTP Basic joins them with `:`, so an id holding one can't be read.
        if oauth_client_id.contains(':') {
            return Err("A client id used with a secret can't contain a colon".into());
        }
        if oauth_client_id.is_empty() {
            return Err(
                "A client secret goes with a client id; add the app's client id too".into(),
            );
        }
        if oauth_client_secret.len() > 512
            || !oauth_client_secret.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("A client secret is one line of up to 512 characters, no spaces".into());
        }
    }
    let empty = BTreeMap::new();
    let env = merge_secrets(
        &input.env,
        existing.map(|c| &c.env).unwrap_or(&empty),
        "environment variable",
        valid_env_key,
    )?;
    let headers = merge_secrets(
        &input.headers,
        existing.map(|c| &c.headers).unwrap_or(&empty),
        "header",
        valid_header,
    )?;
    let mut agents: Vec<String> = Vec::new();
    for a in &input.agents {
        if !matches!(a.as_str(), "claude" | "codex" | "antigravity") {
            return Err(format!("{a} can't use shared connectors"));
        }
        if a != "claude" && input.transport == Transport::ClaudeAi {
            return Err("claude.ai connectors only work with Claude".into());
        }
        if !agents.contains(a) {
            agents.push(a.clone());
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let (name, description) = match builtin {
        Some(b) => (b.name.to_string(), b.description.to_string()),
        None => (name.to_string(), description),
    };
    // A Cua Driver connector is never preapproved (CU9).
    let auto_approve = input.auto_approve && !cua_driver_parts(input.transport, &command, &args);
    Ok(Connector {
        id,
        name,
        description,
        transport: input.transport,
        command,
        args,
        env: if input.transport == Transport::Stdio {
            env
        } else {
            BTreeMap::new()
        },
        url,
        headers: if input.transport == Transport::Http {
            headers
        } else {
            BTreeMap::new()
        },
        server_id,
        icon_url: existing.map(|c| c.icon_url.clone()).unwrap_or_default(),
        agents,
        chat: input.chat,
        automations: input.automations,
        mode: input.mode,
        auto_approve,
        enabled: input.enabled,
        origin: existing
            .map(|c| c.origin.clone())
            .unwrap_or_else(|| "hollow".into()),
        created_at: existing
            .map(|c| c.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
        oauth,
        oauth_client_secret: if oauth && !oauth_client_id.is_empty() {
            oauth_client_secret
        } else {
            String::new()
        },
        oauth_client_id: if oauth {
            oauth_client_id
        } else {
            String::new()
        },
        keep_claudeai: input.transport == Transport::ClaudeAi
            && input
                .keep_claudeai
                .unwrap_or_else(|| existing.is_some_and(|c| c.keep_claudeai)),
    })
}

// ── library operations ─────────────────────────────────────────────────────

pub fn save_in(dir: &Path, input: &ConnectorInput) -> Result<ConnectorView, String> {
    let mut all = load_from(dir)?;
    // A built-in is stored only once the user changes one of its switches.
    if let Some(b) = input.id.as_deref().and_then(crate::builtin_connectors::get) {
        if !all.iter().any(|c| c.id == b.id) {
            all.extend(
                with_builtins(Vec::new())
                    .into_iter()
                    .filter(|c| c.id == b.id),
            );
        }
    }
    let editing = input
        .id
        .as_deref()
        .and_then(|id| all.iter().position(|c| c.id == id));
    let taken: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
    let connector = validate(input, editing.map(|i| &all[i]), &taken)?;
    // A sign-in made with other app credentials, or for another address, is
    // forgotten, so a removed or replaced secret doesn't go on refreshing
    // tokens (Y6 review; the address since B2).
    let credentials_changed = editing.is_some_and(|i| {
        all[i].oauth_client_id != connector.oauth_client_id
            || all[i].oauth_client_secret != connector.oauth_client_secret
            || !same_address(&all[i].url, &connector.url)
    });
    match editing {
        Some(i) => all[i] = connector.clone(),
        None => {
            if all.len() >= MAX_CONNECTORS {
                return Err(format!(
                    "The library holds at most {MAX_CONNECTORS} connectors"
                ));
            }
            all.push(connector.clone());
        }
    }
    // The old sign-in goes first, under its lock, and the library is written
    // while that lock is held: a failed delete leaves the old credentials
    // saved (so the next save tries again) instead of a token that goes on
    // refreshing with a removed secret (B2 review).
    if credentials_changed {
        let _lock = crate::remote_auth::lock_blocking(dir, &connector.id)?;
        crate::remote_auth::delete(dir, &connector.id)?;
        save_to(dir, &all)?;
    } else {
        save_to(dir, &all)?;
    }
    Ok(ConnectorView::from(&connector))
}

pub fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    if crate::builtin_connectors::get(id).is_some() {
        return Err("Built-in connectors can't be removed; switch it off instead".into());
    }
    let mut all = load_from(dir)?;
    let before = all.len();
    all.retain(|c| c.id != id);
    if all.len() == before {
        return Err(format!("No connector {id}"));
    }
    save_to(dir, &all)?;
    // Its Hollow sign-in goes with it (a no-op for any other kind), under
    // its lock so a bridge refreshing now can't write it back (review B2 #9).
    let _lock = crate::remote_auth::lock_blocking(dir, id)?;
    crate::remote_auth::delete(dir, id)
}

/// The claude.ai connector `id` as a remote connector on Hollow's own
/// sign-in (five asks B2): the same address and switches, shared with Claude
/// and Codex, attached only once the person signs in (and from then on it
/// replaces the claude.ai one, see [`supersedes`]). Returns the library's
/// existing one when it already has a Hollow sign-in for that host. Never
/// for mail, Slack or Teams, or an address only Anthropic's proxy serves.
pub fn use_hollow_sign_in_in(dir: &Path, id: &str) -> Result<ConnectorView, String> {
    let mut all = load_from(dir)?;
    let source = all
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("No connector {id}"))?
        .clone();
    if source.transport != Transport::ClaudeAi {
        return Err("Only a claude.ai connector can move to Hollow's sign-in".into());
    }
    if crate::remote_auth::messaging_host(&source.url) {
        return Err(
            "Hollow doesn't connect to mail, Slack or Teams; this one stays on claude.ai".into(),
        );
    }
    if crate::remote_auth::anthropic_hosted(&source.url) {
        return Err(format!(
            "{} is served by Anthropic for claude.ai only; Hollow can't sign in to it",
            source.name
        ));
    }
    let h = host(&source.url);
    if let Some(existing) = all
        .iter()
        .find(|c| c.transport == Transport::Http && c.oauth && host(&c.url) == h)
    {
        return Ok(ConnectorView::from(existing));
    }
    if all.len() >= MAX_CONNECTORS {
        return Err(format!(
            "The library holds at most {MAX_CONNECTORS} connectors"
        ));
    }
    let name = source.name.trim();
    let name = name
        .len()
        .checked_sub(" for Claude".len())
        .filter(|&at| name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(" for Claude"))
        .map(|at| name[..at].trim())
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
        .to_string();
    let base = slug(&name).replace('-', "_");
    let base = if valid_id(&base) {
        base
    } else {
        format!("mcp_{base}")
    };
    let id = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}_{n}")))
        .find(|candidate| !all.iter().any(|c| &c.id == candidate))
        .unwrap_or(base);
    let now = chrono::Utc::now().to_rfc3339();
    let connector = Connector {
        id,
        name,
        description: source.description.clone(),
        transport: Transport::Http,
        url: source.url.clone(),
        icon_url: source.icon_url.clone(),
        agents: vec!["claude".into(), "codex".into()],
        chat: source.chat,
        automations: source.automations,
        mode: source.mode,
        auto_approve: source.auto_approve,
        enabled: true,
        origin: "hollow".into(),
        created_at: now.clone(),
        updated_at: now,
        oauth: true,
        ..Default::default()
    };
    // The claude.ai row stops being kept, or the new one would be left out.
    if let Some(row) = all.iter_mut().find(|c| c.id == source.id) {
        row.keep_claudeai = false;
    }
    all.push(connector.clone());
    save_to(dir, &all)?;
    Ok(ConnectorView::from(&connector))
}

/// Copy a server from an agent's own config into the library, shared with
/// both agents. Hollow never edits the agent's file.
pub fn import_in(
    dir: &Path,
    home: &Path,
    claudeai: &[ClaudeAiServer],
    source: &str,
    name: &str,
) -> Result<ConnectorView, String> {
    let found = discover_raw(home, claudeai)
        .into_iter()
        .find(|(d, _)| d.source == source && d.name == name)
        .ok_or_else(|| format!("{name} is no longer configured in {source}"))?;
    let (discovered, connector) = found;
    if let Some(why) = discovered.unsupported {
        return Err(why);
    }
    let mut all = load_from(dir)?;
    let id = slug(name).replace('-', "_");
    let id = if valid_id(&id) {
        id
    } else {
        format!("mcp_{}", slug(name).replace('-', "_"))
    };
    if all.iter().any(|c| c.id == id) {
        return Err(format!("{id} is already in the library"));
    }
    if all.len() >= MAX_CONNECTORS {
        return Err(format!(
            "The library holds at most {MAX_CONNECTORS} connectors"
        ));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let cua_driver = runs_cua_driver(&connector);
    let agents = if connector.transport == Transport::ClaudeAi {
        vec!["claude".into()]
    } else {
        vec!["claude".into(), "codex".into()]
    };
    let connector = Connector {
        id,
        name: name.to_string(),
        agents,
        chat: true,
        automations: false,
        mode: Mode::Always,
        auto_approve: !cua_driver,
        description: if cua_driver {
            CUA_DRIVER_WARNING.to_string()
        } else {
            connector.description.clone()
        },
        enabled: true,
        origin: source.to_string(),
        created_at: now.clone(),
        updated_at: now,
        ..connector
    };
    all.push(connector.clone());
    save_to(dir, &all)?;
    Ok(ConnectorView::from(&connector))
}

pub fn view_in(
    dir: &Path,
    home: &Path,
    claudeai: &[ClaudeAiServer],
    claudeai_error: Option<String>,
) -> LibraryView {
    let mut problems = Vec::new();
    let connectors = with_builtins(load_from(dir).unwrap_or_else(|e| {
        problems.push(e);
        Vec::new()
    }));
    for c in connectors
        .iter()
        .filter(|c| c.enabled && c.transport == Transport::Stdio && relative_command(&c.command))
    {
        problems.push(format!(
            "{}: its command is a relative path ({}), so it can't start; edit it to the full path",
            c.name, c.command
        ));
    }
    let discovered = discover_raw(home, claudeai)
        .into_iter()
        .map(|(mut d, c)| {
            d.in_library = connectors.iter().any(|x| {
                x.id == slug(&d.name).replace('-', "_")
                    || (!c.server_id.is_empty() && x.server_id == c.server_id)
                    || (c.server_id.is_empty()
                        && x.server_id.is_empty()
                        && x.command == c.command
                        && x.url == c.url
                        && x.args == c.args)
            });
            d
        })
        .collect();
    let claudeai_login = claudeai_login_in(home);
    let views = connectors
        .iter()
        .map(|c| {
            let mut v = ConnectorView::from(c);
            // Tests never read config.json, the Keychain or ~/.plaud.
            if let Some(own) = own_instead(&connectors, c) {
                v.builtin = crate::builtin_connectors::aside(&c.id, &own.name);
            } else if c.transport == Transport::Builtin && !cfg!(test) {
                v.builtin = crate::builtin_connectors::state(&c.id);
            }
            if c.transport == Transport::Http && c.oauth {
                v.remote = Some(crate::remote_auth::status(dir, &c.id, &c.url));
            }
            v
        })
        .collect();
    LibraryView {
        connectors: views,
        discovered,
        problems,
        claudeai_error,
        claudeai_login,
    }
}

// ── discovery ──────────────────────────────────────────────────────────────

fn read_small(path: &Path, cap: u64) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > cap {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Servers from `~/.claude.json` (`mcpServers`, user scope),
/// `~/.codex/config.toml` and the user's claude.ai connectors, each with the
/// connector it would import as.
fn discover_raw(home: &Path, claudeai: &[ClaudeAiServer]) -> Vec<(Discovered, Connector)> {
    let mut out = Vec::new();
    let needs_auth = claudeai_needs_auth(home);
    let as_login = claudeai_login_in(home)
        .map(|l| format!(" as {}", l.describe()))
        .unwrap_or_default();
    for server in claudeai {
        let connector = Connector {
            name: server.name.clone(),
            transport: Transport::ClaudeAi,
            url: server.url.clone(),
            server_id: server.id.clone(),
            icon_url: server.icon_url.clone(),
            ..Default::default()
        };
        let note = if !server.eligible {
            Some("claude.ai says this connector isn't available to Claude Code".to_string())
        } else if needs_auth.contains(&server.id) {
            Some(format!(
                "Sign in to it on claude.ai (Settings → Connectors){as_login} first"
            ))
        } else {
            None
        };
        let mut d = discovered(&server.name, "claudeai", &connector, None);
        d.note = note;
        out.push((d, connector));
    }
    let claude_file = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|_| !cfg!(test))
        .map(|d| PathBuf::from(d).join(".claude.json"))
        .unwrap_or_else(|| home.join(".claude.json"));
    if let Some(text) = read_small(&claude_file, 64 * 1024 * 1024) {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if let Some(servers) = value.get("mcpServers").and_then(Value::as_object) {
                for (name, server) in servers {
                    let kind = server.get("type").and_then(Value::as_str).unwrap_or(
                        if server.get("url").is_some() {
                            "http"
                        } else {
                            "stdio"
                        },
                    );
                    let transport = if kind == "stdio" {
                        Transport::Stdio
                    } else {
                        Transport::Http
                    };
                    let connector = Connector {
                        name: name.clone(),
                        transport,
                        command: server
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        args: server
                            .get("args")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        env: string_map(server.get("env")),
                        url: server
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        headers: string_map(server.get("headers")),
                        ..Default::default()
                    };
                    let unsupported = (kind == "sse")
                        .then(|| "SSE connectors can't be shared with Codex; add its streamable HTTP URL instead".to_string())
                        .or_else(|| relative_unsupported(&connector));
                    out.push((
                        discovered(name, "claude", &connector, unsupported),
                        connector,
                    ));
                }
            }
        }
    }
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|_| !cfg!(test))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    if let Some(text) = read_small(&codex_home.join("config.toml"), 4 * 1024 * 1024) {
        if let Ok(value) = text.parse::<toml::Table>() {
            if let Some(servers) = value.get("mcp_servers").and_then(toml::Value::as_table) {
                for (name, server) in servers {
                    let Some(server) = server.as_table() else {
                        continue;
                    };
                    let get = |k: &str| {
                        server
                            .get(k)
                            .and_then(toml::Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    let table = |k: &str| -> BTreeMap<String, String> {
                        server
                            .get(k)
                            .and_then(toml::Value::as_table)
                            .map(|t| {
                                t.iter()
                                    .filter_map(|(k, v)| {
                                        v.as_str().map(|s| (k.clone(), s.to_string()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    let url = get("url");
                    // Codex runs a relative command from `cwd`; Hollow can
                    // only honor that when `cwd` is absolute.
                    let cwd = get("cwd");
                    let command = match get("command") {
                        c if relative_command(&c) && cwd.starts_with('/') => {
                            Path::new(&cwd).join(&c).to_string_lossy().into_owned()
                        }
                        c => c,
                    };
                    let connector = Connector {
                        name: name.clone(),
                        transport: if url.is_empty() {
                            Transport::Stdio
                        } else {
                            Transport::Http
                        },
                        command,
                        args: server
                            .get("args")
                            .and_then(toml::Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        env: table("env"),
                        headers: table("http_headers"),
                        url,
                        ..Default::default()
                    };
                    let mut d =
                        discovered(name, "codex", &connector, relative_unsupported(&connector));
                    if codex_computer_use(&connector) {
                        d.note = Some(COMPUTER_USE_REPLACED.into());
                    } else if d.note.is_none()
                        && server.get("enabled").and_then(toml::Value::as_bool) == Some(false)
                    {
                        d.note = Some("Turned off in Codex's config".into());
                    }
                    out.push((d, connector));
                }
            }
        }
    }
    out
}

fn relative_unsupported(c: &Connector) -> Option<String> {
    (c.transport == Transport::Stdio && relative_command(&c.command)).then(|| {
        format!("Its command is a path relative to that agent's own folder ({}); add it by hand with the full path", c.command)
    })
}

fn discovered(name: &str, source: &str, c: &Connector, unsupported: Option<String>) -> Discovered {
    Discovered {
        name: name.to_string(),
        source: source.to_string(),
        transport: c.transport,
        command: c.command.clone(),
        url: c.url.clone(),
        icon_url: c.icon_url.clone(),
        in_library: false,
        unsupported,
        note: runs_cua_driver(c).then(|| CUA_DRIVER_WARNING.to_string()),
    }
}

/// The discovered-list note on Codex's own Computer Use app (CU9). It stays
/// unimportable (its command is relative) and is never attached.
pub(crate) const COMPUTER_USE_REPLACED: &str =
    "Replaced by Computer control (Settings, Connectors, Built into Hollow)";

/// The warning on a connector that runs Cua Driver (CU9), in the discovered
/// list and as the imported connector's description.
pub(crate) const CUA_DRIVER_WARNING: &str = "Runs Cua Driver: its tools bypass Hollow's cards, modes and audit, so Hollow never preapproves them. Computer control (Settings, Connectors, Built into Hollow) is the supported way to let agents use the desktop.";

/// OpenAI's Computer Use app as Codex's config names it.
fn codex_computer_use(c: &Connector) -> bool {
    c.command.contains("Codex Computer Use.app")
}

/// Whether a stdio connector runs Cua Driver, by its command or any
/// argument (`npx`, `uvx` or a shell wrapper): the file or package name
/// `cua-driver` or `CuaDriver`, any case, with any version suffix. Such a
/// connector is never preapproved (CU9): attached raw, every Driver tool
/// would run with no card in every mode.
pub(crate) fn runs_cua_driver(c: &Connector) -> bool {
    cua_driver_parts(c.transport, &c.command, &c.args)
}

fn cua_driver_parts(transport: Transport, command: &str, args: &[String]) -> bool {
    transport == Transport::Stdio
        && std::iter::once(command)
            .chain(args.iter().map(String::as_str))
            .any(names_cua_driver)
}

/// Whether any shell word in `text` names Cua Driver: split on whitespace
/// and shell punctuation (`sh -c "cua-driver mcp"`), the last path part,
/// cut at a version or extras suffix (`@`, `==`, `>=`, `~=`, `[`).
fn names_cua_driver(text: &str) -> bool {
    text.split(|ch: char| ch.is_whitespace() || ";&|'\"()`$".contains(ch))
        .filter(|w| !w.is_empty())
        .any(|word| {
            let base = word.rsplit('/').next().unwrap_or("");
            let base = base
                .split(|ch: char| "@=<>~![".contains(ch))
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let base = base.strip_suffix(".exe").unwrap_or(&base);
            matches!(base, "cua-driver" | "cuadriver" | "cua_driver")
        })
}

/// One connector the user added on claude.ai.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeAiServer {
    pub id: String,
    pub name: String,
    pub url: String,
    pub icon_url: String,
    pub eligible: bool,
}

/// The `data` rows of `GET /v1/mcp_servers` (ids, names and https URLs
/// only; everything else in the row is ignored).
pub fn parse_claudeai(body: &Value) -> Vec<ClaudeAiServer> {
    let mut out = Vec::new();
    for row in body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let s = |k: &str| {
            row.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let (id, url) = (s("id"), s("url"));
        let name = match s("display_name") {
            n if !n.is_empty() => n,
            _ => s("label"),
        };
        if !valid_server_id(&id)
            || name.is_empty()
            || !one_line(&name)
            || !url.starts_with("https://")
        {
            continue;
        }
        if out.iter().any(|x: &ClaudeAiServer| x.id == id) {
            continue;
        }
        let eligible = row.get("eligible").and_then(Value::as_bool).unwrap_or(true);
        let icon_url = Some(s("icon_url"))
            .filter(|u| u.starts_with("https://") && u.len() <= 512 && one_line(u))
            .unwrap_or_default();
        out.push(ClaudeAiServer {
            id,
            name: name.chars().take(60).collect(),
            url,
            icon_url,
            eligible,
        });
    }
    out
}

/// Claude Code's own login, as [`ClaudeAiLogin`]. `None` when it isn't
/// signed in with claude.ai (an API key, or no login).
fn claudeai_login_in(home: &Path) -> Option<ClaudeAiLogin> {
    let file = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|_| !cfg!(test))
        .map(|d| PathBuf::from(d).join(".claude.json"))
        .unwrap_or_else(|| home.join(".claude.json"));
    let value: Value = serde_json::from_str(&read_small(&file, 64 * 1024 * 1024)?).ok()?;
    let account = value.get("oauthAccount")?;
    let field = |k: &str| {
        account
            .get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && one_line(s))
            .map(|s| s.chars().take(120).collect::<String>())
    };
    Some(ClaudeAiLogin {
        email: field("emailAddress")?,
        organization: field("organizationName"),
    })
}

/// [`claudeai_login_in`] for this Mac's home folder.
pub fn claudeai_login() -> Option<ClaudeAiLogin> {
    claudeai_login_in(&dirs::home_dir()?)
}

/// Server ids Claude Code last saw needing a claude.ai sign-in
/// (`~/.claude/mcp-needs-auth-cache.json`, `{"claude.ai <name>": {"id"}}`).
fn claudeai_needs_auth(home: &Path) -> Vec<String> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|_| !cfg!(test))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let Some(text) = read_small(&dir.join("mcp-needs-auth-cache.json"), 1024 * 1024) else {
        return Vec::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    map.values()
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// A short-lived copy of the claude.ai list, so opening Settings twice
/// reads the Keychain and calls claude.ai once.
static CLAUDEAI_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<ClaudeAiServer>)>> =
    std::sync::Mutex::new(None);
const CLAUDEAI_CACHE_SECS: u64 = 300;

async fn claudeai_servers(refresh: bool) -> Result<Vec<ClaudeAiServer>, String> {
    if !refresh {
        if let Some((at, list)) = CLAUDEAI_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed().as_secs() < CLAUDEAI_CACHE_SECS {
                return Ok(list.clone());
            }
        }
    }
    let body =
        crate::claude_quota::get_with_login("/v1/mcp_servers?limit=1000", CLAUDEAI_BETA).await?;
    let list = parse_claudeai(&body);
    *CLAUDEAI_CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((std::time::Instant::now(), list.clone()));
    Ok(list)
}

// ── sign-in status ─────────────────────────────────────────────────────────

/// Whether a library connector is ready to use.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    pub id: String,
    /// `connected`, `needsSignIn`, `error` (claude.ai answered with a fault),
    /// or `unchecked`: Hollow couldn't run the check at all (no readable
    /// Claude Code login, no network), which says nothing about the connector.
    pub state: String,
    pub detail: Option<String>,
    /// When the probe ran (RFC 3339), stamped by [`check_claudeai`].
    pub checked_at: Option<String>,
    /// The claude.ai login it ran as (`jo@x.com in Acme`), when known. A check
    /// under one login never certifies another (next-level P16).
    pub login: Option<String>,
}

impl ConnectorStatus {
    fn new(id: &str, state: &str, detail: Option<String>) -> Self {
        ConnectorStatus {
            id: id.to_string(),
            state: state.to_string(),
            detail,
            checked_at: None,
            login: None,
        }
    }
}

const PROXY_BASE: &str = "https://mcp-proxy.anthropic.com/v1/mcp/";

/// An MCP `initialize` answer from Anthropic's proxy, as a status: 200 is
/// signed in; 401/403 means the connector needs signing in on claude.ai.
pub fn status_from(id: &str, http: u16, body: &str) -> ConnectorStatus {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.pointer("/error/message")
            .and_then(Value::as_str)
            .map(|m| crate::claude_runner::truncate(m, 160).to_string())
    });
    let (state, detail) = match http {
        200 => ("connected", None),
        401 | 403 => (
            "needsSignIn",
            message.or_else(|| Some("claude.ai says it needs signing in".into())),
        ),
        _ => (
            "error",
            Some(message.unwrap_or_else(|| format!("claude.ai proxy returned HTTP {http}"))),
        ),
    };
    ConnectorStatus::new(id, state, detail)
}

/// Ask Anthropic's proxy whether each claude.ai connector in the library is
/// signed in (one MCP `initialize` each, in parallel, 10 s bound). Other
/// connectors have no status Hollow can read without starting them. Every
/// answer carries when it ran and the login it ran as.
pub async fn check_claudeai(connectors: &[Connector]) -> Vec<ConnectorStatus> {
    let login = claudeai_login().map(|l| l.describe());
    stamp(
        probe_claudeai(connectors).await,
        &chrono::Utc::now().to_rfc3339(),
        login.as_deref(),
    )
}

/// Record when a batch of probes ran and under which login.
fn stamp(mut out: Vec<ConnectorStatus>, at: &str, login: Option<&str>) -> Vec<ConnectorStatus> {
    for s in &mut out {
        s.checked_at = Some(at.to_string());
        s.login = login.map(str::to_string);
    }
    out
}

async fn probe_claudeai(connectors: &[Connector]) -> Vec<ConnectorStatus> {
    let targets: Vec<(String, String)> = connectors
        .iter()
        .filter(|c| c.transport == Transport::ClaudeAi && valid_server_id(&c.server_id))
        .map(|c| (c.id.clone(), c.server_id.clone()))
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    // Failing before any connector is asked is Hollow's problem, not theirs.
    let fail_all = |detail: String| {
        targets
            .iter()
            .map(|(id, _)| ConnectorStatus::new(id, "unchecked", Some(detail.clone())))
            .collect()
    };
    let auth = match crate::claude_quota::login_bearer().await {
        Ok(auth) => auth,
        Err(e) => return fail_all(e),
    };
    let client = match crate::claude_quota::login_client(std::time::Duration::from_secs(10)) {
        Ok(client) => client,
        Err(e) => return fail_all(e),
    };
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "hollow", "version": env!("CARGO_PKG_VERSION")}}});
    let mut set = tokio::task::JoinSet::new();
    for (id, server_id) in targets {
        let (client, auth, body) = (client.clone(), auth.clone(), init.clone());
        set.spawn(async move {
            let session: String = (0..32)
                .map(|_| format!("{:x}", rand::random::<u8>() % 16))
                .collect();
            let sent = client
                .post(format!("{PROXY_BASE}{server_id}"))
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(
                    reqwest::header::ACCEPT,
                    "application/json, text/event-stream",
                )
                .header("anthropic-beta", CLAUDEAI_BETA)
                .header("X-Mcp-Client-Session-Id", session)
                .json(&body)
                .send()
                .await;
            match sent {
                Ok(response) => {
                    let code = response.status().as_u16();
                    let text = if code == 200 {
                        String::new()
                    } else {
                        response.text().await.unwrap_or_default()
                    };
                    status_from(&id, code, &text)
                }
                Err(e) => ConnectorStatus::new(
                    &id,
                    "unchecked",
                    Some(format!("couldn't reach claude.ai: {e}")),
                ),
            }
        });
    }
    let mut out = Vec::new();
    while let Some(done) = set.join_next().await {
        if let Ok(status) = done {
            out.push(status);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ── attaching to runs ──────────────────────────────────────────────────────

/// One library connector attached to a run.
#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub entry: Value,
    pub auto_approve: bool,
    /// A claude.ai connector: the run must use Claude Code's own login.
    pub claude_login: bool,
    /// For a claude.ai connector: what would keep the run off the login
    /// ([`remedy_for`]), for the receipt's login row (five asks B3).
    pub remedy: Option<String>,
}

/// The mcpServers entry an agent gets. Claude and Codex read the same shape
/// (`agent::codex_owned_servers` converts it to Codex's inline table).
pub fn entry(c: &Connector) -> Value {
    let home = dirs::home_dir().unwrap_or_default();
    let dir = app_dir().ok();
    with_auth_dir(
        c,
        entry_in(c, &home, std::env::var("PATH").ok().as_deref()),
        dir.as_deref(),
    )
}

/// The subprocess may not see HOLLOW_APP_DATA_DIR (Codex clears the
/// environment), so an `oauth` connector's bridge is told where its sign-in
/// lives.
fn with_auth_dir(c: &Connector, mut e: Value, dir: Option<&Path>) -> Value {
    if let (Some(args), Some(dir)) = (e.get_mut("args").and_then(Value::as_array_mut), dir) {
        if c.transport == Transport::Http && c.oauth {
            args.push(json!("--auth-dir"));
            args.push(json!(dir.to_string_lossy()));
        }
    }
    e
}

/// A command given as a relative path (`./x/bin`): it only resolves from
/// the folder its original config ran it in, which a run never is.
fn relative_command(command: &str) -> bool {
    command.contains('/') && !command.starts_with('/')
}

/// `entry` with its seams. A Finder-launched Hollow inherits the bare system
/// PATH, so `uvx`, `npx` and the `node` behind a `#!/usr/bin/env node`
/// server are all missing (blender and plaud died with ENOENT / exit 127):
/// a stdio server gets the user's tool folders on PATH and a bare command
/// resolved against them. A PATH the connector sets itself is kept as is.
fn entry_in(c: &Connector, home: &Path, inherited: Option<&str>) -> Value {
    match c.transport {
        Transport::Stdio => {
            let mut env = c.env.clone();
            let path = env
                .get("PATH")
                .cloned()
                .unwrap_or_else(|| crate::self_install::tool_path(inherited, home));
            let command = if c.command.contains('/') {
                c.command.clone()
            } else {
                path.split(':')
                    .map(|d| Path::new(d).join(&c.command))
                    .find(|p| p.is_file())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| c.command.clone())
            };
            env.insert("PATH".into(), path);
            let mut e = Map::new();
            e.insert("command".into(), json!(command));
            e.insert("args".into(), json!(c.args));
            e.insert("env".into(), json!(env));
            Value::Object(e)
        }
        // Hollow's own sign-in: a stdio bridge that adds the token.
        Transport::Http if c.oauth => {
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            // An approved mail connector's bridge asks on Hollow's card before
            // a write (10-11 B4): it gets the approval broker's socket.
            let env = match crate::approvals::socket_path() {
                Some(sock) if crate::remote_auth::approved_mail(&c.url) => {
                    json!({ crate::approvals::ENV_SOCK: sock.to_string_lossy() })
                }
                _ => json!({}),
            };
            json!({"command": exe, "args": ["remote-mcp", "--id", c.id, "--url", c.url], "env": env})
        }
        Transport::Http => {
            let mut e = Map::new();
            e.insert("type".into(), json!("http"));
            e.insert("url".into(), json!(c.url));
            if !c.headers.is_empty() {
                e.insert("headers".into(), json!(c.headers));
            }
            Value::Object(e)
        }
        Transport::ClaudeAi => json!({"type": "claudeai-proxy", "url": c.url, "id": c.server_id}),
        Transport::Builtin => {
            // Plaud (overnight S8) runs its own installed program, with the
            // PATH and resolution a local connector gets.
            if let Some(program) = crate::builtin_connectors::get(&c.id).and_then(|b| b.program) {
                let local = Connector {
                    transport: Transport::Stdio,
                    command: program.into(),
                    ..Default::default()
                };
                return entry_in(&local, home, inherited);
            }
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let sub = crate::builtin_connectors::get(&c.id)
                .map(|b| b.subcommand)
                .unwrap_or("");
            json!({"command": exe, "args": [sub], "env": {}})
        }
    }
}

/// Eligible connectors before prioritization: enabled, shared with `agent`,
/// on for this surface. A `plan` (read-only) run gets none, since a
/// connector's tools can act outside Hollow.
pub fn eligible(
    all: &[Connector],
    agent: AgentKind,
    perm: &str,
    automation: bool,
) -> Vec<Connector> {
    eligible_with(all, agent, perm, automation, &signed_in)
}

/// Whether a connector that runs on Hollow's own sign-in has it now (no
/// network): a built-in's, or an `oauth` remote one's. True for the rest.
fn signed_in(c: &Connector) -> bool {
    match c.transport {
        Transport::Builtin => crate::builtin_connectors::ready(&c.id),
        Transport::Http if c.oauth => {
            app_dir().is_ok_and(|dir| crate::remote_auth::ready(&dir, &c.id, &c.url))
        }
        _ => true,
    }
}

/// The host a ready Hollow connector takes over from claude.ai: a
/// built-in's `supersedes_host`, or an `oauth` remote connector's own host.
fn supersedes(c: &Connector) -> Option<String> {
    match c.transport {
        Transport::Builtin => crate::builtin_connectors::get(&c.id)
            .and_then(|b| b.supersedes_host)
            .map(str::to_string),
        Transport::Http if c.oauth => host(&c.url),
        _ => None,
    }
}

fn host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// `eligible` with the sign-in check given. A connector on Hollow's own
/// sign-in (built-in, or `oauth` remote) is eligible only while it is signed
/// in, and a signed-in one leaves out the claude.ai connector for its
/// service (`supersedes`), so the run isn't pinned to Claude Code's login.
fn eligible_with(
    all: &[Connector],
    agent: AgentKind,
    perm: &str,
    automation: bool,
    ready: &dyn Fn(&Connector) -> bool,
) -> Vec<Connector> {
    // Antigravity's automations run `agy -p`, which takes no servers.
    if perm == "plan"
        || !crate::agent::capabilities(agent).owned_mcp
        || (automation && agent == AgentKind::Antigravity)
    {
        return Vec::new();
    }
    let on: Vec<&Connector> = all
        .iter()
        .filter(|c| c.enabled && c.agents.iter().any(|a| a == agent.as_str()))
        .filter(|c| c.transport != Transport::Stdio || !relative_command(&c.command))
        // Bypass preapproves every library connector whatever its switch
        // (`mode_contract::connector_preapproved`), so a raw Cua Driver
        // stays out of a bypass run altogether (CU9).
        .filter(|c| perm != "bypassPermissions" || !runs_cua_driver(c))
        .filter(|c| c.transport != Transport::ClaudeAi || agent == AgentKind::Claude)
        .filter(|c| if automation { c.automations } else { c.chat })
        .filter(|c| {
            c.transport != Transport::Builtin || crate::builtin_connectors::get(&c.id).is_some()
        })
        // A built-in the person has their own copy of stands aside (S8).
        .filter(|c| own_instead(all, c).is_none())
        .filter(|c| ready(c))
        .collect();
    // "Use Hollow's instead" off on a claude.ai connector keeps it, and
    // leaves Hollow's connector for that host out instead (five asks B3).
    let kept: Vec<String> = on
        .iter()
        // Never for the approved mail server: once Hollow's is signed in, its
        // writes go through Hollow's card, not claude.ai (10-11 B4).
        .filter(|c| {
            c.transport == Transport::ClaudeAi
                && c.keep_claudeai
                && !crate::remote_auth::approved_mail(&c.url)
        })
        .filter_map(|c| host(&c.url))
        .collect();
    let superseded: Vec<String> = on
        .iter()
        .filter_map(|c| supersedes(c))
        .filter(|h| !kept.contains(h))
        .collect();
    on.into_iter()
        .filter(|c| match c.transport {
            Transport::ClaudeAi => !host(&c.url).is_some_and(|h| superseded.contains(&h)),
            _ => !supersedes(c).is_some_and(|h| kept.contains(&h)),
        })
        .cloned()
        .collect()
}

/// The person's own connector a built-in stands aside for (overnight S8:
/// theirs wins), on or off: a local or remote one for the same service
/// (`builtin_connectors::is_own`). Never a claude.ai one.
fn own_instead<'a>(all: &'a [Connector], c: &Connector) -> Option<&'a Connector> {
    let b = crate::builtin_connectors::get(&c.id).filter(|_| c.transport == Transport::Builtin)?;
    all.iter().find(|x| {
        matches!(x.transport, Transport::Stdio | Transport::Http)
            && crate::builtin_connectors::is_own(b, &x.name, &x.command, &x.args, &x.url)
    })
}

/// What would keep a run off Claude Code's login for the claude.ai
/// connector `c` (five asks B3), knowing the library `all`: turn "Use
/// Hollow's instead" back on, turn on, sign in to or share Hollow's own
/// connector for its service, or "Use Hollow's sign-in" when there is none
/// yet. Nothing for mail/Slack/Teams or an Anthropic-only address with no
/// Hollow equivalent.
fn remedy_for(
    c: &Connector,
    all: &[Connector],
    ready: &dyn Fn(&Connector) -> bool,
) -> Option<String> {
    let h = host(&c.url)?;
    let equivalent = all
        .iter()
        .find(|x| x.id != c.id && supersedes(x).as_deref() == Some(h.as_str()));
    match equivalent {
        Some(_) if c.keep_claudeai => Some(format!("turn on Use Hollow's instead for {}", c.name)),
        Some(eq) if !eq.enabled => Some(format!("turn on Hollow's {}", eq.name)),
        Some(eq) if !ready(eq) => Some(format!("sign in to Hollow's {}", eq.name)),
        Some(eq) => Some(format!(
            "share Hollow's {} with Claude where this ran",
            eq.name
        )),
        None if crate::remote_auth::messaging_host(&c.url)
            || crate::remote_auth::anthropic_hosted(&c.url) =>
        {
            None
        }
        None => Some(format!("Use Hollow's sign-in on {}", c.name)),
    }
}

/// The receipt's remedy sentence from the run's per-connector remedies
/// (duplicates dropped), or nothing.
pub fn login_remedy<'a>(remedies: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut items: Vec<&str> = Vec::new();
    for r in remedies {
        if !items.contains(&r) {
            items.push(r);
        }
    }
    (!items.is_empty()).then(|| {
        format!(
            "Hollow's own connector would avoid it: {} (Settings → Connectors).",
            items.join("; ")
        )
    })
}

/// What [`attach`] gives one run: the connectors, and the context
/// prioritizer's decision for every "when relevant" candidate (one each,
/// with its basis and reason), for the run's record.
#[derive(Clone, Debug, Default)]
pub struct Attached {
    pub connectors: Vec<Attachment>,
    pub selection: crate::context_rank::ConnectorSelection,
    /// The Microsoft 365 token refresh, when that connector was attached
    /// (agent effectiveness F8: the run's preparation timeline).
    pub(crate) auth_warm: Option<crate::prep_timing::Mark>,
    /// The preparation deadline passed before that refresh finished (it
    /// finishes on its own; F8, O2).
    pub(crate) auth_cut: bool,
    /// Spread usage across accounts (2026-10-21): the claude.ai connectors set to
    /// Always that this run left out because Claude Code's login is out of
    /// room and the request didn't need them ([`demoted_ids`]), by name.
    pub set_aside: Vec<String>,
    /// Why: the login's reading, when any were left out.
    pub login_out: Option<crate::claude_accounts::LoginLow>,
}

/// The connectors to attach to one run: every eligible "always" connector,
/// plus the "when relevant" ones the context prioritizer picks for
/// `query` (Jev is sent only `query.request`, through `jev`, until
/// `deadline`). The Microsoft 365 token is refreshed only when that
/// connector is among those attached, after selection (agent effectiveness
/// F8, O2: before, any eligible library refreshed it, however unrelated the
/// request), and waited for until `deadline` at most.
///
/// `session`: the vault and chat conversation this turn belongs to, whose
/// last Jev relevance decision may be reused (S28,
/// `context_rank::select_connectors_in_session`; keyed on both, since a
/// copied vault keeps its session keys); `None` (automations) always
/// selects afresh. Only relevance is reused: the library, and with it every
/// connector's `auto_approve`, is read again on every call.
pub async fn attach(
    agent: AgentKind,
    perm: &str,
    automation: bool,
    query: &crate::context_rank::ConnectorQuery,
    deadline: &crate::prep_deadline::PrepDeadline,
    jev: &crate::jev::Target<'_>,
    session: Option<(&Path, &str)>,
) -> Attached {
    let all = match app_dir().and_then(|dir| load_from(&dir)) {
        Ok(all) => with_builtins(all),
        Err(e) => {
            eprintln!("[connectors] library not attached: {e}");
            return Attached::default();
        }
    };
    let key = session.map(|(vault, s)| crate::context_rank::session_key(vault, s));
    let cache = key
        .as_deref()
        .map(|key| (key, &crate::context_rank::SESSIONS));
    let login_out = login_out_for(agent, automation);
    attach_from(
        &all, agent, perm, automation, query, deadline, jev, cache, login_out,
    )
    .await
}

/// Warm relevance scores only, never attachments, auth refresh or the
/// conversation's remembered decision. Final attachment rereads everything.
pub(crate) async fn prepare_relevance(
    agent: AgentKind,
    perm: &str,
    session: &str,
    query: &crate::context_rank::ConnectorQuery,
    deadline: &crate::prep_deadline::PrepDeadline,
    jev: &crate::jev::Target<'_>,
    allowed: &(dyn Fn() -> bool + Send + Sync),
) {
    let Ok(all) = app_dir().and_then(|dir| load_from(&dir)).map(with_builtins) else {
        return;
    };
    let eligible = eligible(&all, agent, perm, false);
    let demoted = demoted_ids(&eligible, login_out_for(agent, false).is_some());
    let candidates: Vec<_> = eligible
        .iter()
        .filter(|c| c.mode == Mode::Relevant || demoted.contains(&c.id))
        .map(|c| crate::context_rank::ConnectorCandidate {
            id: &c.id,
            name: &c.name,
            description: &c.description,
        })
        .collect();
    if allowed() && !crate::context_rank::reuses_connectors(session, query, &candidates, jev) {
        let _ = crate::context_rank::select_connectors(query, &candidates, deadline, jev).await;
    }
}

/// [`attach`] for the library `all` (just read), with the conversation's
/// relevance cache and the login's state ([`login_out_for`]) given.
#[allow(clippy::too_many_arguments)]
async fn attach_from(
    all: &[Connector],
    agent: AgentKind,
    perm: &str,
    automation: bool,
    query: &crate::context_rank::ConnectorQuery,
    deadline: &crate::prep_deadline::PrepDeadline,
    jev: &crate::jev::Target<'_>,
    session: Option<(&str, &crate::context_rank::SessionCache)>,
    login_out: Option<crate::claude_accounts::LoginLow>,
) -> Attached {
    let eligible = eligible(all, agent, perm, automation);
    let demoted = demoted_ids(&eligible, login_out.is_some());
    let relevant: Vec<&Connector> = eligible
        .iter()
        .filter(|c| c.mode == Mode::Relevant || demoted.contains(&c.id))
        .collect();
    let candidates: Vec<crate::context_rank::ConnectorCandidate> = relevant
        .iter()
        .map(|c| crate::context_rank::ConnectorCandidate {
            id: &c.id,
            name: &c.name,
            description: &c.description,
        })
        .collect();
    let selection = crate::context_rank::select_connectors_in_session(
        session,
        query,
        &candidates,
        deadline,
        jev,
    )
    .await;
    let connectors = attachments_with(&eligible, &selection, all, &signed_in, &demoted);
    // The Microsoft sign-in is refreshed and saved here, in the GUI, so the
    // connector subprocess starts with a fresh token and never has to save.
    let (auth_warm, auth_cut) = warm_selected(&connectors, deadline, crate::m365_auth::warm).await;
    let set_aside = set_aside_names(&eligible, &demoted, &connectors);
    Attached {
        connectors,
        selection,
        auth_warm,
        auth_cut,
        login_out: login_out.filter(|_| !set_aside.is_empty()),
        set_aside,
    }
}

/// [`attach`] for a turn that keeps its conversation's last connector
/// decision (overnight OB18, OB25: a turn that skips retrieval): the
/// "when relevant" ones `kept` attached, the rest decided locally
/// (`context_rank::keep_connectors`), with no Jev call. The Microsoft 365
/// refresh is the same as [`attach`]'s.
pub async fn attach_kept(
    agent: AgentKind,
    perm: &str,
    automation: bool,
    query: &crate::context_rank::ConnectorQuery,
    kept: Option<&crate::context_rank::ConnectorSelection>,
    deadline: &crate::prep_deadline::PrepDeadline,
) -> Attached {
    let all = match app_dir().and_then(|dir| load_from(&dir)) {
        Ok(all) => with_builtins(all),
        Err(e) => {
            eprintln!("[connectors] library not attached: {e}");
            return Attached::default();
        }
    };
    let eligible = eligible(&all, agent, perm, automation);
    let login_out = login_out_for(agent, automation);
    let demoted = kept_demotions(demoted_ids(&eligible, login_out.is_some()), kept);
    let candidates: Vec<crate::context_rank::ConnectorCandidate> = eligible
        .iter()
        .filter(|c| c.mode == Mode::Relevant || demoted.contains(&c.id))
        .map(|c| crate::context_rank::ConnectorCandidate {
            id: &c.id,
            name: &c.name,
            description: &c.description,
        })
        .collect();
    let selection = crate::context_rank::keep_connectors(query, &candidates, kept).await;
    let connectors = attachments_with(&eligible, &selection, &all, &signed_in, &demoted);
    let (auth_warm, auth_cut) = warm_selected(&connectors, deadline, crate::m365_auth::warm).await;
    let set_aside = set_aside_names(&eligible, &demoted, &connectors);
    Attached {
        connectors,
        selection,
        auth_warm,
        auth_cut,
        login_out: login_out.filter(|_| !set_aside.is_empty()),
        set_aside,
    }
}

/// Refresh the Microsoft 365 token (`warm`) when that connector is among
/// `connectors`, waiting until `deadline` at most: its mark, and whether the
/// deadline cut the wait. The refresh runs as a task of its own, so a cut
/// one still finishes and saves (`m365_auth::warm` saves only if the
/// sign-in didn't change meanwhile); the connector subprocess refreshes in
/// memory if it starts first. Nothing at all for any other connector.
pub(crate) async fn warm_selected<W, F>(
    connectors: &[Attachment],
    deadline: &crate::prep_deadline::PrepDeadline,
    warm: W,
) -> (Option<crate::prep_timing::Mark>, bool)
where
    W: FnOnce() -> F,
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    if !connectors
        .iter()
        .any(|a| a.id == crate::builtin_connectors::M365.id)
    {
        return (None, false);
    }
    let started = std::time::Instant::now();
    let refresh = tokio::spawn(warm());
    let cut = match deadline.bound(refresh).await {
        None => true,
        Some(Ok(Err(e))) => {
            eprintln!("[connectors] Microsoft 365 refresh: {e}");
            false
        }
        Some(_) => false,
    };
    (Some(crate::prep_timing::Mark::since(started)), cut)
}

/// Every "always" connector of `eligible`, and each "when relevant" one
/// `selection` attaches, matched by id (never by position). The tests' name
/// for [`attachments_with`] with nothing demoted.
#[cfg(test)]
fn attachments(
    eligible: &[Connector],
    selection: &crate::context_rank::ConnectorSelection,
    all: &[Connector],
    ready: &dyn Fn(&Connector) -> bool,
) -> Vec<Attachment> {
    attachments_with(eligible, selection, all, ready, &[])
}

/// Spread usage across accounts (2026-10-21): the login's reading while it is out
/// of room for a Claude chat run (`claude_accounts::login_out_of_room`),
/// else `None`: any other agent, outside the GUI, failover off, room, or an
/// automation. An automation's prompt is fixed and no one can name a
/// connector back for it, so a scheduled run never loses an Always
/// connector it may need (it stays on the login and warns at its start).
fn login_out_for(agent: AgentKind, automation: bool) -> Option<crate::claude_accounts::LoginLow> {
    (agent == AgentKind::Claude && !automation)
        .then(crate::claude_accounts::login_out_of_room)
        .flatten()
}

/// [`demoted_ids`] for a follow-up that keeps its conversation's last
/// connector decision ([`attach_kept`]): only the ones `kept` decided. One it
/// never decided was attached last turn as Always, and a follow-up ("ok,
/// send it") rarely names it again, so it stays this turn (and the run with
/// it on the login); the next turn that selects afresh decides it.
fn kept_demotions(
    demoted: Vec<String>,
    kept: Option<&crate::context_rank::ConnectorSelection>,
) -> Vec<String> {
    demoted
        .into_iter()
        .filter(|id| kept.is_some_and(|k| k.decisions.iter().any(|d| d.id == *id)))
        .collect()
}

/// The claude.ai connectors set to Always that a run treats as "Only when
/// relevant" while Claude Code's login is out of room (`login_out`): one
/// rides on every run and pins it to the login, which never moves. As a
/// relevance candidate it still comes along when the request needs it
/// (and the run then stays on the login, as before); otherwise it steps
/// aside and the run can use an added account. None while the login has
/// room (Hollow can't tell from a request alone whether an Always
/// connector is needed, so it only relaxes Always when staying costs a
/// run its account).
fn demoted_ids(eligible: &[Connector], login_out: bool) -> Vec<String> {
    if !login_out {
        return Vec::new();
    }
    eligible
        .iter()
        .filter(|c| c.transport == Transport::ClaudeAi && c.mode == Mode::Always)
        .map(|c| c.id.clone())
        .collect()
}

/// [`attachments`] with the `demoted` Always connectors ([`demoted_ids`])
/// attached only when `selection` picks them, or when another claude.ai
/// connector puts the run on the login anyway (nothing would be gained by
/// leaving them out).
fn attachments_with(
    eligible: &[Connector],
    selection: &crate::context_rank::ConnectorSelection,
    all: &[Connector],
    ready: &dyn Fn(&Connector) -> bool,
    demoted: &[String],
) -> Vec<Attachment> {
    let chosen: Vec<&str> = selection.attached().collect();
    let wanted = |c: &Connector| {
        (c.mode == Mode::Always && !demoted.contains(&c.id)) || chosen.contains(&c.id.as_str())
    };
    let on_login_anyway = eligible
        .iter()
        .any(|c| c.transport == Transport::ClaudeAi && wanted(c));
    eligible
        .iter()
        .filter(|c| wanted(c) || (on_login_anyway && demoted.contains(&c.id)))
        .map(|c| Attachment {
            id: c.id.clone(),
            name: c.name.clone(),
            entry: entry(c),
            auto_approve: c.auto_approve && !runs_cua_driver(c),
            claude_login: c.transport == Transport::ClaudeAi,
            remedy: (c.transport == Transport::ClaudeAi)
                .then(|| remedy_for(c, all, ready))
                .flatten(),
        })
        .collect()
}

/// The connectors a turn gets: none for an unattended turn (the night queue,
/// its runbooks), so nothing it reads can make it send, publish, delete or
/// spend through one overnight (master plan #1); every attached one
/// otherwise.
pub(crate) fn for_turn(unattended: bool, attached: Vec<Attachment>) -> Vec<Attachment> {
    if unattended {
        Vec::new()
    } else {
        attached
    }
}

/// Self-hosted models Q5 review: a turn on a self-hosted model never uses
/// Claude Code's own login, so a claude.ai connector (Anthropic's proxy on
/// that login) can't go with it. The attachments it keeps, and the names
/// of those it left out, in order.
pub(crate) fn without_claude_login(attached: Vec<Attachment>) -> (Vec<Attachment>, Vec<String>) {
    let (out, kept): (Vec<Attachment>, Vec<Attachment>) =
        attached.into_iter().partition(|a| a.claude_login);
    (kept, out.into_iter().map(|a| a.name).collect())
}

/// Self-hosted models Q5 review: the prompt line naming the claude.ai
/// connectors a self-hosted turn left out, and why. Empty when none were.
pub fn self_hosted_set_aside_note(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let verb = if names.len() == 1 { "is" } else { "are" };
    format!(
        "<hollow-connectors-set-aside>\n{} {verb} left out of this run: claude.ai connectors run on Claude Code's own Claude login, and this chat runs on a self-hosted model, which never uses it. If one is needed, tell the user to switch this chat to a Claude model in the model menu.\n</hollow-connectors-set-aside>\n\n",
        names.join(", "),
    )
}

/// The names of the `demoted` connectors ([`demoted_ids`]) the run left
/// out, in library order.
fn set_aside_names(
    eligible: &[Connector],
    demoted: &[String],
    attached: &[Attachment],
) -> Vec<String> {
    eligible
        .iter()
        .filter(|c| demoted.contains(&c.id) && !attached.iter().any(|a| a.id == c.id))
        .map(|c| c.name.clone())
        .collect()
}

/// The prompt line telling the agent which connectors stepped aside
/// ([`demoted_ids`]) and why, so it can say so if the request
/// needs one. Empty when none did.
pub fn set_aside_note(names: &[String], why: Option<&str>) -> String {
    if names.is_empty() {
        return String::new();
    }
    let verb = if names.len() == 1 { "is" } else { "are" };
    format!(
        "<hollow-connectors-set-aside>\n{} {verb} left out of this run: the request didn't seem to need {}, and {} If it turns out to be needed, tell the user to ask for it by name: that message brings it back and runs on Claude Code's login.\n</hollow-connectors-set-aside>\n\n",
        names.join(", "),
        if names.len() == 1 { "it" } else { "them" },
        why.unwrap_or("Claude Code's login, which claude.ai connectors need, is out of room, so this run uses an added account."),
    )
}

/// The per-run prompt note naming the attached connectors, so the agent
/// knows they exist (outside the appended system prompt, so a changed
/// selection never resets a session or rewrites its cached prefix). `perm` is the run's canonical
/// permission mode and `automation` whether no one can answer the run:
/// which of them run without a prompt, and the external confirmation every
/// mode keeps, come from the mode contract (`mode_contract`, F6/Q1).
pub fn prompt_note(attached: &[Attachment], perm: &str, automation: bool) -> String {
    let login = if attached.iter().any(|a| a.claude_login) {
        claudeai_login()
    } else {
        None
    };
    prompt_note_with(attached, perm, automation, login.as_ref())
}

/// `prompt_note` with the login given. A claude.ai connector that answers
/// "needs authentication" is signed out for *that* account, even when the
/// user sees it connected on claude.ai under another one, so the agent is
/// told which account to name.
fn prompt_note_with(
    attached: &[Attachment],
    perm: &str,
    automation: bool,
    login: Option<&ClaudeAiLogin>,
) -> String {
    if attached.is_empty() {
        return String::new();
    }
    let list = attached
        .iter()
        .map(|a| format!("{} (tools: mcp__{}__*)", a.name, a.id))
        .collect::<Vec<_>>()
        .join("; ");
    let claudeai: Vec<&str> = attached
        .iter()
        .filter(|a| a.claude_login)
        .map(|a| a.name.as_str())
        .collect();
    let login = match (claudeai.is_empty(), login) {
        (true, _) => String::new(),
        (false, Some(login)) => format!(
            " {} come from claude.ai and run as {}. If one says it needs authentication, tell the user to connect it on claude.ai (Settings → Connectors) while signed in as that account and organization: connected under another account or organization doesn't count.",
            claudeai.join(", "),
            login.describe()
        ),
        (false, None) => format!(
            " {} come from claude.ai and run on Claude Code's own login. If one says it needs authentication, tell the user to connect it on claude.ai (Settings → Connectors) with the account Claude Code is signed in to.",
            claudeai.join(", ")
        ),
    };
    let named: Vec<(&str, bool)> = attached
        .iter()
        .map(|a| (a.name.as_str(), a.auto_approve))
        .collect();
    let approval = crate::mode_contract::connector_sentence(perm, &named);
    let external = crate::mode_contract::external_confirm(automation);
    format!(
        "<hollow-connectors>\nThe user shared these connectors with you through Hollow: {list}. Use them when they help with the request. {approval} {external}{login}\n</hollow-connectors>\n\n"
    )
}

// ── IPC ────────────────────────────────────────────────────────────────────

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "No home folder".to_string())
}

/// `refresh` re-reads the claude.ai list instead of the five-minute copy.
#[tauri::command]
pub async fn list_connectors(refresh: Option<bool>) -> Result<LibraryView, String> {
    let (claudeai, claudeai_error) = match claudeai_servers(refresh.unwrap_or(false)).await {
        Ok(list) => (list, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    tokio::task::spawn_blocking(move || {
        Ok(view_in(&app_dir()?, &home()?, &claudeai, claudeai_error))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sign-in status of the library's claude.ai connectors.
#[tauri::command]
pub async fn check_connectors() -> Result<Vec<ConnectorStatus>, String> {
    let all = tokio::task::spawn_blocking(|| load_from(&app_dir()?))
        .await
        .map_err(|e| e.to_string())??;
    Ok(check_claudeai(&all).await)
}

#[tauri::command]
pub async fn save_connector(input: ConnectorInput) -> Result<ConnectorView, String> {
    tokio::task::spawn_blocking(move || save_in(&app_dir()?, &input))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_connector(id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || delete_in(&app_dir()?, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Add Hollow's own sign-in for a claude.ai connector (five asks B2).
#[tauri::command]
pub async fn connector_use_hollow_sign_in(id: String) -> Result<ConnectorView, String> {
    tokio::task::spawn_blocking(move || use_hollow_sign_in_in(&app_dir()?, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Sign a remote connector in with Hollow's own sign-in, in the browser
/// (waits up to five minutes).
#[tauri::command]
pub async fn remote_connector_sign_in(id: String) -> Result<crate::remote_auth::Status, String> {
    let dir = app_dir()?;
    let c = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || load_from(&dir))
            .await
            .map_err(|e| e.to_string())??
    }
    .into_iter()
    .find(|c| c.id == id && c.transport == Transport::Http && c.oauth)
    .ok_or_else(|| format!("{id} isn't a connector that uses Hollow's sign-in"))?;
    let client_id = Some(c.oauth_client_id.as_str()).filter(|s| !s.is_empty());
    let client_secret = Some(c.oauth_client_secret.as_str()).filter(|s| !s.is_empty());
    let still_current = || sign_in_still_current(&dir, &c);
    crate::remote_auth::sign_in(
        crate::remote_auth::SignIn {
            app: &dir,
            id: &c.id,
            name: &c.name,
            url: &c.url,
            client_id,
            client_secret,
            still_current: Some(&still_current),
        },
        &crate::remote_auth::http()?,
        |url| {
            std::process::Command::new("/usr/bin/open")
                .arg(url)
                .status()
                .map(|_| ())
                .map_err(|e| format!("couldn't open the browser: {e}"))
        },
        crate::remote_auth::SIGN_IN_WAIT,
    )
    .await
}

/// Whether the connector a sign-in started with is still the one saved: same
/// server URL and app credentials, still on Hollow's sign-in. Checked under
/// the token lock before the grant is saved (B2, #537 review): an edit made
/// while the person was in the browser already forgot the old sign-in, and a
/// late grant must not bring a removed secret back.
fn sign_in_still_current(dir: &Path, started: &Connector) -> Result<(), String> {
    let now = load_from(dir)?;
    let same = now.iter().any(|c| {
        c.id == started.id
            && c.transport == Transport::Http
            && c.oauth
            && c.url.trim() == started.url.trim()
            && c.oauth_client_id == started.oauth_client_id
            && c.oauth_client_secret == started.oauth_client_secret
    });
    if same {
        Ok(())
    } else {
        Err(format!(
            "{} changed while you were signing in, so this sign-in wasn't kept; sign in again",
            started.name
        ))
    }
}

/// Forget a remote connector's Hollow sign-in.
#[tauri::command]
pub async fn remote_connector_sign_out(id: String) -> Result<(), String> {
    crate::remote_auth::sign_out(&app_dir()?, &id).await
}

#[tauri::command]
pub async fn import_connector(source: String, name: String) -> Result<ConnectorView, String> {
    let claudeai = if source == "claudeai" {
        claudeai_servers(false).await?
    } else {
        Vec::new()
    };
    tokio::task::spawn_blocking(move || import_in(&app_dir()?, &home()?, &claudeai, &source, &name))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Login override (2026-10-09) ──

    fn ids(list: &[Attachment]) -> Vec<&str> {
        list.iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn a_when_relevant_claudeai_connector_rides_only_when_chosen() {
        // The one filter chats, start_session children (chat turns) and
        // automations share: an unchosen "when relevant" claude.ai connector
        // is not attached, so it never puts the run on the login.
        let conn = |id: &str, mode: Mode| Connector {
            id: id.into(),
            name: id.into(),
            transport: Transport::ClaudeAi,
            mode,
            enabled: true,
            ..Default::default()
        };
        let eligible = vec![
            conn("notion-ai", Mode::Relevant),
            conn("mail-ai", Mode::Always),
        ];
        let unchosen = crate::context_rank::ConnectorSelection::default();
        let attached = attachments(&eligible, &unchosen, &eligible, &|_| true);
        assert_eq!(ids(&attached), ["mail-ai"]);
        assert!(attached.iter().all(|a| a.claude_login));
    }

    fn input(name: &str) -> ConnectorInput {
        ConnectorInput {
            name: name.into(),
            command: "npx".into(),
            args: vec!["-y".into(), "pkg".into()],
            env: BTreeMap::from([("API_KEY".into(), Some("secret".into()))]),
            agents: vec!["claude".into(), "codex".into()],
            chat: true,
            auto_approve: true,
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn relevant_connectors_attach_by_their_decision_id_never_by_position() {
        use crate::context_rank::{Basis, ConnectorDecision, ConnectorSelection};
        let conn = |id: &str, mode: Mode| Connector {
            id: id.into(),
            name: id.into(),
            transport: Transport::Stdio,
            command: "/nonexistent/bed".into(),
            mode,
            ..Default::default()
        };
        let eligible: Vec<Connector> = (1..=30)
            .map(|i| conn(&format!("r{i}"), Mode::Relevant))
            .chain([conn("always", Mode::Always)])
            .collect();
        let decision = |id: &str, attach: bool| ConnectorDecision {
            id: id.into(),
            attach,
            basis: if attach { Basis::Named } else { Basis::Jev },
            score: None,
            reason: None,
        };
        // Decisions in another order than the library: matched by id.
        let selection = ConnectorSelection {
            engine: "jev".into(),
            decisions: (1..=30)
                .rev()
                .map(|i| decision(&format!("r{i}"), i == 25))
                .collect(),
            ..Default::default()
        };
        let ids: Vec<String> = attachments(&eligible, &selection, &eligible, &|_| true)
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, ["r25", "always"]);
    }

    #[test]
    fn save_edit_keeps_secrets_and_views_hide_them() {
        let dir = tempfile::tempdir().unwrap();
        let v = save_in(dir.path(), &input("Superhuman Mail")).unwrap();
        assert_eq!(v.id, "superhuman-mail");
        assert_eq!(v.env_keys, ["API_KEY"]);
        let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.path().join(FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // Editing with a null value keeps the stored secret.
        let mut edit = input("Superhuman Mail");
        edit.id = Some("superhuman-mail".into());
        edit.env = BTreeMap::from([("API_KEY".into(), None)]);
        edit.description = "Mail".into();
        save_in(dir.path(), &edit).unwrap();
        let all = load_from(dir.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].env["API_KEY"], "secret");
        assert_eq!(all[0].description, "Mail");
        let view = view_in(dir.path(), dir.path(), &[], None);
        assert!(!serde_json::to_string(&view.connectors)
            .unwrap()
            .contains("secret"));
        delete_in(dir.path(), "superhuman-mail").unwrap();
        assert!(load_from(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn validation_refuses_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_in(dir.path(), &input("")).is_err());
        assert!(
            save_in(dir.path(), &input("Hollow Vault")).is_err(),
            "hollow ids are reserved"
        );
        let mut http = input("Remote");
        http.transport = Transport::Http;
        http.url = "http://example.com/mcp".into();
        assert!(
            save_in(dir.path(), &http).is_err(),
            "plain http only for localhost"
        );
        http.url = "https://example.com/mcp".into();
        http.headers = BTreeMap::from([("Authorization".into(), Some("Bearer x".into()))]);
        let v = save_in(dir.path(), &http).unwrap();
        assert_eq!(v.header_keys, ["Authorization"]);
        assert!(v.env_keys.is_empty(), "stdio fields are dropped for http");
        let mut bad = input("Other");
        bad.agents = vec!["grok".into()];
        assert!(save_in(dir.path(), &bad).is_err());
        assert!(
            save_in(dir.path(), &input("Remote")).is_err(),
            "ids are unique"
        );
        fs::write(dir.path().join(FILE), "not json").unwrap();
        assert!(save_in(dir.path(), &input("New")).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "not json"
        );
    }

    #[test]
    fn discovers_and_imports_from_both_agents() {
        let home = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(".claude.json"),
            r#"{"mcpServers":{"linear":{"type":"http","url":"https://mcp.linear.app/mcp"},"old":{"type":"sse","url":"https://x/sse"},"fs":{"command":"npx","args":["-y","fs"],"env":{"TOKEN":"t"}}}}"#,
        )
        .unwrap();
        fs::create_dir(home.path().join(".codex")).unwrap();
        fs::write(
            home.path().join(".codex/config.toml"),
            "[mcp_servers.superhuman]\ncommand = \"npx\"\nargs = [\"-y\", \"superhuman-mcp\"]\n[mcp_servers.superhuman.env]\nKEY = \"k\"\n",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let view = view_in(dir.path(), home.path(), &[], None);
        let names: Vec<_> = view
            .discovered
            .iter()
            .map(|d| (d.source.as_str(), d.name.as_str()))
            .collect();
        assert!(names.contains(&("claude", "linear")));
        assert!(names.contains(&("codex", "superhuman")));
        assert!(view
            .discovered
            .iter()
            .find(|d| d.name == "old")
            .unwrap()
            .unsupported
            .is_some());
        assert!(import_in(dir.path(), home.path(), &[], "claude", "old").is_err());
        let v = import_in(dir.path(), home.path(), &[], "codex", "superhuman").unwrap();
        assert_eq!(v.agents, ["claude", "codex"]);
        assert_eq!(v.env_keys, ["KEY"]);
        assert_eq!(v.origin, "codex");
        assert!(
            import_in(dir.path(), home.path(), &[], "codex", "superhuman").is_err(),
            "no duplicates"
        );
        let view = view_in(dir.path(), home.path(), &[], None);
        assert!(
            view.discovered
                .iter()
                .find(|d| d.name == "superhuman")
                .unwrap()
                .in_library
        );
    }

    #[test]
    fn eligibility_and_entries() {
        let mut c = Connector {
            id: "mail".into(),
            name: "Mail".into(),
            command: "npx".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            ..Default::default()
        };
        let all = vec![c.clone()];
        assert_eq!(
            eligible(&all, AgentKind::Claude, "acceptEdits", false).len(),
            1
        );
        assert!(
            eligible(&all, AgentKind::Codex, "acceptEdits", false).is_empty(),
            "not shared with codex"
        );
        assert!(
            eligible(&all, AgentKind::Claude, "acceptEdits", true).is_empty(),
            "automations off"
        );
        assert!(
            eligible(&all, AgentKind::Claude, "plan", false).is_empty(),
            "read-only runs get none"
        );
        assert!(eligible(&all, AgentKind::Antigravity, "acceptEdits", false).is_empty());
        let home = Path::new("/nonexistent-home");
        let missing = Connector {
            command: "hollow-test-no-such-server".into(),
            ..c.clone()
        };
        let stdio = entry_in(&missing, home, Some("/custom"));
        assert_eq!(
            stdio["command"],
            json!("hollow-test-no-such-server"),
            "left bare when no tool folder has it"
        );
        assert_eq!(stdio["args"], json!([]));
        assert!(stdio["env"]["PATH"]
            .as_str()
            .unwrap()
            .contains("/nonexistent-home/.local/bin:/opt/homebrew/bin:"));
        c.transport = Transport::Http;
        c.url = "https://x/mcp".into();
        assert_eq!(entry(&c), json!({"type": "http", "url": "https://x/mcp"}));
        let note = prompt_note(
            &[Attachment {
                id: "mail".into(),
                name: "Mail".into(),
                entry: json!({}),
                auto_approve: true,
                claude_login: false,
                remedy: None,
            }],
            "acceptEdits",
            false,
        );
        assert!(note.contains("mcp__mail__*"));
        assert!(prompt_note(&[], "bypassPermissions", false).is_empty());
        assert!(
            !note.contains("claude.ai"),
            "no login sentence without a claude.ai connector"
        );
    }

    #[test]
    fn builtins_are_listed_editable_never_removed_and_attach_only_when_ready() {
        let dir = tempfile::tempdir().unwrap();
        let view = view_in(dir.path(), dir.path(), &[], None);
        let ids: Vec<&str> = view.connectors.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["hollow_m365", "hollow_plaud"]);
        assert!(view
            .connectors
            .iter()
            .all(|c| c.transport == Transport::Builtin
                && c.agents == ["claude", "codex"]
                && c.mode == Mode::Relevant));
        assert!(!dir.path().join(FILE).exists(), "listing writes nothing");

        // Editing a switch stores it; the name stays Hollow's; it can't be removed or made.
        let edit = ConnectorInput {
            id: Some("hollow_m365".into()),
            name: "Renamed".into(),
            transport: Transport::Builtin,
            agents: vec!["claude".into()],
            chat: true,
            automations: true,
            enabled: true,
            auto_approve: true,
            ..Default::default()
        };
        let v = save_in(dir.path(), &edit).unwrap();
        assert_eq!(
            (v.name.as_str(), v.automations, v.agents.as_slice()),
            ("Microsoft 365", true, ["claude".to_string()].as_slice())
        );
        assert_eq!(
            load_from(dir.path()).unwrap().len(),
            1,
            "only the edited built-in is stored"
        );
        assert!(delete_in(dir.path(), "hollow_m365")
            .unwrap_err()
            .contains("switch it off"));
        assert!(
            save_in(
                dir.path(),
                &ConnectorInput {
                    transport: Transport::Builtin,
                    ..input("Mine")
                }
            )
            .is_err(),
            "no new built-ins"
        );
        assert!(
            save_in(
                dir.path(),
                &ConnectorInput {
                    id: Some("hollow_plaud".into()),
                    ..input("Plaud")
                }
            )
            .is_err(),
            "a built-in stays built-in"
        );

        // Attached only while ready; a ready built-in leaves out claude.ai's
        // connector for the same service (Microsoft 365 since B2).
        let claudeai = |id: &str, url: &str| Connector {
            id: id.into(),
            name: id.into(),
            transport: Transport::ClaudeAi,
            url: url.into(),
            server_id: "mcpsrv_01abc".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            ..Default::default()
        };
        let all = with_builtins(vec![
            claudeai("notion", "https://mcp.notion.com/mcp"),
            claudeai("microsoft_365", "https://microsoft365.mcp.claude.com/mcp"),
        ]);
        let ids = |ready: &[&str], agent| {
            let ready =
                |c: &Connector| c.transport != Transport::Builtin || ready.contains(&c.id.as_str());
            eligible_with(&all, agent, "acceptEdits", false, &ready)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&[], AgentKind::Claude), ["notion", "microsoft_365"]);
        assert_eq!(
            ids(&["hollow_m365"], AgentKind::Claude),
            ["notion", "hollow_m365"],
            "Hollow's Microsoft 365 replaces claude.ai's once signed in"
        );
        assert_eq!(
            ids(&["hollow_m365"], AgentKind::Codex),
            ["hollow_m365"],
            "Codex gets the built-ins"
        );
        // A stored row for the removed `hollow_notion` built-in is left out.
        let stale = with_builtins(vec![Connector {
            id: "hollow_notion".into(),
            name: "Notion".into(),
            transport: Transport::Builtin,
            ..Default::default()
        }]);
        assert!(stale.iter().all(|c| c.id != "hollow_notion"), "{stale:?}");
        assert!(eligible_with(&all, AgentKind::Claude, "plan", false, &|_| true).is_empty());

        // The entry runs this binary's hidden subcommand, with no secrets in it.
        let e = entry(&all[2]);
        assert_eq!(e["args"], json!(["m365-mcp"]));
        assert_eq!(e["env"], json!({}));
        assert!(!e["command"].as_str().unwrap().is_empty());
    }

    /// Overnight S8: Plaud is built in. It runs Plaud's own installed MCP
    /// server with the tool PATH. A Plaud the person added themselves wins:
    /// while one is in the library (on or off), the built-in stands aside,
    /// is never attached, and its row says theirs is in use.
    #[test]
    fn the_plaud_built_in_runs_plaud_mcp_and_a_plaud_the_person_added_wins() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("plaud-mcp"), "#!/usr/bin/env node\n").unwrap();
        let theirs =
            |id: &str, name: &str, transport: Transport, command: &str, args: &[&str]| Connector {
                id: id.into(),
                name: name.into(),
                transport,
                command: command.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                url: if transport == Transport::Http {
                    "https://plaud.example.com/mcp".into()
                } else {
                    String::new()
                },
                agents: vec!["claude".into(), "codex".into()],
                chat: true,
                enabled: true,
                ..Default::default()
            };
        let linear = theirs(
            "linear",
            "Linear",
            Transport::Stdio,
            "npx",
            &["-y", "linear-mcp"],
        );
        let recordings = theirs(
            "rec",
            "Recordings",
            Transport::Stdio,
            "/opt/homebrew/bin/plaud",
            &[],
        );

        let alone = with_builtins(vec![linear.clone(), recordings.clone()]);
        let plaud = alone.iter().find(|c| c.id == "hollow_plaud").unwrap();
        assert!(
            plaud.enabled && plaud.chat,
            "on by default, like the other built-ins"
        );
        assert!(
            plaud.auto_approve,
            "read-only once login/logout are hidden; headless runs can't ask (S8 re-review)"
        );
        let e = entry_in(plaud, home.path(), Some("/usr/bin:/bin"));
        assert_eq!(e["command"], json!(bin.join("plaud-mcp").to_string_lossy()));
        assert_eq!(e["args"], json!([]), "never install or setup");
        assert!(
            e["env"]["PATH"]
                .as_str()
                .unwrap()
                .contains("/opt/homebrew/bin"),
            "node for its shebang"
        );
        let ids = |all: &[Connector], ready: bool| {
            let ready = |c: &Connector| {
                c.transport != Transport::Builtin || (ready && c.id == "hollow_plaud")
            };
            eligible_with(all, AgentKind::Codex, "acceptEdits", false, &ready)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&alone, false),
            ["linear", "rec"],
            "not signed in: not attached"
        );
        assert_eq!(
            ids(&alone, true),
            ["linear", "rec", "hollow_plaud"],
            "the plaud CLI isn't the server"
        );

        // Theirs wins, however they added it: the program, its npm package,
        // or a connector named Plaud. On or off, the built-in stands aside.
        for own in [
            theirs(
                "plaud",
                "plaud",
                Transport::Stdio,
                "/opt/homebrew/bin/plaud-mcp",
                &[],
            ),
            theirs(
                "p2",
                "My recorder",
                Transport::Stdio,
                "npx",
                &["-y", "@plaud-ai/mcp@latest"],
            ),
            theirs("p3", "Plaud", Transport::Http, "", &[]),
        ] {
            let all = with_builtins(vec![linear.clone(), own.clone()]);
            assert_eq!(
                ids(&all, true),
                ["linear", own.id.as_str()],
                "{}: one Plaud, theirs",
                own.id
            );
            let off: Vec<Connector> = all
                .iter()
                .cloned()
                .map(|mut c| {
                    if c.id == own.id {
                        c.enabled = false;
                    }
                    c
                })
                .collect();
            assert_eq!(
                ids(&off, true),
                ["linear"],
                "{}: theirs off is no cue to take over",
                own.id
            );
        }

        // A Plaud connected on claude.ai is left out once the built-in is
        // ready (S8 review), like Notion's; not before.
        let ai = Connector {
            id: "plaud_ai".into(),
            name: "Plaud".into(),
            transport: Transport::ClaudeAi,
            url: "https://mcp.plaud.ai/mcp".into(),
            server_id: "mcpsrv_01plaud".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            ..Default::default()
        };
        let with_ai = with_builtins(vec![ai]);
        let claude = |ready: bool| {
            let ready = |c: &Connector| {
                c.transport != Transport::Builtin || (ready && c.id == "hollow_plaud")
            };
            eligible_with(&with_ai, AgentKind::Claude, "acceptEdits", false, &ready)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(claude(false), ["plaud_ai"]);
        assert_eq!(
            claude(true),
            ["hollow_plaud"],
            "a claude.ai Plaud isn't the person's own; the built-in replaces it"
        );

        // The row says so (no sign-in is read: tests never touch the real files).
        let dir = tempfile::tempdir().unwrap();
        save_to(
            dir.path(),
            &[theirs(
                "plaud",
                "plaud",
                Transport::Stdio,
                "/opt/homebrew/bin/plaud-mcp",
                &[],
            )],
        )
        .unwrap();
        let view = view_in(dir.path(), dir.path(), &[], None);
        let row = view
            .connectors
            .iter()
            .find(|c| c.id == "hollow_plaud")
            .unwrap();
        let state = row.builtin.as_ref().expect("a stand-aside state");
        assert!(!state.ready, "not offered to agents");
        assert_eq!(state.kind, "plaud");
        assert_eq!(state.own.as_deref(), Some("plaud"));
        assert!(
            state.line.contains("your own Plaud connector"),
            "{}",
            state.line
        );
        let mine = view.connectors.iter().find(|c| c.id == "plaud").unwrap();
        assert!(mine.builtin.is_none(), "nothing changes on theirs");
    }

    #[test]
    fn prompt_note_names_the_login_claudeai_connectors_run_as() {
        let notion = Attachment {
            id: "notion".into(),
            name: "Notion".into(),
            entry: json!({}),
            auto_approve: true,
            claude_login: true,
            remedy: None,
        };
        let login = ClaudeAiLogin {
            email: "jo@x.com".into(),
            organization: Some("Acme".into()),
        };
        let note = prompt_note_with(
            std::slice::from_ref(&notion),
            "acceptEdits",
            false,
            Some(&login),
        );
        assert!(
            note.contains("Notion come from claude.ai and run as jo@x.com in Acme"),
            "{note}"
        );
        assert!(note.contains("another account or organization"), "{note}");
        let note = prompt_note_with(&[notion], "acceptEdits", false, None);
        assert!(note.contains("Claude Code's own login"), "{note}");
    }

    #[test]
    fn claudeai_login_reads_oauth_account_without_tokens() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(claudeai_login_in(home.path()), None, "no file");
        fs::write(home.path().join(".claude.json"), r#"{"oauthAccount":{"emailAddress":" jo@x.com ","organizationName":"Acme","accessToken":"nope"}}"#).unwrap();
        assert_eq!(
            claudeai_login_in(home.path()),
            Some(ClaudeAiLogin {
                email: "jo@x.com".into(),
                organization: Some("Acme".into())
            })
        );
        fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"jo@x.com"}}"#,
        )
        .unwrap();
        assert_eq!(
            claudeai_login_in(home.path()).unwrap().describe(),
            "jo@x.com"
        );
        fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"organizationName":"Acme"}}"#,
        )
        .unwrap();
        assert_eq!(claudeai_login_in(home.path()), None, "an email is required");
        let lib = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"jo@x.com","organizationName":"Acme"}}"#,
        )
        .unwrap();
        assert_eq!(
            view_in(lib.path(), home.path(), &[], None)
                .claudeai_login
                .unwrap()
                .describe(),
            "jo@x.com in Acme"
        );
    }

    /// blender (`uvx` in ~/.local/bin) and plaud (a `#!/usr/bin/env node`
    /// script) died at spawn because a Finder-launched Hollow passes the bare
    /// system PATH.
    #[test]
    fn stdio_entries_get_tool_folders_and_a_resolved_command() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("uvx"), "#!/bin/sh\n").unwrap();
        let mut c = Connector {
            command: "uvx".into(),
            args: vec!["blender-mcp".into()],
            ..Default::default()
        };
        c.env.insert("TOKEN".into(), "t".into());
        let e = entry_in(&c, home.path(), Some("/usr/bin:/bin"));
        assert_eq!(e["command"], json!(bin.join("uvx").to_string_lossy()));
        assert_eq!(e["args"], json!(["blender-mcp"]));
        assert_eq!(e["env"]["TOKEN"], json!("t"));
        let path = e["env"]["PATH"].as_str().unwrap();
        assert!(
            path.starts_with(&format!(
                "{}/.cargo/bin:{}:/opt/homebrew/bin:",
                home.path().display(),
                bin.display()
            )),
            "{path}"
        );
        c.env.insert("PATH".into(), "/mine".into());
        let e = entry_in(&c, home.path(), Some("/usr/bin"));
        assert_eq!(
            e["env"]["PATH"],
            json!("/mine"),
            "a connector's own PATH wins"
        );
        assert_eq!(
            e["command"],
            json!("uvx"),
            "and is what the command resolves against"
        );
        let abs = Connector {
            command: "/opt/homebrew/bin/plaud-mcp".into(),
            ..Default::default()
        };
        assert_eq!(
            entry_in(&abs, home.path(), None)["command"],
            json!("/opt/homebrew/bin/plaud-mcp")
        );
    }

    /// computer-use came in from `~/.codex/config.toml` as
    /// `./Codex Computer Use.app/...` with `cwd = "."` and `enabled = false`;
    /// Hollow dropped both and every run failed to spawn it.
    /// CU9: a connector that runs Cua Driver is found by its command or an
    /// argument, any version or case, and only for stdio.
    #[test]
    fn cua_driver_connectors_are_recognised() {
        let stdio = |command: &str, args: &[&str]| Connector {
            transport: Transport::Stdio,
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        };
        for c in [
            stdio("/opt/homebrew/bin/cua-driver", &["mcp"]),
            stdio("/Applications/CuaDriver.app/Contents/MacOS/CuaDriver", &[]),
            stdio("npx", &["-y", "@trycua/cua-driver@0.52.1", "mcp"]),
            stdio("/bin/sh", &["-c", "/usr/local/bin/CUA-DRIVER"]),
            stdio("/bin/sh", &["-c", "cua-driver mcp"]),
            stdio("bash", &["-lc", "exec ~/bin/cua-driver mcp; true"]),
            stdio("uvx", &["cua_driver"]),
            stdio("uvx", &["cua-driver==0.52.1"]),
            stdio("pipx", &["run", "cua-driver>=0.5"]),
        ] {
            assert!(runs_cua_driver(&c), "{c:?}");
        }
        for c in [
            stdio("/usr/local/bin/cua-driverx", &["mcp"]),
            stdio("npx", &["my-cua-driver"]),
            stdio("/opt/bin/server", &["--name", "drivers"]),
            Connector {
                transport: Transport::Http,
                url: "https://example.com/cua-driver".into(),
                command: "cua-driver".into(),
                ..Default::default()
            },
        ] {
            assert!(!runs_cua_driver(&c), "{c:?}");
        }
    }

    /// CU9: a `cua-driver` server in an agent's config is listed with the
    /// warning, imported with auto-approve off and the warning as its
    /// description, and never preapproved at attach, even when its saved
    /// row says otherwise.
    #[test]
    fn a_cua_driver_connector_is_never_preapproved() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".codex")).unwrap();
        fs::write(
            home.path().join(".codex/config.toml"),
            "[mcp_servers.cua]\ncommand = \"/opt/homebrew/bin/cua-driver\"\nargs = [\"mcp\"]\n",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let view = view_in(dir.path(), home.path(), &[], None);
        let found = view.discovered.iter().find(|d| d.name == "cua").unwrap();
        assert_eq!(found.note.as_deref(), Some(CUA_DRIVER_WARNING));
        let imported = import_in(dir.path(), home.path(), &[], "codex", "cua").unwrap();
        assert!(!imported.auto_approve);
        assert_eq!(imported.description, CUA_DRIVER_WARNING);

        let mut row = Connector {
            id: "cua".into(),
            name: "cua".into(),
            transport: Transport::Stdio,
            command: "/opt/homebrew/bin/cua-driver".into(),
            args: vec!["mcp".into()],
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            mode: Mode::Always,
            auto_approve: true,
            ..Default::default()
        };
        assert!(!ConnectorView::from(&row).auto_approve);
        let none = crate::context_rank::ConnectorSelection::default();
        let eligible = vec![row.clone()];
        let attached = attachments(&eligible, &none, &eligible, &|_| true);
        assert_eq!(ids(&attached), ["cua"]);
        assert!(
            !attached[0].auto_approve,
            "a cua-driver connector never preapproves"
        );
        // Bypass preapproves every library connector, so the Driver is left
        // out of a bypass run; other modes attach it without preapproval.
        let all = vec![row.clone()];
        let ids_in = |perm: &str| -> Vec<String> {
            eligible_with(&all, AgentKind::Claude, perm, false, &|_| true)
                .iter()
                .map(|c| c.id.clone())
                .collect()
        };
        assert!(ids_in("bypassPermissions").is_empty());
        // Saved through the form with auto-approve on, it is stored off.
        let lib = tempfile::tempdir().unwrap();
        let mut form = input("Driver");
        form.args = vec!["-y".into(), "@trycua/cua-driver".into(), "mcp".into()];
        form.auto_approve = true;
        assert!(!save_in(lib.path(), &form).unwrap().auto_approve);
        let stored = load_from(lib.path()).unwrap();
        assert!(stored.iter().all(|c| !c.auto_approve));
        assert_eq!(ids_in("acceptEdits"), ["cua"]);
        assert!(!crate::mode_contract::connector_preapproved(
            attached[0].auto_approve,
            "acceptEdits"
        ));
        row.command = "/opt/homebrew/bin/other-server".into();
        let eligible = vec![row];
        let attached = attachments(&eligible, &none, &eligible, &|_| true);
        assert!(
            attached[0].auto_approve,
            "other connectors keep their switch"
        );
    }

    #[test]
    fn relative_commands_are_refused_at_import_and_skipped_at_attach() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".codex")).unwrap();
        fs::write(
            home.path().join(".codex/config.toml"),
            concat!(
                "[mcp_servers.computer-use]\ncommand = \"./Codex Computer Use.app/bin/client\"\nargs = [\"mcp\"]\ncwd = \".\"\nenabled = false\n",
                "[mcp_servers.pinned]\ncommand = \"./bin/server\"\ncwd = \"/opt/tool\"\n",
            ),
        )
        .unwrap();
        fs::write(
            home.path().join(".claude.json"),
            r#"{"mcpServers":{"rel":{"command":"tools/run"}}}"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let view = view_in(dir.path(), home.path(), &[], None);
        let find = |n: &str| {
            view.discovered
                .iter()
                .find(|d| d.name == n)
                .unwrap()
                .clone()
        };
        let cu = find("computer-use");
        assert!(cu.unsupported.as_deref().unwrap().contains("full path"));
        assert_eq!(cu.note.as_deref(), Some(COMPUTER_USE_REPLACED));
        assert!(import_in(dir.path(), home.path(), &[], "codex", "computer-use").is_err());
        assert!(
            find("rel").unsupported.is_some(),
            "the same rule for ~/.claude.json"
        );
        let pinned = find("pinned");
        assert!(pinned.unsupported.is_none());
        assert_eq!(
            pinned.command, "/opt/tool/./bin/server",
            "an absolute cwd is honored"
        );

        // A library entry saved before this fix is skipped, not spawned.
        let stale = Connector {
            id: "computer_use".into(),
            name: "computer-use".into(),
            command: "./Codex Computer Use.app/bin/client".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            ..Default::default()
        };
        assert!(eligible(
            std::slice::from_ref(&stale),
            AgentKind::Claude,
            "acceptEdits",
            false
        )
        .is_empty());
        save_to(dir.path(), &[stale]).unwrap();
        let view = view_in(dir.path(), home.path(), &[], None);
        assert!(
            view.problems
                .iter()
                .any(|p| p.starts_with("computer-use: its command is a relative path")),
            "{:?}",
            view.problems
        );
    }

    #[test]
    fn claudeai_connectors_import_as_claude_only_proxy_entries() {
        let body = json!({"data": [
            {"id": "mcpsrv_017PNovukPYkutaKZDAWdhJR", "display_name": "Superhuman Mail", "url": "https://mcp.mail.superhuman.com/mcp", "eligible": true, "icon_url": "https://www.google.com/s2/favicons?domain=superhuman.com&sz=64"},
            {"id": "mcpsrv_01EYKAFypNeP396XbeoV7xei", "display_name": "Microsoft 365", "url": "https://microsoft365.mcp.claude.com/mcp"},
            {"id": "not-an-id", "display_name": "Bad", "url": "https://x"},
            {"id": "mcpsrv_01X", "display_name": "Plain", "url": "http://x"}
        ]});
        let list = parse_claudeai(&body);
        assert_eq!(
            list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Superhuman Mail", "Microsoft 365"]
        );
        assert_eq!(status_from("m", 200, "").state, "connected");
        let needs = status_from(
            "m",
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"OAuth token has been invalidated. Re-authentication is required."}}"#,
        );
        assert_eq!(needs.state, "needsSignIn");
        assert!(needs.detail.unwrap().contains("Re-authentication"));
        assert_eq!(status_from("m", 502, "gateway").state, "error");
        // Every probe names when it ran and as whom (next-level P16).
        let stamped = stamp(
            vec![status_from("m", 200, "")],
            "2026-10-01T12:00:00Z",
            Some("jo@x.com in Acme"),
        );
        assert_eq!(
            stamped[0].checked_at.as_deref(),
            Some("2026-10-01T12:00:00Z")
        );
        assert_eq!(stamped[0].login.as_deref(), Some("jo@x.com in Acme"));
        let wire = serde_json::to_value(&stamped[0]).unwrap();
        assert_eq!(wire["checkedAt"], "2026-10-01T12:00:00Z");
        assert_eq!(wire["login"], "jo@x.com in Acme");
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".claude")).unwrap();
        fs::write(
            home.path().join(".claude/mcp-needs-auth-cache.json"),
            r#"{"claude.ai Microsoft 365":{"timestamp":1,"id":"mcpsrv_01EYKAFypNeP396XbeoV7xei"}}"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let view = view_in(dir.path(), home.path(), &list, None);
        let m365 = view
            .discovered
            .iter()
            .find(|d| d.name == "Microsoft 365")
            .unwrap();
        assert_eq!(m365.source, "claudeai");
        assert!(
            m365.note.as_deref().unwrap().contains("claude.ai"),
            "needs sign-in noted"
        );
        assert!(view
            .discovered
            .iter()
            .find(|d| d.name == "Superhuman Mail")
            .unwrap()
            .note
            .is_none());
        let v = import_in(
            dir.path(),
            home.path(),
            &list,
            "claudeai",
            "Superhuman Mail",
        )
        .unwrap();
        assert_eq!(v.id, "superhuman_mail");
        assert_eq!(v.agents, ["claude"], "claude.ai connectors are Claude only");
        assert_eq!(v.server_id, "mcpsrv_017PNovukPYkutaKZDAWdhJR");
        assert!(
            v.icon_url.contains("superhuman.com"),
            "claude.ai's icon is kept"
        );
        let all = load_from(dir.path()).unwrap();
        assert_eq!(
            entry(&all[0]),
            json!({"type": "claudeai-proxy", "url": "https://mcp.mail.superhuman.com/mcp", "id": "mcpsrv_017PNovukPYkutaKZDAWdhJR"})
        );
        assert!(eligible(&all, AgentKind::Codex, "acceptEdits", false).is_empty());
        assert_eq!(
            eligible(&all, AgentKind::Claude, "acceptEdits", false).len(),
            1
        );
        let view = view_in(dir.path(), home.path(), &list, None);
        assert!(
            view.discovered
                .iter()
                .find(|d| d.name == "Superhuman Mail")
                .unwrap()
                .in_library
        );
        // Editing keeps the stored claude.ai id; sharing with Codex is refused.
        let mut edit = ConnectorInput {
            id: Some("superhuman_mail".into()),
            name: "Superhuman Mail".into(),
            transport: Transport::ClaudeAi,
            url: "https://mcp.mail.superhuman.com/mcp".into(),
            agents: vec!["claude".into()],
            chat: true,
            automations: true,
            enabled: true,
            auto_approve: true,
            ..Default::default()
        };
        let v = save_in(dir.path(), &edit).unwrap();
        assert_eq!(v.server_id, "mcpsrv_017PNovukPYkutaKZDAWdhJR");
        assert!(v.automations);
        edit.agents = vec!["claude".into(), "codex".into()];
        assert!(save_in(dir.path(), &edit).is_err());
        let mut fresh = edit.clone();
        fresh.id = None;
        fresh.name = "Other".into();
        fresh.agents = vec!["claude".into()];
        assert!(
            save_in(dir.path(), &fresh).is_err(),
            "a new claude.ai connector needs its id"
        );
    }

    // ── Spread usage across accounts (2026-10-21): Always claude.ai connectors step
    // aside while the login is out of room ──

    fn ian_library() -> Vec<Connector> {
        let claudeai = |id: &str, name: &str, mode: Mode| Connector {
            id: id.into(),
            name: name.into(),
            transport: Transport::ClaudeAi,
            url: format!("https://{id}.example.com/mcp"),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            mode,
            ..Default::default()
        };
        vec![
            claudeai("mail-ai", "Superhuman Mail", Mode::Always),
            claudeai("notion-ai", "Notion", Mode::Relevant),
            Connector {
                id: "local-notes".into(),
                name: "Local notes".into(),
                transport: Transport::Stdio,
                command: "/bin/echo".into(),
                agents: vec!["claude".into()],
                chat: true,
                enabled: true,
                mode: Mode::Always,
                ..Default::default()
            },
        ]
    }

    fn login_at(percent: f64) -> crate::claude_accounts::LoginLow {
        crate::claude_accounts::login_low(
            crate::claude_accounts::Routing::Spread,
            90.0,
            &[crate::claude_quota::Reading {
                id: crate::claude_quota::FIVE_HOUR.into(),
                label: "5-hour".into(),
                used_percent: percent,
                resets_at: Some("2026-10-21T20:00:00Z".into()),
            }],
            Some("teammate".into()),
            chrono::Utc::now(),
        )
        .unwrap()
    }

    /// 2026-10-21: Superhuman Mail (claude.ai) on Always pins every
    /// chat to Claude Code's login. While the login is out of room it is
    /// attached only when the request needs it, like "Only when relevant";
    /// otherwise the run carries no claude.ai connector and can spread.
    #[tokio::test]
    async fn an_always_claudeai_connector_waits_for_relevance_while_the_login_is_out() {
        use crate::context_rank::ConnectorQuery;
        use crate::prep_deadline::PrepDeadline;
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let jev = crate::jev::Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: None,
            endpoint: String::new(),
        };
        let deadline = PrepDeadline::with_budget(std::time::Duration::from_secs(5));
        let all = ian_library();
        let run = |request: &'static str, login_out: Option<crate::claude_accounts::LoginLow>| {
            let (all, jev, deadline) = (&all, &jev, &deadline);
            async move {
                attach_from(
                    all,
                    AgentKind::Claude,
                    "acceptEdits",
                    false,
                    &ConnectorQuery::plain(request),
                    deadline,
                    jev,
                    None,
                    login_out,
                )
                .await
            }
        };
        // The login has room: today's behaviour, Always rides on every run.
        let roomy = run("summarise the quarterly plan", None).await;
        assert_eq!(ids(&roomy.connectors), ["mail-ai", "local-notes"]);
        assert!(roomy.set_aside.is_empty() && roomy.login_out.is_none());
        // Out of room, and the request doesn't need mail: it steps aside, so
        // nothing left can put the run on the login.
        let out = run("summarise the quarterly plan", Some(login_at(105.0))).await;
        assert_eq!(ids(&out.connectors), ["local-notes"]);
        assert!(!out.connectors.iter().any(|a| a.claude_login));
        assert_eq!(out.set_aside, ["Superhuman Mail"]);
        assert_eq!(out.login_out.as_ref().map(|l| l.percent), Some(105.0));
        // Its decision is recorded like any "when relevant" candidate's.
        assert!(out
            .selection
            .decisions
            .iter()
            .any(|d| d.id == "mail-ai" && !d.attach));
        // A request that needs it keeps it (and the run stays on the login).
        let mail = run(
            "check my superhuman mail for the invoice",
            Some(login_at(105.0)),
        )
        .await;
        assert_eq!(ids(&mail.connectors), ["mail-ai", "local-notes"]);
        assert!(mail.set_aside.is_empty() && mail.login_out.is_none());
        // Another claude.ai connector the request picked puts the run on the
        // login anyway, so the Always one rides along as before.
        let notion = run("update the notion page", Some(login_at(105.0))).await;
        assert_eq!(
            ids(&notion.connectors),
            ["mail-ai", "notion-ai", "local-notes"]
        );
        assert!(notion.set_aside.is_empty());
        // Codex never carries a claude.ai connector, so nothing changes there.
        assert!(demoted_ids(
            &eligible(&all, AgentKind::Codex, "acceptEdits", false),
            true
        )
        .is_empty());
    }

    /// Review of #871: a follow-up keeps an Always connector its kept
    /// decision never saw (it rode last turn as Always), and an automation
    /// never demotes one (no one can name it back).
    #[test]
    fn follow_ups_and_automations_keep_always_connectors() {
        use crate::context_rank::{Basis, ConnectorDecision, ConnectorSelection};
        let decided = |id: &str| ConnectorDecision {
            id: id.into(),
            attach: false,
            basis: Basis::Local,
            score: None,
            reason: None,
        };
        let demoted = || vec!["mail-ai".to_string(), "drive-ai".to_string()];
        let kept = ConnectorSelection {
            engine: "local".into(),
            decisions: vec![decided("drive-ai"), decided("notion-ai")],
            ..Default::default()
        };
        assert_eq!(kept_demotions(demoted(), Some(&kept)), ["drive-ai"]);
        assert!(kept_demotions(demoted(), None).is_empty());
        // Outside the GUI nothing is out of room; an automation or another
        // agent never asks.
        assert!(login_out_for(AgentKind::Claude, false).is_none());
        let src = include_str!("connectors.rs");
        assert!(src.contains("(agent == AgentKind::Claude && !automation)\n        .then(crate::claude_accounts::login_out_of_room)"));
    }

    #[test]
    fn the_agent_is_told_which_connectors_stepped_aside() {
        assert_eq!(set_aside_note(&[], Some("why")), "");
        let note = set_aside_note(&["Superhuman Mail".into()], Some("the login is at 105%."));
        assert!(note.starts_with("<hollow-connectors-set-aside>\nSuperhuman Mail is left out of this run: the request didn't seem to need it, and the login is at 105%."));
        assert!(note.contains("tell the user to ask for it by name"));
        let two = set_aside_note(&["A".into(), "B".into()], None);
        assert!(two.contains("A, B are left out of this run: the request didn't seem to need them, and Claude Code's login"));
    }

    /// Self-hosted models Q5 review: a self-hosted turn keeps no claude.ai
    /// connector (no `claudeai-proxy` entry reaches its MCP config) and its
    /// note says why and how to get one back.
    #[test]
    fn a_self_hosted_turn_leaves_claude_ai_connectors_out() {
        let claude_ai = Connector {
            id: "mail".into(),
            name: "Superhuman Mail".into(),
            transport: Transport::ClaudeAi,
            url: "https://mcp.mail.superhuman.com/mcp".into(),
            server_id: "mcpsrv_1".into(),
            ..Connector::default()
        };
        let proxy = Attachment {
            id: claude_ai.id.clone(),
            name: claude_ai.name.clone(),
            entry: entry(&claude_ai),
            auto_approve: false,
            claude_login: true,
            remedy: None,
        };
        assert_eq!(proxy.entry["type"], "claudeai-proxy");
        let local = Attachment {
            id: "drive".into(),
            name: "Drive".into(),
            entry: json!({"command": "/bin/echo"}),
            auto_approve: false,
            claude_login: false,
            remedy: None,
        };
        let (kept, out) = without_claude_login(vec![proxy, local]);
        assert_eq!(
            kept.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["drive"]
        );
        assert!(kept.iter().all(|a| a.entry["type"] != "claudeai-proxy"));
        assert_eq!(out, ["Superhuman Mail"]);
        let note = self_hosted_set_aside_note(&out);
        assert!(note.starts_with("<hollow-connectors-set-aside>\nSuperhuman Mail is left out of this run: claude.ai connectors run on Claude Code's own Claude login"));
        assert!(note.contains("switch this chat to a Claude model"));
        assert_eq!(self_hosted_set_aside_note(&[]), "");
    }

    // ── Microsoft 365 refresh after selection (agent effectiveness F8, O2) ──

    fn attached(ids: &[&str]) -> Vec<Attachment> {
        ids.iter()
            .map(|id| Attachment {
                id: id.to_string(),
                name: id.to_string(),
                entry: Value::Null,
                auto_approve: false,
                claude_login: false,
                remedy: None,
            })
            .collect()
    }

    /// Unrelated connectors never refresh the Microsoft 365 token: only a
    /// run the connector is attached to does, and it waits until the
    /// preparation deadline at most; a refresh it stops waiting for still
    /// finishes (and would save) on its own.
    #[tokio::test]
    async fn only_an_attached_microsoft_365_refreshes_its_token_within_the_deadline() {
        use crate::prep_deadline::PrepDeadline;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;
        let m365 = crate::builtin_connectors::M365.id;
        let calls = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let refresh = |delay: u64| {
            let (calls, done) = (calls.clone(), done.clone());
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    done.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        };
        let far = PrepDeadline::with_budget(Duration::from_secs(30));
        // Eligible but not attached (an unrelated request): no refresh.
        for unrelated in [vec![], vec!["crm", "mail"]] {
            let (mark, cut) = warm_selected(&attached(&unrelated), &far, refresh(0)).await;
            assert_eq!((mark, cut), (None, false));
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "unrelated runs refresh nothing"
        );
        // Attached: refreshed, timed.
        let (mark, cut) = warm_selected(&attached(&["crm", m365]), &far, refresh(50)).await;
        assert!(!cut && mark.unwrap().ms >= 50);
        assert_eq!(done.load(Ordering::SeqCst), 1);
        // A slow refresh: waited for until the deadline, then left to finish.
        // It waits on `release`, which only the test opens, so it can never
        // finish by itself: `warm_selected` returning at all proves the
        // deadline cut the wait, whatever the machine's load (S23; no paused
        // clock, `PrepDeadline` and the mark read `std::time::Instant`).
        let release = Arc::new(tokio::sync::Notify::new());
        let gated = {
            let (calls, done, release) = (calls.clone(), done.clone(), release.clone());
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    release.notified().await;
                    done.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), String>(())
                }
            }
        };
        let soon = PrepDeadline::with_budget(Duration::from_millis(200));
        let (mark, cut) = tokio::time::timeout(
            Duration::from_secs(30),
            warm_selected(&attached(&[m365]), &soon, gated),
        )
        .await
        .expect("cut at the deadline, not after the refresh");
        assert!(cut, "the deadline cut the wait");
        assert!(mark.is_some(), "timed");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            done.load(Ordering::SeqCst),
            1,
            "the cut refresh is still waiting"
        );
        // Released, the cut refresh still finishes on its own task.
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(10), async {
            while done.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the cut refresh still finished");
        assert_eq!(done.load(Ordering::SeqCst), 2);
    }

    /// `attach` selects first and refreshes only what it attached (red first
    /// at O1: the refresh ran before selection, for any eligible library).
    #[test]
    fn attach_selects_before_it_refreshes_microsoft_365() {
        let src = include_str!("connectors.rs");
        let start = src.find("async fn attach_from(").unwrap();
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        let select = "select_connectors_in_session(";
        let selected = body.find(select).expect("selects");
        let warmed = body.find("m365_auth::warm").expect("warms");
        assert!(selected < warmed, "the refresh runs before selection");
        assert!(body.contains("warm_selected(&connectors, deadline, "));
        assert!(!body.contains("eligible\n        .iter()\n        .any(|c| c.id == crate::builtin_connectors::M365.id)"));
    }

    /// S28: a conversation's reused relevance decision never carries an
    /// approval. Auto-approve switched off between two turns that reuse the
    /// decision (one Jev call, the second `cached`) applies on the second.
    #[tokio::test]
    async fn a_reused_relevance_decision_rereads_auto_approve_every_turn() {
        use crate::context_rank::{Basis, ConnectorQuery, SessionCache, SESSION_TTL};
        use crate::prep_deadline::PrepDeadline;
        let fake = crate::jev::fake::serve(crate::jev::fake::Reply::ScoreAll(3.0));
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let jev = crate::jev::Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("k".into()),
            endpoint: fake.url.clone(),
        };
        let sessions = SessionCache::new(SESSION_TTL, 8);
        let deadline = PrepDeadline::with_budget(std::time::Duration::from_secs(30));
        let mut crm = Connector {
            id: "zeta-crm".into(),
            name: "Zetacrm".into(),
            transport: Transport::Stdio,
            command: "/nonexistent/bed".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            mode: Mode::Relevant,
            auto_approve: true,
            ..Default::default()
        };
        async fn turn(
            all: Vec<Connector>,
            request: &str,
            jev: &crate::jev::Target<'_>,
            deadline: &PrepDeadline,
            sessions: &SessionCache,
        ) -> Attached {
            attach_from(
                &all,
                AgentKind::Claude,
                "acceptEdits",
                false,
                &ConnectorQuery::plain(request),
                deadline,
                jev,
                Some(("session-1", sessions)),
                None,
            )
            .await
        }
        let first = turn(
            vec![crm.clone()],
            "summarise the notes",
            &jev,
            &deadline,
            &sessions,
        )
        .await;
        assert_eq!(ids(&first.connectors), ["zeta-crm"]);
        assert!(first.connectors[0].auto_approve);
        assert_eq!(first.selection.decisions[0].basis, Basis::Jev);
        crm.auto_approve = false;
        let second = turn(vec![crm], "what about tomorrow", &jev, &deadline, &sessions).await;
        assert_eq!(fake.seen.lock().unwrap().len(), 1, "one Jev call");
        assert_eq!(second.selection.decisions[0].basis, Basis::Cached);
        assert_eq!(ids(&second.connectors), ["zeta-crm"]);
        assert!(
            !second.connectors[0].auto_approve,
            "the switch applies at the next turn"
        );
    }

    fn claudeai_row(id: &str, name: &str, url: &str) -> Connector {
        Connector {
            id: id.into(),
            name: name.into(),
            transport: Transport::ClaudeAi,
            url: url.into(),
            server_id: "mcpsrv_01abc".into(),
            agents: vec!["claude".into()],
            chat: true,
            automations: true,
            mode: Mode::Relevant,
            auto_approve: true,
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_claudeai_connector_moves_to_hollows_sign_in_once() {
        let dir = tempfile::tempdir().unwrap();
        save_to(
            dir.path(),
            &[
                claudeai_row(
                    "zoom_for_claude",
                    "Zoom for Claude",
                    "https://mcp.zoom.us/mcp/zoom/streamable",
                ),
                claudeai_row(
                    "superhuman_mail",
                    "Superhuman Mail",
                    "https://mcp.mail.superhuman.com/mcp",
                ),
                claudeai_row(
                    "microsoft_365",
                    "Microsoft 365",
                    "https://microsoft365.mcp.claude.com/mcp",
                ),
                Connector {
                    id: "zoom".into(),
                    name: "zoom".into(),
                    command: "x".into(),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
        let v = use_hollow_sign_in_in(dir.path(), "zoom_for_claude").unwrap();
        assert_eq!((v.id.as_str(), v.name.as_str()), ("zoom_2", "Zoom"));
        assert_eq!(v.transport, Transport::Http);
        assert!(v.oauth);
        assert_eq!(v.agents, ["claude", "codex"]);
        assert_eq!(
            (v.chat, v.automations, v.mode),
            (true, true, Mode::Relevant)
        );
        assert_eq!(v.url, "https://mcp.zoom.us/mcp/zoom/streamable");
        // Again: the same one, not a second copy.
        assert_eq!(
            use_hollow_sign_in_in(dir.path(), "zoom_for_claude")
                .unwrap()
                .id,
            "zoom_2"
        );
        assert_eq!(load_from(dir.path()).unwrap().len(), 5);
        // 10-11 B4: Superhuman Mail is an approved mail connector: it moves to
        // Hollow's sign-in like any other, shared with Claude and Codex.
        let mail = use_hollow_sign_in_in(dir.path(), "superhuman_mail").unwrap();
        assert_eq!(mail.url, "https://mcp.mail.superhuman.com/mcp");
        assert_eq!(mail.agents, ["claude", "codex"]);
        assert!(mail.oauth);
        assert!(use_hollow_sign_in_in(dir.path(), "microsoft_365")
            .unwrap_err()
            .contains("Anthropic"));
        assert!(
            use_hollow_sign_in_in(dir.path(), "zoom").is_err(),
            "only claude.ai rows"
        );
    }

    #[test]
    fn hollow_sign_in_is_for_remote_non_messaging_servers_without_their_own_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let remote = |url: &str| ConnectorInput {
            transport: Transport::Http,
            url: url.into(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            oauth: Some(true),
            ..input("Remote")
        };
        assert!(save_in(
            dir.path(),
            &ConnectorInput {
                oauth: Some(true),
                ..input("Local")
            }
        )
        .is_err());
        assert!(save_in(dir.path(), &remote("https://mcp.slack.com/mcp")).is_err());
        assert!(save_in(
            dir.path(),
            &remote("https://microsoft365.mcp.claude.com/mcp")
        )
        .is_err());
        for header in ["Authorization", "X-Tenant"] {
            let with_header = ConnectorInput {
                headers: BTreeMap::from([(header.into(), Some("x".into()))]),
                ..remote("https://mcp.example.com/mcp")
            };
            assert!(save_in(dir.path(), &with_header).is_err(), "{header}");
        }
        let bad_client = ConnectorInput {
            oauth_client_id: Some("has space".into()),
            ..remote("https://mcp.example.com/mcp")
        };
        assert!(save_in(dir.path(), &bad_client).is_err());
        let v = save_in(
            dir.path(),
            &ConnectorInput {
                oauth_client_id: Some(" app-1 ".into()),
                ..remote("https://mcp.example.com/mcp")
            },
        )
        .unwrap();
        assert!(v.oauth);
        assert_eq!(v.oauth_client_id, "app-1");
        // An agent flipping one switch (no oauth fields) keeps the sign-in.
        let kept = save_in(
            dir.path(),
            &ConnectorInput {
                id: Some(v.id.clone()),
                enabled: false,
                oauth: None,
                oauth_client_id: None,
                ..remote("https://mcp.example.com/mcp")
            },
        )
        .unwrap();
        assert!(kept.oauth && !kept.enabled);
        assert_eq!(kept.oauth_client_id, "app-1");
        // And the bridge is told where the sign-in lives.
        let saved = load_from(dir.path())
            .unwrap()
            .into_iter()
            .find(|c| c.id == v.id)
            .unwrap();
        let e = with_auth_dir(
            &saved,
            entry_in(&saved, Path::new("/home"), None),
            Some(Path::new("/app")),
        );
        let args: Vec<&str> = e["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(&args[args.len() - 2..], ["--auth-dir", "/app"]);
        let plain = Connector {
            oauth: false,
            ..saved
        };
        let e = with_auth_dir(
            &plain,
            entry_in(&plain, Path::new("/home"), None),
            Some(Path::new("/app")),
        );
        assert!(
            e.get("args").is_none(),
            "a plain remote connector has no bridge"
        );
    }

    /// Overnight 10-11 Y6: a confidential app's secret is saved with its
    /// client id, never shown, kept by a switch flip, dropped with a new id.
    #[test]
    fn a_client_secret_is_saved_with_its_client_id_and_never_shown() {
        let dir = tempfile::tempdir().unwrap();
        let remote =
            |id: Option<String>, client: Option<&str>, secret: Option<&str>| ConnectorInput {
                id,
                transport: Transport::Http,
                url: "https://zoom.example.com/mcp".into(),
                command: String::new(),
                args: Vec::new(),
                env: BTreeMap::new(),
                oauth: Some(true),
                oauth_client_id: client.map(str::to_string),
                oauth_client_secret: secret.map(str::to_string),
                ..input("Zoom")
            };
        assert!(
            save_in(dir.path(), &remote(None, None, Some("s"))).is_err(),
            "a secret needs a client id"
        );
        assert!(save_in(dir.path(), &remote(None, Some("app-1"), Some("two words"))).is_err());
        let v = save_in(dir.path(), &remote(None, Some("app-1"), Some(" s3cret "))).unwrap();
        assert!(v.oauth_client_secret_set);
        assert!(
            !serde_json::to_string(&v).unwrap().contains("s3cret"),
            "the view never carries it"
        );
        let stored = |dir: &Path| {
            load_from(dir)
                .unwrap()
                .into_iter()
                .find(|c| c.id == v.id)
                .unwrap()
        };
        assert_eq!(stored(dir.path()).oauth_client_secret, "s3cret");
        // A switch flip (no oauth fields) keeps it.
        let flip = ConnectorInput {
            enabled: false,
            oauth: None,
            ..remote(Some(v.id.clone()), None, None)
        };
        assert!(save_in(dir.path(), &flip).unwrap().oauth_client_secret_set);
        // A new client id without a new secret drops it.
        let moved = save_in(dir.path(), &remote(Some(v.id.clone()), Some("app-2"), None)).unwrap();
        assert!(!moved.oauth_client_secret_set);
        save_in(
            dir.path(),
            &remote(Some(v.id.clone()), Some("app-2"), Some("x")),
        )
        .unwrap();
        // A sign-in made with the old credentials is forgotten when they change.
        let token = dir.path().join("RemoteAuth").join(format!("{}.json", v.id));
        fs::create_dir_all(token.parent().unwrap()).unwrap();
        fs::write(&token, "{}").unwrap();
        let flip_again = ConnectorInput {
            enabled: false,
            oauth: None,
            ..remote(Some(v.id.clone()), None, None)
        };
        save_in(dir.path(), &flip_again).unwrap();
        assert!(token.exists(), "a switch flip keeps the sign-in");
        // "" clears it.
        let cleared = save_in(
            dir.path(),
            &remote(Some(v.id.clone()), Some("app-2"), Some("")),
        )
        .unwrap();
        assert!(!token.exists(), "a removed secret signs the connector out");
        assert!(!cleared.oauth_client_secret_set);
        assert_eq!(stored(dir.path()).oauth_client_secret, "");
    }

    /// B2 (#537 review, medium): a saved secret belongs to its address. A new
    /// URL without a new secret drops it and signs out; the same address
    /// (whitespace aside) keeps both.
    #[test]
    fn a_new_address_drops_the_client_secret_and_the_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let remote = |id: Option<String>, url: &str, secret: Option<&str>| ConnectorInput {
            id,
            transport: Transport::Http,
            url: url.into(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            oauth: Some(true),
            oauth_client_id: Some("app-1".into()),
            oauth_client_secret: secret.map(str::to_string),
            ..input("Zoom")
        };
        let v = save_in(
            dir.path(),
            &remote(None, "https://zoom.example.com/mcp", Some("s3cret")),
        )
        .unwrap();
        let token = dir.path().join("RemoteAuth").join(format!("{}.json", v.id));
        fs::create_dir_all(token.parent().unwrap()).unwrap();
        fs::write(&token, "{}").unwrap();
        let same = save_in(
            dir.path(),
            &remote(Some(v.id.clone()), " https://zoom.example.com/mcp ", None),
        )
        .unwrap();
        assert!(
            same.oauth_client_secret_set,
            "the same address keeps the secret"
        );
        assert!(token.exists(), "and the sign-in");
        // Another path on the same host may name another authorization
        // server, so it counts as another address (B2 review).
        let moved = save_in(
            dir.path(),
            &remote(Some(v.id.clone()), "https://zoom.example.com/v2/mcp", None),
        )
        .unwrap();
        assert!(!moved.oauth_client_secret_set, "a new address drops it");
        assert!(!token.exists(), "and signs out");
        let mut colon = remote(
            Some(v.id.clone()),
            "https://zoom.example.com/v2/mcp",
            Some("x"),
        );
        colon.oauth_client_id = Some("app:1".into());
        assert!(
            save_in(dir.path(), &colon).is_err(),
            "Basic can't carry a colon in the id"
        );
        let stored = load_from(dir.path())
            .unwrap()
            .into_iter()
            .find(|c| c.id == v.id)
            .unwrap();
        assert_eq!(stored.oauth_client_secret, "");
    }

    /// B2 (#537 review, low): the library holds secrets, so every save leaves
    /// it owner-only, even a file an older Hollow wrote readable to others.
    #[cfg(unix)]
    #[test]
    fn the_library_is_owner_only_after_every_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &input("One")).unwrap();
        let path = library_path(dir.path());
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        save_in(dir.path(), &input("Two")).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    /// B2 (#537 review, low): a sign-in that finishes after its connector
    /// changed (a secret removed, the URL moved) keeps nothing.
    #[test]
    fn a_sign_in_finishing_after_its_connector_changed_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let remote = |id: Option<String>, url: &str, secret: Option<&str>| ConnectorInput {
            id,
            transport: Transport::Http,
            url: url.into(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            oauth: Some(true),
            oauth_client_id: Some("app-1".into()),
            oauth_client_secret: secret.map(str::to_string),
            ..input("Zoom")
        };
        let v = save_in(
            dir.path(),
            &remote(None, "https://zoom.example.com/mcp", Some("s3cret")),
        )
        .unwrap();
        let started = load_from(dir.path())
            .unwrap()
            .into_iter()
            .find(|c| c.id == v.id)
            .unwrap();
        assert!(sign_in_still_current(dir.path(), &started).is_ok());
        save_in(
            dir.path(),
            &remote(Some(v.id.clone()), "https://zoom.example.com/mcp", Some("")),
        )
        .unwrap();
        let err = sign_in_still_current(dir.path(), &started).unwrap_err();
        assert!(err.contains("changed while you were signing in"), "{err}");
    }

    #[test]
    fn a_signed_in_remote_connector_replaces_claudeais_and_runs_through_the_bridge() {
        let hollow_zoom = Connector {
            id: "zoom".into(),
            name: "Zoom".into(),
            transport: Transport::Http,
            url: "https://mcp.zoom.us/mcp/zoom/streamable".into(),
            agents: vec!["claude".into(), "codex".into()],
            chat: true,
            enabled: true,
            oauth: true,
            ..Default::default()
        };
        let all = vec![
            claudeai_row(
                "zoom_for_claude",
                "Zoom for Claude",
                "https://mcp.zoom.us/mcp/zoom/streamable",
            ),
            claudeai_row(
                "wispr_flow",
                "Wispr Flow",
                "https://api.wisprflow.ai/connect/mcp",
            ),
            hollow_zoom.clone(),
        ];
        let ids = |signed_in: bool, agent| {
            let ready = |c: &Connector| !c.oauth || signed_in;
            eligible_with(&all, agent, "acceptEdits", false, &ready)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(false, AgentKind::Claude),
            ["zoom_for_claude", "wispr_flow"]
        );
        assert_eq!(ids(true, AgentKind::Claude), ["wispr_flow", "zoom"]);
        assert_eq!(ids(true, AgentKind::Codex), ["zoom"]);
        assert!(ids(false, AgentKind::Codex).is_empty());
        let e = entry_in(&hollow_zoom, Path::new("/home"), None);
        assert_eq!(
            e["args"],
            json!([
                "remote-mcp",
                "--id",
                "zoom",
                "--url",
                "https://mcp.zoom.us/mcp/zoom/streamable"
            ])
        );
        assert_eq!(e["env"], json!({}));
        assert!(
            e.get("headers").is_none() && e.get("url").is_none(),
            "no token or URL transport in the agent's config"
        );
    }

    #[test]
    fn removing_a_remote_connector_forgets_its_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        save_to(
            dir.path(),
            &[Connector {
                id: "zoom".into(),
                name: "Zoom".into(),
                transport: Transport::Http,
                url: "https://mcp.zoom.us/mcp".into(),
                oauth: true,
                ..Default::default()
            }],
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("RemoteAuth")).unwrap();
        fs::write(dir.path().join("RemoteAuth/zoom.json"), "{}").unwrap();
        delete_in(dir.path(), "zoom").unwrap();
        assert!(!dir.path().join("RemoteAuth/zoom.json").exists());
        let view = view_in(dir.path(), dir.path(), &[], None);
        assert!(view.connectors.iter().all(|c| c.id != "zoom"));
    }

    #[test]
    fn keeping_claudeais_connector_leaves_hollows_out_instead() {
        let hollow_zoom = Connector {
            id: "zoom".into(),
            name: "Zoom".into(),
            transport: Transport::Http,
            url: "https://mcp.zoom.us/mcp/zoom/streamable".into(),
            agents: vec!["claude".into(), "codex".into()],
            chat: true,
            enabled: true,
            oauth: true,
            ..Default::default()
        };
        let kept = Connector {
            keep_claudeai: true,
            ..claudeai_row(
                "zoom_for_claude",
                "Zoom for Claude",
                "https://mcp.zoom.us/mcp/zoom/streamable",
            )
        };
        let ids = |all: &[Connector], agent| {
            eligible_with(all, agent, "acceptEdits", false, &|_| true)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        let all = vec![kept.clone(), hollow_zoom.clone()];
        assert_eq!(ids(&all, AgentKind::Claude), ["zoom_for_claude"]);
        // Codex can't take the claude.ai one, so Hollow's isn't left out there.
        assert_eq!(ids(&all, AgentKind::Codex), ["zoom"]);
        // A kept connector that is switched off keeps nothing.
        let off = vec![
            Connector {
                enabled: false,
                ..kept
            },
            hollow_zoom,
        ];
        assert_eq!(ids(&off, AgentKind::Claude), ["zoom"]);
        // Notion's claude.ai connector stays: no built-in replaces it since
        // Hollow's own was removed (2026-11-10).
        let notion = vec![Connector {
            keep_claudeai: true,
            ..claudeai_row("notion", "Notion", "https://mcp.notion.com/mcp")
        }];
        assert_eq!(
            ids(&with_builtins(notion), AgentKind::Claude),
            ["notion", "hollow_m365", "hollow_plaud"]
        );
    }

    #[test]
    fn the_keep_switch_is_kept_when_left_out_and_only_for_claudeai() {
        let dir = tempfile::tempdir().unwrap();
        save_to(
            dir.path(),
            &[claudeai_row(
                "zoom_for_claude",
                "Zoom for Claude",
                "https://mcp.zoom.us/mcp",
            )],
        )
        .unwrap();
        let base = ConnectorInput {
            id: Some("zoom_for_claude".into()),
            name: "Zoom for Claude".into(),
            transport: Transport::ClaudeAi,
            url: "https://mcp.zoom.us/mcp".into(),
            agents: vec!["claude".into()],
            chat: true,
            enabled: true,
            ..Default::default()
        };
        let v = save_in(
            dir.path(),
            &ConnectorInput {
                keep_claudeai: Some(true),
                ..base.clone()
            },
        )
        .unwrap();
        assert!(v.keep_claudeai);
        let v = save_in(
            dir.path(),
            &ConnectorInput {
                keep_claudeai: None,
                mode: Mode::Always,
                ..base
            },
        )
        .unwrap();
        assert!(
            v.keep_claudeai,
            "an agent's save that leaves it out keeps it"
        );
        let local = save_in(
            dir.path(),
            &ConnectorInput {
                keep_claudeai: Some(true),
                ..input("Local")
            },
        )
        .unwrap();
        assert!(!local.keep_claudeai);
    }

    #[test]
    fn the_login_row_names_the_hollow_connector_that_would_avoid_it() {
        let zoom_ai = claudeai_row(
            "zoom_for_claude",
            "Zoom for Claude",
            "https://mcp.zoom.us/mcp",
        );
        let hollow_zoom = Connector {
            id: "zoom".into(),
            name: "Zoom".into(),
            transport: Transport::Http,
            url: "https://mcp.zoom.us/mcp".into(),
            enabled: true,
            oauth: true,
            ..Default::default()
        };
        let yes = |_: &Connector| true;
        let no = |_: &Connector| false;
        // No Hollow connector yet.
        assert_eq!(
            remedy_for(&zoom_ai, std::slice::from_ref(&zoom_ai), &yes).as_deref(),
            Some("Use Hollow's sign-in on Zoom for Claude")
        );
        // One exists: switch kept off, switched off, not signed in, not shared.
        let kept = Connector {
            keep_claudeai: true,
            ..zoom_ai.clone()
        };
        assert_eq!(
            remedy_for(&kept, &[kept.clone(), hollow_zoom.clone()], &yes).as_deref(),
            Some("turn on Use Hollow's instead for Zoom for Claude")
        );
        let off = Connector {
            enabled: false,
            ..hollow_zoom.clone()
        };
        assert_eq!(
            remedy_for(&zoom_ai, &[zoom_ai.clone(), off], &yes).as_deref(),
            Some("turn on Hollow's Zoom")
        );
        assert_eq!(
            remedy_for(&zoom_ai, &[zoom_ai.clone(), hollow_zoom.clone()], &no).as_deref(),
            Some("sign in to Hollow's Zoom")
        );
        assert_eq!(
            remedy_for(&zoom_ai, &[zoom_ai.clone(), hollow_zoom], &yes).as_deref(),
            Some("share Hollow's Zoom with Claude where this ran")
        );
        // The built-in is Microsoft 365's equivalent.
        let m365_ai = claudeai_row(
            "microsoft_365",
            "Microsoft 365",
            "https://microsoft365.mcp.claude.com/mcp",
        );
        assert_eq!(
            remedy_for(&m365_ai, &with_builtins(vec![m365_ai.clone()]), &no).as_deref(),
            Some("sign in to Hollow's Microsoft 365")
        );
        // Approved mail (10-11 B4) can move; other mail, and an
        // Anthropic-only address with no equivalent: nothing.
        let mail = claudeai_row(
            "superhuman_mail",
            "Superhuman Mail",
            "https://mcp.mail.superhuman.com/mcp",
        );
        assert!(remedy_for(&mail, std::slice::from_ref(&mail), &yes).is_some());
        let gmail = claudeai_row("gmail", "Gmail", "https://gmail.mcp.example.com/mcp");
        assert_eq!(remedy_for(&gmail, std::slice::from_ref(&gmail), &yes), None);
        let odd = claudeai_row("odd", "Odd", "https://odd.mcp.claude.com/mcp");
        assert_eq!(remedy_for(&odd, std::slice::from_ref(&odd), &yes), None);
        // The sentence drops duplicates; nothing for no remedies.
        assert_eq!(
            login_remedy(["sign in to Hollow's Zoom", "sign in to Hollow's Zoom", "Use Hollow's sign-in on Wispr Flow"]).as_deref(),
            Some("Hollow's own connector would avoid it: sign in to Hollow's Zoom; Use Hollow's sign-in on Wispr Flow (Settings → Connectors).")
        );
        assert_eq!(login_remedy(std::iter::empty()), None);
        // attach() carries it on the claude.ai attachment only.
        let selection = crate::context_rank::ConnectorSelection::default();
        let always = Connector {
            mode: Mode::Always,
            ..zoom_ai.clone()
        };
        let attached = attachments(
            std::slice::from_ref(&always),
            &selection,
            std::slice::from_ref(&always),
            &yes,
        );
        assert_eq!(
            attached[0].remedy.as_deref(),
            Some("Use Hollow's sign-in on Zoom for Claude")
        );
    }

    #[test]
    fn moving_to_hollows_sign_in_turns_the_keep_switch_off() {
        let dir = tempfile::tempdir().unwrap();
        let kept = Connector {
            keep_claudeai: true,
            ..claudeai_row(
                "zoom_for_claude",
                "Zoom for Claude",
                "https://mcp.zoom.us/mcp",
            )
        };
        save_to(dir.path(), &[kept]).unwrap();
        use_hollow_sign_in_in(dir.path(), "zoom_for_claude").unwrap();
        let row = load_from(dir.path())
            .unwrap()
            .into_iter()
            .find(|c| c.id == "zoom_for_claude")
            .unwrap();
        assert!(!row.keep_claudeai);
    }
}
