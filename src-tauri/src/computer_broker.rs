//! The computer broker (Cua CU1, part 1): the one place an agent's
//! see-and-act desktop call reaches a computer-use engine.
//!
//! Design: `outputs/cua-computer-use-design-2026-11-15.md` in the hollow
//! workspace. Cua Driver (MIT, `cua-driver mcp`, MCP over stdio) is the
//! engine; this broker holds an MCP **client** to it and is the only thing
//! that talks to it. No agent ever gets the Driver's own MCP server.
//!
//! The `hollow_computer` server (`computer_mcp.rs`, a separate process)
//! forwards its see-and-act calls here over a per-pid Unix socket, as
//! `hollow app-mcp` does to `app_broker.rs`. For each request the broker:
//!
//! 1. refuses everything while Computer control is off or no engine is
//!    pinned (`computer-engine.json` in Hollow's support folder; none ships,
//!    so a fresh Hollow serves nothing until a person pins a version);
//! 2. checks the proof the GUI minted into the server's environment
//!    (`sha256(secret ‖ run id ‖ scope ‖ mode)`) and that the socket's peer
//!    is inside that run (`run_auth::check_peer`);
//! 3. refuses `plan` runs (reading a screen is still an effect);
//! 4. classifies the tool against the **tier map** and refuses anything
//!    unclassified, hidden, or not offered on the chosen target (fail
//!    closed: the Driver ships new tools nearly every release);
//! 5. refuses a tier the target doesn't allow yet: until CU2's cards exist,
//!    only observe and act run, and only in the sandbox VM; This Mac's
//!    screen serves nothing until CU6's per-app grants;
//! 6. starts or reuses the run's Driver session, checking the pinned
//!    version (and, on this Mac, the binary's SHA-256) before any call;
//! 7. cuts the result to the size caps before it reaches the model.
//!
//! Approvals per mode (CU2), the audit row (CU3), the shell path for agents
//! without owned MCP (CU4) and the VM lifecycle (CU5) build on this.

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The engine pin file, in Hollow's support folder.
pub(crate) const ENGINE_FILE: &str = "computer-engine.json";
/// The MCP protocol version the broker offers the Driver (it speaks this
/// legacy version and `2026-07-28`; it answers with the one it picks).
const DRIVER_PROTOCOL: &str = "2025-06-18";
/// A request line from `hollow_computer` is read up to this many bytes.
#[cfg(unix)]
const REQUEST_MAX_BYTES: u64 = 1 << 20;
/// One line from the Driver (a screenshot is base64 inside it).
const DRIVER_LINE_MAX: u64 = 32 << 20;
/// Initialize and tools/list must answer within this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// One tool call must answer within this.
const CALL_TIMEOUT: Duration = Duration::from_secs(90);
/// Size caps on what reaches the model. Provisional: CU0 measures real
/// window states and screenshots and these follow its numbers.
const MAX_TEXT_CHARS: usize = 40_000;
const MAX_IMAGE_B64: usize = 2_000_000;
const MAX_IMAGES: usize = 2;
const MAX_DESCRIPTION_CHARS: usize = 1_200;
const MAX_SCHEMA_BYTES: usize = 16 << 10;
const MAX_TOOLS: usize = 200;
const MAX_LIST_PAGES: usize = 10;
/// Live Driver sessions at once, and how long an unused one is kept.
const MAX_SESSIONS: usize = 4;
const SESSION_IDLE: Duration = Duration::from_secs(10 * 60);
/// A run's bounds (the design's kill switch 4). The same numbers go to the
/// Driver's `bounded` manifest ([`run_manifest`]); the broker enforces them
/// itself, so a Driver that ignores its manifest still stops.
const RUN_EXPIRES: Duration = Duration::from_secs(30 * 60);
/// The Driver's own idle timeout (manifest only): the broker starts a new
/// Driver for the next call, and the run's bounds above carry over.
const RUN_IDLE: Duration = Duration::from_secs(5 * 60);
const MAX_ACTIONS: usize = 300;
/// The same call this many times in a row, or this many failures in a row,
/// ends the run's turn at the loop guard.
const LOOP_REPEATS: usize = 3;
const MAX_FAILURES_IN_A_ROW: usize = 5;

pub(crate) const SWITCHED_OFF: &str =
    "Computer control is switched off in Settings, Connectors; nothing ran.";
pub(crate) const NOT_SET_UP: &str = "Computer control's see-and-act tools aren't set up on this Hollow (no pinned engine); nothing ran. Use the basic verbs (open_url, open_app and the rest), or ask the person to set up Computer control.";
pub(crate) const PROOF_REFUSED: &str =
    "This request doesn't carry a valid Hollow run identity; nothing ran.";
pub(crate) const PLAN_REFUSED: &str =
    "Plan mode reads files only, so desktop tools don't run in it; nothing ran.";
pub(crate) const SANDBOX_OFF: &str =
    "The sandbox Mac is switched off in Settings, Connectors, Computer control; nothing ran.";
pub(crate) const THIS_MAC_OFF: &str =
    "This Mac's screen is switched off in Settings, Connectors, Computer control; nothing ran.";

// ── Tiers ────────────────────────────────────────────────────────────────

/// A tool's tier (design section 4). T3 (outward) is not a tool: it is a
/// T1 act whose target sends, posts, pays or deletes, decided per call by
/// CU2's guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    /// T0: a screenshot, the accessibility tree, window lists.
    Observe,
    /// T1: ordinary in-app work.
    Act,
    /// T2: moves data between apps or out of a window.
    Reach,
}

impl Tier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Tier::Observe => "observe",
            Tier::Act => "act",
            Tier::Reach => "reach",
        }
    }
}

/// Every Driver tool Hollow offers, by name. A name not here is never
/// listed and is refused if called, until a row classifies it.
const TIER_MAP: &[(&str, Tier)] = &[
    ("get_window_state", Tier::Observe),
    ("get_desktop_state", Tier::Observe),
    ("zoom", Tier::Observe),
    ("list_windows", Tier::Observe),
    ("get_screen_size", Tier::Observe),
    ("get_cursor_position", Tier::Observe),
    ("verify_state", Tier::Observe),
    ("click", Tier::Act),
    ("double_click", Tier::Act),
    ("right_click", Tier::Act),
    ("type_text", Tier::Act),
    ("press_key", Tier::Act),
    ("hotkey", Tier::Act),
    ("scroll", Tier::Act),
    ("drag", Tier::Act),
    ("set_value", Tier::Act),
    ("launch_app", Tier::Act),
    ("run_steps", Tier::Act),
    ("kill_app", Tier::Reach),
];

/// Hidden from every agent on every target, always (design section 2).
const HIDDEN: &[&str] = &[
    "install_extension",
    "parse_visual_regions",
    "set_config",
    "get_config",
    "check_for_update",
    "install_ffmpeg",
    "run_script",
];
/// Hidden name prefixes: the deprecated history tools.
const HIDDEN_PREFIXES: &[&str] = &["history_"];
/// Recording tools are hidden in v1 (screenshots are the record).
const HIDDEN_WORDS: &[&str] = &["record"];
/// Offered only on the sandbox VM: on the real desktop `list_apps` and
/// `quit_app` keep Hollow's own meaning, and these have no counterpart.
const SANDBOX_ONLY: &[&str] = &["launch_app", "kill_app"];
/// `hollow_computer`'s own verbs. A Driver tool with one of these names is
/// never listed or forwarded: the name keeps Hollow's meaning.
pub(crate) const NATIVE_VERBS: &[&str] = &[
    "open_url",
    "search_bookmarks",
    "open_app",
    "list_apps",
    "browser_current_tab",
    "browser_list_tabs",
    "quit_app",
];

/// Which machine the Driver drives (the `Target` seam: another engine or
/// another machine plugs in here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetKind {
    /// This Mac's real screen, through the standalone `CuaDriver.app`.
    ThisMac,
    /// A disposable macOS VM on the runner.
    SandboxVm,
}

/// Why a tool can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    Native,
    Hidden,
    NotOnTarget,
    Unclassified,
    TierNotAllowed(Tier),
    BadName,
    BadSteps,
}

impl Refusal {
    pub(crate) fn message(&self, tool: &str) -> String {
        match self {
            Refusal::Native => format!("{tool} is one of Hollow's own computer verbs; call it directly. Nothing ran."),
            Refusal::Hidden => format!("{tool} is not offered to agents in Hollow; nothing ran."),
            Refusal::NotOnTarget => format!("{tool} is offered only in the sandbox Mac; nothing ran."),
            Refusal::Unclassified => format!("{tool} is not a tool Hollow has classified yet, so it is refused; nothing ran."),
            Refusal::TierNotAllowed(tier) => format!("{tool} is a {} action, which needs a card this Hollow can't show yet; nothing ran. Ask the person to do it.", tier.as_str()),
            Refusal::BadName => "That isn't a valid tool name; nothing ran.".to_string(),
            Refusal::BadSteps => "run_steps runs only when every step names a classified tool; nothing ran.".to_string(),
        }
    }
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn hidden(name: &str) -> bool {
    HIDDEN.contains(&name)
        || HIDDEN_PREFIXES.iter().any(|p| name.starts_with(p))
        || HIDDEN_WORDS.iter().any(|w| name.contains(w))
}

/// The tier of `name` on `target`, or why it is refused. Order matters:
/// Hollow's own names first, then the hidden list, then the map.
pub(crate) fn classify(name: &str, target: TargetKind) -> Result<Tier, Refusal> {
    if !valid_tool_name(name) {
        return Err(Refusal::BadName);
    }
    if NATIVE_VERBS.contains(&name) {
        return Err(Refusal::Native);
    }
    if hidden(name) {
        return Err(Refusal::Hidden);
    }
    if target == TargetKind::ThisMac && SANDBOX_ONLY.contains(&name) {
        return Err(Refusal::NotOnTarget);
    }
    TIER_MAP
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| *t)
        .ok_or(Refusal::Unclassified)
}

/// The tier of a whole call and the arguments to forward. For
/// `run_steps`, the highest tier of its steps, each classified on its own:
/// a step must be exactly `{"tool": <name>, "arguments"?: {..}}` and the
/// call exactly `{"steps": [..]}`, and the forwarded arguments are rebuilt
/// from the checked fields only, so the Driver can't read a step's tool
/// from a key the broker didn't check. A nested `run_steps` is refused.
pub(crate) fn checked_call(
    name: &str,
    args: &Value,
    target: TargetKind,
) -> Result<(Tier, Value), Refusal> {
    let tier = classify(name, target)?;
    if name != "run_steps" {
        return Ok((tier, args.clone()));
    }
    let top = args.as_object().ok_or(Refusal::BadSteps)?;
    if top.len() != 1 {
        return Err(Refusal::BadSteps);
    }
    let steps = top
        .get("steps")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty() && s.len() <= 50)
        .ok_or(Refusal::BadSteps)?;
    let mut highest = tier;
    let mut rebuilt = Vec::with_capacity(steps.len());
    for step in steps {
        let fields = step.as_object().ok_or(Refusal::BadSteps)?;
        if fields.keys().any(|k| k != "tool" && k != "arguments") {
            return Err(Refusal::BadSteps);
        }
        let inner = fields
            .get("tool")
            .and_then(Value::as_str)
            .ok_or(Refusal::BadSteps)?;
        if inner == "run_steps" {
            return Err(Refusal::BadSteps);
        }
        let inner_args = match fields.get("arguments") {
            None => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => return Err(Refusal::BadSteps),
        };
        let t = classify(inner, target).map_err(|_| Refusal::BadSteps)?;
        highest = highest.max(t);
        rebuilt.push(json!({ "tool": inner, "arguments": inner_args }));
    }
    Ok((highest, json!({ "steps": rebuilt })))
}

/// Whether the broker serves `tier` on `target` at all. Fail closed: the sandbox
/// serves all three, T2 behind a card ([`required_asks`]); This Mac serves
/// observe and act, each behind its per-app grant ([`crate::computer_thismac`]).
pub(crate) fn tier_served(target: TargetKind, tier: Tier) -> bool {
    match target {
        TargetKind::SandboxVm => tier <= Tier::Reach,
        TargetKind::ThisMac => tier <= Tier::Act,
    }
}

/// The Driver's tool list cut to what an agent may see: classified, not
/// hidden, served on this target, with descriptions and schemas capped
/// and the tier stated. A tool whose schema is oversized is dropped.
pub(crate) fn filter_tools(driver_tools: &[Value], target: TargetKind) -> Vec<Value> {
    let mut out = Vec::new();
    for tool in driver_tools.iter().take(MAX_TOOLS) {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Ok(tier) = classify(name, target) else {
            continue;
        };
        if !tier_served(target, tier) {
            continue;
        }
        let schema = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        if schema.to_string().len() > MAX_SCHEMA_BYTES {
            continue;
        }
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut description = cut_chars(description, MAX_DESCRIPTION_CHARS);
        description.push_str(&match target {
            TargetKind::SandboxVm => format!(" (Hollow: {} tier, in the sandbox Mac.)", tier.as_str()),
            TargetKind::ThisMac => format!(
                " (Hollow: {} tier, on this Mac. Pass `bundle_id`, the app it acts on; the person allows each app.)",
                tier.as_str()
            ),
        });
        out.push(json!({
            "name": name,
            "description": description,
            "inputSchema": schema,
        }));
    }
    out
}

fn cut_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().nth(max).is_some() {
        out.push_str(" [cut]");
    }
    out
}

/// A Driver tool result cut to the size caps: text items share
/// [`MAX_TEXT_CHARS`], at most [`MAX_IMAGES`] images of at most
/// [`MAX_IMAGE_B64`] base64 bytes each, and every other content kind
/// (resources, audio, structured content) dropped, each cut noted.
#[cfg(test)]
pub(crate) fn cap_result(result: &Value) -> Value {
    cap_result_as(result, false)
}

/// [`cap_result`] for what the agent receives: every text item the Driver
/// returned is wrapped as outside data (ND2: an accessibility tree or window
/// text is whatever the page or app shows, so it may carry instructions).
/// Hollow's own cut notes stay unwrapped, after the wrapped items.
pub(crate) fn cap_observed_result(result: &Value) -> Value {
    cap_result_as(result, true)
}

fn cap_result_as(result: &Value, observed: bool) -> Value {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let items = result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut content = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut text_left = MAX_TEXT_CHARS;
    let mut images = 0usize;
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or("");
                let n = text.chars().count();
                let shown = |t: &str| {
                    if observed {
                        crate::untrusted::wrap(
                            crate::untrusted::Origin::Observation,
                            "computer driver",
                            t,
                        )
                    } else {
                        t.to_string()
                    }
                };
                if n <= text_left {
                    text_left -= n;
                    content.push(json!({ "type": "text", "text": shown(text) }));
                } else {
                    let kept: String = text.chars().take(text_left).collect();
                    text_left = 0;
                    notes.push(format!("text cut to {MAX_TEXT_CHARS} characters"));
                    if !kept.is_empty() {
                        content.push(json!({ "type": "text", "text": shown(&kept) }));
                    }
                }
            }
            Some("image") => {
                let data = item.get("data").and_then(Value::as_str).unwrap_or("");
                let mime = item.get("mimeType").and_then(Value::as_str).unwrap_or("");
                if !matches!(mime, "image/png" | "image/jpeg" | "image/webp") {
                    notes.push("an image of an unexpected type was left out".into());
                } else if data.len() > MAX_IMAGE_B64 {
                    notes.push("a screenshot over the size cap was left out".into());
                } else if images >= MAX_IMAGES {
                    notes.push(format!("images past the first {MAX_IMAGES} were left out"));
                } else {
                    images += 1;
                    content.push(json!({ "type": "image", "data": data, "mimeType": mime }));
                }
            }
            _ => notes.push("a result part Hollow doesn't pass on was left out".into()),
        }
    }
    if result.get("structuredContent").is_some() {
        notes.push("structured content was left out".into());
    }
    notes.dedup();
    if !notes.is_empty() {
        content.push(json!({ "type": "text", "text": format!("(Hollow: {}.)", notes.join("; ")) }));
    }
    json!({ "content": content, "isError": is_error })
}

// ── The engine pin ───────────────────────────────────────────────────────

/// The pinned engine and where it runs, read from [`ENGINE_FILE`]. The
/// version and SHA-256 are bumped by a row, never by Hollow itself (the
/// release watch only writes a note).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EngineConfig {
    /// The exact Driver version, as its `serverInfo.version` reports it.
    pub version: String,
    /// Hex SHA-256 of the Driver binary (checked on this Mac before each
    /// session; for the VM it is checked when the golden image is built).
    pub sha256: String,
    pub target: TargetConfig,
    /// The sandbox holds real sign-ins (it should not): an outward act there
    /// asks like one on this Mac. Off by default; the golden image has none.
    #[serde(default)]
    pub signed_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum TargetConfig {
    /// The standalone `CuaDriver.app`'s binary, by absolute path.
    #[serde(rename_all = "camelCase")]
    ThisMac { driver: PathBuf },
    /// `ssh -T user@host cua-driver mcp`, with its own key and no ssh
    /// config. How the broker reaches a VM on the runner (a jump, or the
    /// runner's own forward) is CU5's.
    #[serde(rename_all = "camelCase")]
    SandboxVm {
        host: String,
        user: String,
        key: PathBuf,
    },
}

impl TargetConfig {
    pub(crate) fn kind(&self) -> TargetKind {
        match self {
            TargetConfig::ThisMac { .. } => TargetKind::ThisMac,
            TargetConfig::SandboxVm { .. } => TargetKind::SandboxVm,
        }
    }
}

fn host_like(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
}

fn user_like(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

impl EngineConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let v = &self.version;
        if v.is_empty()
            || v.len() > 40
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
        {
            return Err("the pinned version is not an exact version".into());
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("the pin's sha256 is not 64 lowercase hex characters".into());
        }
        match &self.target {
            TargetConfig::ThisMac { driver } if !driver.is_absolute() => {
                Err("the Driver path must be absolute".into())
            }
            TargetConfig::SandboxVm { host, user, key } => {
                if !host_like(host) || !user_like(user) {
                    return Err("the sandbox host or user is not valid".into());
                }
                if !key.is_absolute() {
                    return Err("the sandbox key path must be absolute".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Parse and validate a pin file's text.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let cfg: EngineConfig =
            serde_json::from_str(text).map_err(|e| format!("unreadable engine pin: {e}"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// The pin from `dir`, or `None` when there is none or it is invalid
    /// (logged: a bad pin serves nothing, it never falls back).
    pub(crate) fn load_from(dir: &Path) -> Option<Self> {
        let path = dir.join(ENGINE_FILE);
        let text = std::fs::read_to_string(&path).ok()?;
        match Self::parse(&text) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                eprintln!("[computer broker] {}: {e}", path.display());
                None
            }
        }
    }

    pub(crate) fn load() -> Option<Self> {
        Self::load_from(&crate::instance::support_dir()?)
    }

    /// The command that starts the Driver's MCP server for this target.
    /// Its environment is cleared to HOME, PATH and LANG, so nothing of
    /// Hollow's (keys, run proofs) reaches the engine.
    /// The Driver's command. On This Mac, `manifest` is the run's bounded
    /// manifest file. The flag name is unverified until the Driver is
    /// installed (see `computer_thismac`'s header).
    pub(crate) fn command_with(&self, manifest: Option<&Path>) -> tokio::process::Command {
        let mut cmd = match &self.target {
            TargetConfig::ThisMac { driver } => {
                let mut c = tokio::process::Command::new(driver);
                c.arg("mcp");
                if let Some(path) = manifest {
                    c.arg("--manifest").arg(path);
                }
                c
            }
            TargetConfig::SandboxVm { host, user, key } => {
                let mut c = tokio::process::Command::new("/usr/bin/ssh");
                c.args([
                    "-T",
                    "-F",
                    "/dev/null",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "IdentitiesOnly=yes",
                    "-i",
                ])
                .arg(key);
                c.arg("--")
                    .arg(format!("{user}@{host}"))
                    .args(["cua-driver", "mcp"]);
                c
            }
        };
        cmd.env_clear();
        for key in ["HOME", "PATH", "LANG"] {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        cmd
    }
}

/// Hex SHA-256 of a file, streamed.
pub(crate) fn file_sha256(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("cannot read the Driver: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("cannot read the Driver: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ── The MCP client ───────────────────────────────────────────────────────

type BoxReader = Pin<Box<dyn AsyncBufRead + Send>>;
type BoxWriter = Pin<Box<dyn AsyncWrite + Send>>;

/// An MCP stdio client: one JSON-RPC message per line each way.
pub(crate) struct DriverClient {
    reader: BoxReader,
    writer: BoxWriter,
    next_id: u64,
}

impl DriverClient {
    pub(crate) fn new(reader: BoxReader, writer: BoxWriter) -> Self {
        Self {
            reader,
            writer,
            next_id: 1,
        }
    }

    async fn send(&mut self, msg: &Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("the Driver stopped listening: {e}"))?;
        self.writer
            .flush()
            .await
            .map_err(|e| format!("the Driver stopped listening: {e}"))
    }

    /// Read one line, capped at [`DRIVER_LINE_MAX`].
    async fn read_line(&mut self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let n = (&mut self.reader)
            .take(DRIVER_LINE_MAX)
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| format!("the Driver's answer was unreadable: {e}"))?;
        if n == 0 {
            return Err("the Driver closed its connection".into());
        }
        if buf.last() != Some(&b'\n') && n as u64 >= DRIVER_LINE_MAX {
            return Err("the Driver's answer was over the size cap".into());
        }
        Ok(buf)
    }

    /// One request and its response. Notifications in between are skipped;
    /// a request from the Driver (sampling, roots) is answered "not
    /// supported", so the engine can never ask Hollow's model for anything.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;
        loop {
            let line = self.read_line().await?;
            let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            let msg_id = msg.get("id").cloned();
            if msg.get("method").is_some() {
                if let Some(their) = msg_id.filter(|v| !v.is_null()) {
                    self.send(&json!({ "jsonrpc": "2.0", "id": their, "error": { "code": -32601, "message": "not supported by Hollow" } }))
                        .await?;
                }
                continue;
            }
            if msg_id.as_ref().and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(err) = msg.get("error") {
                let text = err.get("message").and_then(Value::as_str).unwrap_or("");
                return Err(format!(
                    "the Driver refused {method}: {}",
                    cut_chars(text, 300)
                ));
            }
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Initialize, check the reported version against the pin, and list
    /// the tools (all pages, capped).
    pub(crate) async fn handshake(&mut self, pinned_version: &str) -> Result<Vec<Value>, String> {
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": DRIVER_PROTOCOL,
                    "capabilities": {},
                    "clientInfo": { "name": "hollow-computer-broker", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        let version = init
            .pointer("/serverInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("");
        if version != pinned_version {
            return Err(format!(
                "the Driver reports version {:?}, but Hollow pins {pinned_version:?}; nothing ran",
                cut_chars(version, 40)
            ));
        }
        self.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self.request("tools/list", params).await?;
            if let Some(list) = page.get("tools").and_then(Value::as_array) {
                tools.extend(list.iter().cloned());
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() || tools.len() >= MAX_TOOLS {
                break;
            }
        }
        tools.truncate(MAX_TOOLS);
        Ok(tools)
    }

    pub(crate) async fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        self.request("tools/call", json!({ "name": name, "arguments": args }))
            .await
    }
}

// ── Approvals (CU2) ──────────────────────────────────────────────────────

/// A card the person must answer before a call runs. The broker decides
/// which ([`required_asks`]); `hollow_computer` shows it and re-sends the
/// call with the allowed kinds in `approved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    /// "Use the sandbox Mac in this chat?" Once per chat, `default` mode only.
    UseSandbox,
    /// This Mac's screen, first use in a chat, naming who receives the
    /// screenshots. Every mode, bypass included. (Served from CU6.)
    FirstUseThisMac,
    /// Tier 2: data moves between apps or out of a window. Every mode but
    /// bypass, each time.
    Reach,
    /// A send, post, pay, delete or confirm. Every mode, bypass included.
    Outward,
    /// One app on This Mac, per chat (every mode but bypass). The card names
    /// the app; the key is `app:<id>`.
    App,
    /// A look at the whole display or all windows on This Mac, per chat
    /// (every mode, bypass included).
    Display,
    /// Typing into a password or secret field. Every mode, bypass included;
    /// the practical answer is Deny and the person types it.
    SecureField,
}

impl Ask {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Ask::UseSandbox => "useSandbox",
            Ask::FirstUseThisMac => "firstUseThisMac",
            Ask::App => "app",
            Ask::Display => "display",
            Ask::Reach => "reach",
            Ask::Outward => "outward",
            Ask::SecureField => "secureField",
        }
    }

    /// Remembered for the chat once allowed (the rest ask every time).
    fn per_chat(self) -> bool {
        matches!(
            self,
            Ask::UseSandbox | Ask::FirstUseThisMac | Ask::App | Ask::Display
        )
    }

    /// What the card says. Never carries an argument's value (typed text,
    /// a label from the screen): a card is read by the person, and the tool
    /// name is checked ([`safe_name`]).
    pub(crate) fn text(self, tool: &str, subject: &str) -> String {
        let tool = safe_name(tool);
        // The subject came from `computer_thismac::look`, which only passes a
        // plain identifier (letters, digits, `.-_` and spaces, 128 at most).
        let app = subject.trim_start_matches("name:");
        match self {
            Ask::App => format!("Let the agent use {app} on this Mac in this chat? It will see that app's windows and click and type in it. Nothing else on this Mac is reachable."),
            Ask::Display => "Let the agent look at this Mac's whole screen in this chat? It can show any window, a password manager or a private message included, and all of it reaches the company behind the agent's model. Asks in every mode.".into(),
            Ask::UseSandbox => "Use the sandbox Mac in this chat? The agent will see its screen and click and type there. Nothing on your own Mac is touched.".into(),
            Ask::FirstUseThisMac => "Let the agent see and use this Mac's screen in this chat? Every screenshot goes to the company behind the agent's model, and may show anything open here.".into(),
            Ask::Reach => format!("Let the agent move data between apps or out of a window ({tool}): clipboard, files, or closing another app?"),
            Ask::Outward => format!("The agent is about to do something that may send, post, pay, delete or confirm ({tool}). Allow this one action?"),
            Ask::SecureField => "The agent wants to type into a password or secret field. Hollow never lets an agent type a secret: Deny, and type it yourself.".into(),
        }
    }
}

/// The key a card is remembered and answered under: the kind, plus the app
/// for a per-app card (`app:com.apple.notes`).
pub(crate) fn ask_key(ask: Ask, subject: &str) -> String {
    match ask {
        Ask::App => format!("app:{subject}"),
        _ => ask.as_str().to_string(),
    }
}

/// What a call's arguments say about it, for the two always-ask rules. A
/// heuristic: it reads only the call's own arguments (labels, keys, app
/// ids), so it misses an unlabeled icon button and any path it doesn't know.
/// The sandbox is the real boundary; CU15 measures the misses. Reading the
/// element's label from the last window-state snapshot waits on CU0 showing
/// the Driver's real snapshot format.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub outward: bool,
    pub secure: bool,
}

const OUTWARD_WORDS: &[&str] = &[
    "send", "submit", "post", "reply", "buy", "pay", "publish", "share", "confirm", "transfer",
    "delete", "purchase", "checkout", "tweet", "invite",
];
const OUTWARD_PHRASES: &[&str] = &[
    "place order",
    "buy now",
    "send now",
    "move to trash",
    "empty trash",
];
/// Apps whose every click or key press is outward on a desktop.
const OUTWARD_APPS: &[&str] = &[
    "com.apple.mail",
    "com.apple.mobilesms",
    "com.apple.messages",
    "com.apple.facetime",
    "com.microsoft.outlook",
    "com.tinyspeck.slackmacgap",
    "com.hnc.discord",
    "ru.keepcoder.telegram",
];
const LABEL_KEYS: &[&str] = &[
    "element",
    "ref",
    "label",
    "title",
    "name",
    "description",
    "role",
];
const APP_KEYS: &[&str] = &["bundle_id", "bundleId", "app", "app_name"];

fn lower_text(args: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| args.get(*k).and_then(Value::as_str))
        .map(|t| cut_chars(t, 200).to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_word(text: &str, words: &[&str]) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|w| words.contains(&w))
}

/// [`guard_flags`] for a whole call: a `run_steps` call is judged by every
/// step it carries (already rebuilt from checked fields), so an outward or
/// secret-typing step can't hide inside one.
fn guard_flags_for_call(tool: &str, args: &Value) -> Flags {
    if tool != "run_steps" {
        return guard_flags(tool, args);
    }
    let mut all = Flags::default();
    for step in args
        .get("steps")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let inner = step.get("tool").and_then(Value::as_str).unwrap_or("");
        let f = guard_flags(inner, step.get("arguments").unwrap_or(&Value::Null));
        all.outward |= f.outward;
        all.secure |= f.secure;
    }
    all
}

pub(crate) fn guard_flags(tool: &str, args: &Value) -> Flags {
    let label = lower_text(args, LABEL_KEYS);
    let app = lower_text(args, APP_KEYS);
    let keys = lower_text(args, &["key", "keys", "combo", "hotkey", "modifiers"]);
    let acts = matches!(
        tool,
        "click" | "double_click" | "right_click" | "press_key" | "hotkey" | "set_value" | "drag"
    );
    let compose_send = (keys.contains("cmd") || keys.contains("command"))
        && (keys.contains("return") || keys.contains("enter"));
    let outward = (acts
        && (has_word(&label, OUTWARD_WORDS)
            || OUTWARD_PHRASES.iter().any(|p| label.contains(p))
            || OUTWARD_APPS.iter().any(|a| app.contains(a))))
        || (matches!(tool, "press_key" | "hotkey") && compose_send);
    let typing = matches!(tool, "type_text" | "set_value");
    let secure = typing
        && (label.contains("secure")
            || has_word(
                &label,
                &["password", "passcode", "passphrase", "pin", "secret"],
            ));
    Flags { outward, secure }
}

/// The cards this call needs, in the order they are asked (design section
/// 4): `plan` never reaches here. `signed_in` is the pin's flag for a
/// sandbox that holds real sign-ins.
pub(crate) fn required_asks(
    mode: crate::mode_contract::Mode,
    target: TargetKind,
    tier: Tier,
    flags: &Flags,
    signed_in: bool,
) -> Vec<Ask> {
    use crate::mode_contract::Mode;
    let mut asks = Vec::new();
    match target {
        TargetKind::SandboxVm => {
            if mode == Mode::Default {
                asks.push(Ask::UseSandbox);
            }
        }
        TargetKind::ThisMac => asks.push(Ask::FirstUseThisMac),
    }
    if tier == Tier::Reach && mode != Mode::Bypass {
        asks.push(Ask::Reach);
    }
    if flags.outward && (target == TargetKind::ThisMac || signed_in) {
        asks.push(Ask::Outward);
    }
    if flags.secure {
        asks.push(Ask::SecureField);
    }
    asks
}

/// The Driver's `bounded` manifest for one run (design section 4, kill
/// switch 4). CU5 and CU6 hand it to the Driver they start; the broker
/// enforces the same bounds itself.
#[allow(dead_code)] // CU5 and CU6 pass it to the Driver they start
pub(crate) fn run_manifest(run_id: &str) -> Value {
    json!({
        "mode": "bounded",
        "run": safe_name(run_id),
        "expires_after": RUN_EXPIRES.as_secs(),
        "idle_timeout": RUN_IDLE.as_secs(),
        "max_actions": MAX_ACTIONS,
    })
}

fn call_hash(tool: &str, args: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tool.hash(&mut h);
    args.to_string().hash(&mut h);
    h.finish()
}

// ── Sessions ─────────────────────────────────────────────────────────────

/// One run's live Driver: the client, the process behind it (none for a
/// test's in-memory driver), and the tools it listed at the handshake.
pub(crate) struct Session {
    client: DriverClient,
    _child: Option<tokio::process::Child>,
    tools: Vec<Value>,
    last_used: Instant,
    /// The manifest file the Driver reads (This Mac), removed with the session.
    _manifest_file: Option<crate::computer_thismac::ManifestFile>,
}

impl Session {
    fn new(client: DriverClient, child: Option<tokio::process::Child>, tools: Vec<Value>) -> Self {
        Self {
            client,
            _child: child,
            tools,
            last_used: Instant::now(),
            _manifest_file: None,
        }
    }

    pub(crate) fn offers(&self, name: &str) -> bool {
        self.tools
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some(name))
    }
}

/// One run's bounds ([`run_manifest`]): when it began, how many acts it
/// made, and a trail for the loop guard. Kept per run, **outside** the
/// session, so ending a session (a limit, a Driver error, an idle reap)
/// never gives the run a fresh allowance. A tripped limit is sticky.
struct RunState {
    started: Instant,
    actions: usize,
    recent: Vec<u64>,
    failures: usize,
    tripped: Option<&'static str>,
    /// What this run's Driver on This Mac may reach.
    manifest: crate::computer_thismac::Manifest,
    /// The chat (scope) this run belongs to, for Stop on that chat.
    scope: String,
}

impl RunState {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            actions: 0,
            recent: Vec::new(),
            failures: 0,
            tripped: None,
            manifest: crate::computer_thismac::Manifest::default(),
            scope: String::new(),
        }
    }

    /// Why this run may not make another call, if it may not (sticky).
    fn limit_reached(&mut self, call_hash: u64) -> Option<&'static str> {
        if self.tripped.is_none() {
            let n = self.recent.len();
            self.tripped = if self.started.elapsed() >= RUN_EXPIRES {
                Some("run time limit")
            } else if self.actions >= MAX_ACTIONS {
                Some("action limit")
            } else if self.failures >= MAX_FAILURES_IN_A_ROW {
                Some("repeated failures")
            } else if n >= LOOP_REPEATS
                && self.recent[n - LOOP_REPEATS..]
                    .iter()
                    .all(|h| *h == call_hash)
            {
                Some("repeated identical action")
            } else {
                None
            };
        }
        self.tripped
    }

    fn note(&mut self, call_hash: u64, tier: Tier, failed: bool) {
        if tier >= Tier::Act {
            self.actions += 1;
        }
        self.recent.push(call_hash);
        if self.recent.len() > LOOP_REPEATS {
            self.recent.remove(0);
        }
        self.failures = if failed { self.failures + 1 } else { 0 };
    }
}

/// Runs remembered at once; the oldest is forgotten past this.
const MAX_RUN_STATES: usize = 256;

type SessionFuture = Pin<Box<dyn Future<Output = Result<Session, String>> + Send>>;
/// How a session is started (the real one spawns the pinned Driver; tests
/// pass an in-memory fake).
/// The manifest is the run's reach on This Mac (none for the sandbox).
pub(crate) type Connector = Arc<
    dyn Fn(EngineConfig, Option<crate::computer_thismac::Manifest>) -> SessionFuture + Send + Sync,
>;

/// Start the pinned Driver as a child process and shake hands with it.
pub(crate) fn spawn_session(
    cfg: EngineConfig,
    manifest: Option<crate::computer_thismac::Manifest>,
) -> SessionFuture {
    Box::pin(async move {
        if let TargetConfig::ThisMac { driver } = &cfg.target {
            let path = driver.clone();
            let sha = tokio::task::spawn_blocking(move || file_sha256(&path))
                .await
                .map_err(|e| format!("cannot check the Driver: {e}"))??;
            if sha != cfg.sha256 {
                return Err("the Driver on disk doesn't match the pinned SHA-256; nothing ran. Its version changed outside Hollow, so it is refused until the pin is updated.".into());
            }
        }
        // This Mac's Driver starts bounded by the run's manifest (the apps the
        // person allowed); a start with no manifest is refused, never a
        // Driver that can reach anything.
        let manifest_file = match (&cfg.target, &manifest) {
            (TargetConfig::ThisMac { .. }, None) => {
                return Err("This Mac's Driver needs the run's manifest; nothing ran.".into());
            }
            (TargetConfig::ThisMac { .. }, Some(m)) => Some(
                crate::computer_thismac::ManifestFile::write(&crate::instance::cache_dir(), m)?,
            ),
            _ => None,
        };
        let mut child = cfg
            .command_with(manifest_file.as_ref().map(|f| f.path()))
            .spawn()
            .map_err(|e| format!("cannot start the Driver: {e}"))?;
        let stdin = child.stdin.take().ok_or("the Driver has no stdin")?;
        let stdout = child.stdout.take().ok_or("the Driver has no stdout")?;
        let mut client =
            DriverClient::new(Box::pin(tokio::io::BufReader::new(stdout)), Box::pin(stdin));
        let tools = tokio::time::timeout(HANDSHAKE_TIMEOUT, client.handshake(&cfg.version))
            .await
            .map_err(|_| "the Driver didn't answer in time; nothing ran".to_string())??;
        let mut session = Session::new(client, Some(child), tools);
        session._manifest_file = manifest_file;
        Ok(session)
    })
}

/// A session over an in-memory driver (tests).
#[cfg(test)]
pub(crate) async fn session_over(
    reader: BoxReader,
    writer: BoxWriter,
    cfg: &EngineConfig,
) -> Result<Session, String> {
    let mut client = DriverClient::new(reader, writer);
    let tools = client.handshake(&cfg.version).await?;
    Ok(Session::new(client, None, tools))
}

type Shared = Arc<tokio::sync::Mutex<Session>>;
/// A run's session with the pin and manifest it started under.
type LiveSession = (
    EngineConfig,
    Option<crate::computer_thismac::Manifest>,
    Shared,
);

/// Live sessions by run id.
#[derive(Default)]
pub(crate) struct Sessions {
    /// Each run's session with the pin it started under, so a pin check
    /// never waits on a session busy in a call.
    by_run: Mutex<HashMap<String, LiveSession>>,
    /// Held while a session starts (never while waiting on a session), so
    /// the cap can't be overrun and one run never starts two Drivers.
    starting: tokio::sync::Mutex<()>,
    /// The tools the pinned Driver listed at its last handshake. A chat's
    /// tool list is served from here and never starts a Driver; a changed
    /// pin forgets it.
    listed: Mutex<Option<(EngineConfig, Vec<Value>)>>,
    /// A warm-up handshake is under way (one at a time).
    warming: std::sync::atomic::AtomicBool,
    /// The per-chat cards the person has allowed ([`Ask::per_chat`]), as
    /// `scope|kind`. Cleared by Stop all and Computer control off.
    grants: Mutex<std::collections::HashSet<String>>,
    /// Each run's bounds, by run id ([`RunState`]).
    runs: Mutex<HashMap<String, RunState>>,
}

impl Sessions {
    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<String, LiveSession>> {
        self.by_run.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// End one run's session (its Driver process is killed on drop).
    pub(crate) fn end(&self, run_id: &str) -> bool {
        self.live().remove(run_id.trim()).is_some()
    }

    /// End every session (Computer control switched off, Stop all).
    pub(crate) fn end_all(&self) -> usize {
        let n = {
            let mut live = self.live();
            let n = live.len();
            live.clear();
            n
        };
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        n
    }

    /// End the sessions that have outlived their bounds without waiting for
    /// another call: a run past its expiry, and a This Mac session idle past
    /// the Driver's idle timeout (its process is killed on drop). Run every
    /// half minute by the broker's sweeper. Returns how many ended.
    pub(crate) fn sweep(&self) -> usize {
        self.sweep_with(RUN_EXPIRES, RUN_IDLE)
    }

    pub(crate) fn sweep_with(&self, expires: Duration, idle: Duration) -> usize {
        let expired: Vec<String> = {
            let runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            runs.iter()
                .filter(|(_, r)| r.started.elapsed() >= expires)
                .map(|(k, _)| k.clone())
                .collect()
        };
        let mut live = self.live();
        let before = live.len();
        live.retain(|run, (pin, _, s)| {
            if expired.contains(run) {
                return false;
            }
            let this_mac = pin.target.kind() == TargetKind::ThisMac;
            !(this_mac && s.try_lock().is_ok_and(|s| s.last_used.elapsed() >= idle))
        });
        before - live.len()
    }

    /// Whether this run may make another call: its state outlives its
    /// sessions, so ending one never gives a fresh allowance.
    fn run_limit(&self, run_id: &str, scope: &str, call_hash: u64) -> Option<&'static str> {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        if runs.len() >= MAX_RUN_STATES && !runs.contains_key(run_id) {
            if let Some(oldest) = runs
                .iter()
                .min_by_key(|(_, r)| r.started)
                .map(|(k, _)| k.clone())
            {
                runs.remove(&oldest);
            }
        }
        let state = runs.entry(run_id.to_string()).or_insert_with(RunState::new);
        if state.scope.is_empty() {
            state.scope = scope.to_string();
        }
        state.limit_reached(call_hash)
    }

    fn note_run(&self, run_id: &str, call_hash: u64, tier: Tier, failed: bool) {
        if let Some(r) = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(run_id)
        {
            r.note(call_hash, tier, failed);
        }
    }

    /// Stop on one chat: end the sessions of every run of that chat and
    /// forget what the chat was allowed (the next call asks again).
    pub(crate) fn end_scope(&self, scope: &str) -> usize {
        let runs: Vec<String> = {
            let runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            runs.iter()
                .filter(|(_, r)| r.scope == scope)
                .map(|(k, _)| k.clone())
                .collect()
        };
        let ended = runs.iter().filter(|r| self.end(r)).count();
        {
            let mut runs_map = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            for r in &runs {
                runs_map.remove(r);
            }
        }
        let prefix = format!("{scope}|");
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|g| !g.starts_with(&prefix));
        ended
    }

    pub(crate) fn count(&self) -> usize {
        self.live().len()
    }

    fn granted(&self, scope: &str, key: &str) -> bool {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&format!("{scope}|{key}"))
    }

    fn grant(&self, scope: &str, key: &str) {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(format!("{scope}|{key}"));
    }

    /// What this run's Driver may reach: the apps and display it has passed
    /// the checks for so far, plus these, minus any app the person has since
    /// blocked (`policy`).
    fn manifest_for(
        &self,
        run_id: &str,
        apps: impl IntoIterator<Item = String>,
        display: bool,
        policy: &crate::computer_thismac::Policy,
    ) -> crate::computer_thismac::Manifest {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let state = runs.entry(run_id.to_string()).or_insert_with(RunState::new);
        if state.manifest.run.is_empty() {
            state.manifest.run = run_id.to_string();
        }
        state.manifest.apps.extend(apps);
        state.manifest.apps.retain(|a| !policy.denied(a));
        state.manifest.display |= display;
        state.manifest.clone()
    }

    /// The tools listed under `cfg` at the last handshake, if any.
    fn listed_for(&self, cfg: &EngineConfig) -> Option<Vec<Value>> {
        let listed = self.listed.lock().unwrap_or_else(|e| e.into_inner());
        listed
            .as_ref()
            .filter(|(pin, _)| pin == cfg)
            .map(|(_, tools)| tools.clone())
    }

    fn remember(&self, cfg: &EngineConfig, tools: &[Value]) {
        *self.listed.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((cfg.clone(), tools.to_vec()));
    }

    /// Drop sessions idle past [`SESSION_IDLE`] and sessions started under
    /// another pin than `cfg` (a session busy in a call keeps running: its
    /// caller holds it, and it is dropped from the map either way).
    fn reap(&self, cfg: &EngineConfig) {
        self.live().retain(|_, (pin, _, s)| {
            pin == cfg
                && match s.try_lock() {
                    Ok(s) => s.last_used.elapsed() < SESSION_IDLE,
                    Err(_) => true,
                }
        });
    }

    fn existing(
        &self,
        run_id: &str,
        cfg: &EngineConfig,
        manifest: Option<&crate::computer_thismac::Manifest>,
    ) -> Option<Shared> {
        self.live()
            .get(run_id)
            .filter(|(pin, m, _)| pin == cfg && m.as_ref() == manifest)
            .map(|(_, _, s)| s.clone())
    }

    async fn get_or_start(
        &self,
        run_id: &str,
        cfg: &EngineConfig,
        manifest: Option<&crate::computer_thismac::Manifest>,
        connect: &Connector,
    ) -> Result<Shared, String> {
        let run_id = run_id.trim();
        self.reap(cfg);
        if let Some(s) = self.existing(run_id, cfg, manifest) {
            return Ok(s);
        }
        let _starting = self.starting.lock().await;
        if let Some(s) = self.existing(run_id, cfg, manifest) {
            return Ok(s);
        }
        // A wider manifest (a new app was allowed) ends this run's Driver and
        // starts one that can reach it; a narrower one never happens.
        self.end(run_id);
        if self.count() >= MAX_SESSIONS {
            return Err("Hollow is already driving the most desktops it allows at once; try again when another chat is done. Nothing ran.".into());
        }
        let session = connect(cfg.clone(), manifest.cloned()).await?;
        self.remember(cfg, &session.tools);
        let shared = Arc::new(tokio::sync::Mutex::new(session));
        self.live().insert(
            run_id.to_string(),
            (cfg.clone(), manifest.cloned(), shared.clone()),
        );
        Ok(shared)
    }

    /// Learn the pinned Driver's tools with one handshake and end that
    /// session at once (its process is killed), so the next tool list has
    /// them. Started after a list found nothing for this pin; one at a time.
    pub(crate) async fn warm(&self, cfg: EngineConfig, connect: Connector) {
        use std::sync::atomic::Ordering;
        if self.listed_for(&cfg).is_some() || self.warming.swap(true, Ordering::SeqCst) {
            return;
        }
        // This Mac's warm-up Driver starts with a manifest that allows
        // nothing: it only learns the tool list.
        let manifest = (cfg.target.kind() == TargetKind::ThisMac)
            .then(|| crate::computer_thismac::Manifest::empty("warm-up"));
        if let Ok(Ok(session)) =
            tokio::time::timeout(HANDSHAKE_TIMEOUT, connect(cfg.clone(), manifest)).await
        {
            self.remember(&cfg, &session.tools);
        }
        self.warming.store(false, Ordering::SeqCst);
    }
}

// ── Requests ─────────────────────────────────────────────────────────────

/// What `hollow_computer` sends: one line per request.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ComputerRequest {
    /// `tools` (list what this run may call) or `call`.
    pub op: String,
    pub run_id: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub perm: String,
    pub proof: String,
    #[serde(default)]
    pub tool: String,
    /// The cards the person has just allowed for this call ([`Ask::as_str`]).
    /// Only `hollow_computer`, Hollow's own server, sets it, after its card
    /// was answered; an agent never speaks to this socket.
    #[serde(default)]
    pub approved: Vec<String>,
    #[serde(default)]
    pub arguments: Value,
}

/// Random per-process secret behind every proof.
static SECRET: OnceLock<[u8; 32]> = OnceLock::new();

/// The proof a run's `hollow_computer` carries: hex sha256 over this
/// process's secret and the NUL-separated trimmed run id, scope and mode.
pub(crate) fn proof_for(run_id: &str, scope: &str, perm: &str) -> String {
    let secret = SECRET.get_or_init(rand::random::<[u8; 32]>);
    let mut hasher = Sha256::new();
    hasher.update(secret);
    for part in ["computer", run_id.trim(), scope.trim(), perm.trim()] {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn proof_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What a request is served against: the switch, the pin, the live
/// sessions and how a new one starts.
pub(crate) struct BrokerCtx<'a> {
    pub enabled: bool,
    /// The Settings switches for the two targets ([`set_targets`]).
    pub sandbox_enabled: bool,
    pub this_mac_enabled: bool,
    /// The person's allowances over This Mac's default deny list.
    pub policy: crate::computer_thismac::Policy,
    /// The host lists the touch audit checks against (BG1, BG2).
    pub hosts: crate::host_scope::Scope,
    /// Where each call's `computer_action` row goes (`None` in tests that
    /// don't look at it).
    pub audit_vault: Option<PathBuf>,
    pub engine: Option<EngineConfig>,
    pub sessions: &'a Sessions,
    pub connect: Connector,
}

fn refused(error: impl Into<String>) -> Value {
    json!({ "ok": false, "error": error.into() })
}

/// Serve one request. Never panics on agent input; every refusal says
/// nothing ran.
pub(crate) async fn serve(req: &ComputerRequest, ctx: &BrokerCtx<'_>) -> Value {
    if !ctx.enabled {
        ctx.sessions.end_all();
        return refused(SWITCHED_OFF);
    }
    let expected = proof_for(&req.run_id, &req.scope, &req.perm);
    if req.run_id.trim().is_empty() || !proof_matches(&expected, req.proof.trim()) {
        return refused(PROOF_REFUSED);
    }
    if crate::app_mcp::normalize_perm(&req.perm) == "plan" {
        audit_early_refusal(ctx, req, "plan mode");
        return refused(PLAN_REFUSED);
    }
    let Some(engine) = ctx.engine.as_ref() else {
        audit_early_refusal(ctx, req, "not set up");
        return refused(NOT_SET_UP);
    };
    let target = engine.target.kind();
    let (on, why) = match target {
        TargetKind::SandboxVm => (ctx.sandbox_enabled, SANDBOX_OFF),
        TargetKind::ThisMac => (ctx.this_mac_enabled, THIS_MAC_OFF),
    };
    if !on {
        audit_early_refusal(ctx, req, "target switched off");
        return refused(why);
    }
    match req.op.as_str() {
        "tools" => {
            // Never a handshake here: an agent CLI drops a server whose tool
            // list is slow, the seven verbs with it. With nothing learned for
            // this pin, the caller starts one warm-up ([`Sessions::warm`]).
            match ctx.sessions.listed_for(engine) {
                Some(tools) => json!({ "ok": true, "tools": filter_tools(&tools, target) }),
                None => json!({ "ok": true, "tools": [], "warming": true }),
            }
        }
        "call" => {
            let (answer, record) = serve_call(req, ctx, engine, target).await;
            let mut record = record;
            touch_audit(&mut record, &ctx.hosts, &answer);
            if let Some(vault) = ctx.audit_vault.as_deref() {
                audit_call(vault, req, Some(target), &record);
            }
            note_activity(req, target, &record, &answer);
            answer
        }
        _ => refused("unknown request; nothing ran."),
    }
}

/// A `call` refused before any target is known (plan mode, nothing set up)
/// still leaves its one row.
fn audit_early_refusal(ctx: &BrokerCtx<'_>, req: &ComputerRequest, detail: &'static str) {
    if req.op != "call" {
        return;
    }
    if let Some(vault) = ctx.audit_vault.as_deref() {
        let rec = CallRecord {
            outcome: "refused",
            detail,
            args: req.arguments.clone(),
            ..CallRecord::default()
        };
        audit_call(vault, req, None, &rec);
    }
}

/// BG2, the after-the-fact touch audit: scan what the call said (its
/// arguments) and what came back (the result's text) for web addresses, note
/// the hosts, and flag each one the host lists do not allow. The row then
/// shows "touched other.example.com, not in the grant". Never changes the
/// call; a flag is evidence, not a refusal.
fn touch_audit(rec: &mut CallRecord, scope: &crate::host_scope::Scope, answer: &Value) {
    let mut text = String::new();
    collect_strings(&rec.args, &mut text);
    if let Some(result) = answer.get("result") {
        collect_strings(&result["content"], &mut text);
    }
    for host in crate::host_scope::hosts_in(&text) {
        if let crate::host_scope::Decision::Denied(why) = scope.decide(&host) {
            rec.out_of_scope.push(format!("{host}: {why}"));
        }
        rec.hosts.push(host);
    }
}

fn collect_strings(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| k.as_str() != "data")
            .for_each(|(_, x)| collect_strings(x, out)),
        _ => {}
    }
}

/// Whether the person holds this chat's desktop (paused or taken over): the
/// audit detail and the words the agent gets. Checked at the top of a call and
/// again once the call has the session, so a call that waited behind another
/// never runs after the person pressed Pause or Take over.
fn held_by_person(scope: &str) -> Option<(&'static str, &'static str)> {
    crate::computer_activity::pause_state(scope).map(|taken_over| {
        if taken_over {
            ("taken over", "The person has taken over the controls for this chat; wait until they hand them back. Nothing ran.")
        } else {
            ("paused", "The person paused computer control for this chat; wait until they resume it. Nothing ran.")
        }
    })
}

/// Tell the chat's card what this call did (CU8): the row and, when the
/// Driver returned one, the screenshot. A card-needed answer is a question,
/// not an action, and is left out.
fn note_activity(req: &ComputerRequest, target: TargetKind, rec: &CallRecord, answer: &Value) {
    if rec.detail == "card needed" {
        return;
    }
    let scope = {
        let s = req.scope.trim();
        if s.is_empty() {
            req.run_id.trim()
        } else {
            s
        }
    };
    let app = match crate::computer_thismac::look(&rec.args) {
        crate::computer_thismac::Look::App(a) if target == TargetKind::ThisMac => {
            Some(a.trim_start_matches("name:").to_string())
        }
        _ => None,
    };
    let entry = crate::computer_activity::Entry {
        id: 0,
        at_ms: 0,
        tool: safe_name(&req.tool),
        tier: rec.tier.map(Tier::as_str).unwrap_or("").to_string(),
        target: match target {
            TargetKind::ThisMac => "thisMac",
            TargetKind::SandboxVm => "sandboxVm",
        }
        .to_string(),
        outcome: rec.outcome.to_string(),
        detail: rec.detail.to_string(),
        app,
        has_frame: false,
    };
    let frame = answer
        .get("result")
        .and_then(crate::computer_activity::first_frame);
    crate::computer_activity::record(scope, entry, frame);
}

/// What one call's audit row says ([`AUDIT_KIND`]).
#[derive(Debug, Default)]
pub(crate) struct CallRecord {
    tier: Option<Tier>,
    /// The arguments as checked (`run_steps` rebuilt), or as sent.
    args: Value,
    /// `ok`, `failed` or `refused`.
    outcome: &'static str,
    /// A fixed category, never the Driver's words (they can echo input).
    detail: &'static str,
    /// SHA-256 of each screenshot's base64 the Driver returned.
    screenshots: Vec<String>,
    /// The cards that let this call run (or the one it is waiting on).
    card: Vec<String>,
    /// BG2: the hosts the call's arguments and result name, and those the
    /// host lists do not allow.
    hosts: Vec<String>,
    out_of_scope: Vec<String>,
}

fn refusal_category(r: &Refusal) -> &'static str {
    match r {
        Refusal::Native => "native verb",
        Refusal::Hidden => "hidden",
        Refusal::NotOnTarget => "not on target",
        Refusal::Unclassified => "unclassified",
        Refusal::TierNotAllowed(_) => "tier not served",
        Refusal::BadName => "bad name",
        Refusal::BadSteps => "bad steps",
    }
}

/// Serve one call and say what its audit row records.
async fn serve_call(
    req: &ComputerRequest,
    ctx: &BrokerCtx<'_>,
    engine: &EngineConfig,
    target: TargetKind,
) -> (Value, CallRecord) {
    let mut rec = CallRecord {
        args: req.arguments.clone(),
        outcome: "refused",
        ..CallRecord::default()
    };
    let scope = {
        let s = req.scope.trim();
        if s.is_empty() {
            req.run_id.trim()
        } else {
            s
        }
    };
    // The person paused this chat's desktop, or took the controls over: the
    // agent waits (nothing ran), and the row says so.
    if let Some((detail, message)) = held_by_person(scope) {
        rec.detail = detail;
        return (refused(message), rec);
    }
    let args = match &req.arguments {
        Value::Null => json!({}),
        Value::Object(_) => req.arguments.clone(),
        _ => {
            rec.detail = "bad arguments";
            return (refused("Arguments must be an object; nothing ran."), rec);
        }
    };
    let (tier, args) = match checked_call(&req.tool, &args, target) {
        Ok(t) => t,
        Err(r) => {
            rec.detail = refusal_category(&r);
            return (refused(r.message(&req.tool)), rec);
        }
    };
    rec.tier = Some(tier);
    rec.args = args.clone();
    // OC3: a key combination that ends the session or the work is refused
    // before it is sent, in every mode.
    if let Some(why) = crate::computer_keys::blocked_hotkey(&req.tool, &args) {
        rec.detail = "blocked hotkey";
        return (
            refused(format!("blocked: this key combination {why}; nothing ran.")),
            rec,
        );
    }
    if !tier_served(target, tier) {
        rec.detail = "tier not served";
        return (
            refused(Refusal::TierNotAllowed(tier).message(&req.tool)),
            rec,
        );
    }
    // Cards: the first one not yet allowed stops the call (nothing ran), and
    // `hollow_computer` shows it, then asks again with the answer. A refused
    // card leaves this row as the record.
    let mode = crate::mode_contract::Mode::of(&req.perm);
    let flags = guard_flags_for_call(&req.tool, &args);
    // This Mac: every action names its app, a denied app is refused in every
    // mode (no card can allow it), and each app (or the whole display) is its
    // own per-chat card outside bypass.
    let mut app_subjects: Vec<(Ask, String)> = Vec::new();
    if target == TargetKind::ThisMac {
        use crate::computer_thismac::Look;
        let policy = &ctx.policy;
        for look in crate::computer_thismac::looks_for_call(&req.tool, &args) {
            match look {
                Look::Bad => {
                    rec.detail = "bad app name";
                    return (refused("An app on this Mac must be named by a plain bundle id or name; nothing ran."), rec);
                }
                Look::Unnamed if tier >= Tier::Act => {
                    rec.detail = "names no app";
                    return (
                        refused(
                            "On this Mac every action must name its app (bundle_id); nothing ran.",
                        ),
                        rec,
                    );
                }
                Look::Unnamed => {
                    if !app_subjects.iter().any(|(a, _)| *a == Ask::Display) {
                        app_subjects.push((Ask::Display, String::new()));
                    }
                }
                Look::App(id) => {
                    if policy.denied(&id) {
                        rec.detail = "app denied";
                        return (refused(format!("{} is on this Mac's default deny list (terminals, System Settings, Keychain Access, password managers and Hollow itself); the person can allow it in Settings. Nothing ran.", safe_name(id.trim_start_matches("name:")))), rec);
                    }
                    if !app_subjects.iter().any(|(a, s)| *a == Ask::App && *s == id) {
                        app_subjects.push((Ask::App, id));
                    }
                }
            }
        }
    }
    let mut needs: Vec<(Ask, String)> = Vec::new();
    for ask in required_asks(mode, target, tier, &flags, engine.signed_in) {
        needs.push((ask, String::new()));
        if ask == Ask::FirstUseThisMac {
            // Per-app cards are skipped in bypass; a whole-screen look is not
            // (it can show a denied app, which no card for one app covers).
            let bypass = mode == crate::mode_contract::Mode::Bypass;
            needs.extend(
                app_subjects
                    .iter()
                    .filter(|(a, _)| !bypass || *a == Ask::Display)
                    .cloned(),
            );
        }
    }
    for (ask, subject) in needs {
        let key = ask_key(ask, &subject);
        let allowed_now = req.approved.contains(&key);
        if !(allowed_now || ask.per_chat() && ctx.sessions.granted(scope, &key)) {
            rec.card.push(key.clone());
            rec.detail = "card needed";
            return (
                json!({
                    "ok": false,
                    "error": "This needs the person's approval first; nothing ran.",
                    "needsCard": { "kind": key, "text": ask.text(&req.tool, &subject) },
                }),
                rec,
            );
        }
        if allowed_now {
            rec.card.push(key.clone());
            if ask.per_chat() {
                ctx.sessions.grant(scope, &key);
            }
        }
    }
    // This Mac's Driver reaches exactly the apps this run has passed the
    // checks for (every app a call names, plus the display on a whole-screen
    // look); the manifest only grows, and a wider one restarts the Driver.
    let manifest = (target == TargetKind::ThisMac).then(|| {
        ctx.sessions.manifest_for(
            req.run_id.trim(),
            app_subjects
                .iter()
                .filter(|(a, _)| *a == Ask::App)
                .map(|(_, s)| s.clone()),
            app_subjects.iter().any(|(a, _)| *a == Ask::Display),
            &ctx.policy,
        )
    });
    let hash = call_hash(&req.tool, &args);
    if let Some(why) = ctx.sessions.run_limit(req.run_id.trim(), scope, hash) {
        ctx.sessions.end(&req.run_id);
        rec.detail = why;
        return (
            refused(format!(
                "This run hit its limit ({why}) and its desktop session was ended; nothing ran. Ask the person to start it again."
            )),
            rec,
        );
    }
    let shared = match ctx
        .sessions
        .get_or_start(&req.run_id, engine, manifest.as_ref(), &ctx.connect)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            rec.detail = "no session";
            return (refused(e), rec);
        }
    };
    let mut session = shared.lock().await;
    if !session.offers(&req.tool) {
        rec.detail = "not offered";
        return (
            refused(format!(
                "The pinned Driver doesn't offer {}; nothing ran.",
                req.tool
            )),
            rec,
        );
    }
    // A call that waited for the session behind another may have been paused
    // or taken over in the meantime.
    if let Some((detail, message)) = held_by_person(scope) {
        drop(session);
        rec.detail = detail;
        return (refused(message), rec);
    }
    session.last_used = Instant::now();
    let outcome = tokio::time::timeout(CALL_TIMEOUT, session.client.call(&req.tool, &args)).await;
    session.last_used = Instant::now();
    let failed = !matches!(&outcome, Ok(Ok(r)) if !r.get("isError").and_then(Value::as_bool).unwrap_or(false));
    ctx.sessions.note_run(req.run_id.trim(), hash, tier, failed);
    drop(session);
    match outcome {
        Ok(Ok(result)) => {
            rec.screenshots = screenshot_hashes(&result);
            let is_error = result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (rec.outcome, rec.detail) = if is_error {
                ("failed", "tool error")
            } else {
                ("ok", "ran")
            };
            let answer = json!({ "ok": true, "tier": tier.as_str(), "result": cap_observed_result(&result) });
            (answer, rec)
        }
        Ok(Err(e)) => {
            ctx.sessions.end(&req.run_id);
            (rec.outcome, rec.detail) = ("failed", "driver error");
            (refused(e), rec)
        }
        Err(_) => {
            // A Driver that missed its deadline may still act later: end
            // the session so its process is killed.
            ctx.sessions.end(&req.run_id);
            (rec.outcome, rec.detail) = ("failed", "timeout");
            (
                refused("The Driver didn't answer in time and was stopped; the action may or may not have happened."),
                rec,
            )
        }
    }
}

// ── Audit (CU3) ──────────────────────────────────────────────────────────

/// The `audit_log` kind for the person's own controls (Pause, Stop, Take
/// over and the rest, from the chat or the tray).
pub(crate) const CONTROL_KIND: &str = "computer_control";

/// Record one of the person's controls in the audit log. Never fails it.
pub(crate) fn audit_control(scope: &str, action: &str) {
    let Some(vault) = AUDIT_VAULT.get() else {
        return;
    };
    let session = (!scope.trim().is_empty()).then(|| scope.trim().to_string());
    crate::audit::record(
        vault,
        crate::audit::AuditEntry {
            kind: CONTROL_KIND.into(),
            session_key: session,
            action: Some(safe_name(action)),
            outcome: Some("ok".into()),
            ..crate::audit::AuditEntry::default()
        },
    );
}

/// The `audit_log` kind for a see-and-act call.
pub(crate) const AUDIT_KIND: &str = "computer_action";
/// Typed text is hashed only from this many characters up: a hash of a
/// short PIN can be reversed by trying them all, so a short one keeps its
/// length only. The text itself is never stored.
const TYPED_HASH_MIN_CHARS: usize = 16;
/// Element labels and app names are cut to this many characters.
const AUDIT_LABEL_CHARS: usize = 80;

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn screenshot_hashes(result: &Value) -> Vec<String> {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("image"))
                .filter_map(|i| i.get("data").and_then(Value::as_str))
                .map(|d| sha256_hex(d.as_bytes()))
                .collect()
        })
        .unwrap_or_default()
}

fn first_text(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match args.get(*k) {
        Some(Value::String(s)) => Some(cut_chars(s, AUDIT_LABEL_CHARS)),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

const AUDIT_MAX_ARG_NAMES: usize = 16;

/// A tool or argument name as logged: only a short plain identifier, so an
/// agent can't smuggle text (a secret, clipboard content) through a name that
/// was refused before it was ever checked.
fn safe_name(name: &str) -> String {
    let ok = !name.is_empty()
        && name.chars().count() <= 48
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
    if ok {
        name.to_string()
    } else {
        "<unnamed>".to_string()
    }
}

/// The row's `fields`: what was asked and where, never what was typed or
/// seen. Typed text keeps its length, plus a SHA-256 only when long enough
/// not to be guessed; screenshots keep their hashes; `card` stays empty
/// until CU2's cards.
pub(crate) fn audit_fields(target: Option<TargetKind>, tool: &str, rec: &CallRecord) -> Value {
    let args = &rec.args;
    let mut f = serde_json::Map::new();
    f.insert(
        "target".into(),
        json!(match target {
            Some(TargetKind::ThisMac) => "thisMac",
            Some(TargetKind::SandboxVm) => "sandboxVm",
            None => "none",
        }),
    );
    f.insert("tool".into(), json!(safe_name(tool)));
    f.insert("tier".into(), json!(rec.tier.map(Tier::as_str)));
    let names: Vec<String> = args
        .as_object()
        .map(|o| {
            o.keys()
                .take(AUDIT_MAX_ARG_NAMES)
                .map(|k| safe_name(k))
                .collect()
        })
        .unwrap_or_default();
    f.insert("argNames".into(), json!(names));
    if let Some(e) = first_text(args, &["element", "ref", "element_index", "label"]) {
        f.insert("element".into(), json!(e));
    }
    if let Some(a) = first_text(args, &["bundle_id", "bundleId", "app", "app_name"]) {
        f.insert("app".into(), json!(a));
    }
    for axis in ["x", "y"] {
        if let Some(n) = args.get(axis).filter(|v| v.is_number()) {
            f.insert(axis.into(), n.clone());
        }
    }
    let typed: Vec<&str> = ["text", "value"]
        .iter()
        .filter_map(|k| args.get(*k).and_then(Value::as_str))
        .collect();
    if !typed.is_empty() {
        let chars: usize = typed.iter().map(|t| t.chars().count()).sum();
        f.insert("typedChars".into(), json!(chars));
        if chars >= TYPED_HASH_MIN_CHARS {
            f.insert(
                "typedSha256".into(),
                json!(sha256_hex(typed.concat().as_bytes())),
            );
        }
    }
    f.insert("screenshots".into(), json!(rec.screenshots));
    if !rec.hosts.is_empty() {
        f.insert("hosts".into(), json!(rec.hosts));
    }
    if !rec.out_of_scope.is_empty() {
        f.insert("outOfScope".into(), json!(rec.out_of_scope));
    }
    f.insert(
        "card".into(),
        if rec.card.is_empty() {
            Value::Null
        } else {
            json!(rec.card)
        },
    );
    Value::Object(f)
}

/// Append one `computer_action` row for a call whose proof checked out.
/// Never fails the call (`audit::record` logs and counts its errors).
fn audit_call(vault: &Path, req: &ComputerRequest, target: Option<TargetKind>, rec: &CallRecord) {
    let scope = req.scope.trim();
    let session = (!scope.is_empty()).then(|| scope.to_string());
    crate::audit::record(
        vault,
        crate::audit::AuditEntry {
            kind: AUDIT_KIND.into(),
            session_key: session.clone(),
            asked_by: session,
            action: Some(safe_name(&req.tool)),
            fields: Some(audit_fields(target, &req.tool, rec)),
            outcome: Some(rec.outcome.into()),
            detail: Some(rec.detail.into()),
            ..crate::audit::AuditEntry::default()
        },
    );
}

// ── Socket ───────────────────────────────────────────────────────────────

static SOCKET_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(crate) fn socket_path() -> Option<PathBuf> {
    SOCKET_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Point [`socket_path`] somewhere for a test (callers hold
/// `mcp_attach::SERVER_TOGGLES`).
#[cfg(test)]
pub(crate) fn set_socket_path_for_test(path: Option<PathBuf>) {
    *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

static SANDBOX_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// This Mac's screen is off until the person turns it on (design section 4).
static THIS_MAC_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The Settings switches for the sandbox Mac and This Mac's screen, as the
/// frontend last sent them. Turning one off ends the live sessions (the next
/// call would be refused anyway; this kills the Driver now).
pub(crate) fn set_targets(sandbox: bool, this_mac: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    let was = targets();
    SANDBOX_ENABLED.store(sandbox, Relaxed);
    THIS_MAC_ENABLED.store(this_mac, Relaxed);
    if (was.0 && !sandbox) || (was.1 && !this_mac) {
        end_all_sessions();
    }
}

/// `(sandbox, this_mac)`.
pub(crate) fn targets() -> (bool, bool) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        SANDBOX_ENABLED.load(Relaxed),
        THIS_MAC_ENABLED.load(Relaxed),
    )
}

/// End every live session now (Computer control switched off). Dropping a
/// session kills its Driver process.
pub(crate) fn end_all_sessions() -> usize {
    crate::computer_activity::clear();
    sessions().end_all()
}

/// The broker's sessions (one set per Hollow process).
pub(crate) fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(Sessions::default)
}

/// The runtime every Driver session lives on (their processes and pipes
/// belong to its reactor).
fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RT: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hollow-computer-driver")
            .enable_all()
            .build()
            .map_err(|e| eprintln!("[computer broker] no runtime: {e}"))
            .ok()
    })
    .as_ref()
}

/// Bind the broker socket and serve. Called once from boot, beside
/// `app_broker::start`. Without a socket the see-and-act tools never list.
/// The vault whose `audit_log` takes the broker's rows (set by [`start`]).
static AUDIT_VAULT: OnceLock<PathBuf> = OnceLock::new();

#[cfg(unix)]
pub(crate) fn start(vault: PathBuf) {
    let _ = AUDIT_VAULT.set(vault);
    let Some(rt) = runtime() else {
        return;
    };
    let dir = crate::instance::cache_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("[computer broker] cannot create {}: {error}", dir.display());
        return;
    }
    let path = dir.join(format!("computer-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(error) => {
            eprintln!("[computer broker] bind {} failed: {error}", path.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
    let handle = rt.handle().clone();
    let spawned = std::thread::Builder::new()
        .name("hollow-computer-broker".into())
        .spawn(move || {
            let mut errors = 0u32;
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        errors = 0;
                        let handle = handle.clone();
                        let _ = std::thread::Builder::new()
                            .name("hollow-computer-request".into())
                            .spawn(move || handle_conn(stream, &handle));
                    }
                    Err(error) => {
                        errors += 1;
                        if errors >= 20 {
                            eprintln!("[computer broker] accept: {error}; stopping");
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50 * u64::from(errors)));
                    }
                }
            }
            let mut current = SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner());
            if current.as_deref() == Some(path.as_path()) {
                *current = None;
            }
            let _ = std::fs::remove_file(&path);
            sessions().end_all();
        });
    // The bounds hold with no call to notice them: every 30 seconds, end the
    // sessions past their expiry or idle timeout.
    let _ = std::thread::Builder::new()
        .name("hollow-computer-sweeper".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            sessions().sweep();
        });
    if let Err(error) = spawned {
        eprintln!("[computer broker] cannot start: {error}");
        *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(not(unix))]
pub(crate) fn start(_vault: PathBuf) {}

/// One request line in, one answer line out, after the peer check.
#[cfg(unix)]
fn handle_conn(stream: std::os::unix::net::UnixStream, rt: &tokio::runtime::Handle) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::fd::AsRawFd;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    {
        let mut reader = BufReader::new((&stream).take(REQUEST_MAX_BYTES));
        if reader.read_line(&mut line).is_err() {
            return;
        }
    }
    let answer = match serde_json::from_str::<ComputerRequest>(line.trim()) {
        Err(error) => refused(format!("unreadable request ({error}); nothing ran.")),
        Ok(req) => match crate::run_auth::check_peer(
            crate::run_auth::global(),
            stream.as_raw_fd(),
            "computer broker",
            Some(&req.run_id),
        ) {
            Err(refusal) => refused(format!("{refusal} Nothing ran.")),
            Ok(()) => {
                let (sandbox_enabled, this_mac_enabled) = targets();
                let ctx = BrokerCtx {
                    enabled: crate::computer_mcp::computer_control_enabled(),
                    sandbox_enabled,
                    this_mac_enabled,
                    policy: crate::computer_thismac::Policy::load(),
                    hosts: crate::host_scope::Scope::effective(),
                    audit_vault: AUDIT_VAULT.get().cloned(),
                    engine: EngineConfig::load(),
                    sessions: sessions(),
                    connect: Arc::new(|cfg, manifest| spawn_session(cfg, manifest)),
                };
                let answer = rt.block_on(serve(&req, &ctx));
                if answer.get("warming").is_some() {
                    if let Some(engine) = ctx.engine.clone() {
                        rt.spawn(sessions().warm(engine, ctx.connect.clone()));
                    }
                }
                answer
            }
        },
    };
    let mut out = answer.to_string();
    out.push('\n');
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
#[path = "computer_broker_tests.rs"]
mod tests;
