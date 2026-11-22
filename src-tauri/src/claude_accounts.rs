//! Several Claude subscription accounts, and which one each run uses.
//!
//! Claude plan limits (the five-hour block and the weekly allowance) are per
//! account. Someone who pays for more than one plan can spread Hollow's
//! Claude runs across them and stay out of extra usage. This module holds
//! the accounts, measures each one's usage, and picks the account for every
//! Claude run Hollow starts from the GUI process.
//!
//! **How a run switches account.** Claude Code keeps its browser login in ONE
//! Keychain item shared by every process on the machine, but the
//! `CLAUDE_CODE_OAUTH_TOKEN` environment variable overrides it for a single
//! process (and fails closed: a bad token errors, it never falls back). So an
//! extra account is a token minted with `claude setup-token` while signed in
//! as that account (valid a year, inference-only), stored in Hollow's own
//! Keychain item (service [`KEYCHAIN_SERVICE`], account = the account id).
//! [`apply`] sets it on the child. Only accounts added in Settings are listed
//! and routed; Claude Code's own login ([`LOGIN_ID`], selected by leaving the
//! variable unset) is used when no account has been added. If added accounts
//! have no room, admission waits instead of falling back to that login. The
//! Claude config dir stays shared (never `CLAUDE_CONFIG_DIR`), so `--resume`
//! works across accounts: the account paying changes, the conversation
//! continues. Inference-only scope loses nothing here: Hollow already runs
//! Claude with `--strict-mcp-config`, so claude.ai connectors never apply.
//!
//! **Automatic rotation.** With no token account saved, [`apply`] does nothing.
//! Otherwise every pick uses binding-window headroom (see [`pick`]); retired
//! single-account and failover-off settings migrate away on load.
//!
//! **Usage.** Every OAuth API response carries `anthropic-ratelimit-unified-*`
//! headers with the credential's five-hour and weekly utilization, and a 429
//! carries them too (a capped account is a reading, not an error). [`probe`]
//! sends a one-token Haiku request per token account, at most once per
//! [`PROBE_MIN_INTERVAL_SECS`] unless forced. Runs also report their usage
//! for free: each Claude stream's `rate_limit_event` is attributed to the
//! account that run used ([`observe_stream`], called by the usage ledger,
//! which learns the account from [`ACCOUNT_ENV`] on the command). A run on
//! the login feeds only the Settings, Usage plan card; Hollow never reads
//! Claude Code's Keychain item here.
//!
//! **GUI only.** Nothing is chosen until [`register_gui`] runs (lib.rs
//! setup), so a `hollow team-mcp` delegate never reads a token: its Claude
//! teammate inherits whatever its parent passed down.
//!
//! **Hygiene.** Tokens never reach IPC, logs or the vault. They are held in
//! [`Secret`] (redacted `Debug`, zeroed on drop) and sent only in a
//! sensitive header or the child's environment. `claude_runner::redact`
//! already masks `sk-ant-oat01-…`.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;
use tokio::process::Command;

use crate::claude_quota::StreamQuota;

/// The built-in account: Claude Code's own Keychain login.
pub const LOGIN_ID: &str = "login";
/// Claude Code's per-process credential override.
pub const TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// Which account a command was given (the id, never a secret). The usage
/// ledger reads it back from the command to attribute the run's usage.
pub const ACCOUNT_ENV: &str = "HOLLOW_CLAUDE_ACCOUNT";
pub const FILE: &str = "claude-accounts.json";
const SCHEMA: u32 = 1;
const MAX_ACCOUNTS: usize = 8;
const PROBE_BASE: &str = "https://api.anthropic.com";
const PROBE_MODEL: &str = "claude-haiku-4-5";
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("Hollow/", env!("CARGO_PKG_VERSION"));
/// A probe per account at most this often, unless forced.
pub const PROBE_MIN_INTERVAL_SECS: i64 = 120;
/// How often the background refresher probes, while spreading.
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// A conversation keeps its account while that account is within this many
/// percentage points of the least-used one (its prompt cache lives there).
const DEFAULT_CEILING: u8 = 90;
/// Long unattended work (automations, the night queue, subagents) avoids
/// starting on an account at or above [`long_work_ceiling`] of a live window
/// when another eligible account has room (master plan #16, decision 6).
/// Since the CFO (COO rebuild L7) that line is `100 - reserve`, the reserve
/// from Settings, Usage (`cfo::settings`); this is its default reserve.
const DEFAULT_LONG_WORK_RESERVE: u8 = 20;
static LONG_WORK_RESERVE: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(DEFAULT_LONG_WORK_RESERVE);

/// The CFO's pool ceiling: the percent of a window long work may start at.
pub fn long_work_ceiling() -> f64 {
    100.0 - f64::from(LONG_WORK_RESERVE.load(Ordering::Relaxed).min(90))
}

/// Set by the CFO whenever it reads or saves its policy (never in tests:
/// `cfo::settings::publish_reserve`).
#[cfg_attr(test, allow(dead_code))]
pub fn set_long_work_reserve(reserve: u8) {
    LONG_WORK_RESERVE.store(reserve.min(90), Ordering::Relaxed);
}
const MAX_AFFINITIES: usize = 500;

// ── Stored settings (no secrets) ────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Routing {
    /// Legacy input only; migrated to automatic rotation.
    Single,
    /// The least-used eligible account for each run.
    #[default]
    Spread,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAccount {
    id: String,
    label: String,
    #[serde(default = "yes")]
    enabled: bool,
    created_at: String,
    /// The organisation the person says this account belongs to (#16).
    /// Failover and long-work routing stay inside one organisation unless
    /// [`Store::failover_across_organizations`] is on. Unset accounts form
    /// one group of their own, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    organization: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    schema_version: u32,
    #[serde(default, skip_serializing)]
    routing: Routing,
    /// Legacy pin input, cleared on load and never serialized.
    #[serde(default, skip_serializing)]
    pinned: String,
    #[serde(default = "default_ceiling")]
    ceiling_percent: u8,
    /// Legacy opt-out input, migrated to true and never serialized.
    #[serde(default = "yes", skip_serializing)]
    failover: bool,
    /// Let failover and long-work routing move to an account of another
    /// organisation (#16). Off unless the person turns it on.
    #[serde(default)]
    failover_across_organizations: bool,
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    /// A provider rejection outranks meters until its reset, including after restart.
    #[serde(default)]
    exhausted_accounts: BTreeMap<String, Rejection>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rejection {
    reset_at: Option<i64>,
    meter_mismatch: bool,
}

impl Rejection {
    fn live(&self, now: DateTime<Utc>) -> bool {
        self.reset_at.is_none_or(|reset| reset > now.timestamp())
    }
}

fn yes() -> bool {
    true
}
fn default_ceiling() -> u8 {
    DEFAULT_CEILING
}

impl Default for Store {
    fn default() -> Self {
        Store {
            schema_version: SCHEMA,
            routing: Routing::Spread,
            pinned: String::new(),
            ceiling_percent: DEFAULT_CEILING,
            failover: true,
            failover_across_organizations: false,
            accounts: Vec::new(),
            exhausted_accounts: BTreeMap::new(),
        }
    }
}

fn load_store(dir: &Path) -> Result<Store, String> {
    let path = dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(e) => return Err(format!("Claude accounts file unreadable: {e}")),
    };
    let raw: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "Claude accounts file needs repair ({}): {e}",
            path.display()
        )
    })?;
    let legacy_settings = ["routing", "failover", "pinned"]
        .iter()
        .any(|key| raw.get(key).is_some());
    let mut store: Store = serde_json::from_value(raw).map_err(|e| {
        format!(
            "Claude accounts file needs repair ({}): {e}",
            path.display()
        )
    })?;
    if store.schema_version != SCHEMA {
        return Err(format!(
            "Unsupported Claude accounts version {}",
            store.schema_version
        ));
    }
    // Retired settings cannot turn automatic rotation off on upgraded installs.
    if legacy_settings {
        store.routing = Routing::Spread;
        store.failover = true;
        store.pinned.clear();
        save_store(dir, &store)?;
    }
    Ok(store)
}

fn save_store(dir: &Path, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let value = serde_json::to_value(store).map_err(|e| e.to_string())?;
    let map: BTreeMap<String, serde_json::Value> =
        serde_json::from_value(value).map_err(|e| e.to_string())?;
    crate::setup::publish_config(&dir.join(FILE), &map)
}

// ── Secrets ─────────────────────────────────────────────────────────────────

/// A minted account token. Never printed, zeroed on drop.
pub(crate) struct Secret(Vec<u8>);

impl Secret {
    fn new(token: String) -> Self {
        Secret(token.into_bytes())
    }
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Secret(self.0.clone())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        crate::claude_quota::zero(&mut self.0);
    }
}

/// Where tokens live. The Keychain in the app; memory in tests.
trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<Secret>, String>;
    /// Whether a token is saved, without reading it (a token set to come from
    /// 1Password counts, and `op` is not run).
    fn present(&self, id: &str) -> Result<bool, String> {
        self.get(id).map(|found| found.is_some())
    }
    fn set(&self, id: &str, token: &Secret) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// Tokens live in Hollow's one shared Keychain item (`secret_store`), so
/// several accounts cost at most one Keychain prompt per build.
struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, id: &str) -> Result<Option<Secret>, String> {
        crate::secret_store::get(&crate::secret_store::claude_key(id))
            .map(|found| found.map(Secret::new))
            .map_err(|_| "Could not read a Claude account token from the Keychain.".into())
    }
    fn present(&self, id: &str) -> Result<bool, String> {
        crate::secret_store::present(&crate::secret_store::claude_key(id))
            .map_err(|_| "Could not read a Claude account token from the Keychain.".into())
    }
    fn set(&self, id: &str, token: &Secret) -> Result<(), String> {
        crate::secret_store::set(&crate::secret_store::claude_key(id), token.as_str())
            .map_err(|_| "Could not save the token in the Keychain.".to_string())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        crate::secret_store::delete(&crate::secret_store::claude_key(id))
            .map_err(|_| "Could not remove the token from the Keychain.".into())
    }
}

/// Strip every whitespace character (a token copied from a wrapped terminal
/// line brings the wrap back as spaces, which the API rejects exactly like a
/// revoked token) and check the shape.
pub(crate) fn normalize_token(raw: &str) -> Result<String, String> {
    let token: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if token.is_empty() {
        return Err("Paste the token `claude setup-token` printed.".into());
    }
    if token.starts_with("sk-ant-api") {
        return Err("That is an API key, which bills per token instead of using a plan. Paste the sk-ant-oat01-… token `claude setup-token` printed.".into());
    }
    if !token.starts_with("sk-ant-oat") {
        return Err("That doesn't look like a Claude Code token. Run `claude setup-token` while signed in as the account and paste the sk-ant-oat01-… line.".into());
    }
    if token.len() < 40 || token.len() > 512 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(
            "The token looks incomplete or damaged. Copy the whole sk-ant-oat01-… line again."
                .into(),
        );
    }
    Ok(token)
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSource {
    /// A one-token probe's response headers.
    Probe,
    /// A `rate_limit_event` in one of the account's own runs.
    Stream,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    used_percent: f64,
    resets_at: Option<i64>,
}

impl Window {
    fn live(&self, now: DateTime<Utc>) -> bool {
        self.resets_at.is_none_or(|at| at > now.timestamp())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Usage {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
    /// `allowed` | `allowed_warning` | `rejected`.
    status: Option<String>,
    overage_reason: Option<String>,
    source: UsageSource,
    observed: DateTime<Utc>,
}

impl Usage {
    fn windows(&self) -> impl Iterator<Item = &Window> {
        self.five_hour.iter().chain(self.seven_day.iter())
    }

    /// The higher of the live windows, in percent. A window past its reset
    /// counts as empty.
    fn load(&self, now: DateTime<Utc>) -> f64 {
        self.windows()
            .filter(|w| w.live(now))
            .map(|w| w.used_percent)
            .fold(0.0, f64::max)
    }

    /// Out of plan allowance until a window resets: a live window at 100 %,
    /// or a `rejected` status while some window is still live.
    fn blocked(&self, now: DateTime<Utc>) -> bool {
        let live: Vec<&Window> = self.windows().filter(|w| w.live(now)).collect();
        live.iter().any(|w| w.used_percent >= 100.0)
            || (self.overage_reason.is_some() && self.load(now) >= 98.0)
            || (self.status.as_deref() == Some("rejected") && !live.is_empty())
    }

    /// No room for a run (R1): out of allowance, or at or over `ceiling`
    /// in a live window. What [`pick`] calls ineligible.
    fn exhausted(&self, ceiling: f64, now: DateTime<Utc>) -> bool {
        self.blocked(now) || self.load(now) >= ceiling
    }

    /// When an [`exhausted`](Self::exhausted) account has room again: the
    /// latest reset of the live windows holding it back (every live one for
    /// a `rejected` status with no full window). `None` when one of them
    /// has no reset time.
    fn room_again_at(&self, ceiling: f64, now: DateTime<Utc>) -> Option<i64> {
        let live: Vec<&Window> = self.windows().filter(|w| w.live(now)).collect();
        let full: Vec<&Window> = live
            .iter()
            .copied()
            .filter(|w| w.used_percent >= ceiling.min(100.0))
            .collect();
        let holding = if full.is_empty() { live } else { full };
        holding
            .iter()
            .map(|w| w.resets_at)
            .collect::<Option<Vec<i64>>>()?
            .into_iter()
            .max()
    }
}

/// What a probe learned.
#[derive(Debug, Clone, PartialEq)]
enum ProbeOutcome {
    Usage(Usage),
    /// 401/403 with no usage headers: revoked, expired or mangled.
    Invalid,
    Failed(String),
}

/// Read the `anthropic-ratelimit-unified-*` headers. Any status: a 429 from a
/// capped account carries the same headers as a 200.
fn parse_headers(
    status: u16,
    get: &dyn Fn(&str) -> Option<String>,
    now: DateTime<Utc>,
) -> ProbeOutcome {
    let h = |name: &str| get(&format!("anthropic-ratelimit-unified-{name}"));
    let window = |prefix: &str| -> Option<Window> {
        let used = h(&format!("{prefix}-utilization"))?
            .trim()
            .parse::<f64>()
            .ok()?;
        if !used.is_finite() || used < 0.0 {
            return None;
        }
        Some(Window {
            used_percent: round2(used * 100.0),
            resets_at: h(&format!("{prefix}-reset")).and_then(|v| v.trim().parse::<i64>().ok()),
        })
    };
    let five_hour = window("5h");
    let seven_day = window("7d");
    if five_hour.is_none() && seven_day.is_none() {
        return match status {
            401 | 403 => ProbeOutcome::Invalid,
            other => ProbeOutcome::Failed(format!("usage check returned HTTP {other}")),
        };
    }
    let worst = [h("5h-status"), h("7d-status"), h("status")]
        .into_iter()
        .flatten()
        .max_by_key(|s| match s.as_str() {
            "rejected" => 2,
            "allowed_warning" => 1,
            _ => 0,
        });
    ProbeOutcome::Usage(Usage {
        five_hour,
        seven_day,
        status: worst,
        overage_reason: h("overage-disabled-reason").filter(|v| !v.trim().is_empty()),
        source: UsageSource::Probe,
        observed: now,
    })
}

fn round2(n: f64) -> f64 {
    (n * 100.0).round() / 100.0
}

/// Direct provider reset text wins. Without it, wait for the blocking
/// window, or the next known window boundary before testing capacity again.
fn rejection_reset(usage: &Usage, ceiling: f64, now: DateTime<Utc>) -> Option<i64> {
    if usage.exhausted(ceiling, now) {
        usage.room_again_at(ceiling, now)
    } else {
        usage
            .windows()
            .filter_map(|w| w.resets_at)
            .filter(|reset| *reset > now.timestamp())
            .min()
    }
}

fn usage_from_stream(quota: &StreamQuota, now: DateTime<Utc>) -> Usage {
    let window = |w: Option<crate::claude_quota::StreamWindow>| {
        w.map(|w| Window {
            used_percent: round2(w.utilization * 100.0),
            resets_at: w.resets_at,
        })
    };
    Usage {
        five_hour: window(quota.five_hour),
        seven_day: window(quota.seven_day),
        status: quota.status.clone(),
        overage_reason: None,
        source: UsageSource::Stream,
        observed: now,
    }
}

/// A probe's outcome and, A2, the organisation uuid its answer named
/// (`anthropic-organization-id`), which [`crate::peer_usage::shared_key`]
/// turns into the key another Mac matches this account by.
async fn probe_http(base: &str, token: &Secret) -> (ProbeOutcome, Option<String>) {
    let mut org = None;
    let outcome = probe_http_inner(base, token, &mut org).await;
    (outcome, org)
}

async fn probe_http_inner(base: &str, token: &Secret, org: &mut Option<String>) -> ProbeOutcome {
    use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT as UA};
    let mut raw = format!("Bearer {}", token.as_str()).into_bytes();
    let auth = HeaderValue::from_bytes(&raw);
    crate::claude_quota::zero(&mut raw);
    let Ok(mut auth) = auth else {
        return ProbeOutcome::Invalid;
    };
    auth.set_sensitive(true);
    let http = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(http) => http,
        Err(e) => return ProbeOutcome::Failed(format!("could not build the HTTP client: {e}")),
    };
    let body = serde_json::json!({
        "model": PROBE_MODEL,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "."}],
    });
    let response = match http
        .post(format!("{}/v1/messages", base.trim_end_matches('/')))
        .header(AUTHORIZATION, auth)
        .header("anthropic-beta", OAUTH_BETA)
        .header("anthropic-version", "2023-06-01")
        .header(CONTENT_TYPE, "application/json")
        .header(UA, USER_AGENT)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return ProbeOutcome::Failed(format!("usage check failed: {}", e.without_url())),
    };
    let headers = response.headers().clone();
    *org = headers
        .get("anthropic-organization-id")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::peer_usage::plain_id);
    let status = response.status().as_u16();
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    parse_headers(status, &get, Utc::now())
}

// ── Choosing ────────────────────────────────────────────────────────────────

/// One account as [`pick`] sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub id: String,
    /// Percent of the fuller live window; `None` when never measured.
    pub load: Option<f64>,
    pub blocked: bool,
    /// When it was last chosen (a sequence number; 0 = never).
    pub last_used: u64,
}

/// Always choose the least-loaded eligible account. Unknown readings rank
/// after measured room. The previous conversation account never overrides
/// headroom, and no exhausted account is a fallback.
pub(crate) fn pick(
    candidates: &[Candidate],
    ceiling: f64,
    _sticky: Option<&str>,
) -> Option<String> {
    // Unknown is tried only after measured accounts with room, never as 0%.
    let load = |c: &Candidate| c.load.unwrap_or(100.0);
    let best_of = |pool: Vec<&Candidate>| -> Option<String> {
        pool.into_iter()
            .min_by(|a, b| {
                load(a)
                    .total_cmp(&load(b))
                    .then(a.last_used.cmp(&b.last_used))
            })
            .map(|c| c.id.clone())
    };
    let eligible: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| !c.blocked && c.load.is_none_or(|load| load < ceiling.min(98.0)))
        .collect();
    best_of(eligible)
}

/// The account a chat turn moves to when its own ran out (R1): [`pick`]
/// over the ELIGIBLE accounts that are not in `exclude` (the exhausted
/// ones). `None` when none is eligible: unlike a new run, a failover never
/// falls back to an account that is itself at its limit.
pub(crate) fn pick_failover(
    candidates: &[Candidate],
    ceiling: f64,
    exclude: &[String],
) -> Option<String> {
    let eligible: Vec<Candidate> = candidates
        .iter()
        .filter(|c| !exclude.contains(&c.id) && !c.blocked && c.load.unwrap_or(0.0) < ceiling)
        .cloned()
        .collect();
    pick(&eligible, ceiling, None)
}

/// Whether `to` may take over from `from`: the same organisation (two unset
/// ones count as the same), or the person allowed crossing (#16).
pub(crate) fn same_organization(from: Option<&str>, to: Option<&str>, across: bool) -> bool {
    let norm = |o: Option<&str>| o.map(|o| o.trim().to_lowercase()).filter(|o| !o.is_empty());
    across || norm(from) == norm(to)
}

/// Long unattended work (#16): keep `routed` unless it is at or above
/// `ceiling` ([`long_work_ceiling`]; or out of allowance) and another
/// candidate is under it, then the least-loaded such candidate. Never
/// refuses: with no account under the ceiling the routed one runs, as
/// before (the CFO holds such work before it starts: `run_pump`).
pub(crate) fn pick_long_work(candidates: &[Candidate], routed: &str, ceiling: f64) -> String {
    let load = |c: &Candidate| c.load.unwrap_or(0.0);
    let roomy = |c: &Candidate| !c.blocked && load(c) < ceiling;
    if candidates.iter().any(|c| c.id == routed && roomy(c)) {
        return routed.to_string();
    }
    let others: Vec<Candidate> = candidates.iter().filter(|c| roomy(c)).cloned().collect();
    pick(&others, ceiling, None).unwrap_or_else(|| routed.to_string())
}

// ── Runtime state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Health {
    Unknown,
    Ok,
    Invalid,
    Error(String),
}

#[derive(Default)]
struct Runtime {
    /// The settings, read once and replaced on every save.
    store: Option<Result<Store, String>>,
    usage: HashMap<String, Usage>,
    health: HashMap<String, Health>,
    last_probe: HashMap<String, DateTime<Utc>>,
    last_used: HashMap<String, (u64, DateTime<Utc>)>,
    /// Conversation → account it last ran on.
    affinity: HashMap<String, String>,
    tokens: HashMap<String, Secret>,
    /// A2: conversations run on the login with no account added, so only
    /// a conversation's first turn counts as a pick.
    login_conversations: std::collections::HashSet<String>,
    /// A2: account → its shared key (from its probe's organisation).
    keys: HashMap<String, String>,
    /// A2: account → when runs were started on it, the last
    /// [`crate::peer_usage::RECENT_PICK_SECS`] (at most [`MAX_PICKS`]).
    picks: HashMap<String, Vec<DateTime<Utc>>>,
}

/// A2: picks remembered per account.
const MAX_PICKS: usize = 64;

/// What paired Macs know of these accounts, for [`Registry::candidates`].
type PeerSource = Box<dyn Fn(DateTime<Utc>) -> Vec<crate::peer_usage::PeerUsage> + Send + Sync>;

pub(crate) struct Registry {
    dir: PathBuf,
    secrets: Box<dyn SecretStore>,
    rt: Mutex<Runtime>,
    seq: AtomicU64,
    probe_gate: tokio::sync::Mutex<()>,
    probe_base: String,
    /// A2: the paired Macs' snapshots (none unless set: tests, and before
    /// [`global`] gives the app's).
    peers: PeerSource,
}

static REGISTERED: AtomicBool = AtomicBool::new(false);
static GLOBAL: OnceLock<Option<Registry>> = OnceLock::new();

/// Mark this process as the GUI (lib.rs setup). Until then nothing is chosen.
pub fn register_gui() {
    REGISTERED.store(true, Ordering::SeqCst);
}

fn global() -> Option<&'static Registry> {
    if !REGISTERED.load(Ordering::SeqCst) {
        return None;
    }
    GLOBAL
        .get_or_init(|| {
            crate::agent_settings::app_data_dir().ok().map(|dir| {
                Registry::new(dir, Box::new(Keychain), PROBE_BASE)
                    .with_peers(Box::new(crate::peer_usage::cached))
            })
        })
        .as_ref()
}

/// The account a built command was given, if any.
pub fn account_of(cmd: &Command) -> Option<String> {
    cmd.as_std()
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new(ACCOUNT_ENV))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
}

/// Give a Claude command its account. A no-op outside the GUI and while no
/// token account is saved, so the default is exactly the old behaviour.
/// `affinity` keys a conversation so its turns stay on one account.
/// `forced` is the account a chat failover moved the turn to (R1); it wins
/// while it is still usable, and the conversation then keeps it.
pub fn apply(cmd: &mut Command, affinity: Option<&str>, forced: Option<&str>) {
    if let Some(registry) = global() {
        registry.apply(cmd, affinity, forced, Utc::now());
    }
}

/// Put a Claude command on Claude Code's own login, whatever the routing.
/// Used for runs that attach a claude.ai connector (`connectors.rs`): the
/// connector belongs to that login's claude.ai account, and an added
/// account's inference-only token cannot reach it.
pub fn apply_login(cmd: &mut Command) {
    if let Some(registry) = global() {
        cmd.env_remove(TOKEN_ENV);
        cmd.env(ACCOUNT_ENV, LOGIN_ID);
        registry.note_pick(LOGIN_ID, Utc::now());
    }
}

/// A2: whether this process routes accounts (the app or `hollow serve`),
/// so its `GET /v1/machine/usage` has something to say.
pub(crate) fn routes_here() -> bool {
    global().is_some()
}

/// A2: this Hollow's Claude accounts for `GET /v1/machine/usage`
/// ([`Registry::usage_snapshot`]); empty outside the app and serve.
pub(crate) fn usage_snapshot(now: DateTime<Utc>) -> Vec<crate::phone_access::AccountUsage> {
    global()
        .map(|r| {
            r.usage_snapshot(
                crate::peer_usage::claude_login_org(),
                crate::claude_quota::login_view(),
                now,
            )
        })
        .unwrap_or_default()
}

/// A `rate_limit_event` from a run that used `account` (`None`: a run built
/// without an account, which is the login). The login's also feeds the
/// Settings, Usage plan card, which describes the login only.
pub fn observe_stream(account: Option<&str>, quota: &StreamQuota) {
    let account = account.unwrap_or(LOGIN_ID);
    if account == LOGIN_ID {
        crate::claude_quota::observe_stream(quota);
    } else if let Some(registry) = global() {
        registry.observe(account, usage_from_stream(quota, Utc::now()));
    }
}

// ── Login override (2026-10-09) ─────────────────────────────────────────
//
// A Claude run carrying a claude.ai connector goes on Claude Code's own
// login ([`apply_login`]), whatever the routing, and the login never fails
// over or moves at the ceiling. These say so, and warn when the login is
// low while an added account has room.

/// Whether an added account could run now, so a run put on the login was
/// kept off it (outside the GUI, with none added, or with every added one
/// off or signed out, the login is simply the account).
pub fn has_added() -> bool {
    global().is_some_and(|r| r.has_usable(Utc::now()))
}

/// Whether the next run would carry an added account's token rather than
/// the Claude Code login, as a fresh conversation's turn ([`apply`], no
/// affinity yet) or long work ([`apply_long_work`]) is routed now. A remote
/// run on the Mac mini needs one (`remote_session::preferred_blocker`,
/// overnight N6). False outside the GUI and with no account added.
pub fn next_run_has_token(long_work: bool) -> bool {
    global().is_some_and(|registry| {
        let now = Utc::now();
        let chosen = if long_work {
            registry.choose_long_work(now)
        } else {
            registry.choose(None, now)
        };
        chosen.is_some_and(|id| id != LOGIN_ID && registry.has_token(&id))
    })
}

/// The login is at or over its line while an added account has room: the
/// fullest window past the line, and the account with room (a label).
#[derive(Debug, Clone, PartialEq)]
pub struct LoginLow {
    /// `five_hour` | `seven_day`.
    pub window: String,
    pub percent: f64,
    /// The line it crossed: the Spread ceiling, or 100 in Single mode.
    pub line: f64,
    /// When that window resets (RFC 3339), if known.
    pub resets_at: Option<String>,
    pub room_on: String,
    /// One key per window past the line (`<window>@<resets>`, or a slice of
    /// the window's length when its reset is unknown): the warning is once
    /// per window.
    keys: Vec<String>,
}

impl LoginLow {
    fn window_name(&self) -> &'static str {
        if self.window == crate::claude_quota::FIVE_HOUR {
            "5-hour"
        } else {
            "weekly"
        }
    }

    /// What the chat notice and the bell say.
    pub fn message(&self) -> String {
        format!(
            "Claude Code's login is at {:.0}% of its {} limit, and {} has room. Runs that carry a claude.ai connector stay on the login and never move. To use your added accounts, set those connectors to “Only when relevant” in Connectors.",
            self.percent,
            self.window_name(),
            self.room_on
        )
    }

    /// Why connectors set to Always stepped aside for a run
    /// (`connectors::demoted_ids`), for its prompt note.
    pub fn set_aside_reason(&self) -> String {
        format!(
            "Claude Code's login, which claude.ai connectors need, is at {:.0}% of its {} limit, so this run uses an added account.",
            self.percent,
            self.window_name()
        )
    }

    /// The chat row when connectors set to Always stepped aside.
    pub fn set_aside_message(&self, names: &[String]) -> String {
        let (verb, it) = if names.len() == 1 {
            ("was", "it")
        } else {
            ("were", "one")
        };
        format!(
            "{} {verb} left out of this chat: Claude Code's login is at {:.0}% of its {} limit, and claude.ai connectors only run there, so messages that don't need {it} use your added accounts. Name {it} in a message to bring it back for that message (it then runs on the login). Where Connectors offers Hollow's own sign-in for a connector, that works on every account.",
            names.join(", "),
            self.percent,
            self.window_name()
        )
    }
}

/// Spread usage across accounts (2026-10-21): the login is past its line (the
/// Spread ceiling, or 100% with one account) while an added account has
/// room, and the person lets Hollow move runs between accounts (the
/// failover setting). Connectors set to Always then step aside for new
/// runs unless the request needs them (`connectors::demoted_ids`). `None` outside the GUI.
pub fn login_out_of_room() -> Option<LoginLow> {
    if !failover_enabled() {
        return None;
    }
    login_low_last_seen()
}

/// Conversations already told that connectors stepped aside, per window.
static SET_ASIDE_TOLD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether `conversation` hasn't yet been told, for `low`'s windows, that
/// connectors stepped aside (a row per conversation per window, not per
/// turn; the agent's note says it every turn).
pub fn first_set_aside_notice(conversation: &str, low: &LoginLow) -> bool {
    let keyed = LoginLow {
        keys: low
            .keys
            .iter()
            .map(|k| format!("{conversation}|{k}"))
            .collect(),
        ..low.clone()
    };
    first_warning_in(&SET_ASIDE_TOLD, &keyed)
}

/// A window's once-per-window key: its reset, or (unknown) which slice of
/// the window's length `now` falls in.
fn window_key(id: &str, resets_at: Option<&str>, now: DateTime<Utc>) -> String {
    match resets_at {
        Some(at) => format!("{id}@{at}"),
        None => {
            let length = if id == crate::claude_quota::FIVE_HOUR {
                5 * 3600
            } else {
                7 * 86400
            };
            format!("{id}#{}", now.timestamp().div_euclid(length))
        }
    }
}

/// The pure rule: `readings` are the login's windows (id, percent, resets);
/// the line is the ceiling in Spread mode and 100% in Single; `room_on` is
/// an added account under the ceiling. `None` when nothing is past the line
/// or nowhere has room.
pub(crate) fn login_low(
    routing: Routing,
    ceiling: f64,
    readings: &[crate::claude_quota::Reading],
    room_on: Option<String>,
    now: DateTime<Utc>,
) -> Option<LoginLow> {
    let line = match routing {
        Routing::Spread => ceiling,
        Routing::Single => 100.0,
    };
    let past: Vec<&crate::claude_quota::Reading> = readings
        .iter()
        .filter(|r| {
            r.id == crate::claude_quota::FIVE_HOUR || r.id == crate::claude_quota::SEVEN_DAY
        })
        .filter(|r| r.used_percent >= line)
        .collect();
    let fullest = past
        .iter()
        .max_by(|a, b| a.used_percent.total_cmp(&b.used_percent))?;
    Some(LoginLow {
        window: fullest.id.clone(),
        percent: fullest.used_percent,
        line,
        resets_at: fullest.resets_at.clone(),
        room_on: room_on?,
        keys: past
            .iter()
            .map(|r| window_key(&r.id, r.resets_at.as_deref(), now))
            .collect(),
    })
}

/// [`login_low`] for the live registry: `None` outside the GUI or with no
/// added account.
pub fn login_low_now(readings: &[crate::claude_quota::Reading]) -> Option<LoginLow> {
    global()?.login_low(readings, Utc::now())
}

/// [`login_low_now`] from the login's windows as last seen (its runs'
/// streams or the plan card's fetch), for a run about to start on the login
/// (an automation): `None` when nothing has been seen.
pub fn login_low_last_seen() -> Option<LoginLow> {
    let readings: Vec<crate::claude_quota::Reading> = crate::claude_quota::login_view()?
        .windows
        .into_iter()
        .map(|w| crate::claude_quota::Reading {
            id: w.id,
            label: w.label,
            used_percent: w.used_percent,
            resets_at: w.resets_at,
        })
        .collect();
    login_low_now(&readings)
}

/// The windows already warned about (once per window, process-wide).
static LOGIN_WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether `low` is the first warning for its windows: true when a window
/// past the line hasn't warned yet (all of them are then remembered), so
/// the fuller window changing hands warns nothing new. Bounded.
pub fn first_login_warning(low: &LoginLow) -> bool {
    first_warning_in(&LOGIN_WARNED, low)
}

pub(crate) fn first_warning_in(seen: &Mutex<Vec<String>>, low: &LoginLow) -> bool {
    let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
    let fresh: Vec<String> = low
        .keys
        .iter()
        .filter(|k| !seen.contains(k))
        .cloned()
        .collect();
    if fresh.is_empty() {
        return false;
    }
    for key in fresh {
        if seen.len() >= 64 {
            seen.remove(0);
        }
        seen.push(key);
    }
    true
}

/// The bell's copy of the warning (the chat row says the same).
pub fn notify_login_low(vault: &Path, low: &LoginLow) {
    let _ = crate::notifications::create_registered(
        vault,
        crate::notifications::NotificationInput {
            title: "Claude Code's login is running low".into(),
            body: low.message(),
            kind: "warning".into(),
            source: Some("claude-accounts".into()),
            action_label: Some("Connectors".into()),
            action_url: Some("settings:connectors".into()),
            notify_native: false,
        },
    );
}

// ── Chat failover (R1): what `chat.rs` asks. All are GUI-only no-ops. ─────

/// Whether chat failover is on and added accounts exist.
pub fn failover_enabled() -> bool {
    global().is_some_and(Registry::failover_enabled)
}

/// Turn guards (2026-10-09): whether a running chat turn may move at the
/// ceiling: Spread usage, the failover toggle on, and added accounts.
pub fn ceiling_switch_enabled() -> bool {
    global().is_some_and(Registry::ceiling_switch_enabled)
}

/// Whether a run's own `rate_limit_event` puts its account out of room.
pub fn stream_exhausted(quota: &StreamQuota) -> bool {
    global().is_some_and(|r| r.stream_exhausted(quota, Utc::now()))
}

/// One fresh, unthrottled probe of `account`: `Some(true)` when it is out
/// of room, `None` when the probe could not tell.
pub async fn probe_exhausted(account: &str) -> Option<bool> {
    global()?.probe_exhausted(account).await
}

/// The eligible account a failed-over chat turn moves to, never one in
/// `exclude`.
pub fn failover_target(exclude: &[String]) -> Option<String> {
    global()?.failover_target(exclude, Utc::now())
}

/// Refresh stale readings before admission or selecting a replacement.
pub(crate) async fn refresh_before_pick() {
    if let Some(registry) = global() {
        registry.refresh(false).await;
    }
}

/// A confirmed rejection is stronger evidence than a meter or later probe.
pub(crate) fn reject_account(account: &str, reset: Option<i64>) -> Option<String> {
    global()?.reject_account(account, reset, Utc::now())
}

pub(crate) fn waiting_for_room() -> Option<String> {
    let registry = global()?;
    let store = match registry.store() {
        Ok(store) => store,
        Err(error) => {
            return Some(format!(
                "Claude accounts need repair before a run can start: {error}"
            ))
        }
    };
    if store.accounts.is_empty() || registry.route(&store, None, Utc::now()).is_some() {
        return None;
    }
    Some(registry.soonest_room(Utc::now())
        .and_then(|at| crate::chat_resume::reset_label(at, &chrono::Local::now()))
        .map(|reset| format!("Waiting until {reset}: every eligible Claude account is out of room."))
        .unwrap_or_else(|| "Waiting for Claude account capacity. No eligible account has room; its reset time is not reported.".to_string()))
}

/// Captured when the command is built, not reconstructed after the run.
pub(crate) fn pick_receipt(cmd: &Command) -> Option<String> {
    cmd.as_std()
        .get_envs()
        .find(|(key, _)| *key == "HOLLOW_CLAUDE_PICK")
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
}

/// When the soonest account has room again (unix seconds), if known.
pub fn soonest_room() -> Option<i64> {
    global()?.soonest_room(Utc::now())
}

/// Give a long unattended run (an automation) its account (#16): like
/// [`apply`] with no conversation, but away from an account at or above
/// [`long_work_ceiling`] when another has room. `forced` (a failover's
/// target) wins while usable.
pub fn apply_long_work(cmd: &mut Command, forced: Option<&str>) {
    if let Some(registry) = global() {
        let now = Utc::now();
        match forced {
            Some(id) => registry.apply(cmd, None, Some(id), now),
            None => {
                let chosen = registry.choose_long_work(now);
                registry.apply(cmd, None, chosen.as_deref(), now);
            }
        }
    }
}

/// COO rebuild L7: the accounts unattended work may start on (the routed
/// one and those long work may move to); None outside the GUI.
pub fn long_work_eligible() -> Option<Vec<String>> {
    global().map(|r| r.long_work_eligible(Utc::now()))
}

/// Account headroom (#16, `accounts.headroom`): each added
/// account's hashed ledger label, name, organisation, live windows and
/// whether a chat or long work could start there now, plus where the next
/// chat and the next long run would go. Never a token. An error outside the
/// GUI.
#[tauri::command]
pub fn headroom() -> Result<serde_json::Value, String> {
    global()
        .map(|r| r.headroom(Utc::now()))
        .ok_or_else(|| "Claude accounts are read only in the app.".to_string())
}

/// The accounts view for the CFO: live windows only,
/// never a token; None outside the GUI.
pub(crate) fn accounts_view() -> Option<AccountsView> {
    global().map(|r| r.view())
}

/// An added account's name (its id when it has none any more).
pub fn label(account: &str) -> String {
    global()
        .and_then(|r| r.label(account))
        .unwrap_or_else(|| account.to_string())
}

impl Registry {
    fn new(dir: PathBuf, secrets: Box<dyn SecretStore>, probe_base: &str) -> Self {
        Registry {
            dir,
            secrets,
            rt: Mutex::new(Runtime::default()),
            seq: AtomicU64::new(0),
            probe_gate: tokio::sync::Mutex::const_new(()),
            probe_base: probe_base.to_string(),
            peers: Box::new(|_| Vec::new()),
        }
    }

    /// A2: read paired Macs' snapshots from `peers` when choosing.
    fn with_peers(mut self, peers: PeerSource) -> Self {
        self.peers = peers;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Runtime> {
        self.rt.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn store(&self) -> Result<Store, String> {
        let mut rt = self.lock();
        if rt.store.is_none() {
            rt.store = Some(load_store(&self.dir));
        }
        rt.store.clone().expect("store loaded")
    }

    fn save(&self, mut store: Store) -> Result<(), String> {
        let mut rt = self.lock();
        if let Some(Ok(current)) = &rt.store {
            for (id, rejection) in &current.exhausted_accounts {
                if store.accounts.iter().any(|a| a.id == *id) {
                    store
                        .exhausted_accounts
                        .insert(id.clone(), rejection.clone());
                }
            }
        }
        save_store(&self.dir, &store)?;
        rt.store = Some(Ok(store));
        Ok(())
    }

    /// Whether `id` has a token, without reading one: a status line, the
    /// accounts view and routing must not run `op` (1Password) to ask.
    fn has_token(&self, id: &str) -> bool {
        self.lock().tokens.contains_key(id) || matches!(self.secrets.present(id), Ok(true))
    }

    fn token(&self, id: &str) -> Option<Secret> {
        // A token read through 1Password is read again at each use (a
        // rotated one is picked up, and none is kept here), so a copy
        // cached before the reference was set doesn't win either.
        let via_onepassword =
            crate::onepassword::has_reference(&crate::secret_store::claude_key(id));
        if !via_onepassword {
            if let Some(token) = self.lock().tokens.get(id) {
                return Some(token.clone());
            }
        }
        let token = match self.secrets.get(id) {
            Ok(Some(token)) => token,
            Ok(None) => return None,
            Err(error) => {
                eprintln!("[claude-accounts] {error}");
                return None;
            }
        };
        if !via_onepassword {
            self.lock().tokens.insert(id.to_string(), token.clone());
        }
        Some(token)
    }

    fn observe(&self, id: &str, usage: Usage) {
        let mut rt = self.lock();
        let newer = rt
            .usage
            .get(id)
            .is_none_or(|held| held.observed <= usage.observed);
        if newer {
            rt.usage.insert(id.to_string(), usage);
        }
        rt.health.insert(id.to_string(), Health::Ok);
    }

    fn usage_for(&self, id: &str) -> Option<Usage> {
        self.lock().usage.get(id).cloned()
    }

    /// Whether an added account can run: enabled, its token stored, and not
    /// refused by Claude.
    fn usable(&self, account: &StoredAccount) -> bool {
        account.enabled
            && self.lock().health.get(&account.id) != Some(&Health::Invalid)
            && self.has_token(&account.id)
    }

    /// The added accounts that may run right now, in display order. A2:
    /// with two or more to choose from, what paired Macs know of them is
    /// folded in ([`crate::peer_usage::merge`]); with no peer snapshot the
    /// list is exactly the local one.
    fn candidates(&self, store: &Store, now: DateTime<Utc>) -> Vec<Candidate> {
        let local = self.local_candidates(store, now);
        let peers = if local.len() >= 2 && local.iter().any(|l| l.key.is_some()) {
            (self.peers)(now)
        } else {
            Vec::new()
        };
        crate::peer_usage::merge(local, &peers, f64::from(store.ceiling_percent), now)
    }

    /// [`Self::candidates`] as this Mac alone knows them.
    fn local_candidates(&self, store: &Store, now: DateTime<Utc>) -> Vec<crate::peer_usage::Local> {
        store
            .accounts
            .iter()
            .filter(|a| self.usable(a))
            .map(|a| {
                let usage = self.usage_for(&a.id);
                let rt = self.lock();
                let last_used = rt.last_used.get(&a.id).map(|u| u.0).unwrap_or(0);
                crate::peer_usage::Local {
                    candidate: Candidate {
                        id: a.id.clone(),
                        load: usage.as_ref().map(|u| u.load(now)),
                        blocked: usage.as_ref().is_some_and(|u| u.blocked(now))
                            || store
                                .exhausted_accounts
                                .get(&a.id)
                                .is_some_and(|r| r.live(now)),
                        last_used,
                    },
                    key: rt.keys.get(&a.id).cloned(),
                    measured_at: usage.as_ref().map(|u| u.observed),
                    seven_day_resets_at: usage
                        .as_ref()
                        .and_then(|u| u.seven_day)
                        .filter(|w| w.live(now))
                        .and_then(|w| w.resets_at),
                }
            })
            .collect()
    }

    /// A2: a run was started on `id` (the login included).
    fn note_pick(&self, id: &str, now: DateTime<Utc>) {
        let mut rt = self.lock();
        let picks = rt.picks.entry(id.to_string()).or_default();
        picks.retain(|at| {
            (now - *at).num_seconds() < crate::peer_usage::RECENT_PICK_SECS && *at <= now
        });
        if picks.len() >= MAX_PICKS {
            picks.remove(0);
        }
        picks.push(now);
    }

    /// A2: `(recent picks, last pick)` of `id`.
    fn recent_picks(&self, id: &str, now: DateTime<Utc>) -> (u32, Option<DateTime<Utc>>) {
        let rt = self.lock();
        let picks: Vec<&DateTime<Utc>> = rt
            .picks
            .get(id)
            .map(|p| {
                p.iter()
                    .filter(|at| (now - **at).num_seconds() < crate::peer_usage::RECENT_PICK_SECS)
                    .collect()
            })
            .unwrap_or_default();
        (picks.len() as u32, picks.last().map(|at| **at))
    }

    /// A2: remember `id`'s shared key from its probe's organisation.
    fn note_org(&self, id: &str, org: Option<String>) {
        if let Some(org) = org {
            self.lock().keys.insert(
                id.to_string(),
                crate::peer_usage::shared_key("claude-org", &org),
            );
        }
    }

    /// A2 (`GET /v1/machine/usage`): every usable added account with a
    /// shared key, and the Claude Code login when it is what runs use here
    /// (no account added or none usable: the Mac mini's case). Never a
    /// token or an account id.
    fn usage_snapshot(
        &self,
        login: Option<(String, Option<String>)>,
        login_quota: Option<crate::claude_quota::ClaudeQuota>,
        now: DateTime<Utc>,
    ) -> Vec<crate::phone_access::AccountUsage> {
        use crate::peer_usage::{rfc3339 as at, wire_text};
        let store = self.store().unwrap_or_default();
        // This Mac's own view: no peer is asked while answering one.
        let local: Vec<Candidate> = self
            .local_candidates(&store, now)
            .into_iter()
            .map(|l| l.candidate)
            .collect();
        let routed = Self::route_over(&store, &local, None);
        let mut out = Vec::new();
        for account in store.accounts.iter().filter(|a| self.usable(a)) {
            let Some(key) = self.lock().keys.get(&account.id).cloned() else {
                continue;
            };
            let usage = self.usage_for(&account.id);
            let (picks, last) = self.recent_picks(&account.id, now);
            let rejection = store
                .exhausted_accounts
                .get(&account.id)
                .filter(|r| r.live(now));
            let blocked = rejection.is_some() || usage.as_ref().is_some_and(|u| u.blocked(now));
            out.push(crate::phone_access::AccountUsage {
                key,
                provider: "claude",
                label: wire_text(&account.label),
                billing: account.organization.as_deref().map(wire_text),
                routed: store.routing == Routing::Spread
                    || routed.as_deref() == Some(account.id.as_str()),
                load_percent: usage.as_ref().map(|u| u.load(now)),
                blocked,
                blocked_until: rejection
                    .and_then(|r| r.reset_at)
                    .or_else(|| {
                        usage
                            .as_ref()
                            .filter(|_| blocked)
                            .and_then(|u| u.room_again_at(100.0, now))
                    })
                    .and_then(|t| DateTime::<Utc>::from_timestamp(t, 0))
                    .map(at),
                seven_day_resets_at: usage
                    .as_ref()
                    .and_then(|u| u.seven_day)
                    .filter(|w| w.live(now))
                    .and_then(|w| w.resets_at)
                    .and_then(|t| DateTime::<Utc>::from_timestamp(t, 0))
                    .map(at),
                measured_at: usage.as_ref().map(|u| at(u.observed)),
                recent_picks: picks,
                last_picked_at: last.map(at),
            });
        }
        let login_routes = routed.is_none();
        if let (true, Some((org, billing))) = (login_routes, login) {
            let windows: Vec<&crate::claude_quota::ClaudeQuotaWindow> = login_quota
                .as_ref()
                .map(|q| {
                    q.windows
                        .iter()
                        .filter(|w| w.id == "five_hour" || w.id == "seven_day")
                        .collect()
                })
                .unwrap_or_default();
            let full: Vec<&&crate::claude_quota::ClaudeQuotaWindow> =
                windows.iter().filter(|w| w.used_percent >= 100.0).collect();
            let (picks, last) = self.recent_picks(LOGIN_ID, now);
            out.push(crate::phone_access::AccountUsage {
                key: crate::peer_usage::shared_key("claude-org", &org),
                provider: "claude",
                label: "Claude Code login".to_string(),
                billing,
                routed: true,
                load_percent: windows
                    .iter()
                    .map(|w| w.used_percent)
                    .fold(None, |h: Option<f64>, l| Some(h.map_or(l, |h| h.max(l)))),
                blocked: !full.is_empty(),
                blocked_until: full.iter().filter_map(|w| w.resets_at.clone()).max(),
                seven_day_resets_at: windows
                    .iter()
                    .find(|w| w.id == "seven_day")
                    .and_then(|w| w.resets_at.clone()),
                measured_at: login_quota.as_ref().map(|q| q.observed_at.clone()),
                recent_picks: picks,
                last_picked_at: last.map(at),
            });
        }
        out
    }

    /// The added account a run would use under `store`'s routing, or `None`
    /// when none is usable (disabled, token missing or refused).
    fn route(&self, store: &Store, sticky: Option<&str>, now: DateTime<Utc>) -> Option<String> {
        Self::route_over(store, &self.candidates(store, now), sticky)
    }

    /// [`Self::route`] over given candidates.
    fn route_over(store: &Store, candidates: &[Candidate], sticky: Option<&str>) -> Option<String> {
        pick(candidates, f64::from(store.ceiling_percent), sticky)
    }

    /// The account the next run uses. `None`: no account has been added, so
    /// nothing is chosen and the Claude Code login is used as before. With
    /// accounts added but none usable, admission waits instead.
    fn choose(&self, affinity: Option<&str>, now: DateTime<Utc>) -> Option<String> {
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("[claude-accounts] cannot select an account: {error}");
                return None;
            }
        };
        if store.accounts.is_empty() {
            return None;
        }
        let sticky = affinity.and_then(|key| self.lock().affinity.get(key).cloned());
        self.route(&store, sticky.as_deref(), now)
    }

    fn apply(
        &self,
        cmd: &mut Command,
        affinity: Option<&str>,
        forced: Option<&str>,
        now: DateTime<Utc>,
    ) {
        if let Err(error) = self.store() {
            refuse_account_command(
                cmd,
                &format!("Claude accounts need repair before a run can start: {error}"),
            );
            return;
        }
        let chosen = if let Some(id) = forced {
            (|| {
                let store = self.store().ok()?;
                let candidates = self.same_org_candidates(&store, Some(id), now);
                pick(
                    &candidates
                        .into_iter()
                        .filter(|c| c.id == id)
                        .collect::<Vec<_>>(),
                    f64::from(store.ceiling_percent),
                    None,
                )
            })()
        } else {
            self.choose(affinity, now)
        };
        let Some(id) = chosen else {
            if self.store().is_ok_and(|s| !s.accounts.is_empty()) {
                let cwd = cmd.as_std().get_current_dir().map(Path::to_path_buf);
                *cmd = Command::new("/bin/sh");
                let reset = self
                    .soonest_room(now)
                    .and_then(|at| crate::chat_resume::reset_label(at, &chrono::Local::now()));
                let message = reset
                    .map(|at| format!("Waiting until {at}: no eligible Claude account has room."))
                    .unwrap_or_else(|| {
                        "Waiting for Claude account capacity; reset time is unknown.".to_string()
                    });
                cmd.args([
                    "-c",
                    "printf '%s\\n' \"$1\" >&2; exit 1",
                    crate::agent::UNAVAILABLE_ARGV0,
                    &message,
                ]);
                cmd.stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
                if let Some(cwd) = cwd {
                    cmd.current_dir(cwd);
                }
                return;
            }
            // No account added: the login runs it. A2 counts the pick when
            // it starts a run or a conversation, not a conversation's next
            // turn.
            let new = {
                let mut rt = self.lock();
                match affinity {
                    Some(key) => {
                        if rt.login_conversations.len() >= MAX_AFFINITIES
                            && !rt.login_conversations.contains(key)
                        {
                            rt.login_conversations.clear();
                        }
                        rt.login_conversations.insert(key.to_string())
                    }
                    None => true,
                }
            };
            if new {
                self.note_pick(LOGIN_ID, now);
            }
            return;
        };
        let token = if id == LOGIN_ID {
            None
        } else {
            self.token(&id)
        };
        if id != LOGIN_ID && token.is_none() {
            refuse_account_command(cmd, "The selected Claude account was removed or lost its token before launch. Retry with an eligible account.");
            return;
        }
        match &token {
            Some(token) => {
                cmd.env(TOKEN_ENV, token.as_str());
            }
            // The login is selected by the variable's absence.
            None => {
                cmd.env_remove(TOKEN_ENV);
            }
        }
        cmd.env(ACCOUNT_ENV, &id);
        let selected_load = self
            .store()
            .ok()
            .and_then(|store| {
                self.candidates(&store, now)
                    .into_iter()
                    .find(|c| c.id == id)
            })
            .and_then(|c| c.load);
        let headroom = selected_load
            .map(|load| {
                format!(
                    "{:.1}% headroom in the binding 5-hour/weekly window",
                    100.0 - load
                )
            })
            .unwrap_or_else(|| "headroom unknown after usage refresh".to_string());
        cmd.env("HOLLOW_CLAUDE_PICK", format!("{}: {headroom}; most available room among eligible accounts, excluding provider rejections", self.label(&id).unwrap_or_else(|| id.clone())));
        // A2: a pick is a run or conversation placed on `id`, not the next
        // turn of a conversation already there (a failover's move counts).
        let continues = affinity.is_some_and(|key| {
            self.lock().affinity.get(key).map(String::as_str) == Some(id.as_str())
        });
        if !continues {
            self.note_pick(&id, now);
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut rt = self.lock();
        rt.last_used.insert(id.clone(), (seq, now));
        if let Some(key) = affinity {
            if rt.affinity.len() >= MAX_AFFINITIES && !rt.affinity.contains_key(key) {
                rt.affinity.clear();
            }
            rt.affinity.insert(key.to_string(), id);
        }
    }

    fn headroom(&self, now: DateTime<Utc>) -> serde_json::Value {
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => return serde_json::json!({ "error": error }),
        };
        let ceiling = f64::from(store.ceiling_percent);
        let candidates = self.candidates(&store, now);
        let label_of = |id: &str| {
            if id == LOGIN_ID {
                return "Claude Code login".to_string();
            }
            store
                .accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.label.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let accounts: Vec<serde_json::Value> = store
            .accounts
            .iter()
            .map(|a| {
                let usage = self.usage_for(&a.id);
                let window = |w: Option<&Window>| w.map(|w| w.used_percent);
                let candidate = candidates.iter().find(|c| c.id == a.id);
                let load = candidate.and_then(|c| c.load);
                let blocked = candidate.is_some_and(|c| c.blocked);
                serde_json::json!({
                    "ledgerRef": crate::run_attribution::account_ref(&a.id),
                    "label": a.label,
                    "organization": a.organization,
                    "usable": candidate.is_some(),
                    "fiveHourPercent": usage.as_ref().and_then(|u| window(u.five_hour.as_ref())),
                    "sevenDayPercent": usage.as_ref().and_then(|u| window(u.seven_day.as_ref())),
                    "measured": usage.is_some(),
                    "roomForChat": candidate.is_some() && !blocked && load.unwrap_or(0.0) < ceiling,
                    "roomForLongWork": candidate.is_some() && !blocked && load.unwrap_or(0.0) < long_work_ceiling(),
                })
            })
            .collect();
        let next_chat = self.choose(None, now);
        let next_long = self.choose_long_work(now);
        serde_json::json!({
            "routing": store.routing,
            "ceilingPercent": store.ceiling_percent,
            "longWorkCeilingPercent": long_work_ceiling(),
            "failover": store.failover,
            "failoverAcrossOrganizations": store.failover_across_organizations,
            "accounts": accounts,
            "nextChat": next_chat.as_deref().map(label_of),
            "nextLongWork": next_long.as_deref().map(label_of),
            "note": "Accounts are measured by their own runs and probes; an unmeasured account counts as empty. Outages (overloads, 5xx) never move a run; the machine's Claude Code login is never a failover target.",
        })
    }

    // ── login override ──

    /// [`login_low`] with this store's routing and ceiling, and the account
    /// failover would pick with nothing excluded.
    fn login_low(
        &self,
        readings: &[crate::claude_quota::Reading],
        now: DateTime<Utc>,
    ) -> Option<LoginLow> {
        let store = self.store().ok()?;
        if store.accounts.is_empty() {
            return None;
        }
        let ceiling = f64::from(store.ceiling_percent);
        let room = pick_failover(&self.candidates(&store, now), ceiling, &[])
            .map(|id| self.label(&id).unwrap_or(id));
        login_low(store.routing, ceiling, readings, room, now)
    }

    /// Whether an added account could take a run now (turned on, signed
    /// in): the routing's own candidates.
    fn has_usable(&self, now: DateTime<Utc>) -> bool {
        self.store()
            .is_ok_and(|store| !self.candidates(&store, now).is_empty())
    }

    // ── chat failover (R1) ──

    /// Whether a chat turn moves to another account when its own runs out:
    /// the setting, and only while added accounts exist.
    fn failover_enabled(&self) -> bool {
        self.store().is_ok_and(|s| !s.accounts.is_empty())
    }

    /// [`Self::failover_enabled`] in Spread mode only: Single mode pins one
    /// account, so a ceiling never applies.
    fn ceiling_switch_enabled(&self) -> bool {
        self.failover_enabled()
    }

    /// Whether a run's own `rate_limit_event` puts its account out of room.
    fn stream_exhausted(&self, quota: &StreamQuota, now: DateTime<Utc>) -> bool {
        let ceiling = self
            .store()
            .map(|s| f64::from(s.ceiling_percent))
            .unwrap_or(f64::from(DEFAULT_CEILING));
        usage_from_stream(quota, now).exhausted(ceiling, now)
    }

    /// The `exhausted(id)` query: whether what is known of `id` (its own
    /// runs' events or a probe) says it has no room.
    fn exhausted(&self, id: &str, now: DateTime<Utc>) -> bool {
        let ceiling = self
            .store()
            .map(|s| f64::from(s.ceiling_percent))
            .unwrap_or(f64::from(DEFAULT_CEILING));
        self.store()
            .is_ok_and(|s| s.exhausted_accounts.get(id).is_some_and(|r| r.live(now)))
            || self
                .usage_for(id)
                .is_some_and(|u| u.exhausted(ceiling, now))
    }

    /// One fresh probe of `id` (never throttled): `Some(true)` when it says
    /// the account is out of room, `None` when it could not tell (no token,
    /// a network failure, a refused token).
    async fn probe_exhausted(&self, id: &str) -> Option<bool> {
        let token = self.token(id)?;
        self.lock().last_probe.insert(id.to_string(), Utc::now());
        let (outcome, org) = probe_http(&self.probe_base, &token).await;
        self.note_org(id, org);
        self.record_probe(id, outcome.clone());
        match outcome {
            ProbeOutcome::Usage(_) => Some(self.exhausted(id, Utc::now())),
            ProbeOutcome::Invalid | ProbeOutcome::Failed(_) => None,
        }
    }

    /// The account a failed-over turn resumes on ([`pick_failover`]),
    /// inside the organisation of the account that ran out (the last of
    /// `exclude`) unless crossing is allowed (#16).
    fn failover_target(&self, exclude: &[String], now: DateTime<Utc>) -> Option<String> {
        let store = self.store().ok()?;
        let candidates = self.same_org_candidates(&store, exclude.last().map(String::as_str), now);
        pick_failover(&candidates, f64::from(store.ceiling_percent), exclude)
    }

    /// [`Self::candidates`] that may take over from `from` (its organisation).
    fn same_org_candidates(
        &self,
        store: &Store,
        from: Option<&str>,
        now: DateTime<Utc>,
    ) -> Vec<Candidate> {
        let org_of = |id: &str| {
            store
                .accounts
                .iter()
                .find(|a| a.id == id)
                .and_then(|a| a.organization.clone())
        };
        let from_org = from.and_then(org_of);
        self.candidates(store, now)
            .into_iter()
            .filter(|c| {
                same_organization(
                    from_org.as_deref(),
                    org_of(&c.id).as_deref(),
                    store.failover_across_organizations,
                )
            })
            .collect()
    }

    /// The account long unattended work starts on (#16): the usual route,
    /// moved off an account at or above [`long_work_ceiling`] to one with
    /// room in the same organisation, only while failover is on (the person
    /// lets Hollow move runs between accounts).
    fn choose_long_work(&self, now: DateTime<Utc>) -> Option<String> {
        let routed = self.choose(None, now)?;
        if routed == LOGIN_ID || !self.failover_enabled() {
            return Some(routed);
        }
        let store = self.store().ok()?;
        let candidates = self.same_org_candidates(&store, Some(&routed), now);
        Some(pick_long_work(&candidates, &routed, long_work_ceiling()))
    }

    /// COO rebuild L7: the accounts long work may start on, the ONE set the
    /// CFO holds and picks from: the routed account, plus (only while
    /// failover is on, as [`Self::choose_long_work`] moves runs) every other
    /// usable account in its organisation (any organisation when crossing
    /// is allowed) that is not out of allowance. The login when it is the
    /// route, including when no account is added (`choose` gives `None`:
    /// the run uses the login), so its readings still count.
    fn long_work_eligible(&self, now: DateTime<Utc>) -> Vec<String> {
        let Some(routed) = self.choose(None, now) else {
            return if self.store().is_ok_and(|s| s.accounts.is_empty()) {
                vec![LOGIN_ID.to_string()]
            } else {
                Vec::new()
            };
        };
        if routed == LOGIN_ID || !self.failover_enabled() {
            return vec![routed];
        }
        let Ok(store) = self.store() else {
            return vec![routed];
        };
        let mut out = vec![routed.clone()];
        for candidate in self.same_org_candidates(&store, Some(&routed), now) {
            if candidate.id != routed && !candidate.blocked {
                out.push(candidate.id);
            }
        }
        out
    }

    /// The soonest time any usable account has room again (unix seconds).
    fn soonest_room(&self, now: DateTime<Utc>) -> Option<i64> {
        let store = self.store().ok()?;
        let ceiling = f64::from(store.ceiling_percent);
        store
            .accounts
            .iter()
            .filter(|a| self.usable(a))
            .filter_map(|a| {
                let rejection = store.exhausted_accounts.get(&a.id).filter(|r| r.live(now));
                let usage_reset = self
                    .usage_for(&a.id)
                    .filter(|u| u.exhausted(ceiling.min(98.0), now))
                    .and_then(|u| u.room_again_at(ceiling.min(98.0), now));
                match rejection {
                    Some(r) => r
                        .reset_at
                        .map(|reset| usage_reset.unwrap_or(reset).max(reset)),
                    None => usage_reset,
                }
            })
            .min()
    }

    fn reject_account(&self, id: &str, reset: Option<i64>, now: DateTime<Utc>) -> Option<String> {
        let snapshot = self.store().ok()?;
        if !snapshot.accounts.iter().any(|a| a.id == id) {
            return None;
        }
        let usage = self.usage_for(id);
        let mismatch = usage
            .as_ref()
            .is_some_and(|u| !u.exhausted(f64::from(snapshot.ceiling_percent), now));
        let reset_at = reset.filter(|at| *at > now.timestamp()).or_else(|| {
            usage
                .as_ref()
                .and_then(|u| rejection_reset(u, f64::from(snapshot.ceiling_percent), now))
        });
        let label = self.label(id).unwrap_or_else(|| id.to_string());
        let mut rt = self.lock();
        let mut store = rt.store.as_ref()?.as_ref().ok()?.clone();
        if !store.accounts.iter().any(|a| a.id == id) {
            return None;
        }
        let previous = store.exhausted_accounts.get(id).filter(|r| r.live(now));
        let reset_at = match (previous.and_then(|r| r.reset_at), reset_at) {
            (Some(old), Some(new)) => Some(old.max(new)),
            (Some(old), None) => Some(old),
            (None, reset) => reset,
        };
        let mismatch = mismatch || previous.is_some_and(|r| r.meter_mismatch);
        store.exhausted_accounts.insert(
            id.to_string(),
            Rejection {
                reset_at,
                meter_mismatch: mismatch,
            },
        );
        let message = format!(
            "{}: quota rejection{}; exhausted until {}",
            label,
            if mismatch {
                " contradicts the usage meter"
            } else {
                ""
            },
            reset_at
                .map(|at| at.to_string())
                .unwrap_or_else(|| "the provider reports a reset".to_string())
        );
        if let Err(error) = save_store(&self.dir, &store) {
            eprintln!("[claude-accounts] could not persist rejection: {error}");
        }
        // Keep this process safe even if durable storage is unavailable.
        rt.store = Some(Ok(store));
        eprintln!("[claude-accounts] {message}");
        Some(message)
    }

    /// An account's name, for the chat notice.
    fn label(&self, id: &str) -> Option<String> {
        let store = self.store().ok()?;
        store
            .accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.label.clone())
    }

    /// Probe one token account unless it was probed recently.
    async fn probe_one(&self, id: &str, force: bool, now: DateTime<Utc>) {
        let recent = self.lock().last_probe.get(id).is_some_and(|last| {
            now.signed_duration_since(*last) < chrono::Duration::seconds(PROBE_MIN_INTERVAL_SECS)
        });
        if recent && !force {
            return;
        }
        let Some(token) = self.token(id) else {
            return;
        };
        self.lock().last_probe.insert(id.to_string(), now);
        let (outcome, org) = probe_http(&self.probe_base, &token).await;
        self.note_org(id, org);
        self.record_probe(id, outcome);
    }

    fn record_probe(&self, id: &str, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Usage(usage) => {
                // A probe may supply a previously unknown reset, but an
                // allowed 0% reading can never clear a live rejection.
                let mut rt = self.lock();
                if let Some(Ok(store)) = rt.store.as_mut() {
                    let ceiling = f64::from(store.ceiling_percent);
                    if let Some(rejected) = store
                        .exhausted_accounts
                        .get_mut(id)
                        .filter(|r| r.reset_at.is_none())
                    {
                        rejected.reset_at = rejection_reset(&usage, ceiling, Utc::now());
                        if let Err(error) = save_store(&self.dir, store) {
                            eprintln!("[claude-accounts] could not persist reset: {error}");
                        }
                    }
                }
                drop(rt);
                self.observe(id, usage);
            }
            ProbeOutcome::Invalid => {
                self.lock().health.insert(id.to_string(), Health::Invalid);
            }
            ProbeOutcome::Failed(error) => {
                let error = crate::claude_runner::redact(&error);
                eprintln!("[claude-accounts] {id}: {error}");
                self.lock()
                    .health
                    .insert(id.to_string(), Health::Error(error));
            }
        }
    }

    /// Probe every enabled account (throttled unless `force`).
    async fn refresh(&self, force: bool) {
        let _gate = self.probe_gate.lock().await;
        let Ok(store) = self.store() else {
            return;
        };
        let now = Utc::now();
        let mut probes = tokio::task::JoinSet::new();
        for account in store.accounts.iter().filter(|a| a.enabled) {
            let fresh = self
                .usage_for(&account.id)
                .is_some_and(|u| (now - u.observed).num_seconds() < PROBE_MIN_INTERVAL_SECS);
            let recent = self
                .lock()
                .last_probe
                .get(&account.id)
                .is_some_and(|last| (now - *last).num_seconds() < PROBE_MIN_INTERVAL_SECS);
            if !force && (fresh || recent) {
                continue;
            }
            let Some(token) = self.token(&account.id) else {
                continue;
            };
            self.lock().last_probe.insert(account.id.clone(), now);
            let base = self.probe_base.clone();
            let id = account.id.clone();
            probes.spawn(async move { (id, probe_http(&base, &token).await) });
        }
        while let Some(Ok((id, (outcome, org)))) = probes.join_next().await {
            self.note_org(&id, org);
            self.record_probe(&id, outcome);
        }
    }

    fn view(&self) -> AccountsView {
        let now = Utc::now();
        let (store, error) = match self.store() {
            Ok(store) => (store, None),
            Err(error) => (Store::default(), Some(error)),
        };
        // None: no added account can run, so runs use the Claude Code login.
        let next = self.route(&store, None, now);
        let accounts = store
            .accounts
            .iter()
            .map(|a| self.account_view(a, &store, now))
            .collect();
        // The account the "Use" picker shows: the saved one, else the first.
        let pinned = store
            .accounts
            .iter()
            .find(|a| a.id == store.pinned)
            .or_else(|| store.accounts.first())
            .map(|a| a.id.clone())
            .unwrap_or_default();
        AccountsView {
            routing: store.routing,
            pinned,
            ceiling_percent: store.ceiling_percent,
            failover: store.failover,
            failover_across_organizations: store.failover_across_organizations,
            next_account: next,
            accounts,
            login_usage: crate::claude_quota::login_view().map(|q| login_usage_view(&q)),
            error,
        }
    }

    fn account_view(
        &self,
        account: &StoredAccount,
        store: &Store,
        now: DateTime<Utc>,
    ) -> AccountView {
        let id = account.id.as_str();
        let usage = self.usage_for(id);
        let health = self
            .lock()
            .health
            .get(id)
            .cloned()
            .unwrap_or(Health::Unknown);
        let has_token = self.has_token(id);
        let ceiling = f64::from(store.ceiling_percent);
        let (status, note) = if !has_token {
            (
                AccountStatus::MissingToken,
                Some(
                    "Its token is missing from the Keychain. Remove the account and add it again."
                        .to_string(),
                ),
            )
        } else if health == Health::Invalid {
            (AccountStatus::Invalid, Some("Claude refused this token: it was revoked, expired, or copied with a line break. Mint a new one with `claude setup-token`.".to_string()))
        } else if let Some(rejection) = store.exhausted_accounts.get(id).filter(|r| r.live(now)) {
            (AccountStatus::Limited, Some(if rejection.meter_mismatch {
                "Claude rejected this account despite the meter reporting room. Waiting for its reset."
            } else {
                "Claude rejected this account at its limit. Waiting for its reset."
            }.to_string()))
        } else if let Some(usage) = usage.as_ref() {
            if usage.blocked(now) {
                (AccountStatus::Limited, usage.overage_reason.clone())
            } else if usage.load(now) >= ceiling {
                (AccountStatus::Near, None)
            } else {
                (AccountStatus::Ok, None)
            }
        } else if let Health::Error(error) = &health {
            (AccountStatus::Unknown, Some(error.clone()))
        } else {
            (AccountStatus::Unknown, None)
        };
        let last_used = self.lock().last_used.get(id).map(|u| rfc3339(u.1));
        AccountView {
            id: id.to_string(),
            label: account.label.clone(),
            enabled: account.enabled,
            status,
            note,
            usage: usage.map(|u| UsageView {
                five_hour: u.five_hour.filter(|w| w.live(now)).map(window_view),
                seven_day: u.seven_day.filter(|w| w.live(now)).map(window_view),
                status: u.status,
                overage_disabled_reason: u.overage_reason,
                source: u.source,
                observed_at: rfc3339(u.observed),
            }),
            last_used_at: last_used,
            created_at: Some(account.created_at.clone()),
        }
    }

    // ── mutations (IPC) ──

    fn add(&self, label: &str, token: &str) -> Result<String, String> {
        let label = clean_label(label)?;
        let token = Secret::new(normalize_token(token)?);
        let mut store = self.store()?;
        if store.accounts.len() >= MAX_ACCOUNTS {
            return Err(format!(
                "Hollow holds at most {MAX_ACCOUNTS} extra accounts."
            ));
        }
        if store
            .accounts
            .iter()
            .any(|a| a.label.eq_ignore_ascii_case(&label))
        {
            return Err("Another account already has that name.".into());
        }
        let id = format!("acct-{:08x}", rand::random::<u32>());
        self.secrets.set(&id, &token)?;
        store.accounts.push(StoredAccount {
            id: id.clone(),
            label,
            enabled: true,
            created_at: rfc3339(Utc::now()),
            organization: None,
        });
        if let Err(error) = self.save(store) {
            let _ = self.secrets.delete(&id);
            return Err(error);
        }
        let mut rt = self.lock();
        rt.tokens.insert(id.clone(), token);
        rt.health.insert(id.clone(), Health::Unknown);
        Ok(id)
    }

    fn update(&self, input: &AccountUpdate) -> Result<(), String> {
        let mut store = self.store()?;
        let label = input.label.as_deref().map(clean_label).transpose()?;
        let account = store
            .accounts
            .iter_mut()
            .find(|a| a.id == input.id)
            .ok_or("That account no longer exists.")?;
        if let Some(label) = label {
            account.label = label;
        }
        if let Some(enabled) = input.enabled {
            account.enabled = enabled;
        }
        if let Some(organization) = input.organization.as_deref() {
            let organization = organization.trim();
            if organization.chars().count() > 80 || organization.contains(['\n', '\r']) {
                return Err("Use an organisation name of at most 80 characters.".into());
            }
            account.organization = (!organization.is_empty()).then(|| organization.to_string());
        }
        if let Some(token) = input.token.as_deref().filter(|t| !t.trim().is_empty()) {
            let token = Secret::new(normalize_token(token)?);
            self.secrets.set(&input.id, &token)?;
            let mut rt = self.lock();
            rt.tokens.insert(input.id.clone(), token);
            rt.health.insert(input.id.clone(), Health::Unknown);
            rt.last_probe.remove(&input.id);
            rt.keys.remove(&input.id);
        }
        self.save(store)
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        let mut store = self.store()?;
        let before = store.accounts.len();
        store.accounts.retain(|a| a.id != id);
        if store.accounts.len() == before {
            return Ok(());
        }
        if store.pinned == id {
            store.pinned = String::new();
        }
        self.save(store)?;
        self.secrets.delete(id)?;
        let mut rt = self.lock();
        rt.tokens.remove(id);
        rt.usage.remove(id);
        rt.health.remove(id);
        rt.last_probe.remove(id);
        rt.keys.remove(id);
        rt.picks.remove(id);
        rt.affinity.retain(|_, account| account != id);
        Ok(())
    }

    fn set_routing(
        &self,
        routing: Routing,
        pinned: Option<String>,
        ceiling: Option<u8>,
    ) -> Result<(), String> {
        let mut store = self.store()?;
        let _ = routing; // Old clients may still send the retired setting.
        store.routing = Routing::Spread;
        if let Some(pinned) = pinned {
            if !store.accounts.iter().any(|a| a.id == pinned) {
                return Err("That account no longer exists.".into());
            }
            store.pinned.clear();
        }
        if let Some(ceiling) = ceiling {
            if !(50..=100).contains(&ceiling) {
                return Err("Choose a ceiling between 50 and 100 percent.".into());
            }
            store.ceiling_percent = ceiling;
        }
        self.save(store)
    }

    fn set_failover(&self, enabled: bool) -> Result<(), String> {
        let mut store = self.store()?;
        let _ = enabled; // Compatibility command; rotation is always on.
        store.failover = true;
        self.save(store)
    }

    fn set_failover_across_organizations(&self, enabled: bool) -> Result<(), String> {
        let mut store = self.store()?;
        store.failover_across_organizations = enabled;
        self.save(store)
    }
}

fn clean_label(label: &str) -> Result<String, String> {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return Err("Give the account a name.".into());
    }
    if label.chars().count() > 60 {
        return Err("Keep the name under 60 characters.".into());
    }
    Ok(label)
}

fn refuse_account_command(cmd: &mut Command, message: &str) {
    let cwd = cmd.as_std().get_current_dir().map(Path::to_path_buf);
    *cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        "printf '%s\\n' \"$1\" >&2; exit 1",
        crate::agent::UNAVAILABLE_ARGV0,
        message,
    ]);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn window_view(w: Window) -> WindowView {
    WindowView {
        used_percent: w.used_percent,
        resets_at: w
            .resets_at
            .and_then(|s| DateTime::<Utc>::from_timestamp(s, 0))
            .map(rfc3339),
    }
}

// ── Wire types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    Ok,
    /// At or over the spread ceiling in a live window.
    Near,
    /// Out of plan allowance until a window resets.
    Limited,
    Invalid,
    MissingToken,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    pub used_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    pub five_hour: Option<WindowView>,
    pub seven_day: Option<WindowView>,
    pub status: Option<String>,
    pub overage_disabled_reason: Option<String>,
    pub source: UsageSource,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub status: AccountStatus,
    pub note: Option<String>,
    pub usage: Option<UsageView>,
    pub last_used_at: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsView {
    pub routing: Routing,
    pub pinned: String,
    pub ceiling_percent: u8,
    /// Resume a chat turn on another account when its own runs out (R1).
    pub failover: bool,
    /// Failover and long-work routing may move to another organisation's
    /// account (#16).
    pub failover_across_organizations: bool,
    /// The account a new run would use now.
    pub next_account: Option<String>,
    pub accounts: Vec<AccountView>,
    /// Claude Code's own login as last seen (its runs' streams or the Usage
    /// card's fetch, live windows only; never a Keychain read here), for
    /// the login row Settings shows while a claude.ai connector set to
    /// Always pins chats to it (Spread usage across accounts, 2026-10-21).
    pub login_usage: Option<LoginUsageView>,
    pub error: Option<String>,
}

/// [`AccountsView::login_usage`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginUsageView {
    pub five_hour: Option<WindowView>,
    pub seven_day: Option<WindowView>,
    pub observed_at: String,
}

/// The login's five-hour and weekly windows from the plan card's view.
fn login_usage_view(quota: &crate::claude_quota::ClaudeQuota) -> LoginUsageView {
    let window = |id: &str| {
        quota
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| WindowView {
                used_percent: w.used_percent,
                resets_at: w.resets_at.clone(),
            })
    };
    LoginUsageView {
        five_hour: window(crate::claude_quota::FIVE_HOUR),
        seven_day: window(crate::claude_quota::SEVEN_DAY),
        observed_at: quota.observed_at.clone(),
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    pub id: String,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    /// The account's organisation (#16); `""` clears it.
    pub organization: Option<String>,
    /// A replacement token (never returned).
    pub token: Option<String>,
}

// ── Background refresh ──────────────────────────────────────────────────────

/// Probe token accounts every [`BACKGROUND_INTERVAL`] while spreading, so the
/// choice stays current between runs. The first pass waits a minute after
/// launch. Never touches the login's Keychain item.
pub fn start_refresher() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Some(registry) = global() {
                let spreading = registry
                    .store()
                    .is_ok_and(|s| s.routing == Routing::Spread && !s.accounts.is_empty());
                if spreading {
                    registry.refresh(false).await;
                }
            }
            tokio::time::sleep(BACKGROUND_INTERVAL).await;
        }
    });
}

// ── IPC ─────────────────────────────────────────────────────────────────────

fn registry() -> Result<&'static Registry, String> {
    global().ok_or_else(|| "Claude accounts are unavailable in this process.".to_string())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce(&'static Registry) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let registry = registry()?;
    tokio::task::spawn_blocking(move || f(registry))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn claude_accounts() -> Result<AccountsView, String> {
    blocking(|r| Ok(r.view())).await
}

/// Save a new account and check its token straight away, so a mistyped or
/// revoked token shows at once.
#[tauri::command]
pub async fn add_claude_account(label: String, token: String) -> Result<AccountsView, String> {
    let id = blocking(move |r| r.add(&label, &token)).await?;
    let registry = registry()?;
    registry.probe_one(&id, true, Utc::now()).await;
    blocking(|r| Ok(r.view())).await
}

#[tauri::command]
pub async fn update_claude_account(input: AccountUpdate) -> Result<AccountsView, String> {
    let id = input.id.clone();
    let retoken = input.token.as_deref().is_some_and(|t| !t.trim().is_empty());
    blocking(move |r| r.update(&input)).await?;
    if retoken {
        registry()?.probe_one(&id, true, Utc::now()).await;
    }
    blocking(|r| Ok(r.view())).await
}

#[tauri::command]
pub async fn remove_claude_account(id: String) -> Result<AccountsView, String> {
    blocking(move |r| {
        r.remove(&id)?;
        Ok(r.view())
    })
    .await
}

#[tauri::command]
pub async fn set_claude_account_routing(
    routing: Routing,
    pinned: Option<String>,
    ceiling_percent: Option<u8>,
) -> Result<AccountsView, String> {
    save_routing(routing, pinned, ceiling_percent).await
}

/// What [`set_claude_account_routing`] does, for callers that aren't the
/// webview (the `accounts.claudeRouting` setting key).
pub(crate) async fn save_routing(
    routing: Routing,
    pinned: Option<String>,
    ceiling_percent: Option<u8>,
) -> Result<AccountsView, String> {
    blocking(move |r| {
        r.set_routing(routing, pinned, ceiling_percent)?;
        Ok(r.view())
    })
    .await
}

/// Settings, Usage, "Resume on another account when one runs out" (R1).
#[tauri::command]
pub async fn set_claude_account_failover(enabled: bool) -> Result<AccountsView, String> {
    save_failover(enabled).await
}

/// What [`set_claude_account_failover`] does, for callers that aren't the
/// webview (the `accounts.claudeFailover` setting key).
pub(crate) async fn save_failover(enabled: bool) -> Result<AccountsView, String> {
    blocking(move |r| {
        r.set_failover(enabled)?;
        Ok(r.view())
    })
    .await
}

/// Let failover cross organisations (#16; no Settings control shows it, the COO sets it).
pub async fn set_claude_account_failover_across_organizations(
    enabled: bool,
) -> Result<AccountsView, String> {
    blocking(move |r| {
        r.set_failover_across_organizations(enabled)?;
        Ok(r.view())
    })
    .await
}

/// Check every account's usage now (Settings' Check usage button). The login
/// is read through the plan card's endpoint, which may ask for Keychain
/// access to Claude Code's login the first time.
#[tauri::command]
pub async fn refresh_claude_accounts() -> Result<AccountsView, String> {
    registry()?.refresh(true).await;
    blocking(|r| Ok(r.view())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ── Login override (2026-10-09) ──

    fn reading(id: &str, percent: f64, resets: &str) -> crate::claude_quota::Reading {
        crate::claude_quota::Reading {
            id: id.into(),
            label: id.into(),
            used_percent: percent,
            resets_at: Some(resets.into()),
        }
    }

    #[test]
    fn the_login_is_low_past_the_spread_ceiling_or_100_in_single_with_room_elsewhere() {
        let five = |p| {
            vec![reading(
                crate::claude_quota::FIVE_HOUR,
                p,
                "2026-10-09T21:00:00Z",
            )]
        };
        let room = || Some("Tech".to_string());
        // Spread: the ceiling is the line.
        assert!(login_low(Routing::Spread, 90.0, &five(89.0), room(), Utc::now()).is_none());
        let low = login_low(Routing::Spread, 90.0, &five(90.0), room(), Utc::now()).unwrap();
        assert_eq!(
            (low.percent, low.line, low.room_on.as_str()),
            (90.0, 90.0, "Tech")
        );
        // Single: only past 100% (overage counts).
        assert!(login_low(Routing::Single, 90.0, &five(95.0), room(), Utc::now()).is_none());
        assert!(login_low(Routing::Single, 90.0, &five(101.0), room(), Utc::now()).is_some());
        // Nowhere with room: no warning.
        assert!(login_low(Routing::Spread, 90.0, &five(101.0), None, Utc::now()).is_none());
        // The fullest of the plan windows; per-model rows don't count.
        let mixed = vec![
            reading(crate::claude_quota::FIVE_HOUR, 92.0, "a"),
            reading(crate::claude_quota::SEVEN_DAY, 97.0, "b"),
            reading(crate::claude_quota::SEVEN_DAY_OPUS, 120.0, "c"),
        ];
        let low = login_low(Routing::Spread, 90.0, &mixed, room(), Utc::now()).unwrap();
        assert_eq!(
            (low.window.as_str(), low.percent),
            (crate::claude_quota::SEVEN_DAY, 97.0)
        );
        assert!(
            low.message().contains("97% of its weekly limit"),
            "{}",
            low.message()
        );
    }

    #[test]
    fn set_aside_wording_and_once_per_conversation_and_window() {
        let low = |resets: &str| {
            login_low(
                Routing::Spread,
                90.0,
                &[reading(crate::claude_quota::FIVE_HOUR, 105.0, resets)],
                Some("teammate".into()),
                Utc::now(),
            )
            .unwrap()
        };
        let now = low("2026-10-21T20:00:00Z");
        assert_eq!(
            now.set_aside_message(&["Superhuman Mail".into()]),
            "Superhuman Mail was left out of this chat: Claude Code's login is at 105% of its 5-hour limit, and claude.ai connectors only run there, so messages that don't need it use your added accounts. Name it in a message to bring it back for that message (it then runs on the login). Where Connectors offers Hollow's own sign-in for a connector, that works on every account."
        );
        assert!(now
            .set_aside_message(&["A".into(), "B".into()])
            .starts_with("A, B were left out"));
        assert!(now
            .set_aside_reason()
            .contains("is at 105% of its 5-hour limit"));
        // A row per conversation per window, not per turn.
        let conversation = format!("s-set-aside-{}", rand::random::<u32>());
        assert!(first_set_aside_notice(&conversation, &now));
        assert!(!first_set_aside_notice(&conversation, &now));
        assert!(first_set_aside_notice(
            &format!("{conversation}-other"),
            &now
        ));
        assert!(first_set_aside_notice(
            &conversation,
            &low("2026-10-22T01:00:00Z")
        ));
        // Outside the GUI nothing steps aside.
        assert_eq!(login_out_of_room(), None);
    }

    #[test]
    fn the_login_warning_is_once_per_window() {
        let seen = Mutex::new(Vec::new());
        let low = |resets: &str| {
            login_low(
                Routing::Spread,
                90.0,
                &[reading(crate::claude_quota::FIVE_HOUR, 95.0, resets)],
                Some("Tech".into()),
                Utc::now(),
            )
            .unwrap()
        };
        assert!(first_warning_in(&seen, &low("2026-10-09T21:00:00Z")));
        assert!(!first_warning_in(&seen, &low("2026-10-09T21:00:00Z")));
        // The window reset: a new window warns again.
        assert!(first_warning_in(&seen, &low("2026-10-10T02:00:00Z")));
        // Both windows past the line: the fuller one changing hands within
        // the same windows warns nothing new.
        let seen = Mutex::new(Vec::new());
        let both = |five: f64, week: f64| {
            login_low(
                Routing::Spread,
                90.0,
                &[
                    reading(crate::claude_quota::FIVE_HOUR, five, "a"),
                    reading(crate::claude_quota::SEVEN_DAY, week, "b"),
                ],
                Some("Tech".into()),
                Utc::now(),
            )
            .unwrap()
        };
        assert!(first_warning_in(&seen, &both(95.0, 92.0)));
        assert!(!first_warning_in(&seen, &both(95.0, 99.0)));
        // An unknown reset is keyed on a slice of the window's length, so it
        // warns again once that slice has passed, not never.
        let seen = Mutex::new(Vec::new());
        let unknown = |now: DateTime<Utc>| {
            let mut r = reading(crate::claude_quota::FIVE_HOUR, 95.0, "");
            r.resets_at = None;
            login_low(Routing::Spread, 90.0, &[r], Some("Tech".into()), now).unwrap()
        };
        let t0 = DateTime::parse_from_rfc3339("2026-10-09T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(first_warning_in(&seen, &unknown(t0)));
        assert!(!first_warning_in(
            &seen,
            &unknown(t0 + chrono::Duration::minutes(30))
        ));
        assert!(first_warning_in(
            &seen,
            &unknown(t0 + chrono::Duration::hours(6))
        ));
    }

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemSecrets {
        fn get(&self, id: &str) -> Result<Option<Secret>, String> {
            Ok(self.0.lock().unwrap().get(id).cloned().map(Secret::new))
        }
        fn set(&self, id: &str, token: &Secret) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(id.into(), token.as_str().into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }

    const TOKEN_A: &str = "sk-ant-oat01-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TOKEN_B: &str = "sk-ant-oat01-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn registry() -> (tempfile::TempDir, Registry) {
        let dir = tempfile::Builder::new()
            .prefix("accounts-")
            .tempdir()
            .unwrap();
        let reg = Registry::new(
            dir.path().to_path_buf(),
            Box::<MemSecrets>::default(),
            "http://127.0.0.1:1",
        );
        (dir, reg)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    fn usage(five: f64, seven: f64, resets: i64, now: DateTime<Utc>) -> Usage {
        Usage {
            five_hour: Some(Window {
                used_percent: five,
                resets_at: Some(resets),
            }),
            seven_day: Some(Window {
                used_percent: seven,
                resets_at: Some(resets),
            }),
            status: Some("allowed".into()),
            overage_reason: None,
            source: UsageSource::Probe,
            observed: now,
        }
    }

    fn env_of(cmd: &Command, key: &str) -> Option<Option<String>> {
        cmd.as_std()
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(key))
            .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    fn cand(id: &str, load: Option<f64>, blocked: bool, last_used: u64) -> Candidate {
        Candidate {
            id: id.into(),
            load,
            blocked,
            last_used,
        }
    }

    #[test]
    fn six_accounts_choose_binding_headroom_and_rejection_beats_zero_meter() {
        let (dir, reg) = registry();
        let now = Utc::now();
        let reset = now.timestamp() + 3600;
        let mut ids = HashMap::new();
        for (name, five, week, off) in [
            ("jimmy jam", 99.0, 0.0, true),
            ("Tech", 0.0, 28.0, true),
            ("Admin", 0.0, 4.0, false),
            ("acme", 12.0, 4.0, false),
            ("ferumbas", 0.0, 14.0, false),
            ("man", 1.0, 50.0, false),
        ] {
            let token = format!("sk-ant-oat01-{name:0<54}").replace(' ', "x");
            let id = reg.add(name, &token).unwrap();
            let mut u = usage(five, week, reset, now);
            if off {
                u.overage_reason = Some("org_spend_cap_reached".into());
            }
            reg.observe(&id, u);
            ids.insert(name, id);
        }
        let tech = &ids["Tech"];
        assert!(reg
            .reject_account(tech, Some(reset), now)
            .unwrap()
            .contains("contradicts"));
        reg.record_probe(
            tech,
            ProbeOutcome::Usage(usage(0.0, 0.0, reset, now + chrono::Duration::seconds(1))),
        );
        assert_eq!(reg.choose(None, now), Some(ids["Admin"].clone()));
        assert_eq!(
            reg.choose(Some("previous-tech"), now),
            Some(ids["Admin"].clone())
        );
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert_eq!(account_of(&cmd), Some(ids["Admin"].clone()));
        assert!(pick_receipt(&cmd).unwrap().contains("96.0% headroom"));
        assert!(load_store(dir.path()).unwrap().exhausted_accounts[tech].live(now));
        assert!(!load_store(dir.path()).unwrap().exhausted_accounts[tech].live(at(reset)));
        for name in ["Admin", "acme", "ferumbas", "man"] {
            reg.reject_account(&ids[name], Some(reset), now);
        }
        assert_eq!(reg.choose(None, now), None);
        assert_eq!(reg.soonest_room(now), Some(reset));
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert!(
            crate::agent::is_unavailable_command(&cmd),
            "never fall back to login or a full account"
        );
    }

    #[test]
    fn legacy_single_and_disabled_failover_are_migrated_and_cannot_return() {
        let (dir, reg) = registry();
        let mut old = serde_json::to_value(Store::default()).unwrap();
        old["routing"] = serde_json::json!("single");
        old["failover"] = serde_json::json!(false);
        old["pinned"] = serde_json::json!("old-account");
        std::fs::write(dir.path().join(FILE), old.to_string()).unwrap();
        let migrated = reg.store().unwrap();
        assert_eq!(migrated.routing, Routing::Spread);
        assert!(migrated.failover);
        reg.set_failover(false).unwrap();
        reg.set_routing(Routing::Single, None, None).unwrap();
        assert!(reg.store().unwrap().failover);
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        let stored: serde_json::Value = serde_json::from_str(&text).unwrap();
        for key in ["routing", "failover", "pinned"] {
            assert!(stored.get(key).is_none());
        }
    }

    #[test]
    fn legacy_enabled_rotation_fields_are_also_removed_on_load() {
        let (dir, _) = registry();
        let mut old = serde_json::to_value(Store::default()).unwrap();
        old["routing"] = serde_json::json!("spread");
        old["failover"] = serde_json::json!(true);
        old["pinned"] = serde_json::json!("");
        std::fs::write(dir.path().join(FILE), old.to_string()).unwrap();
        load_store(dir.path()).unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(FILE)).unwrap()).unwrap();
        for key in ["routing", "failover", "pinned"] {
            assert!(stored.get(key).is_none());
        }
    }

    #[test]
    fn rejection_survives_stale_settings_and_learns_an_unknown_reset() {
        let (dir, reg) = registry();
        let id = reg.add("Tech", TOKEN_A).unwrap();
        let now = Utc::now();
        let stale = reg.store().unwrap();
        reg.reject_account(&id, None, now);
        assert!(reg.exhausted(&id, now));
        let reset = now.timestamp() + 3600;
        reg.record_probe(&id, ProbeOutcome::Usage(usage(0.0, 0.0, reset, now)));
        assert!(
            reg.exhausted(&id, now),
            "allowed probe does not clear rejection"
        );
        assert_eq!(
            load_store(dir.path()).unwrap().exhausted_accounts[&id].reset_at,
            Some(reset)
        );
        reg.save(stale).unwrap();
        assert_eq!(
            reg.store().unwrap().exhausted_accounts[&id].reset_at,
            Some(reset)
        );
        let stale = reg.store().unwrap();
        reg.reject_account(&id, Some(reset + 60), now);
        reg.reject_account(&id, Some(reset - 60), now);
        reg.reject_account(&id, None, now);
        reg.save(stale).unwrap();
        assert_eq!(
            load_store(dir.path()).unwrap().exhausted_accounts[&id].reset_at,
            Some(reset + 60)
        );
        assert!(!reg.exhausted(&id, at(reset + 61)));
        reg.remove(&id).unwrap();
        let replacement = reg.add("Tech", TOKEN_B).unwrap();
        assert!(!reg.exhausted(&replacement, now));
    }

    #[test]
    fn corrupt_store_refuses_a_launch_instead_of_using_login() {
        let (dir, reg) = registry();
        std::fs::write(dir.path().join(FILE), "{broken").unwrap();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, Utc::now());
        assert!(crate::agent::is_unavailable_command(&cmd));
        assert_eq!(env_of(&cmd, TOKEN_ENV), None);
    }

    #[tokio::test]
    async fn stale_usage_is_refreshed_before_ranking_and_fresh_usage_is_reused() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (dir, _) = registry();
        let reg = Registry::new(
            dir.path().to_path_buf(),
            Box::<MemSecrets>::default(),
            &format!("http://{}", listener.local_addr().unwrap()),
        );
        let a = reg.add("Admin", TOKEN_A).unwrap();
        let b = reg.add("Other", TOKEN_B).unwrap();
        let now = Utc::now();
        let reset = now.timestamp() + 3600;
        let old = now - chrono::Duration::seconds(PROBE_MIN_INTERVAL_SECS + 1);
        reg.observe(&a, usage(99.0, 0.0, reset, old));
        reg.observe(&b, usage(0.0, 0.0, reset, old));
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let len = socket.read(&mut bytes).await.unwrap();
                let load = if String::from_utf8_lossy(&bytes[..len]).contains(TOKEN_A) {
                    "0.04"
                } else {
                    "0.14"
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: 0\r\nanthropic-ratelimit-unified-5h-utilization: 0\r\nanthropic-ratelimit-unified-5h-reset: {reset}\r\nanthropic-ratelimit-unified-7d-utilization: {load}\r\nanthropic-ratelimit-unified-7d-reset: {reset}\r\n\r\n").as_bytes()).await.unwrap();
            }
        });
        tokio::time::timeout(Duration::from_secs(5), reg.refresh(false))
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(reg.choose(None, Utc::now()), Some(a.clone()));
        let observed = reg.usage_for(&a).unwrap().observed;
        reg.refresh(false).await;
        assert_eq!(reg.usage_for(&a).unwrap().observed, observed);
        assert_eq!(
            reg.lock().health.get(&a),
            Some(&Health::Ok),
            "no second request after server closes"
        );
    }

    #[test]
    fn tokens_are_stripped_of_whitespace_and_shape_checked() {
        let wrapped = format!("{}\n  {}", &TOKEN_A[..30], &TOKEN_A[30..]);
        assert_eq!(normalize_token(&wrapped).unwrap(), TOKEN_A);
        assert!(normalize_token("").is_err());
        assert!(
            normalize_token("sk-ant-api03-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
                .unwrap_err()
                .contains("API key")
        );
        assert!(normalize_token("hello").is_err());
        assert!(normalize_token("sk-ant-oat01-short").is_err());
    }

    #[test]
    fn secrets_never_print() {
        let secret = Secret::new(TOKEN_A.into());
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn probe_headers_parse_on_200_and_429() {
        let now = at(1_790_000_000);
        let headers: HashMap<&str, &str> = HashMap::from([
            ("anthropic-ratelimit-unified-5h-utilization", "0.42"),
            ("anthropic-ratelimit-unified-5h-reset", "1790003600"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-7d-utilization", "1.0"),
            ("anthropic-ratelimit-unified-7d-reset", "1790300000"),
            ("anthropic-ratelimit-unified-7d-status", "rejected"),
            (
                "anthropic-ratelimit-unified-overage-disabled-reason",
                "org_spend_cap_reached",
            ),
        ]);
        let get = |k: &str| headers.get(k).map(|v| v.to_string());
        for status in [200, 429] {
            let ProbeOutcome::Usage(u) = parse_headers(status, &get, now) else {
                panic!("usage expected on {status}");
            };
            assert_eq!(u.five_hour.unwrap().used_percent, 42.0);
            assert_eq!(u.seven_day.unwrap().used_percent, 100.0);
            assert_eq!(u.status.as_deref(), Some("rejected"));
            assert_eq!(u.overage_reason.as_deref(), Some("org_spend_cap_reached"));
            assert!(u.blocked(now));
            assert_eq!(u.load(now), 100.0);
        }
        let none = |_: &str| None;
        assert_eq!(parse_headers(401, &none, now), ProbeOutcome::Invalid);
        assert!(matches!(
            parse_headers(500, &none, now),
            ProbeOutcome::Failed(_)
        ));
    }

    #[test]
    fn a_window_past_its_reset_counts_as_empty() {
        let now = at(1_790_000_000);
        let u = usage(95.0, 60.0, now.timestamp() - 1, now);
        assert_eq!(u.load(now), 0.0);
        assert!(!u.blocked(now));
    }

    #[test]
    fn spread_picks_least_loaded_eligible_then_least_recent() {
        let c = [
            cand("login", Some(70.0), false, 3),
            cand("a", Some(20.0), false, 2),
            cand("b", Some(20.0), false, 1),
        ];
        assert_eq!(pick(&c, 90.0, None).as_deref(), Some("b"));
        // Measured room outranks an unknown reading.
        let c = [cand("a", Some(10.0), false, 0), cand("b", None, false, 5)];
        assert_eq!(pick(&c, 90.0, None).as_deref(), Some("a"));
    }

    #[test]
    fn spread_skips_blocked_and_over_ceiling_without_fallback() {
        let c = [
            cand("login", Some(95.0), false, 0),
            cand("a", Some(10.0), true, 0),
            cand("b", Some(40.0), false, 0),
        ];
        assert_eq!(pick(&c, 90.0, None).as_deref(), Some("b"));
        // Nothing under the ceiling: wait.
        let c = [
            cand("login", Some(97.0), false, 0),
            cand("a", Some(92.0), false, 0),
            cand("b", Some(5.0), true, 0),
        ];
        assert_eq!(pick(&c, 90.0, None), None);
        // Everything blocked: still wait.
        let c = [
            cand("a", Some(100.0), true, 0),
            cand("b", Some(101.0), true, 0),
        ];
        assert_eq!(pick(&c, 90.0, None), None);
        assert_eq!(pick(&[], 90.0, None), None);
    }

    #[test]
    fn a_conversation_always_moves_to_more_room() {
        let c = [
            cand("a", Some(30.0), false, 9),
            cand("b", Some(20.0), false, 1),
        ];
        assert_eq!(pick(&c, 90.0, Some("a")).as_deref(), Some("b"));
        let c = [
            cand("a", Some(50.0), false, 9),
            cand("b", Some(20.0), false, 1),
        ];
        assert_eq!(pick(&c, 90.0, Some("a")).as_deref(), Some("b"));
        let c = [
            cand("a", Some(91.0), false, 9),
            cand("b", Some(89.0), false, 1),
        ];
        assert_eq!(pick(&c, 90.0, Some("a")).as_deref(), Some("b"));
    }

    #[test]
    fn with_no_token_accounts_apply_changes_nothing() {
        let (_dir, reg) = registry();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, at(1_790_000_000));
        assert_eq!(cmd.as_std().get_envs().count(), 0);
    }

    #[test]
    fn single_routing_uses_added_accounts_only() {
        let (dir, reg) = registry();
        let id = reg.add("Second", TOKEN_A).unwrap();
        let other = reg.add("Third", TOKEN_B).unwrap();
        let now = at(1_790_000_000);
        // Nothing pinned: the first added account, never the login.
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert_eq!(env_of(&cmd, TOKEN_ENV), Some(Some(TOKEN_A.to_string())));
        assert_eq!(account_of(&cmd), Some(id.clone()));
        assert_eq!(reg.view().pinned, id);

        // The pinned account switched off: the next usable one.
        reg.update(&AccountUpdate {
            id: id.clone(),
            enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert_eq!(account_of(&cmd), Some(other.clone()));

        // None usable: the Claude Code login is the fallback (variable removed).
        reg.update(&AccountUpdate {
            id: other.clone(),
            enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert!(crate::agent::is_unavailable_command(&cmd));
        assert_eq!(account_of(&cmd), None);
        assert_eq!(reg.view().next_account, None);
        reg.update(&AccountUpdate {
            id: id.clone(),
            enabled: Some(true),
            ..Default::default()
        })
        .unwrap();

        reg.set_routing(Routing::Single, Some(id.clone()), None)
            .unwrap();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert_eq!(env_of(&cmd, TOKEN_ENV), Some(Some(TOKEN_A.to_string())));
        assert_eq!(account_of(&cmd), Some(id.clone()));

        // The file holds no secret.
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("sk-ant"));
        assert!(text.contains("Second"));

        // A fresh registry reads the same settings back.
        let again = Registry::new(
            dir.path().to_path_buf(),
            Box::<MemSecrets>::default(),
            "http://127.0.0.1:1",
        );
        assert!(again.store().unwrap().pinned.is_empty());
    }

    #[test]
    fn spread_routes_by_usage_and_keeps_a_conversation() {
        let (_dir, reg) = registry();
        let a = reg.add("A", TOKEN_A).unwrap();
        let b = reg.add("B", TOKEN_B).unwrap();
        reg.set_routing(Routing::Spread, None, Some(90)).unwrap();
        let now = at(1_790_000_000);
        let later = now.timestamp() + 3600;
        reg.observe(&a, usage(60.0, 30.0, later, now));
        reg.observe(&b, usage(10.0, 20.0, later, now));

        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, Some("chat:s-1"), None, now);
        assert_eq!(account_of(&cmd), Some(b.clone()));
        assert_eq!(env_of(&cmd, TOKEN_ENV), Some(Some(TOKEN_B.to_string())));

        // B fills up past A plus the margin: the conversation moves.
        reg.observe(&b, usage(80.0, 40.0, later, now));
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, Some("chat:s-1"), None, now);
        assert_eq!(account_of(&cmd), Some(a.clone()));

        // A refused token drops out of rotation.
        reg.record_probe(&a, ProbeOutcome::Invalid);
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, None, now);
        assert_eq!(account_of(&cmd), Some(b.clone()));
        let view = reg.view();
        let a_view = view.accounts.iter().find(|x| x.id == a).unwrap();
        assert_eq!(a_view.status, AccountStatus::Invalid);
    }

    #[test]
    fn removing_an_account_deletes_its_token_and_unpins_it() {
        let (_dir, reg) = registry();
        let id = reg.add("Second", TOKEN_A).unwrap();
        reg.set_routing(Routing::Single, Some(id.clone()), None)
            .unwrap();
        reg.remove(&id).unwrap();
        assert!(reg.secrets.get(&id).unwrap().is_none());
        assert_eq!(reg.store().unwrap().pinned, "");
        assert!(reg
            .set_routing(Routing::Single, Some(LOGIN_ID.into()), None)
            .is_err());
        assert!(reg
            .update(&AccountUpdate {
                id: LOGIN_ID.into(),
                ..Default::default()
            })
            .is_err());
    }

    #[test]
    fn add_refuses_duplicates_and_bad_tokens() {
        let (_dir, reg) = registry();
        reg.add("Work", TOKEN_A).unwrap();
        assert!(reg.add("work", TOKEN_B).is_err());
        assert!(reg.add("Other", "nope").is_err());
        assert!(reg.add("  ", TOKEN_B).is_err());
        assert!(reg.set_routing(Routing::Spread, None, Some(10)).is_err());
    }

    /// R1: a failover takes the best ELIGIBLE account outside the
    /// exhausted ones, and never falls back to one that is itself full.
    #[test]
    fn a_failover_picks_only_an_eligible_account_outside_the_exhausted_ones() {
        let c = [
            cand("admin", Some(99.0), false, 0),
            cand("tech", Some(0.0), false, 2),
            cand("jam", Some(0.0), false, 1),
        ];
        assert_eq!(
            pick_failover(&c, 90.0, &["admin".into()]).as_deref(),
            Some("jam"),
            "least recently used on a tie"
        );
        assert_eq!(
            pick_failover(&c, 90.0, &["admin".into(), "jam".into()]).as_deref(),
            Some("tech")
        );
        // Unmeasured counts as empty, as for a new run.
        assert_eq!(
            pick_failover(
                &[
                    cand("admin", Some(99.0), false, 0),
                    cand("new", None, false, 0)
                ],
                90.0,
                &["admin".into()]
            )
            .as_deref(),
            Some("new")
        );
        // Nothing eligible: none, where `pick` would fall back.
        let full = [
            cand("admin", Some(100.0), true, 0),
            cand("tech", Some(95.0), false, 0),
            cand("jam", Some(20.0), true, 0),
        ];
        assert_eq!(pick_failover(&full, 90.0, &["admin".into()]), None);
        assert!(pick(&full, 90.0, None).is_none());
    }

    fn quota(five: f64, status: &str, resets: i64) -> StreamQuota {
        StreamQuota {
            five_hour: Some(crate::claude_quota::StreamWindow {
                utilization: five,
                resets_at: Some(resets),
            }),
            seven_day: Some(crate::claude_quota::StreamWindow {
                utilization: 0.1,
                resets_at: Some(resets + 86_400),
            }),
            status: Some(status.into()),
            using_overage: None,
        }
    }

    /// Turn guards: overage counts as crossed. The recorded fixtures' event
    /// (`utilization` 1.01 on overage, `rejected`) is over any ceiling, 100%
    /// included, and so is 1.01 with an `allowed` status; the ceiling switch
    /// is on only in Spread mode with failover on.
    #[test]
    fn overage_counts_as_crossed_and_the_switch_needs_spread() {
        let (_dir, reg) = registry();
        reg.add("Admin", TOKEN_A).unwrap();
        reg.add("Tech", TOKEN_B).unwrap();
        let now = at(1_790_000_000);
        let soon = now.timestamp() + 1800;
        let recorded = crate::claude_quota::parse_rate_limit_event(&serde_json::json!({
            "type": "rate_limit_event",
            "rate_limit_info": {"status": "rejected", "isUsingOverage": true,
                "unifiedWindows": {"five_hour": {"utilization": 1.01, "resetsAt": soon}}}
        }))
        .unwrap();
        for ceiling in [90, 100] {
            reg.set_routing(Routing::Spread, None, Some(ceiling))
                .unwrap();
            assert!(reg.stream_exhausted(&recorded, now), "{ceiling}%");
            assert!(
                reg.stream_exhausted(&quota(1.01, "allowed", soon), now),
                "{ceiling}%"
            );
        }
        assert!(
            !reg.stream_exhausted(&quota(0.95, "allowed_warning", soon), now),
            "under 100%"
        );
        assert!(reg.ceiling_switch_enabled());
        reg.set_failover(false).unwrap();
        assert!(
            reg.ceiling_switch_enabled(),
            "retired toggle cannot disable rotation"
        );
        reg.set_failover(true).unwrap();
        let first = reg.view().accounts[0].id.clone();
        reg.set_routing(Routing::Single, Some(first), None).unwrap();
        assert!(
            reg.ceiling_switch_enabled(),
            "legacy Single mode is migrated"
        );
    }

    const TOKEN_C: &str = "sk-ant-oat01-cccccccccccccccccccccccccccccccccccccccccccccccccccc";

    /// #16: long unattended work starts away from an account at or above
    /// 80% when another has room, never refuses, and leaves chat routing
    /// alone.
    #[test]
    fn long_work_avoids_an_account_above_eighty_percent() {
        let c = |id: &str, load: Option<f64>, blocked: bool| Candidate {
            id: id.into(),
            load,
            blocked,
            last_used: 0,
        };
        let pool = [c("a", Some(85.0), false), c("b", Some(40.0), false)];
        assert_eq!(pick_long_work(&pool, "a", 80.0), "b");
        assert_eq!(pick_long_work(&pool, "b", 80.0), "b", "room: stays");
        let full = [c("a", Some(85.0), false), c("b", Some(95.0), false)];
        assert_eq!(
            pick_long_work(&full, "a", 80.0),
            "a",
            "none under 80: runs as before"
        );
        let blocked = [c("a", Some(10.0), true), c("b", None, false)];
        assert_eq!(
            pick_long_work(&blocked, "a", 80.0),
            "b",
            "out of allowance moves too"
        );
    }

    /// COO rebuild L7: the long-work line is the CFO's `100 - reserve`; a
    /// larger reserve moves work off a fuller account sooner.
    #[test]
    fn long_work_follows_the_cfo_reserve() {
        let c = |id: &str, load: Option<f64>| Candidate {
            id: id.into(),
            load,
            blocked: false,
            last_used: 0,
        };
        let pool = [c("a", Some(65.0)), c("b", Some(20.0))];
        assert_eq!(pick_long_work(&pool, "a", 80.0), "a", "default 20% reserve");
        assert_eq!(pick_long_work(&pool, "a", 60.0), "b", "40% reserve");
        assert_eq!(
            DEFAULT_LONG_WORK_RESERVE,
            crate::cfo::settings::DEFAULT_RESERVE
        );
    }

    #[test]
    fn organisations_bound_failover_unless_allowed() {
        assert!(same_organization(None, None, false));
        assert!(same_organization(Some("Acme "), Some("acme"), false));
        assert!(!same_organization(Some("Acme"), Some("Personal"), false));
        assert!(!same_organization(Some("Acme"), None, false));
        assert!(same_organization(Some("Acme"), Some("Personal"), true));
    }

    /// #16: a failover (chat or automation) stays inside the exhausted
    /// account's organisation until the person allows crossing, and long
    /// work starts on a roomy account of the same organisation.
    #[tokio::test]
    async fn failover_and_long_work_stay_inside_the_organisation() {
        let (_dir, reg) = registry();
        let work = reg.add("Work", TOKEN_A).unwrap();
        let personal = reg.add("Personal", TOKEN_B).unwrap();
        let work2 = reg.add("Work 2", TOKEN_C).unwrap();
        let set_org = |id: &str, org: &str| {
            reg.update(&AccountUpdate {
                id: id.into(),
                organization: Some(org.into()),
                ..Default::default()
            })
            .unwrap();
        };
        set_org(&work, "Acme");
        set_org(&personal, "Personal");
        let now = at(1_790_000_000);
        let later = now.timestamp() + 7200;
        reg.observe(&work, usage(99.0, 30.0, later, now));
        reg.observe(&personal, usage(5.0, 5.0, later, now));
        reg.observe(&work2, usage(10.0, 10.0, later, now));
        // Work 2 has no organisation: not Acme's, so not a target.
        assert_eq!(reg.failover_target(std::slice::from_ref(&work), now), None);
        set_org(&work2, "acme");
        assert_eq!(
            reg.failover_target(std::slice::from_ref(&work), now),
            Some(work2.clone())
        );
        // Work 2 full too: Personal only once crossing is allowed.
        reg.observe(&work2, usage(100.0, 40.0, later, now));
        assert_eq!(reg.failover_target(std::slice::from_ref(&work), now), None);
        reg.set_failover_across_organizations(true).unwrap();
        assert_eq!(
            reg.failover_target(std::slice::from_ref(&work), now),
            Some(personal.clone())
        );
        assert!(reg.view().failover_across_organizations);
        reg.set_failover_across_organizations(false).unwrap();

        // Initial picks ignore the retired pin and use the most room.
        reg.observe(&work2, usage(30.0, 10.0, later, now));
        reg.observe(&personal, usage(50.0, 10.0, later, now));
        assert_eq!(
            reg.choose(None, now),
            Some(work2.clone()),
            "chat: most room"
        );
        assert_eq!(reg.choose_long_work(now), Some(work2.clone()));
        // COO rebuild L7: the CFO holds and picks among the same accounts:
        // the route and its organisation, never Personal unless crossing.
        let eligible = reg.long_work_eligible(now);
        assert!(
            eligible.contains(&work) && eligible.contains(&work2),
            "{eligible:?}"
        );
        assert!(!eligible.contains(&personal), "another organisation");
        assert!(reg.long_work_eligible(now).contains(&work2));
        assert!(!reg.long_work_eligible(now).contains(&personal));
        reg.set_failover_across_organizations(true).unwrap();
        assert!(reg.long_work_eligible(now).contains(&personal));
        reg.set_failover_across_organizations(false).unwrap();
        // The retired toggle cannot disable automatic rotation.
        reg.set_failover(false).unwrap();
        assert_eq!(reg.choose_long_work(now), Some(work2.clone()));
        assert!(reg.long_work_eligible(now).contains(&work2));
        reg.set_failover(true).unwrap();

        // Headroom: labels, organisations and the next routes; never a token.
        let view = reg.headroom(now);
        let text = view.to_string();
        assert!(!text.contains("sk-ant"), "{text}");
        assert!(!text.contains(&work), "ids stay out: {text}");
        assert_eq!(view["nextChat"], "Work 2");
        assert_eq!(view["nextLongWork"], "Work 2");
        assert_eq!(view["longWorkCeilingPercent"], 80.0);
        let row = view["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["label"] == "Work")
            .unwrap();
        assert_eq!(row["roomForLongWork"], false);
        assert_eq!(row["ledgerRef"], crate::run_attribution::account_ref(&work));
        // Organisation names are bounded.
        assert!(reg
            .update(&AccountUpdate {
                id: work.clone(),
                organization: Some("x".repeat(81)),
                ..Default::default()
            })
            .is_err());
    }

    /// R1: what the chat failover asks the registry. A run's own event over
    /// the ceiling (or rejected) is exhaustion; the target skips the
    /// exhausted account; a forced account wins and the conversation keeps
    /// it; the soonest reset is the earliest account's; the setting is on by
    /// default and persists.
    #[tokio::test]
    async fn failover_queries_and_the_forced_account() {
        let (dir, reg) = registry();
        let admin = reg.add("Admin", TOKEN_A).unwrap();
        let tech = reg.add("Tech", TOKEN_B).unwrap();
        let now = at(1_790_000_000);
        let soon = now.timestamp() + 1800;
        let later = now.timestamp() + 7200;
        assert!(
            reg.failover_enabled() && reg.view().failover,
            "on by default"
        );

        assert!(
            reg.stream_exhausted(&quota(0.95, "allowed_warning", soon), now),
            "over the 90 % ceiling"
        );
        assert!(
            reg.stream_exhausted(&quota(0.2, "rejected", soon), now),
            "rejected"
        );
        assert!(!reg.stream_exhausted(&quota(0.5, "allowed", soon), now));

        reg.observe(&admin, usage(99.0, 30.0, soon, now));
        assert!(reg.exhausted(&admin, now) && !reg.exhausted(&tech, now));
        assert_eq!(
            reg.failover_target(std::slice::from_ref(&admin), now),
            Some(tech.clone())
        );
        assert_eq!(reg.label(&tech).as_deref(), Some("Tech"));

        // Forced: the chosen account whatever the routing, and the
        // conversation's affinity follows it.
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, Some("chat:s-9"), Some(&tech), now);
        assert_eq!(account_of(&cmd), Some(tech.clone()));
        assert_eq!(env_of(&cmd, TOKEN_ENV), Some(Some(TOKEN_B.to_string())));
        assert_eq!(reg.lock().affinity.get("chat:s-9"), Some(&tech));
        // A forced account that can no longer run is ignored.
        reg.update(&AccountUpdate {
            id: tech.clone(),
            enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, Some(&tech), now);
        assert!(crate::agent::is_unavailable_command(&cmd));
        reg.update(&AccountUpdate {
            id: tech.clone(),
            enabled: Some(true),
            ..Default::default()
        })
        .unwrap();

        // Both full: no target, and the soonest reset is Admin's.
        reg.observe(&tech, usage(100.0, 40.0, later, now));
        assert_eq!(reg.failover_target(std::slice::from_ref(&admin), now), None);
        assert_eq!(reg.soonest_room(now), Some(soon));

        // A probe that cannot reach Anthropic cannot tell.
        assert_eq!(reg.probe_exhausted(&admin).await, None);

        reg.set_failover(false).unwrap();
        assert!(reg.failover_enabled() && reg.view().failover);
        let reread = Registry::new(
            dir.path().to_path_buf(),
            Box::<MemSecrets>::default(),
            "http://127.0.0.1:1",
        );
        assert!(reread.store().unwrap().failover, "rotation is always on");
    }

    #[test]
    fn with_no_added_account_failover_is_off() {
        let (_dir, reg) = registry();
        assert!(!reg.failover_enabled());
    }

    #[test]
    fn the_login_row_reads_the_five_hour_and_weekly_windows_over_100_kept() {
        use crate::claude_quota::{ClaudeQuota, ClaudeQuotaWindow, QuotaSource};
        let w = |id: &str, used: f64| ClaudeQuotaWindow {
            id: id.into(),
            label: id.into(),
            used_percent: used,
            resets_at: Some("2026-10-21T20:00:00Z".into()),
            source: QuotaSource::Stream,
            observed_at: "2026-10-21T18:30:00Z".into(),
        };
        let quota = ClaudeQuota {
            source: QuotaSource::Stream,
            observed_at: "2026-10-21T18:30:00Z".into(),
            windows: vec![
                w("seven_day_opus", 40.0),
                w("five_hour", 105.0),
                w("seven_day", 11.0),
            ],
        };
        let view = login_usage_view(&quota);
        assert_eq!(view.five_hour.unwrap().used_percent, 105.0);
        assert_eq!(view.seven_day.unwrap().used_percent, 11.0);
        assert_eq!(view.observed_at, "2026-10-21T18:30:00Z");
        // Outside the GUI nothing has been seen: no login row data.
        let (_dir, reg) = registry();
        assert_eq!(reg.view().login_usage, None);
    }

    #[test]
    fn views_never_carry_a_token() {
        let (_dir, reg) = registry();
        reg.add("Second", TOKEN_A).unwrap();
        let wire = serde_json::to_string(&reg.view()).unwrap();
        assert!(!wire.contains("sk-ant"));
        // Only added accounts are listed; the login never is.
        let view = reg.view();
        assert_eq!(view.accounts.len(), 1);
        assert!(view.accounts.iter().all(|a| a.id != LOGIN_ID));
    }

    // ── A2: two Hollows on the same accounts ──

    /// One Hollow of a pair: its registry, the other's slot, and the
    /// accounts (the same two logins on both Macs: same tokens, same
    /// organisations, so the same shared keys).
    struct Twin {
        _dir: tempfile::TempDir,
        reg: Arc<Registry>,
        a: String,
        b: String,
    }

    type Slot = Arc<Mutex<Option<Arc<Registry>>>>;

    /// The other Hollow's `GET /v1/machine/usage`, through the wire shape
    /// and the client's reading of it, as `peer_usage::cached` holds it.
    fn peer_of(slot: Slot, machine_id: &'static str) -> PeerSource {
        Box::new(move |now| {
            let Some(other) = slot.lock().unwrap().clone() else {
                return Vec::new();
            };
            let wire = crate::phone_access::MachineUsage {
                usage_api: crate::phone_access::USAGE_API,
                machine_id: machine_id.into(),
                observed_at: crate::peer_usage::rfc3339(now),
                accounts: other.usage_snapshot(None, None, now),
            };
            let json = serde_json::to_string(&wire).unwrap();
            vec![serde_json::from_str(&json).unwrap()]
        })
    }

    fn twin(peer: Slot, peer_id: &'static str, now: DateTime<Utc>) -> Twin {
        let dir = tempfile::Builder::new()
            .prefix("accounts-")
            .tempdir()
            .unwrap();
        let reg = Registry::new(
            dir.path().to_path_buf(),
            Box::<MemSecrets>::default(),
            "http://127.0.0.1:1",
        )
        .with_peers(peer_of(peer, peer_id));
        let a = reg.add("Work", TOKEN_A).unwrap();
        let b = reg.add("Second", TOKEN_B).unwrap();
        reg.set_routing(Routing::Spread, None, Some(90)).unwrap();
        reg.note_org(&a, Some("1f0e8d3c-0000-4000-8000-00000000000a".into()));
        reg.note_org(&b, Some("1f0e8d3c-0000-4000-8000-00000000000b".into()));
        let later = now.timestamp() + 3600;
        reg.observe(&a, usage(10.0, 5.0, later, now));
        reg.observe(&b, usage(20.0, 5.0, later, now));
        Twin {
            _dir: dir,
            reg: Arc::new(reg),
            a,
            b,
        }
    }

    fn pick_on(twin: &Twin, now: DateTime<Utc>) -> String {
        let mut cmd = Command::new("claude");
        twin.reg.apply(&mut cmd, None, None, now);
        account_of(&cmd).unwrap()
    }

    #[test]
    fn two_hollows_keep_real_headroom_ahead_of_recent_pick_bias() {
        let now = at(1_790_000_000);
        let (slot_1, slot_2): (Slot, Slot) = Default::default();
        let mac = twin(slot_2.clone(), "m-0000000000000002", now);
        let mini = twin(slot_1.clone(), "m-0000000000000001", now);
        *slot_1.lock().unwrap() = Some(mac.reg.clone());
        *slot_2.lock().unwrap() = Some(mini.reg.clone());
        // Both measure A as the least loaded; alone, each would pick it.
        assert_eq!(
            pick(
                &mac.reg.candidates(&mac.reg.store().unwrap(), now),
                90.0,
                None
            ),
            Some(mac.a.clone())
        );

        let first = pick_on(&mac, now);
        assert_eq!(first, mac.a);
        // A has more real room; a peer pick only breaks equal-load ties.
        let second = pick_on(&mini, now + chrono::Duration::seconds(5));
        assert_eq!(second, mini.a, "headroom wins over recent picks");
        // B is now the mini's: the MacBook's next new run stays off it.
        assert_eq!(pick_on(&mac, now + chrono::Duration::seconds(10)), mac.a);

        // With B out of allowance (measured on the MacBook only), A is the
        // one eligible account, so the mini uses it despite the pick.
        let blocked_at = now + chrono::Duration::seconds(20);
        mac.reg.observe(
            &mac.b,
            usage(100.0, 5.0, now.timestamp() + 3600, blocked_at),
        );
        let snapshot = mac.reg.usage_snapshot(None, None, blocked_at);
        let b_wire = snapshot.iter().find(|u| u.label == "Second").unwrap();
        assert!(b_wire.blocked && b_wire.blocked_until.is_some());
        assert_eq!(pick_on(&mini, blocked_at), mini.a);

        // Ten minutes on, the picks are old: each Hollow is on its own again.
        let later = now + chrono::Duration::minutes(11);
        mac.reg
            .observe(&mac.b, usage(20.0, 5.0, now.timestamp() + 3600, later));
        mini.reg
            .observe(&mini.b, usage(20.0, 5.0, now.timestamp() + 3600, later));
        assert_eq!(pick_on(&mini, later), mini.a);
    }

    #[test]
    fn a_conversations_next_turns_are_not_picks_so_the_other_macs_sticky_chat_stays() {
        let now = at(1_790_000_000);
        let (slot_1, slot_2): (Slot, Slot) = Default::default();
        let mac = twin(slot_2.clone(), "m-0000000000000002", now);
        let mini = twin(slot_1.clone(), "m-0000000000000001", now);
        *slot_1.lock().unwrap() = Some(mac.reg.clone());
        *slot_2.lock().unwrap() = Some(mini.reg.clone());
        let turn = |twin: &Twin, chat: &str, when: DateTime<Utc>| {
            let mut cmd = Command::new("claude");
            twin.reg.apply(&mut cmd, Some(chat), None, when);
            account_of(&cmd).unwrap()
        };
        let mins = |m: i64| now + chrono::Duration::minutes(m);
        assert_eq!(turn(&mac, "chat:s-1", now), mac.a);
        assert_eq!(turn(&mac, "chat:s-1", mins(1)), mac.a);
        assert_eq!(turn(&mac, "chat:s-1", mins(2)), mac.a);
        assert_eq!(
            mac.reg.recent_picks(&mac.a, mins(2)).0,
            1,
            "one conversation, one pick"
        );
        // Its first pick is old now; the chat goes on, and that turn is no
        // pick, so the mini's new chat may take A (were every turn a pick,
        // A would look busy and the mini would go to B).
        assert_eq!(turn(&mac, "chat:s-1", mins(11)), mac.a);
        assert_eq!(mac.reg.recent_picks(&mac.a, mins(11)).0, 0);
        assert_eq!(turn(&mini, "chat:s-2", mins(12)), mini.a);
        assert_eq!(turn(&mini, "chat:s-2", mins(13)), mini.a, "it stays");
        // With no account added, a login conversation counts once too.
        let (_dir, solo) = registry();
        for m in 0..3 {
            solo.apply(&mut Command::new("claude"), Some("chat:s-3"), None, mins(m));
        }
        assert_eq!(solo.recent_picks(LOGIN_ID, mins(3)).0, 1);
    }

    #[test]
    fn the_usage_snapshot_names_accounts_by_shared_key_only() {
        let now = at(1_790_000_000);
        let (_dir, reg) = registry();
        let a = reg.add("Work", TOKEN_A).unwrap();
        let b = reg.add("No key yet", TOKEN_B).unwrap();
        reg.note_org(&a, Some("ORG-1f0e8d3c".into()));
        reg.observe(&a, usage(42.5, 12.0, now.timestamp() + 3600, now));
        let mut cmd = Command::new("claude");
        reg.apply(&mut cmd, None, Some(&a), now);
        let snapshot = reg.usage_snapshot(None, None, now);
        assert_eq!(snapshot.len(), 1, "an account with no key is left out");
        let one = &snapshot[0];
        assert_eq!(
            one.key,
            crate::peer_usage::shared_key("claude-org", "ORG-1f0e8d3c")
        );
        assert_eq!((one.provider, one.label.as_str()), ("claude", "Work"));
        assert_eq!(one.load_percent, Some(42.5));
        assert_eq!(one.recent_picks, 1);
        let text = serde_json::to_string(&snapshot).unwrap();
        for secret in [TOKEN_A, TOKEN_B, a.as_str(), b.as_str()] {
            assert!(!text.contains(secret), "{secret} crossed");
        }
        // With an account usable, the login is not what runs use here.
        let login = Some(("1f0e8d3c-aaaa".to_string(), Some("Acme".to_string())));
        assert_eq!(reg.usage_snapshot(login.clone(), None, now).len(), 1);
        // The mini's case: nothing added, the login is the account.
        let (_dir2, mini) = registry();
        let mut cmd = Command::new("claude");
        mini.apply(&mut cmd, None, None, now);
        assert_eq!(
            account_of(&cmd),
            None,
            "no account added: the command is untouched"
        );
        let only = mini.usage_snapshot(login, None, now);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].label, "Claude Code login");
        assert_eq!(only[0].billing.as_deref(), Some("Acme"));
        assert_eq!(only[0].recent_picks, 1);
        assert_eq!(
            only[0].key,
            crate::peer_usage::shared_key("claude-org", "1f0e8d3c-aaaa")
        );
    }

    /// A store that says a token is there, as a 1Password-backed one does
    /// without running `op`, and counts every real read.
    struct PresenceOnly(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl SecretStore for PresenceOnly {
        fn get(&self, _id: &str) -> Result<Option<Secret>, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(None)
        }
        fn present(&self, _id: &str) -> Result<bool, String> {
            Ok(true)
        }
        fn set(&self, _id: &str, _token: &Secret) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn the_accounts_view_and_routing_ask_for_presence_never_for_the_token() {
        let gets = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::new(
            dir.path().to_path_buf(),
            Box::new(PresenceOnly(gets.clone())),
            "http://127.0.0.1:1",
        );
        reg.add("Tech", TOKEN_A).unwrap();
        // Adding caches the typed token; start as after a relaunch.
        reg.lock().tokens.clear();
        let view = reg.view();
        assert_eq!(view.accounts.len(), 1);
        assert_ne!(view.accounts[0].status, AccountStatus::MissingToken);
        assert!(view.next_account.is_some(), "routing sees it as usable");
        assert_eq!(
            gets.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no read, so no `op` run, to draw a status line"
        );
    }
}
