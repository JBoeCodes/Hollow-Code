//! Agent CLI versions and updates (Settings, Accounts, Agent
//! updates; the `agents.update_cli` app action).
//!
//! For each agent CLI Hollow finds where it was installed from, and
//! updates it only with that install method's own updater:
//!
//! | Install | Latest version from | Update | Previous version |
//! |---|---|---|---|
//! | Claude Code native installer (`~/.local/share/claude/versions/`) | `downloads.claude.ai/claude-code-releases/<channel>` (what `claude update` reads) | `claude update` | `claude install <version>` |
//! | npm global (`<prefix>/lib/node_modules/<package>`) | `npm view <package> version` | `npm install -g <package>@<latest>` | `npm install -g <package>@<version>` |
//! | Homebrew (`Cellar/` or `Caskroom/`) | `brew info --json=v2` | `brew upgrade [--cask] <name>` | not offered |
//! | Grok's installer | `grok update --check --json` | `grok update` | `grok update --version <version>` |
//! | Antigravity's installer | can't check without updating | `agy update` | not offered |
//!
//! Each agent has an Updates choice: Automatic (the default), Ask first or
//! Off. The worker ([`start_worker`]) checks about daily; Automatic updates
//! only while no Hollow run uses that agent (runs register in
//! [`note_run`]), and while one updates new runs of it are refused with a
//! plain message ([`blocked`]). After an update it re-reads `--version` and
//! the model list, and reports a failure plainly with the previous version
//! offered where the install method can reinstall it.
//!
//! Every command runs under process-group supervision with a bound, in the
//! system temp folder, with Hollow's tool PATH. Output shown to the person
//! is one ANSI-stripped line with anything token-shaped removed.
use crate::agent::AgentKind;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const VERSION_LIMIT: Duration = Duration::from_secs(15);
const CHECK_LIMIT: Duration = Duration::from_secs(45);
const UPDATE_LIMIT: Duration = Duration::from_secs(10 * 60);
const OUTPUT_CAP: usize = 512_000;
/// How often the worker checks each agent.
const CHECK_EVERY: chrono::Duration = chrono::Duration::hours(20);
/// A run's command is built before its process is adopted; count it as in
/// use for this long even before the ledger knows it.
const STARTING_GRACE: Duration = Duration::from_secs(120);
const CLAUDE_RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";

/// When Hollow updates an agent's CLI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
    /// Check about daily and update when no run uses it (the default).
    #[default]
    Automatic,
    /// Check about daily and say when an update is ready.
    Ask,
    /// Never check or update.
    Off,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "automatic" | "auto" => Some(Self::Automatic),
            "ask" | "ask first" => Some(Self::Ask),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Ask => "ask",
            Self::Off => "off",
        }
    }
}

/// One agent's CLI for Settings.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentCliStatus {
    pub agent: String,
    pub label: String,
    pub installed: bool,
    pub version: Option<String>,
    /// The newest version its install method offers; `None` when unknown.
    pub latest: Option<String>,
    pub update_available: bool,
    /// How it was installed, in words ("npm (global)").
    pub method: String,
    /// What Update runs, for the confirm card and the tooltip.
    pub update_command: Option<String>,
    pub can_update: bool,
    /// False when the updater can't check without updating (Antigravity).
    pub can_check: bool,
    /// The version before Hollow's last update or restore.
    pub previous_version: Option<String>,
    /// Whether the install method can reinstall `previous_version`.
    pub can_restore: bool,
    /// Updating while chats run on it is safe (a new version installs
    /// beside the running one), so "Update now" is offered while busy.
    pub safe_while_running: bool,
    /// automatic | ask | off.
    pub policy: String,
    pub last_checked: Option<String>,
    /// idle | checking | updating | waiting (queued until no run uses it).
    pub state: String,
    /// Hollow runs using this agent now.
    pub in_use: u32,
    pub message: Option<String>,
    /// success | warning | danger.
    pub tone: Option<String>,
    /// Repo ideas 2026-10-27 #9: why the installed version is on Hollow's
    /// list of versions known to break it ([`known_bad`]), or None.
    pub known_bad: Option<String>,
    /// Why the latest version (at the last check) is on that list, or None:
    /// Update warns first and Automatic doesn't install it by itself.
    pub latest_known_bad: Option<String>,
}

// ── Versions known to break Hollow ───────────────────────────────────────

/// One entry of `agent_cli_known_bad.json`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
struct KnownBad {
    agent: String,
    version: String,
    reason: String,
}

#[derive(Deserialize)]
struct KnownBadList {
    versions: Vec<KnownBad>,
}

/// Repo ideas 2026-10-27 #9: CLI versions known to break Hollow, each with a
/// reason, shipped inside Hollow (nothing is fetched). Warn, never block.
static KNOWN_BAD: Lazy<Vec<KnownBad>> = Lazy::new(|| {
    // A malformed list can't ship: `the_known_bad_list_ships_valid` fails.
    parse_known_bad(include_str!("agent_cli_known_bad.json")).unwrap_or_default()
});

/// The list, checked: a known agent, an exact version (no spaces, at most
/// 64 characters) and a one-sentence reason (at most 300).
fn parse_known_bad(json: &str) -> Result<Vec<KnownBad>, String> {
    let mut list: KnownBadList = serde_json::from_str(json).map_err(|e| e.to_string())?;
    for entry in &mut list.versions {
        // An alias or another case (`agy`, `Codex`) names the agent too, so
        // it is kept as the agent's own name, which matching compares.
        let Some(kind) = AgentKind::from_opt(&entry.agent) else {
            return Err(format!("{:?} is not an agent Hollow runs", entry.agent));
        };
        entry.agent = kind.as_str().to_string();
        let version = entry.version.trim();
        if version.is_empty() || version.len() > 64 || version.contains(char::is_whitespace) {
            return Err(format!("{:?} is not an exact version", entry.version));
        }
        let reason = entry.reason.trim();
        if reason.is_empty() || reason.chars().count() > 300 {
            return Err(format!(
                "{} {}: give a reason of at most 300 characters",
                entry.agent, version
            ));
        }
    }
    Ok(list.versions)
}

/// Why `version` of `kind` is known to break Hollow, or None.
pub(crate) fn known_bad(kind: AgentKind, version: &str) -> Option<String> {
    known_bad_in(&KNOWN_BAD, kind, version)
}

fn known_bad_in(list: &[KnownBad], kind: AgentKind, version: &str) -> Option<String> {
    let bare = |v: &str| v.trim().trim_start_matches('v').to_string();
    let wanted = bare(version);
    list.iter()
        .find(|e| e.agent == kind.as_str() && bare(&e.version) == wanted)
        .map(|e| e.reason.trim().to_string())
}

#[derive(Clone, Debug, PartialEq)]
enum Method {
    ClaudeNative,
    Npm {
        npm: PathBuf,
        package: &'static str,
    },
    Homebrew {
        brew: PathBuf,
        name: String,
        cask: bool,
    },
    /// The CLI's own `update` command (Antigravity, Grok).
    Own,
    Unknown,
}

fn npm_package(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Claude => Some("@anthropic-ai/claude-code"),
        AgentKind::Codex => Some("@openai/codex"),
        _ => None,
    }
}

/// Where `canonical` (the binary with symlinks resolved) was installed from.
fn detect(kind: AgentKind, canonical: &Path, home: &Path) -> Method {
    let text = canonical.to_string_lossy();
    for (marker, cask) in [("/Caskroom/", true), ("/Cellar/", false)] {
        if let Some((prefix, rest)) = text.split_once(marker) {
            let name = rest.split('/').next().unwrap_or_default();
            if !name.is_empty() {
                return Method::Homebrew {
                    brew: Path::new(prefix).join("bin/brew"),
                    name: name.to_string(),
                    cask,
                };
            }
        }
    }
    if let Some(package) = npm_package(kind) {
        let marker = format!("/lib/node_modules/{package}/");
        if let Some((prefix, _)) = text.split_once(&marker) {
            return Method::Npm {
                npm: Path::new(prefix).join("bin/npm"),
                package,
            };
        }
    }
    match kind {
        AgentKind::Claude if canonical.starts_with(home.join(".local/share/claude/versions")) => {
            Method::ClaudeNative
        }
        AgentKind::Antigravity | AgentKind::Grok => Method::Own,
        _ => Method::Unknown,
    }
}

fn method_label(kind: AgentKind, method: &Method) -> String {
    match method {
        Method::ClaudeNative => "Claude Code's installer".into(),
        Method::Npm { .. } => "npm (global)".into(),
        Method::Homebrew { cask: true, .. } => "Homebrew (cask)".into(),
        Method::Homebrew { .. } => "Homebrew".into(),
        Method::Own => format!("{}'s updater", kind.label()),
        Method::Unknown => "an install Hollow can't update".into(),
    }
}

/// Whether the install method can say what its newest version is
/// (Antigravity's updater can't without updating).
fn can_check(kind: AgentKind, method: &Method) -> bool {
    match method {
        Method::Unknown => false,
        Method::Own => kind != AgentKind::Antigravity,
        _ => true,
    }
}

/// The argv that updates the CLI. npm takes `@latest` (its own newest, never
/// an older version Hollow saw at its last check).
fn update_argv(binary: &Path, method: &Method) -> Option<Vec<String>> {
    let bin = binary.to_string_lossy().into_owned();
    Some(match method {
        Method::ClaudeNative => vec![bin, "update".into()],
        Method::Npm { npm, package } => vec![
            npm.to_string_lossy().into_owned(),
            "install".into(),
            "-g".into(),
            format!("{package}@latest"),
        ],
        Method::Homebrew { brew, name, cask } => {
            let mut argv = vec![brew.to_string_lossy().into_owned(), "upgrade".into()];
            if *cask {
                argv.push("--cask".into());
            }
            argv.push(name.clone());
            argv
        }
        Method::Own => vec![bin, "update".into()],
        Method::Unknown => return None,
    })
}

/// The argv that reinstalls `version`, where the install method can.
fn restore_argv(
    kind: AgentKind,
    binary: &Path,
    method: &Method,
    version: &str,
) -> Option<Vec<String>> {
    if !is_version(version) {
        return None;
    }
    let bin = binary.to_string_lossy().into_owned();
    match method {
        Method::ClaudeNative => Some(vec![bin, "install".into(), version.into()]),
        Method::Npm { npm, package } => Some(vec![
            npm.to_string_lossy().into_owned(),
            "install".into(),
            "-g".into(),
            format!("{package}@{version}"),
        ]),
        Method::Own if kind == AgentKind::Grok => Some(vec![
            bin,
            "update".into(),
            "--version".into(),
            version.into(),
        ]),
        _ => None,
    }
}

/// `2.1.296`, `0.162.1`, `0.1.151-alpha.2`.
fn is_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<_> = core.split('.').collect();
    v.len() <= 40
        && parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

/// The first version-shaped word in `--version` output
/// (`2.1.296 (Claude Code)`, `codex-cli 0.153.4`, `grok 1.0.46 (…)`).
fn parse_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ','))
        .map(|w| w.trim_matches(|c: char| c == '"' || c == '\''))
        .map(|w| w.strip_prefix('v').unwrap_or(w))
        .find(|w| is_version(w))
        .map(str::to_string)
}

/// Numeric dotted compare; a prerelease sorts before its release.
fn newer(a: &str, b: &str) -> bool {
    let split = |v: &str| {
        let (core, pre) = v
            .split_once('-')
            .map_or((v, None), |(c, p)| (c, Some(p.to_string())));
        let nums: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        (nums, pre)
    };
    let (an, ap) = split(a);
    let (bn, bp) = split(b);
    let len = an.len().max(bn.len());
    for i in 0..len {
        let (x, y) = (
            an.get(i).copied().unwrap_or(0),
            bn.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    match (ap, bp) {
        (None, Some(_)) => true,
        (Some(x), Some(y)) => newer_pre(&x, &y),
        _ => false,
    }
}

/// SemVer prerelease order: dot-separated, numbers numerically, build
/// metadata (`+…`) ignored.
fn newer_pre(a: &str, b: &str) -> bool {
    let strip = |v: &str| v.split('+').next().unwrap_or_default().to_string();
    let (a, b) = (strip(a), strip(b));
    let (ai, bi): (Vec<_>, Vec<_>) = (a.split('.').collect(), b.split('.').collect());
    for (x, y) in ai.iter().zip(bi.iter()) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            _ => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord == std::cmp::Ordering::Greater;
        }
    }
    ai.len() > bi.len()
}

/// One line for the person: the last non-empty line, ANSI codes and
/// anything token-shaped removed, at most 200 characters.
fn plain_line(text: &str) -> String {
    let mut clean = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        clean.push(c);
    }
    let line = clean
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default();
    let line = hide_secrets(line);
    if line.chars().count() > 200 {
        format!("{}…", line.chars().take(199).collect::<String>())
    } else {
        line
    }
}

static SECRET_PATTERNS: Lazy<[(regex::Regex, &'static str); 4]> = Lazy::new(|| {
    let re = |p: &str| regex::Regex::new(p).expect("valid pattern");
    [
        // `Bearer abc…`, `Authorization: abc…`.
        (re(r"(?i)\b(bearer|basic)\s+\S+"), "$1 [hidden]"),
        // `_authToken=…`, `api_key: …`, `password=…`, `token=…`.
        (
            re(
                r"(?i)([a-z_]*(?:token|key|secret|password|passwd|authorization|auth|credential)[a-z_]*\s*[=:]\s*)\S+",
            ),
            "$1[hidden]",
        ),
        // `https://user:secret@host`.
        (re(r"(://)[^/\s:@]+:[^/\s@]+@"), "$1[hidden]@"),
        // `sk-…` keys.
        (re(r"\bsk-[A-Za-z0-9_\-]+"), "[hidden]"),
    ]
});
static LONG_RUN: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"[A-Za-z0-9_\-+=]{24,}").expect("valid pattern"));

/// `line` with anything secret-shaped replaced by `[hidden]`: credentials
/// after a key-like word, bearer values, URL passwords, `sk-` keys, and any
/// run of 24+ token characters holding both letters and digits.
fn hide_secrets(line: &str) -> String {
    let mut out = line.to_string();
    for (pattern, with) in SECRET_PATTERNS.iter() {
        out = pattern.replace_all(&out, *with).into_owned();
    }
    LONG_RUN
        .replace_all(&out, |m: &regex::Captures| {
            let run = &m[0];
            let mixed = run.bytes().any(|b| b.is_ascii_digit())
                && run.bytes().any(|b| b.is_ascii_alphabetic());
            if mixed {
                "[hidden]".to_string()
            } else {
                run.to_string()
            }
        })
        .into_owned()
}

// ── Saved state ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AgentState {
    last_checked: Option<String>,
    latest: Option<String>,
    previous_version: Option<String>,
    message: Option<String>,
    tone: Option<String>,
    /// Update asked for while runs used it; the worker does it once free.
    pending: bool,
    /// The latest version known when that Update was asked for. A waiting
    /// Update installs a version on Hollow's known-bad list only if it is
    /// this one, which the person was warned about (repo ideas 10-27 #9).
    pending_for: Option<String>,
    /// The latest version an "update ready" notice was posted for.
    notified: Option<String>,
    /// A version Automatic leaves alone: one it failed to install (until
    /// `skip_until`), or one the person restored away from (no end). A
    /// newer version, or Update, clears it.
    skip: Option<String>,
    skip_until: Option<String>,
}

impl AgentState {
    /// The person's Update waits for runs to finish, for the version known now.
    fn wait_for_runs(&mut self) {
        self.pending = true;
        self.pending_for = self.latest.clone();
    }

    /// Whether a waiting Update may go ahead now that `latest` is the newest
    /// version: always, unless `latest` is on the known-bad list
    /// (`latest_bad`) and isn't the version the person was warned about.
    fn pending_may_install(&self, latest: Option<&str>, latest_bad: bool) -> bool {
        self.pending && (!latest_bad || self.pending_for.as_deref() == latest)
    }

    /// Whether Automatic should leave `latest` alone now.
    fn skips(&self, latest: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
        let Some(latest) = latest else { return false };
        if self.skip.as_deref() != Some(latest) {
            return false;
        }
        match self
            .skip_until
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        {
            Some(until) => now < until,
            None => true,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    policies: BTreeMap<String, Policy>,
    agents: BTreeMap<String, AgentState>,
}

static FILE_LOCK: Mutex<()> = Mutex::new(());

fn saved_path() -> Option<PathBuf> {
    crate::agent_settings::app_data_dir()
        .ok()
        .map(|d| d.join("agent-cli.json"))
}

fn load() -> Saved {
    saved_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Read-modify-write of the saved state.
fn change<T>(f: impl FnOnce(&mut Saved) -> T) -> Result<T, String> {
    let _lock = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut saved = load();
    let out = f(&mut saved);
    let path = saved_path().ok_or("Hollow's app-data folder is unavailable.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, &text)?;
    Ok(out)
}

fn change_agent(kind: AgentKind, f: impl FnOnce(&mut AgentState)) {
    let _ = change(|s| f(s.agents.entry(kind.as_str().into()).or_default()));
}

pub fn policy(kind: AgentKind) -> Policy {
    load()
        .policies
        .get(kind.as_str())
        .copied()
        .unwrap_or_default()
}

pub fn set_policy(agent: &str, value: &str) -> Result<String, String> {
    let kind = AgentKind::from_opt(agent)
        .ok_or_else(|| format!("{agent} is not an agent Hollow runs."))?;
    let policy = Policy::parse(value).ok_or("Updates are automatic, ask or off.")?;
    change(|s| {
        s.policies.insert(kind.as_str().into(), policy);
    })?;
    Ok(policy.as_str().into())
}

// ── Who is using an agent ──────────────────────────────────────────────

static RUNS: Lazy<Mutex<HashMap<String, (AgentKind, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BUSY: Lazy<Mutex<HashMap<&'static str, &'static str>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn count_runs(
    runs: &mut HashMap<String, (AgentKind, Instant)>,
    live: &HashSet<String>,
    kind: AgentKind,
    now: Instant,
) -> u32 {
    runs.retain(|tag, (_, at)| live.contains(tag) || now.duration_since(*at) < STARTING_GRACE);
    runs.values().filter(|(k, _)| *k == kind).count() as u32
}

/// Hollow runs using `kind` now: tagged agent processes still alive, and
/// chat turns in flight.
pub fn runs_using(kind: AgentKind) -> u32 {
    let live = crate::process_group::orphans::global()
        .map(|l| l.live_tags())
        .unwrap_or_default();
    let tagged = count_runs(
        &mut RUNS.lock().unwrap_or_else(|e| e.into_inner()),
        &live,
        kind,
        Instant::now(),
    );
    let chats = crate::agent_sessions::live_all()
        .iter()
        .filter(|s| s.agent == kind.as_str())
        .count() as u32;
    tagged.max(chats)
}

/// The folder (or, for a lone binary, the file) an install lives in: a
/// process running from it is using that CLI, whoever started it.
fn install_root(canonical: &Path, method: &Method, home: &Path) -> (PathBuf, bool) {
    let text = canonical.to_string_lossy();
    match method {
        Method::ClaudeNative => (home.join(".local/share/claude/versions"), true),
        Method::Npm { package, .. } => {
            match text.split_once(&format!("/lib/node_modules/{package}/")) {
                Some((prefix, _)) => (
                    Path::new(prefix).join("lib/node_modules").join(package),
                    true,
                ),
                None => (canonical.to_path_buf(), false),
            }
        }
        Method::Homebrew { name, cask, .. } => {
            let marker = if *cask { "/Caskroom/" } else { "/Cellar/" };
            match text.split_once(marker) {
                Some((prefix, _)) => (
                    Path::new(prefix).join(marker.trim_matches('/')).join(name),
                    true,
                ),
                None => (canonical.to_path_buf(), false),
            }
        }
        Method::Own | Method::Unknown => (canonical.to_path_buf(), false),
    }
}

/// Every process's arguments (`ps -axww -o args=`), for [`count_in`];
/// `None` when `ps` couldn't give the whole listing.
async fn ps_listing() -> Option<String> {
    let mut cmd = tokio::process::Command::new("/bin/ps");
    cmd.args(["-axww", "-o", "args="]);
    let Ok(out) = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        VERSION_LIMIT,
        None,
        32_000_000,
    )
    .await
    else {
        return None;
    };
    if out.stdout_truncated || out.timed_out {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Programs that run a script named on their command line: the script,
/// not the interpreter, says which CLI the process is.
const SCRIPT_RUNNERS: [&str; 4] = ["node", "nodejs", "bun", "bunx"];
/// Script-runner options (node's and bun's) that take the next word as their
/// value (`--require hook.js`); `--opt=value` and every other option are one
/// word. Not exhaustive: after an unlisted bare option the word after the
/// supposed script is checked too, in case that option took a value.
const VALUE_OPTIONS: [&str; 22] = [
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "--env-file",
    "--env-file-if-exists",
    "--title",
    "-C",
    "--conditions",
    "--inspect-port",
    "--debug-port",
    "--input-type",
    "--disable-warning",
    "--redirect-warnings",
    "--watch-path",
    "--max-old-space-size",
    "--stack-size",
    "--preload",
    "--cwd",
    "--tsconfig-override",
    "--port",
];
/// Options that run inline code, so no script follows (`node -e …`).
const INLINE_CODE: [&str; 4] = ["-e", "--eval", "-p", "--print"];
/// `bunx`'s options naming the package to install (`bunx -p <pkg> <bin>`):
/// there `-p` is not node's or bun's `--print`.
const PACKAGE_OPTIONS: [&str; 2] = ["-p", "--package"];
/// The most words one path may span: `ps` joins arguments with spaces, so
/// `/Users/me/My Tools/cli.js` is two words.
const PATH_WORDS: usize = 5;
/// The words of a line read: enough for a program, its options and a
/// script, and no more of a long argv (a `claude -p` carries its prompt).
const LINE_WORDS: usize = 32;

/// How many lines of a `ps` listing run from `root`: the whole install
/// (`tree`) or that one file. A line counts by its program, by the word
/// after it (`node /path/cli.js`), or by a script runner's script past its
/// options (`node --max-old-space-size=4096 /path/cli.js`, `bun run
/// /path/cli.js`), for `bunx`/`bun x` by the install's package name (also
/// as `-p <pkg>`) or one of its commands (`bunx claude`), and for `bun run`
/// by one of its commands (`bun run claude`, `bun claude`); paths resolve through
/// symlinks and may have spaces in them.
///
/// A program typed by bare name in Terminal shows in `ps` as typed (`grok
/// --resume`, no path), so a line whose first word has no `/` also counts
/// when that word is one of the install's `package.json` `bin` commands or
/// one of `names` (the located binary's file name: Grok's own install and
/// Homebrew's are native binaries, with no script to find). This leans
/// towards counting: a different program of that name on PATH counts too,
/// and the cost is only an update that waits. Only the first word: the name
/// later on a line (`echo grok`, a `claude -p` prompt) is an argument.
fn count_in(listing: &str, root: &Path, tree: bool, names: &[String]) -> u32 {
    let package = if tree { package_of(root) } else { None };
    let commands = if tree { commands_of(root) } else { Vec::new() };
    let names_command = |word: Option<&&str>| {
        commands
            .iter()
            .any(|command| names_package(word, Some(command)))
    };
    let bare_name = |word: Option<&&str>| {
        word.is_some_and(|word| {
            let name = program_name(word);
            !word.contains('/')
                && commands
                    .iter()
                    .chain(names.iter())
                    .any(|known| known.as_str() == name)
        })
    };
    // Per path as written: whether it is in the install, and whether it
    // exists (one that doesn't is compared as written).
    let mut resolved: HashMap<String, (bool, bool)> = HashMap::new();
    let mut check = |path: &str| {
        *resolved.entry(path.to_string()).or_insert_with(|| {
            let real = std::fs::canonicalize(path).ok();
            let exists = real.is_some();
            let real = real.unwrap_or_else(|| PathBuf::from(path));
            let hit = if tree {
                real.starts_with(root)
            } else {
                real == root
            };
            (hit, exists)
        })
    };
    let mut count = 0;
    for line in listing.lines() {
        let words: Vec<&str> = line.split_whitespace().take(LINE_WORDS).collect();
        let hit = bare_name(words.first())
            || path_hits(&words, 0, &mut check)
            || path_hits(&words, 1, &mut check)
            || script_at(&words).is_some_and(|script| {
                script.packages.iter().any(|&at| {
                    let word = words
                        .get(at)
                        .map(|w| w.strip_prefix("--package=").unwrap_or(w));
                    names_package(word.as_ref(), package.as_deref())
                }) || script.candidates().any(|at| {
                    (at > 1 && path_hits(&words, at, &mut check))
                        || (script.package && names_package(words.get(at), package.as_deref()))
                        || (script.command && names_command(words.get(at)))
                })
            });
        if hit {
            count += 1;
        }
    }
    count
}

/// Whether the absolute path that starts at `words[at]` is in the install.
/// A path that doesn't exist takes the next word too (a path with spaces),
/// until it exists, an option starts or [`PATH_WORDS`] are used.
fn path_hits(words: &[&str], at: usize, check: &mut impl FnMut(&str) -> (bool, bool)) -> bool {
    if !words.get(at).is_some_and(|w| w.starts_with('/')) {
        return false;
    }
    let mut path = String::new();
    for (i, word) in words[at..].iter().take(PATH_WORDS).enumerate() {
        if i > 0 {
            // An option or another absolute path starts a new word, never a
            // path's continuation (and saves needless disk checks).
            if word.starts_with('-') || word.starts_with('/') {
                return false;
            }
            path.push(' ');
        }
        path.push_str(word);
        let (hit, exists) = check(&path);
        if hit || exists {
            return hit;
        }
    }
    false
}

/// A program's name: `word` past its last `/`.
fn program_name(word: &str) -> &str {
    word.rsplit_once('/').map_or(word, |(_, name)| name)
}

/// Whether `word` is a script runner, by name or at any path.
fn runs_scripts(word: &str) -> bool {
    SCRIPT_RUNNERS.contains(&program_name(word))
}

/// The npm package an install root is (`…/node_modules/@scope/name`).
fn package_of(root: &Path) -> Option<String> {
    let (_, name) = root.to_str()?.rsplit_once("/node_modules/")?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The commands an npm install provides: its `package.json` `bin` names
/// (a lone `bin` path is named after the package, without its scope).
fn commands_of(root: &Path) -> Vec<String> {
    let Some(json) = std::fs::read(root.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return Vec::new();
    };
    match json.get("bin") {
        Some(serde_json::Value::Object(bins)) => bins.keys().cloned().collect(),
        Some(serde_json::Value::String(_)) => json
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| vec![program_name(name).to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Whether a `bunx` argument names `package`, bare or at a version
/// (`@openai/codex@latest`).
fn names_package(word: Option<&&str>, package: Option<&str>) -> bool {
    let (Some(word), Some(package)) = (word, package) else {
        return false;
    };
    word.strip_prefix(package)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('@'))
}

/// Where a script runner's script may be.
struct Script {
    /// The first word after the runner, its options and (for bun) its `run`
    /// or `x` subcommand.
    at: usize,
    /// The last option was bare and unlisted, so it may have taken `at` as
    /// its value: the word after is a candidate too.
    also_next: bool,
    /// A package name may stand there (`bunx pkg`, `bun x pkg`).
    package: bool,
    /// A command of the install may stand there (`bunx claude`, `bun run
    /// claude` or `bun claude`, which run a package's command when no script
    /// has its name).
    command: bool,
    /// The values of `bunx -p <pkg>` / `--package <pkg>`: package names.
    packages: Vec<usize>,
    /// The value words of listed value options, skipped on the way: a path
    /// into the install there (`bun --cwd <install> run cli.js`) still counts,
    /// as it did before those options were listed (review of the H5 follow-up).
    values: Vec<usize>,
}

impl Script {
    /// The word positions the script may be at.
    fn candidates(&self) -> impl Iterator<Item = usize> + '_ {
        (self.at..=self.at + usize::from(self.also_next)).chain(self.values.iter().copied())
    }
}

/// Where a script runner's script is: see [`Script`]. The runner may sit at
/// a path with spaces in it (fnm's `~/Library/Application Support/…/node`).
/// `None` when the line runs no script runner, or runs inline code.
fn script_at(words: &[&str]) -> Option<Script> {
    let program = &words[..words.len().min(PATH_WORDS)];
    let runner = program.iter().position(|&w| runs_scripts(w))?;
    if program[..runner].iter().any(|w| w.starts_with('-')) {
        return None;
    }
    let name = program_name(program[runner]);
    let mut package = name == "bunx";
    let mut subcommand = name == "bun";
    let mut also_next = false;
    let mut values = Vec::new();
    let mut packages = Vec::new();
    let mut at = runner + 1;
    while let Some(&word) = words.get(at) {
        if word == "--" {
            at += 1;
            also_next = false;
            break;
        }
        if subcommand && (word == "run" || word == "x") {
            subcommand = false;
            package = word == "x";
            also_next = false;
            at += 1;
            continue;
        }
        if !word.starts_with('-') {
            break;
        }
        if package {
            if PACKAGE_OPTIONS.contains(&word) {
                packages.push(at + 1);
                also_next = false;
                at += 2;
                continue;
            }
            if word.starts_with("--package=") {
                // [`count_in`] reads the value past the `=`.
                packages.push(at);
                also_next = false;
                at += 1;
                continue;
            }
        }
        if INLINE_CODE.contains(&word) {
            return None;
        }
        let value = VALUE_OPTIONS.contains(&word);
        also_next = !value && !word.contains('=');
        if value {
            values.push(at + 1);
        }
        at += if value { 2 } else { 1 };
    }
    // `bun run <word>` (or its shorthand `bun <word>`): a script of the
    // folder's package.json, else a package's command. Only the command can
    // be recognised from `ps`.
    let command = package || name == "bun";
    Some(Script {
        at,
        also_next,
        package,
        command,
        values,
        packages,
    })
}

/// Everything using this CLI now: Hollow runs, or any process running from
/// its install (teammate CLIs inside `hollow team-mcp`, Terminal sessions:
/// each process's program, by path or by the bare name it was typed as,
/// and, for `node …` wrappers, its script; see [`count_in`]). A missing
/// listing counts as one. Claude Code's native install counts
/// Hollow runs only: a new version installs beside running ones, so a
/// long-lived Terminal or bot session mustn't hold every update back.
fn in_use_from(kind: AgentKind, binary: &Path, method: &Method, listing: Option<&str>) -> u32 {
    let runs = runs_using(kind);
    if *method == Method::ClaudeNative {
        return runs;
    }
    let canonical = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let (root, tree) = install_root(&canonical, method, &dirs::home_dir().unwrap_or_default());
    // The name it is typed as in Terminal (`grok`, `codex`): the located
    // binary's, not the resolved one's (`grok-1.0.52`, `grok-native`).
    let names: Vec<String> = binary
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .into_iter()
        .collect();
    runs.max(listing.map_or(1, |l| count_in(l, &root, tree, &names)))
}

async fn in_use(kind: AgentKind, binary: &Path, method: &Method) -> u32 {
    let listing = if *method == Method::ClaudeNative {
        None
    } else {
        ps_listing().await
    };
    in_use_from(kind, binary, method, listing.as_deref())
}

/// While an update runs, a file in app data tells other Hollow processes
/// (`hollow team-mcp`, where teammates start) to refuse new runs of it.
fn marker(kind: AgentKind) -> Option<PathBuf> {
    crate::agent_settings::app_data_dir()
        .ok()
        .map(|d| d.join(format!("agent-cli-updating-{}", kind.as_str())))
}

/// A marker younger than the longest update (one left by a crash expires).
fn marked(kind: AgentKind) -> bool {
    marker(kind)
        .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < UPDATE_LIMIT + Duration::from_secs(60))
}

/// Admit a run of `kind` (`agent::build_scoped`): refused, with the reason,
/// while its CLI is being replaced; otherwise counted as in use under `tag`.
/// Both under the `BUSY` lock, so an update that takes it next sees the run.
pub(crate) fn admit(kind: AgentKind, tag: Option<&str>) -> Option<String> {
    let busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(why) = refusal(&busy, kind) {
        return Some(why);
    }
    if let Some(tag) = tag {
        RUNS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tag.to_string(), (kind, Instant::now()));
    }
    None
}

#[cfg(test)]
fn blocked(kind: AgentKind) -> Option<String> {
    refusal(&BUSY.lock().unwrap_or_else(|e| e.into_inner()), kind)
}

fn refusal(busy: &HashMap<&'static str, &'static str>, kind: AgentKind) -> Option<String> {
    (busy.get(kind.as_str()) == Some(&"updating") || marked(kind)).then(|| {
        format!(
            "{} is updating (Settings → Accounts → Agent updates). Try again in a minute.",
            kind.label()
        )
    })
}

struct BusyGuard(&'static str, Option<PathBuf>);
impl BusyGuard {
    fn take(kind: AgentKind, what: &'static str) -> Option<Self> {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains_key(kind.as_str()) {
            return None;
        }
        busy.insert(kind.as_str(), what);
        let marker = (what == "updating").then(|| marker(kind)).flatten();
        if let Some(path) = &marker {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, b"");
        }
        Some(Self(kind.as_str(), marker))
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.1 {
            let _ = std::fs::remove_file(path);
        }
        BUSY.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(self.0);
    }
}

// ── Running the CLIs ───────────────────────────────────────────────────

struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

async fn run(argv: &[String], limit: Duration) -> Result<Ran, String> {
    let (program, args) = argv.split_first().ok_or("Nothing to run.")?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .env("PATH", crate::agent::cli_path())
        .current_dir(std::env::temp_dir());
    let out =
        crate::process_group::run_to_completion_capped(&mut cmd, None, limit, None, OUTPUT_CAP)
            .await
            .map_err(|_| {
                format!(
                    "Could not start {}.",
                    Path::new(program)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            })?;
    Ok(Ran {
        ok: out.status.is_some_and(|s| s.success()) && !out.timed_out,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: out.stderr,
        timed_out: out.timed_out,
    })
}

type VersionKey = (PathBuf, Option<std::time::SystemTime>);
static VERSIONS: Lazy<Mutex<HashMap<VersionKey, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// [`installed_version`], remembered per resolved binary and its modified
/// time (an update replaces one or the other), for Settings and the worker.
/// The version cache's answer for `binary` as it is now, without running it.
fn cached_version_now(binary: &Path) -> Option<String> {
    let canonical = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let modified = std::fs::metadata(&canonical)
        .and_then(|m| m.modified())
        .ok();
    VERSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(canonical, modified))
        .cloned()
}

async fn cached_version(binary: &Path) -> Option<String> {
    let canonical = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let key = (
        canonical.clone(),
        std::fs::metadata(&canonical)
            .and_then(|m| m.modified())
            .ok(),
    );
    if let Some(hit) = VERSIONS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(hit.clone());
    }
    let version = installed_version(binary).await?;
    let mut versions = VERSIONS.lock().unwrap_or_else(|e| e.into_inner());
    if versions.len() > 32 {
        versions.clear();
    }
    versions.insert(key, version.clone());
    Some(version)
}

/// Version-only read for transport qualification. Tests never probe an
/// installed agent: they exercise steering's pure version seam instead.
pub(crate) async fn steering_version(kind: AgentKind) -> Option<String> {
    #[cfg(test)]
    {
        let _ = kind;
        None
    }
    #[cfg(not(test))]
    {
        let (binary, _) = locate(kind)?;
        cached_version(&binary).await
    }
}

async fn installed_version(binary: &Path) -> Option<String> {
    let ran = run(
        &[binary.to_string_lossy().into_owned(), "--version".into()],
        VERSION_LIMIT,
    )
    .await
    .ok()?;
    ran.ok.then(|| parse_version(&ran.stdout)).flatten()
}

/// Claude Code's update channel (`autoUpdatesChannel` in its settings:
/// `latest`, the default, or `stable`).
fn claude_channel() -> &'static str {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")));
    let channel = dir
        .and_then(|d| std::fs::read(d.join("settings.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["autoUpdatesChannel"].as_str().map(str::to_string));
    if channel.as_deref() == Some("stable") {
        "stable"
    } else {
        "latest"
    }
}

/// `brew info --json=v2` → the stable version of the formula or cask.
fn parse_brew(json: &str, cask: bool) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let found = if cask {
        v["casks"][0]["version"].as_str()
    } else {
        v["formulae"][0]["versions"]["stable"].as_str()
    }?;
    // Casks can carry a build suffix (`2.1.296,abc`).
    let version = found.split(',').next()?;
    is_version(version).then(|| version.to_string())
}

/// `grok update --check --json` → `latestVersion`.
fn parse_grok_check(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    v["latestVersion"]
        .as_str()
        .filter(|s| is_version(s))
        .map(str::to_string)
}

/// The newest version the install method offers. `Ok(None)`: it can't say.
async fn latest_version(
    kind: AgentKind,
    binary: &Path,
    method: &Method,
) -> Result<Option<String>, String> {
    let checked = |ran: Ran| -> Result<String, String> {
        if ran.timed_out {
            return Err("the check timed out".into());
        }
        if !ran.ok {
            let line = plain_line(&ran.stderr);
            return Err(if line.is_empty() {
                "the check failed".into()
            } else {
                line
            });
        }
        Ok(ran.stdout)
    };
    match method {
        Method::ClaudeNative => {
            let url = format!("{CLAUDE_RELEASES}/{}", claude_channel());
            let text = reqwest::Client::new()
                .get(url)
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|_| "Claude Code's release server didn't answer".to_string())?
                .text()
                .await
                .map_err(|_| "Claude Code's release server didn't answer".to_string())?;
            Ok(Some(text.trim().to_string()).filter(|v| is_version(v)))
        }
        Method::Npm { npm, package } => {
            let out = checked(
                run(
                    &[
                        npm.to_string_lossy().into_owned(),
                        "view".into(),
                        (*package).into(),
                        "version".into(),
                    ],
                    CHECK_LIMIT,
                )
                .await?,
            )?;
            Ok(parse_version(&out))
        }
        Method::Homebrew { brew, name, cask } => {
            let kind_flag = if *cask { "--cask" } else { "--formula" };
            let out = checked(
                run(
                    &[
                        brew.to_string_lossy().into_owned(),
                        "info".into(),
                        "--json=v2".into(),
                        kind_flag.into(),
                        name.clone(),
                    ],
                    CHECK_LIMIT,
                )
                .await?,
            )?;
            Ok(parse_brew(&out, *cask))
        }
        Method::Own if kind == AgentKind::Grok => {
            let out = checked(
                run(
                    &[
                        binary.to_string_lossy().into_owned(),
                        "update".into(),
                        "--check".into(),
                        "--json".into(),
                    ],
                    CHECK_LIMIT,
                )
                .await?,
            )?;
            Ok(parse_grok_check(&out))
        }
        Method::Own | Method::Unknown => Ok(None),
    }
}

/// The CLI on this machine: its binary and how it was installed.
fn locate(kind: AgentKind) -> Option<(PathBuf, Method)> {
    let binary = crate::agent::installed_binary(kind)?;
    let canonical = std::fs::canonicalize(&binary).unwrap_or_else(|_| binary.clone());
    let home = dirs::home_dir().unwrap_or_default();
    Some((binary, detect(kind, &canonical, &home)))
}

/// Check one agent: the newest version its install method offers.
async fn check(kind: AgentKind) {
    let Some((binary, method)) = locate(kind) else {
        return;
    };
    let Some(_busy) = BusyGuard::take(kind, "checking") else {
        return;
    };
    let latest = latest_version(kind, &binary, &method).await;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    change_agent(kind, |s| {
        s.last_checked = Some(now);
        match latest {
            Ok(latest) => {
                s.latest = latest;
                if s.tone.as_deref() == Some("warning")
                    && s.message
                        .as_deref()
                        .is_some_and(|m| m.starts_with("Couldn't check"))
                {
                    s.message = None;
                    s.tone = None;
                }
            }
            Err(why) => {
                s.message = Some(format!(
                    "Couldn't check for a newer {}: {why}.",
                    kind.label()
                ));
                s.tone = Some("warning".into());
            }
        }
    });
}

fn now_text() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

#[derive(Clone, Copy, PartialEq)]
enum Trigger {
    Person,
    Worker,
}

/// What an update or restore did, for the saved message.
struct Outcome {
    message: String,
    tone: &'static str,
    previous: Option<String>,
}

/// Run an update (or a restore to `restore_to`) now.
/// `Ok(None)`: something started using the agent before the update could
/// begin, so it waits (unless `beside`, where updating beside it is safe).
async fn replace(
    kind: AgentKind,
    restore_to: Option<&str>,
    beside: bool,
) -> Result<Option<Outcome>, String> {
    let (binary, method) =
        locate(kind).ok_or_else(|| format!("{} isn't installed.", kind.label()))?;
    let latest = load()
        .agents
        .get(kind.as_str())
        .and_then(|s| s.latest.clone());
    let argv = match restore_to {
        Some(v) => restore_argv(kind, &binary, &method, v).ok_or_else(|| {
            format!(
                "{} can't reinstall an earlier version when installed with {}.",
                kind.label(),
                method_label(kind, &method)
            )
        })?,
        None => update_argv(&binary, &method).ok_or_else(|| {
            format!(
                "Hollow can't update {} installed this way. Update it the way you installed it.",
                kind.label()
            )
        })?,
    };
    // A check under way: the update waits for the next pass.
    let Some(_busy) = BusyGuard::take(kind, "updating") else {
        return Ok(None);
    };
    // New runs are refused from here on; one that started a moment ago
    // makes the update wait for it.
    if !beside && in_use(kind, &binary, &method).await > 0 {
        return Ok(None);
    }
    let before = installed_version(&binary).await;
    let ran = run(&argv, UPDATE_LIMIT).await?;
    let after = installed_version(&binary).await;
    crate::agent_models::forget(kind);
    let verb = if restore_to.is_some() {
        "restore"
    } else {
        "update"
    };
    if !ran.ok {
        let why = if ran.timed_out {
            "it took longer than 10 minutes".to_string()
        } else {
            let line = plain_line(if ran.stderr.trim().is_empty() {
                &ran.stdout
            } else {
                &ran.stderr
            });
            if line.is_empty() {
                "its updater failed".into()
            } else {
                line
            }
        };
        let changed = after.is_some() && after != before;
        return Ok(Some(Outcome {
            message: format!(
                "{} didn't {verb} ({}): {why}.{}",
                kind.label(),
                now_text(),
                if changed { "" } else { " Nothing changed." }
            ),
            tone: "danger",
            previous: if changed { before } else { None },
        }));
    }
    let Some(after) = after else {
        return Ok(Some(Outcome {
            message: format!(
                "{} {verb}d, but it no longer starts (`--version` fails).{}",
                kind.label(),
                before
                    .as_deref()
                    .map(|b| format!(" Restore {b} below if it doesn't recover."))
                    .unwrap_or_default()
            ),
            tone: "danger",
            previous: before,
        }));
    };
    if restore_to.is_none() && before.as_deref() == Some(after.as_str()) {
        if let Some(latest) = latest.filter(|l| newer(l, &after)) {
            return Ok(Some(Outcome {
                message: format!(
                    "{}'s updater finished, but it is still {after} ({latest} is out).",
                    kind.label()
                ),
                tone: "warning",
                previous: None,
            }));
        }
        let models = crate::agent_models::catalog(kind.as_str(), true).await;
        return Ok(Some(Outcome {
            message: match models.warning {
                Some(w) => format!(
                    "{} is up to date ({after}), but its model list didn't load: {w}",
                    kind.label()
                ),
                None => format!("{} is up to date ({after}).", kind.label()),
            },
            tone: "success",
            previous: None,
        }));
    }
    let models = crate::agent_models::catalog(kind.as_str(), true).await;
    let from = before
        .clone()
        .unwrap_or_else(|| "an unknown version".into());
    let done = if restore_to.is_some() {
        format!(
            "Restored {} {after} ({}), from {from}.",
            kind.label(),
            now_text()
        )
    } else {
        format!(
            "Updated {} from {from} to {after} ({}).",
            kind.label(),
            now_text()
        )
    };
    Ok(Some(match models.warning.filter(|_| models.models.is_empty()) {
        Some(w) => Outcome {
            message: format!("{done} Its model list didn't load afterwards: {w} Restore {from} below if it stays broken."),
            tone: "warning",
            previous: before,
        },
        None => match helper_note(kind, &after) {
            Some(note) => Outcome {
                message: format!("{done} {note}"),
                tone: "warning",
                previous: before,
            },
            None => Outcome {
                message: done,
                tone: "success",
                previous: before,
            },
        },
    }))
}

/// Codex's tool-free helpers run only on versions Hollow verified
/// (`codex_probe::VERIFIED`); a new one turns them off until it is.
fn helper_note(kind: AgentKind, version: &str) -> Option<String> {
    let label = format!("codex-cli {version}");
    (kind == AgentKind::Codex && !crate::codex_probe::VERIFIED.contains(&label.as_str())).then(|| {
        format!(
            "Codex's tool-free helpers (text assistance and extraction on Codex) stay off on {version} until Hollow verifies that version; chats and workflows are unaffected."
        )
    })
}

async fn apply(
    kind: AgentKind,
    restore_to: Option<&str>,
    trigger: Trigger,
    beside: bool,
) -> Result<(), String> {
    let outcome = match replace(kind, restore_to, beside).await {
        Ok(Some(outcome)) => Ok(outcome),
        Ok(None) => {
            match (restore_to, trigger) {
                // Never turn a restore into an update: ask to try again.
                (Some(_), _) => change_agent(kind, |s| {
                    s.message = Some(format!(
                        "{} is busy (a check or a run started). Restore again in a moment.",
                        kind.label()
                    ));
                    s.tone = Some("warning".into());
                }),
                (None, Trigger::Person) => change_agent(kind, AgentState::wait_for_runs),
                // The worker tries again on its next pass.
                (None, Trigger::Worker) => {}
            }
            return Ok(());
        }
        Err(e) => Err(e),
    };
    let (message, tone) = match &outcome {
        Ok(o) => (o.message.clone(), o.tone),
        Err(e) => (e.clone(), "danger"),
    };
    let changed = matches!(
        &outcome,
        Ok(Outcome {
            previous: Some(_),
            ..
        })
    );
    let restored = restore_to.is_some() && changed;
    change_agent(kind, |s| {
        s.pending = false;
        s.message = Some(message.clone());
        s.tone = Some(tone.into());
        if restored {
            // Automatic leaves the version restored away from alone.
            s.skip = s.latest.clone();
            s.skip_until = None;
        } else if !changed && tone != "success" && trigger == Trigger::Worker {
            // Don't retry a failed version until the next daily check.
            s.skip = s.latest.clone();
            s.skip_until = Some(
                (chrono::Utc::now() + CHECK_EVERY)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            );
        } else if trigger == Trigger::Person && restore_to.is_none() {
            s.skip = None;
            s.skip_until = None;
        }
        if let Ok(Outcome {
            previous: Some(p), ..
        }) = &outcome
        {
            s.previous_version = Some(p.clone());
        }
    });
    if trigger == Trigger::Worker {
        notify(
            if changed && tone == "success" {
                format!("{} updated", kind.label())
            } else if changed {
                format!("{} updated, but needs a look", kind.label())
            } else if tone == "success" {
                format!("{} is up to date", kind.label())
            } else {
                format!("{} didn't update", kind.label())
            },
            message.clone(),
            if tone == "success" { "info" } else { "warning" },
        );
    }
    outcome.map(|_| ())
}

fn notify(title: String, body: String, kind: &str) {
    use tauri::Manager;
    let Some(app) = crate::notifications::registered_app() else {
        return;
    };
    let Some(vault) = app
        .try_state::<crate::commands::AppState>()
        .filter(|s| s.configured)
        .map(|s| s.config.vault_path.clone())
    else {
        return;
    };
    let _ = crate::notifications::create_registered(
        &vault,
        crate::notifications::NotificationInput {
            title,
            body,
            kind: kind.into(),
            source: Some("agent-updates".into()),
            action_label: Some("Agent updates".into()),
            action_url: Some("settings:accounts".into()),
            notify_native: false,
        },
    );
}

// ── What Settings and the app actions call ─────────────────────────────

async fn status_of(kind: AgentKind, saved: &Saved, listing: Option<&str>) -> AgentCliStatus {
    let state = saved.agents.get(kind.as_str()).cloned().unwrap_or_default();
    let policy = saved
        .policies
        .get(kind.as_str())
        .copied()
        .unwrap_or_default();
    let busy = BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(kind.as_str())
        .copied();
    let located = locate(kind);
    let in_use = match &located {
        Some((binary, method)) => in_use_from(kind, binary, method, listing),
        None => 0,
    };
    let version = match &located {
        Some((binary, _)) => cached_version(binary).await,
        None => None,
    };
    let (method, update_command, can_update, can_check, can_restore, safe) = match &located {
        Some((binary, method)) => {
            let argv = update_argv(binary, method);
            let restore = state
                .previous_version
                .as_deref()
                .and_then(|v| restore_argv(kind, binary, method, v))
                .is_some();
            (
                method_label(kind, method),
                argv.as_ref().map(|a| display_argv(a)),
                argv.is_some(),
                can_check(kind, method),
                restore,
                matches!(method, Method::ClaudeNative),
            )
        }
        None => ("not installed".into(), None, false, false, false, false),
    };
    let update_available = match (&version, &state.latest) {
        (Some(v), Some(l)) => newer(l, v),
        _ => false,
    };
    let known_bad_now = version.as_deref().and_then(|v| known_bad(kind, v));
    let latest_known_bad = state
        .latest
        .as_deref()
        .filter(|_| update_available)
        .and_then(|l| known_bad(kind, l));
    let state_word = match busy {
        Some(b) => b,
        None if state.pending => "waiting",
        None => "idle",
    };
    let message = if state.pending && busy.is_none() {
        Some(format!(
            "Waiting until nothing is using {} ({in_use} running now); it updates then.",
            kind.label()
        ))
    } else {
        state.message.clone()
    };
    AgentCliStatus {
        agent: kind.as_str().into(),
        label: kind.label().into(),
        installed: located.is_some(),
        version,
        latest: state.latest.clone(),
        update_available,
        method,
        update_command,
        can_update,
        can_check,
        previous_version: state.previous_version.clone(),
        can_restore,
        safe_while_running: safe,
        policy: policy.as_str().into(),
        last_checked: state.last_checked.clone(),
        state: state_word.into(),
        in_use,
        message,
        tone: if state.pending {
            Some("warning".into())
        } else {
            state.tone.clone()
        },
        known_bad: known_bad_now,
        latest_known_bad,
    }
}

/// The argv as one line, the program by name.
fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .enumerate()
        .map(|(i, a)| match Path::new(a).file_name() {
            // The program by name; arguments as they are.
            Some(name) if i == 0 => name.to_string_lossy().into_owned(),
            _ => a.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every agent's CLI; `refresh` checks each installed one first.
pub async fn status(refresh: bool) -> Vec<AgentCliStatus> {
    if refresh {
        let mut checks = tokio::task::JoinSet::new();
        for kind in AgentKind::all() {
            checks.spawn(check(kind));
        }
        while checks.join_next().await.is_some() {}
    }
    let saved = load();
    let listing = ps_listing().await;
    let mut rows = Vec::new();
    for kind in AgentKind::all() {
        rows.push(status_of(kind, &saved, listing.as_deref()).await);
    }
    rows
}

pub async fn status_one(kind: AgentKind) -> AgentCliStatus {
    status_of(kind, &load(), ps_listing().await.as_deref()).await
}

/// The person's (or an approved agent's) Update. While runs use the agent
/// it waits for them, unless `now` and updating beside them is safe.
pub async fn update(agent: &str, now: bool) -> Result<AgentCliStatus, String> {
    // A review copy never changes the shared CLIs (and its worker, which
    // would apply a waiting update, doesn't run): plan completion L5.
    if !crate::instance::may_change_shared_setup() {
        return Err(crate::instance::REVIEW_AGENT_CLI_REFUSAL.into());
    }
    let kind = AgentKind::from_opt(agent)
        .ok_or_else(|| format!("{agent} is not an agent Hollow runs."))?;
    let (binary, method) =
        locate(kind).ok_or_else(|| format!("{} isn't installed.", kind.label()))?;
    let beside = now && method == Method::ClaudeNative;
    if !beside && in_use(kind, &binary, &method).await > 0 {
        change_agent(kind, AgentState::wait_for_runs);
        return Ok(status_one(kind).await);
    }
    let _ = apply(kind, None, Trigger::Person, beside).await;
    Ok(status_one(kind).await)
}

/// Reinstall the version before Hollow's last update or restore.
pub async fn restore(agent: &str) -> Result<AgentCliStatus, String> {
    if !crate::instance::may_change_shared_setup() {
        return Err(crate::instance::REVIEW_AGENT_CLI_REFUSAL.into());
    }
    let kind = AgentKind::from_opt(agent)
        .ok_or_else(|| format!("{agent} is not an agent Hollow runs."))?;
    let (binary, method) =
        locate(kind).ok_or_else(|| format!("{} isn't installed.", kind.label()))?;
    let previous = load()
        .agents
        .get(kind.as_str())
        .and_then(|s| s.previous_version.clone())
        .ok_or("There is no earlier version to restore.")?;
    let beside = method == Method::ClaudeNative;
    let in_use = in_use(kind, &binary, &method).await;
    if in_use > 0 && !beside {
        return Err(format!(
            "{} is in use ({in_use} running now). Restore once nothing is using it.",
            kind.label()
        ));
    }
    let _ = apply(kind, Some(&previous), Trigger::Person, beside).await;
    Ok(status_one(kind).await)
}

/// Cancel an update waiting for runs to finish.
pub fn cancel_pending(agent: &str) -> Result<(), String> {
    let kind = AgentKind::from_opt(agent)
        .ok_or_else(|| format!("{agent} is not an agent Hollow runs."))?;
    change_agent(kind, |s| s.pending = false);
    Ok(())
}

/// The `agents.update_cli` confirm card's line (sync: saved state only).
pub fn card_line(agent: &str, now: bool) -> Result<String, String> {
    let kind = AgentKind::from_opt(agent)
        .ok_or_else(|| format!("{agent} is not an agent Hollow runs. Nothing changed."))?;
    let (binary, method) = locate(kind)
        .ok_or_else(|| format!("{} isn't installed. Nothing changed.", kind.label()))?;
    let latest = load()
        .agents
        .get(kind.as_str())
        .and_then(|s| s.latest.clone());
    let argv = update_argv(&binary, &method).ok_or_else(|| {
        format!(
            "Hollow can't update {} installed this way. Nothing changed.",
            kind.label()
        )
    })?;
    let when = if now && method == Method::ClaudeNative {
        "Now, beside any running chats (each keeps the version it started with)."
    } else {
        "If a run is using it, Hollow waits for it to finish."
    };
    // Warn, never block: the latest version, and the one installed now (as
    // the version cache knows it; no process runs for a card), when either
    // is on Hollow's known-bad list.
    let mut warning = latest
        .as_deref()
        .and_then(|l| known_bad(kind, l).map(|why| (l.to_string(), why)))
        .map(|(l, why)| {
            format!(
                " Warning: {} {l} is on Hollow's list of versions known to break it: {why}",
                kind.label()
            )
        })
        .unwrap_or_default();
    if let Some((v, why)) =
        cached_version_now(&binary).and_then(|v| known_bad(kind, &v).map(|why| (v, why)))
    {
        warning.push_str(&format!(
            " The installed {} {v} is on that list too: {why}",
            kind.label()
        ));
    }
    Ok(format!(
        "Update {} to its newest version{} with {} (`{}`). {when}{warning}",
        kind.label(),
        latest
            .map(|l| format!(" ({l} at the last check)"))
            .unwrap_or_default(),
        method_label(kind, &method),
        display_argv(&argv)
    ))
}

// ── The worker ─────────────────────────────────────────────────────────

fn due(last: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    last.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_none_or(|t| now.signed_duration_since(t) > CHECK_EVERY)
}

/// One pass: check what is due, then update what Automatic (or a waiting
/// Update) allows while nothing uses it.
async fn tick() {
    for kind in AgentKind::all() {
        if !crate::agent::is_available(kind) {
            continue;
        }
        let saved = load();
        let policy = saved
            .policies
            .get(kind.as_str())
            .copied()
            .unwrap_or_default();
        let state = saved.agents.get(kind.as_str()).cloned().unwrap_or_default();
        if policy == Policy::Off && !state.pending {
            continue;
        }
        let checking_due =
            policy != Policy::Off && due(state.last_checked.as_deref(), chrono::Utc::now());
        if checking_due {
            check(kind).await;
        }
        let status = status_one(kind).await;
        // Antigravity can't check without updating: Automatic runs its
        // updater once a day instead.
        let blind = !status.can_check && status.can_update && checking_due;
        let skipped = state.skips(status.latest.as_deref(), chrono::Utc::now());
        // A latest version known to break Hollow is never installed by
        // Automatic; the row says why and Update still installs it. A waiting
        // Update installs it only if it is the version the person saw when
        // they asked; one that became listed since waits for them again.
        // Straight from the list, whether or not the installed version could
        // be read (review of #1057).
        let held = status
            .latest
            .as_deref()
            .is_some_and(|l| known_bad(kind, l).is_some());
        let pending = state.pending_may_install(status.latest.as_deref(), held);
        if state.pending && !pending {
            let latest = status.latest.clone().unwrap_or_default();
            let asked_for = state.pending_for.clone();
            change_agent(kind, |s| {
                // Only the request read above: an Update asked for since
                // (for the listed version, after its warning) stays.
                if !s.pending || s.pending_for != asked_for {
                    return;
                }
                s.pending = false;
                s.pending_for = None;
                s.message = Some(format!(
                    "{} {latest} came out while the update waited, and it is known to break Hollow. Update again to install it anyway.",
                    kind.label()
                ));
                s.tone = Some("warning".into());
            });
        }
        let wanted = pending
            || (policy == Policy::Automatic
                && status.can_update
                && ((status.update_available && !skipped && !held) || blind));
        if wanted && status.in_use == 0 {
            let _ = apply(kind, None, Trigger::Worker, false).await;
        } else if policy == Policy::Ask
            && status.update_available
            && state.notified != status.latest
        {
            if let Some(latest) = status.latest.clone() {
                change_agent(kind, |s| s.notified = Some(latest.clone()));
                notify(
                    format!("A {} update is ready", kind.label()),
                    format!(
                        "{} {} is out (you have {}). Update it in Settings → Accounts → Agent updates.",
                        kind.label(),
                        latest,
                        status.version.unwrap_or_else(|| "an older version".into())
                    ),
                    "info",
                );
            }
        }
    }
}

static WORKER: Mutex<bool> = Mutex::new(false);

/// Start the background checker (GUI process, once).
pub fn start_worker() {
    {
        let mut started = WORKER.lock().unwrap_or_else(|e| e.into_inner());
        if *started {
            return;
        }
        *started = true;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(120)).await;
        loop {
            tick().await;
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repo ideas 2026-10-27 #9: the list Hollow ships parses, and every
    /// entry names an agent, an exact version and a reason.
    #[test]
    fn the_known_bad_list_ships_valid() {
        let shipped = parse_known_bad(include_str!("agent_cli_known_bad.json")).unwrap();
        assert_eq!(shipped, *KNOWN_BAD);
        let entry = |agent: &str, version: &str, reason: &str| {
            format!(
                r#"{{"versions":[{{"agent":"{agent}","version":"{version}","reason":"{reason}"}}]}}"#
            )
        };
        assert!(
            parse_known_bad(&entry("claude", "2.1.9", "Stream JSON drops tool results.")).is_ok()
        );
        for bad in [
            entry("cursor", "1.0.0", "x"),
            entry("codex", "", "x"),
            entry("codex", "0.1 beta", "x"),
            entry("codex", "0.1.0", " "),
            entry("codex", "0.1.0", &"x".repeat(301)),
            r#"{"versions":[{"agent":"codex","version":"0.1.0"}]}"#.to_string(),
            "not json".to_string(),
        ] {
            assert!(parse_known_bad(&bad).is_err(), "{bad}");
        }
    }

    /// An exact version (a leading `v` either side is the same), for that
    /// agent only, with its reason.
    #[test]
    fn a_known_bad_version_matches_its_agent_exactly() {
        let list = parse_known_bad(
            r#"{"versions":[
            {"agent":"codex","version":"0.162.1","reason":"Its exec mode ignores --output-schema."},
            {"agent":"grok","version":"v1.0.62","reason":"It hangs on a closed stdin."}
        ]}"#,
        )
        .unwrap();
        assert_eq!(
            known_bad_in(&list, AgentKind::Codex, "0.162.1").as_deref(),
            Some("Its exec mode ignores --output-schema.")
        );
        assert!(known_bad_in(&list, AgentKind::Codex, "v0.162.1").is_some());
        assert!(known_bad_in(&list, AgentKind::Grok, "1.0.62").is_some());
        assert!(known_bad_in(&list, AgentKind::Codex, "0.162.10").is_none());
        assert!(
            known_bad_in(&list, AgentKind::Claude, "0.162.1").is_none(),
            "another agent's version"
        );
    }

    /// Review of #1057: an alias or another case in the list still matches.
    #[test]
    fn a_known_bad_entry_may_name_its_agent_by_alias() {
        let aliased = parse_known_bad(
            r#"{"versions":[
            {"agent":"agy","version":"1.2.15","reason":"Its file tools write outside the folder."},
            {"agent":"Codex","version":"0.170.0","reason":"It drops the session id."}
        ]}"#,
        )
        .unwrap();
        assert!(known_bad_in(&aliased, AgentKind::Antigravity, "1.2.15").is_some());
        assert!(known_bad_in(&aliased, AgentKind::Codex, "0.170.0").is_some());
    }

    /// Review of #1057: a waiting Update installs a listed version only if it
    /// is the one the person saw (and was warned about) when they asked.
    #[test]
    fn a_waiting_update_never_installs_a_version_listed_since() {
        let mut state = AgentState {
            latest: Some("1.2.0".into()),
            ..Default::default()
        };
        assert!(
            !state.pending_may_install(Some("1.2.0"), false),
            "nothing waits"
        );
        state.wait_for_runs();
        assert_eq!(state.pending_for.as_deref(), Some("1.2.0"));
        assert!(state.pending_may_install(Some("1.2.0"), false));
        assert!(
            state.pending_may_install(Some("1.3.0"), false),
            "a newer good version is fine"
        );
        assert!(
            state.pending_may_install(Some("1.2.0"), true),
            "the listed version they were warned about"
        );
        assert!(
            !state.pending_may_install(Some("1.3.0"), true),
            "listed since they asked"
        );
    }

    #[test]
    fn install_methods_are_read_from_the_resolved_path() {
        let home = Path::new("/Users/me");
        assert_eq!(
            detect(
                AgentKind::Claude,
                Path::new("/Users/me/.local/share/claude/versions/2.1.296"),
                home
            ),
            Method::ClaudeNative
        );
        assert_eq!(
            detect(
                AgentKind::Codex,
                Path::new("/opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js"),
                home
            ),
            Method::Npm {
                npm: "/opt/homebrew/bin/npm".into(),
                package: "@openai/codex"
            }
        );
        assert_eq!(
            detect(
                AgentKind::Claude,
                Path::new("/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
                home
            ),
            Method::Npm {
                npm: "/usr/local/bin/npm".into(),
                package: "@anthropic-ai/claude-code"
            }
        );
        assert_eq!(
            detect(
                AgentKind::Codex,
                Path::new("/opt/homebrew/Caskroom/codex/0.162.1/codex-aarch64-apple-darwin"),
                home
            ),
            Method::Homebrew {
                brew: "/opt/homebrew/bin/brew".into(),
                name: "codex".into(),
                cask: true
            }
        );
        assert_eq!(
            detect(
                AgentKind::Codex,
                Path::new("/opt/homebrew/Cellar/codex/0.162.1/bin/codex"),
                home
            ),
            Method::Homebrew {
                brew: "/opt/homebrew/bin/brew".into(),
                name: "codex".into(),
                cask: false
            }
        );
        assert_eq!(
            detect(
                AgentKind::Grok,
                Path::new("/Users/me/.grok/bin/grok-1.0.46"),
                home
            ),
            Method::Own
        );
        assert_eq!(
            detect(
                AgentKind::Antigravity,
                Path::new("/Users/me/.local/bin/agy"),
                home
            ),
            Method::Own
        );
        assert_eq!(
            detect(
                AgentKind::Codex,
                Path::new("/Applications/Codex.app/Contents/Resources/codex"),
                home
            ),
            Method::Unknown
        );
        assert_eq!(
            detect(AgentKind::Claude, Path::new("/Users/me/bin/claude"), home),
            Method::Unknown
        );
    }

    #[test]
    fn each_method_updates_and_restores_with_its_own_tool() {
        let bin = Path::new("/x/claude");
        let npm = Method::Npm {
            npm: "/p/bin/npm".into(),
            package: "@openai/codex",
        };
        assert_eq!(
            update_argv(bin, &Method::ClaudeNative).unwrap(),
            ["/x/claude", "update"]
        );
        // npm's own newest, never a version Hollow saw earlier.
        assert_eq!(
            update_argv(bin, &npm).unwrap(),
            ["/p/bin/npm", "install", "-g", "@openai/codex@latest"]
        );
        let cask = Method::Homebrew {
            brew: "/b/brew".into(),
            name: "codex".into(),
            cask: true,
        };
        assert_eq!(
            update_argv(bin, &cask).unwrap(),
            ["/b/brew", "upgrade", "--cask", "codex"]
        );
        assert!(update_argv(bin, &Method::Unknown).is_none());
        assert_eq!(
            restore_argv(AgentKind::Claude, bin, &Method::ClaudeNative, "2.1.286").unwrap(),
            ["/x/claude", "install", "2.1.286"]
        );
        assert_eq!(
            restore_argv(AgentKind::Codex, bin, &npm, "0.153.4").unwrap()[3],
            "@openai/codex@0.153.4"
        );
        assert_eq!(
            restore_argv(AgentKind::Grok, bin, &Method::Own, "1.0.46").unwrap(),
            ["/x/claude", "update", "--version", "1.0.46"]
        );
        assert!(restore_argv(AgentKind::Antigravity, bin, &Method::Own, "1.3.4").is_none());
        assert!(restore_argv(AgentKind::Codex, bin, &cask, "0.1.0").is_none());
        assert!(restore_argv(AgentKind::Claude, bin, &Method::ClaudeNative, "--force").is_none());
    }

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(
            parse_version("2.1.296 (Claude Code)\n").as_deref(),
            Some("2.1.296")
        );
        assert_eq!(
            parse_version("codex-cli 0.153.4").as_deref(),
            Some("0.153.4")
        );
        assert_eq!(
            parse_version("grok 1.0.46 (2765805b9442) [alpha]").as_deref(),
            Some("1.0.46")
        );
        assert_eq!(parse_version("1.3.4").as_deref(), Some("1.3.4"));
        assert_eq!(
            parse_version("v0.1.151-alpha.2").as_deref(),
            Some("0.1.151-alpha.2")
        );
        assert_eq!(parse_version("nothing here"), None);
        assert!(newer("0.162.1", "0.153.4"));
        assert!(newer("2.1.296", "2.1.287"));
        assert!(newer("1.10.0", "1.9.9"));
        assert!(!newer("1.0.46", "1.0.46"));
        assert!(newer("1.0.0", "1.0.0-alpha.1"));
        assert!(newer("1.0.0-alpha.10", "1.0.0-alpha.2"));
        assert!(newer("1.0.0-beta", "1.0.0-alpha.9"));
        assert!(!newer("1.0.0-alpha.1+build.9", "1.0.0-alpha.1"));
        assert_eq!(
            parse_version("x 1.2.0-dev.3").as_deref(),
            Some("1.2.0-dev.3")
        );
        assert!(!newer("1.0.0-alpha.1", "1.0.0"));
    }

    #[test]
    fn checks_parse_brew_and_grok() {
        assert_eq!(
            parse_brew(
                r#"{"formulae":[{"versions":{"stable":"0.162.1"}}],"casks":[]}"#,
                false
            )
            .as_deref(),
            Some("0.162.1")
        );
        assert_eq!(
            parse_brew(
                r#"{"formulae":[],"casks":[{"version":"2.1.296,abc"}]}"#,
                true
            )
            .as_deref(),
            Some("2.1.296")
        );
        assert_eq!(parse_brew("{}", true), None);
        assert_eq!(
            parse_grok_check(r#"{"currentVersion":"1.0.46","latestVersion":"1.0.52","updateAvailable":true,"error":null}"#).as_deref(),
            Some("1.0.52")
        );
        assert_eq!(parse_grok_check("garbage"), None);
    }

    #[test]
    fn updater_output_is_one_plain_line_without_secrets() {
        assert_eq!(
            plain_line("\u{1b}[31mnpm ERR!\u{1b}[0m code EACCES\n\n"),
            "npm ERR! code EACCES"
        );
        let line = plain_line(
            "auth failed with sk-ant-oat01-abc and abcdefghijklmnopqrstuvwxyz0123456789",
        );
        assert!(
            !line.contains("sk-ant") && !line.contains("abcdefghijklmnop"),
            "{line}"
        );
        assert!(plain_line(&"x ".repeat(300)).chars().count() <= 200);
        for leak in [
            "npm ERR! //registry.npmjs.org/:_authToken=npm_abcDEF0123456789abcdef0123",
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig",
            "fetch https://jb:hunter2pass@example.com/x failed",
            "API_KEY=abc123 rejected",
            "got key c2VjcmV0LXRva2VuLXZhbHVlLWhlcmUxMjM0NTY3OA== back",
        ] {
            let line = plain_line(leak);
            for secret in ["npm_abc", "eyJhb", "hunter2", "abc123", "c2VjcmV0"] {
                assert!(!line.contains(secret), "{line}");
            }
        }
        // Versions and paths survive.
        assert_eq!(
            plain_line("/Users/dev/.local/share/claude/versions/2.1.296 failed"),
            "/Users/dev/.local/share/claude/versions/2.1.296 failed"
        );
        assert_eq!(
            plain_line("installed @openai/codex@0.162.1"),
            "installed @openai/codex@0.162.1"
        );
    }

    #[test]
    fn runs_count_while_alive_or_just_starting() {
        let now = Instant::now();
        let mut runs = HashMap::new();
        runs.insert(
            "a".into(),
            (AgentKind::Claude, now - Duration::from_secs(600)),
        );
        runs.insert("b".into(), (AgentKind::Claude, now));
        runs.insert(
            "c".into(),
            (AgentKind::Claude, now - Duration::from_secs(600)),
        );
        runs.insert("d".into(), (AgentKind::Codex, now));
        let live = HashSet::from(["a".to_string()]);
        assert_eq!(count_runs(&mut runs, &live, AgentKind::Claude, now), 2);
        assert!(!runs.contains_key("c"), "a finished run is forgotten");
        assert_eq!(count_runs(&mut runs, &live, AgentKind::Codex, now), 1);
    }

    #[test]
    fn a_codex_update_says_when_its_helpers_stop() {
        assert!(helper_note(AgentKind::Codex, "0.153.4").is_none());
        // 0.162.1 is verified since night ST12; a newer unverified one says so.
        assert!(helper_note(AgentKind::Codex, "0.162.1").is_none());
        assert!(helper_note(AgentKind::Codex, "0.170.0")
            .unwrap()
            .contains("stay off on 0.170.0"));
        assert!(helper_note(AgentKind::Claude, "9.9.9").is_none());
    }

    #[test]
    fn any_process_from_the_install_counts_as_in_use() {
        let home = Path::new("/Users/me");
        let npm = Method::Npm {
            npm: "/p/bin/npm".into(),
            package: "@openai/codex",
        };
        assert_eq!(
            install_root(
                Path::new("/p/lib/node_modules/@openai/codex/bin/codex.js"),
                &npm,
                home
            ),
            (PathBuf::from("/p/lib/node_modules/@openai/codex"), true)
        );
        assert_eq!(
            install_root(
                Path::new("/x/versions/2.1.296"),
                &Method::ClaudeNative,
                home
            ),
            (
                PathBuf::from("/Users/me/.local/share/claude/versions"),
                true
            )
        );
        let cask = Method::Homebrew {
            brew: "/b/bin/brew".into(),
            name: "codex".into(),
            cask: true,
        };
        assert_eq!(
            install_root(Path::new("/b/Caskroom/codex/1.0/codex"), &cask, home),
            (PathBuf::from("/b/Caskroom/codex"), true)
        );
        assert_eq!(
            install_root(
                Path::new("/Users/me/.grok/bin/grok-1.0.46"),
                &Method::Own,
                home
            ),
            (PathBuf::from("/Users/me/.grok/bin/grok-1.0.46"), false)
        );
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let inside = root.join("vendor/codex");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, "").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&inside, &link).unwrap();
        let listing = format!(
            "/bin/zsh\nnode\n{}\n{}\n{}\n",
            inside.display(),
            link.display(),
            inside.display()
        );
        assert_eq!(
            count_in(&listing, &root, true, &[]),
            3,
            "launch paths resolve through symlinks"
        );
        assert_eq!(count_in(&listing, &inside, false, &[]), 3);
        assert_eq!(count_in(&listing, &root.join("other"), true, &[]), 0);
    }

    #[test]
    fn automatic_skips_a_failed_version_for_a_day_and_a_restored_one_for_good() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-13T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let failed = AgentState {
            skip: Some("0.162.1".into()),
            skip_until: Some("2026-10-14T08:00:00Z".into()),
            ..Default::default()
        };
        assert!(failed.skips(Some("0.162.1"), now));
        assert!(
            !failed.skips(Some("0.163.0"), now),
            "a newer version is tried"
        );
        assert!(!failed.skips(Some("0.162.1"), now + chrono::Duration::days(1)));
        let restored = AgentState {
            skip: Some("0.162.1".into()),
            ..Default::default()
        };
        assert!(restored.skips(Some("0.162.1"), now + chrono::Duration::days(30)));
        assert!(!AgentState::default().skips(Some("0.162.1"), now));
    }

    #[test]
    fn a_node_wrapper_counts_by_its_script() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let cli = root.join("cli.js");
        std::fs::write(&cli, "").unwrap();
        let listing = format!(
            "/opt/homebrew/bin/node {} -p hello\n/bin/zsh -l\nnode {}\n",
            cli.display(),
            cli.display()
        );
        assert_eq!(count_in(&listing, &root, true, &[]), 2);
    }

    /// Nits H5: a script runner's options no longer hide its script, and a
    /// path with spaces in it (which `ps` prints as several words) matches.
    #[test]
    fn a_node_wrapper_counts_past_its_options_and_through_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let root = base.join("My Tools");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("cli.js");
        std::fs::write(&path, "").unwrap();
        let cli = path.display();
        let fnm = base.join("Application Support/fnm/bin/node");
        let lines = [
            format!("node --max-old-space-size=4096 {cli} -p"),
            format!("/usr/local/bin/node --require /x/hook.js --inspect {cli}"),
            format!("{} {cli}", fnm.display()),
            format!("{cli} --resume"),
            format!("node -e {cli}"),
            format!("vim -- {cli}"),
        ];
        let listing = lines.join("\n");
        assert_eq!(count_in(&listing, &root, true, &[]), 4);
        assert_eq!(count_in(&listing, &path, false, &[]), 4);
        assert_eq!(count_in(&listing, &base.join("other"), true, &[]), 0);
        // An install that isn't on disk (the literal comparison), under the
        // test's own folder so no real install can change the answer.
        let npm = base.join("lib/node_modules/@anthropic-ai/claude-code");
        let listing = format!(
            "node --max-old-space-size=4096 {}/cli.js -p\n",
            npm.display()
        );
        assert_eq!(
            count_in(&listing, &npm, true, &[]),
            1,
            "a flag isn't a script"
        );
    }

    /// Review of H5: `bun run <script>` and value-taking node options no
    /// longer hide the script, so the process counts as in use.
    #[test]
    fn bun_scripts_and_value_options_count_by_their_script() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let root = base.join("claude");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("cli.js");
        std::fs::write(&path, "").unwrap();
        let other = base.join("other.js");
        std::fs::write(&other, "").unwrap();
        let cli = path.display();
        let lines = [
            format!("bun run {cli} -p"),
            format!("/Users/me/.bun/bin/bun {cli}"),
            format!("node -r ./hook.js {cli}"),
            format!("node --import tsx {cli}"),
            format!("node --max-old-space-size 4096 {cli}"),
            format!("node --unlisted-option value {cli}"),
        ];
        assert_eq!(count_in(&lines.join("\n"), &root, true, &[]), 6);
        assert_eq!(count_in(&lines.join("\n"), &path, false, &[]), 6);
        // A listed option's value inside the install still counts (it did
        // before the option was listed): a relative script run from there.
        let from_install = format!("bun --cwd {cli} run cli.js");
        assert_eq!(count_in(&from_install, &root, true, &[]), 1);
        let idle = [
            format!("node -e require('{cli}')"),
            format!("node --eval 1 {cli}"),
            format!("bun run {} {cli}", other.display()),
            format!("bun run {}", other.display()),
            format!("bun install {cli}"),
        ];
        assert_eq!(
            count_in(&idle.join("\n"), &root, true, &[]),
            0,
            "inline code and other scripts aren't a CLI run"
        );
    }

    #[test]
    fn bunx_counts_by_its_package() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let npm = base.join("lib/node_modules/@anthropic-ai/claude-code");
        let listing = [
            "bunx @anthropic-ai/claude-code -p hello",
            "/Users/me/.bun/bin/bunx @anthropic-ai/claude-code@latest",
            "bun x --bun @anthropic-ai/claude-code",
            "bunx @anthropic-ai/claude-code-router",
            "bunx cowsay @anthropic-ai/claude-code",
        ]
        .join("\n");
        assert_eq!(count_in(&listing, &npm, true, &[]), 3);
        assert_eq!(
            package_of(&npm).as_deref(),
            Some("@anthropic-ai/claude-code")
        );
        assert_eq!(package_of(&base.join("bin/grok")), None);
    }

    /// Lane H's open review findings: `bunx <command>`, `bunx -p <pkg>` and
    /// `bun run <command>` run the install too.
    #[test]
    fn bunx_and_bun_run_count_by_the_install_commands() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let npm = base.join("lib/node_modules/@openai/codex");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(
            npm.join("package.json"),
            r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();
        let listing = [
            "bunx codex exec hi",
            "bunx codex@latest",
            "bunx -p @openai/codex codex",
            "bunx --package @openai/codex@0.162.1 sh",
            "bunx --package=@openai/codex codex",
            "bun x codex",
            "bun run codex",
            "/Users/me/.bun/bin/bun --bun run codex --help",
            "bun codex",
        ]
        .join("\n");
        assert_eq!(count_in(&listing, &npm, true, &[]), 9);
        let idle = [
            "bunx -p cowsay cowsay codex",
            "bunx codex-helper",
            "bun run dev",
            "bun -p codex",
            "bun install codex",
            "node -p codex",
        ]
        .join("\n");
        assert_eq!(
            count_in(&idle, &npm, true, &[]),
            0,
            "other packages and commands"
        );
        assert_eq!(commands_of(&npm), vec!["codex".to_string()]);
        let lone = base.join("lib/node_modules/@scope/tool");
        std::fs::create_dir_all(&lone).unwrap();
        std::fs::write(
            lone.join("package.json"),
            r#"{"name":"@scope/tool","bin":"cli.js"}"#,
        )
        .unwrap();
        assert_eq!(commands_of(&lone), vec!["tool".to_string()]);
        assert!(commands_of(&base).is_empty(), "no package.json");
    }

    /// Second-pass review of #653/#654: a CLI typed by bare name in Terminal
    /// shows in `ps` without a path (`grok --resume`), and native binaries
    /// have no script to find, so it never counted and an automatic update
    /// could replace it while it ran.
    #[test]
    fn a_cli_typed_by_name_counts_as_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let bin = base.join(".grok/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let real = bin.join("grok-1.0.52");
        std::fs::write(&real, "").unwrap();
        let link = bin.join("grok");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let listing = "grok --resume\n/bin/zsh -l\ngrokker\n";
        assert_eq!(
            in_use_from(AgentKind::Grok, &link, &Method::Own, Some(listing)),
            1
        );
        let idle = [
            "grokker",
            "node",
            "echo grok",
            "claude -p ask grok about it",
            "/bin/zsh -l",
        ]
        .join("\n");
        let names = vec!["grok".to_string()];
        assert_eq!(count_in(listing, &real, false, &names), 1);
        assert_eq!(
            count_in(&idle, &real, false, &names),
            0,
            "only the first word names the program"
        );
        // An npm tree (as Claude's or Codex's): its `bin` commands are its
        // names. (npm's native Grok is detected as its own install, so it
        // matches through the binary's name instead.)
        let npm = base.join("lib/node_modules/@xai-official/grok");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(
            npm.join("package.json"),
            r#"{"name":"@xai-official/grok","bin":{"grok":"bin/grok"}}"#,
        )
        .unwrap();
        assert_eq!(count_in("grok\n", &npm, true, &[]), 1);
        assert_eq!(count_in("echo grok\n", &npm, true, &[]), 0);
        assert_eq!(count_in("grok\n", &base.join("other"), true, &[]), 0);
    }

    #[test]
    fn policies_parse_and_default_to_automatic() {
        assert_eq!(Policy::default(), Policy::Automatic);
        assert_eq!(Policy::parse("Ask first"), Some(Policy::Ask));
        assert_eq!(Policy::parse("off"), Some(Policy::Off));
        assert_eq!(Policy::parse("sometimes"), None);
    }

    #[test]
    fn checks_fall_due_daily() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-13T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(due(None, now));
        assert!(!due(Some("2026-10-13T08:00:00Z"), now));
        assert!(due(Some("2026-10-12T08:00:00Z"), now));
    }

    #[test]
    fn a_run_is_refused_only_while_its_agent_updates() {
        assert!(blocked(AgentKind::Grok).is_none());
        let guard = BusyGuard::take(AgentKind::Grok, "updating").unwrap();
        assert!(blocked(AgentKind::Grok)
            .unwrap()
            .contains("Grok is updating"));
        assert!(
            BusyGuard::take(AgentKind::Grok, "checking").is_none(),
            "one at a time"
        );
        drop(guard);
        assert!(blocked(AgentKind::Grok).is_none());
        let _check = BusyGuard::take(AgentKind::Grok, "checking").unwrap();
        assert!(
            blocked(AgentKind::Grok).is_none(),
            "a check doesn't block runs"
        );
        // Another Hollow process sees an update by its marker.
        let path = marker(AgentKind::Codex).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        assert!(refusal(&HashMap::new(), AgentKind::Codex).is_some());
        std::fs::remove_file(&path).unwrap();
        assert!(refusal(&HashMap::new(), AgentKind::Codex).is_none());
    }

    /// A fake CLI whose `update` bumps the version `--version` prints.
    fn fake_cli(dir: &Path, update: &str) -> PathBuf {
        let bin = dir.join("fake-agent");
        let state = dir.join("version");
        std::fs::write(&state, "1.0.0").unwrap();
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n--version) cat '{s}';;\nupdate) {update};;\ninstall) printf '%s' \"$2\" > '{s}';;\n*) exit 2;;\nesac\n",
                s = state.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        bin
    }

    #[tokio::test]
    async fn a_fake_cli_updates_and_its_version_is_reread() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_cli(
            dir.path(),
            &format!("printf 1.1.0 > '{}'", dir.path().join("version").display()),
        );
        assert_eq!(installed_version(&bin).await.as_deref(), Some("1.0.0"));
        let ran = run(&update_argv(&bin, &Method::Own).unwrap(), VERSION_LIMIT)
            .await
            .unwrap();
        assert!(ran.ok);
        assert_eq!(installed_version(&bin).await.as_deref(), Some("1.1.0"));
        let restore =
            restore_argv(AgentKind::Claude, &bin, &Method::ClaudeNative, "1.0.0").unwrap();
        assert!(run(&restore, VERSION_LIMIT).await.unwrap().ok);
        assert_eq!(installed_version(&bin).await.as_deref(), Some("1.0.0"));
        // A failing updater's message comes back as one plain line.
        let failing = fake_cli(
            dir.path(),
            "echo 'npm ERR! EACCES: permission denied' >&2; exit 1",
        );
        let ran = run(&update_argv(&failing, &Method::Own).unwrap(), VERSION_LIMIT)
            .await
            .unwrap();
        assert!(!ran.ok);
        assert_eq!(
            plain_line(&ran.stderr),
            "npm ERR! EACCES: permission denied"
        );
    }

    #[test]
    fn the_update_command_reads_short() {
        let home = dirs::home_dir().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            display_argv(&[format!("{home}/.local/bin/claude"), "update".into()]),
            "claude update"
        );
        assert_eq!(
            display_argv(&[
                "/opt/homebrew/bin/npm".into(),
                "install".into(),
                "-g".into(),
                "@openai/codex@0.162.1".into()
            ]),
            "npm install -g @openai/codex@0.162.1"
        );
    }
}
