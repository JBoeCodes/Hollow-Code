//! Claude plan usage for the Settings, Usage card (Paperclip PR 8c, gate
//! D2b). Display only: nothing here is a budget, and no decision reads the
//! card's snapshot. The stream parser ([`parse_rate_limit_event`]) is shared:
//! `claude_accounts` turns each event into an added account's usage, which
//! R1 failover and the mid-turn ceiling switch decide on.
//!
//! **Two sources, stream first.**
//!
//! 1. The primary source is the last `rate_limit_event` Hollow's own Claude
//!    runs printed. Claude Code 2.1.280 emits one near the start of every
//!    `stream-json` run (all four recorded fixtures carry exactly one), and
//!    again whenever the state it reads from an API response's headers
//!    changes (2.1.286's bundle, read 2026-10-09: "emitted when rate limit
//!    info changes", plus at most every 30 s while `rejected`), with
//!    `rate_limit_info.unifiedWindows.{five_hour,seven_day}.{utilization,
//!    resetsAt}`. `utilization` is a **fraction** (0-1) that exceeds 1 while
//!    the account runs on overage (the fixture has `1.01` with
//!    `isUsingOverage: true`), and `resetsAt` is epoch seconds. Reading it
//!    needs no Keychain access and no request. `agent.rs` turns the line into
//!    an [`StreamQuota`] via [`parse_rate_limit_event`], and every Claude path
//!    hands it to [`observe_stream`]. A top-level stdout line comes from the
//!    CLI itself (tool output is nested inside `tool_result` content), so PR
//!    5's forged-state threat model does not apply; and since the value is
//!    display-only, a wrong one misleads a meter, never a run.
//! 2. The refresh source is Claude Code's own usage endpoint, `GET
//!    https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer
//!    <claude.ai OAuth access token>` and `anthropic-beta: oauth-2025-04-20`.
//!    Confirmed by **reading** the installed bundles (no process run, no
//!    request): 2.1.280, and again 2.1.283 (the version `~/.local/bin/claude`
//!    pointed at on 2026-09-25). Both carry the `plain` path
//!    `/api/oauth/usage` (a GET with a 5 s timeout), `BASE_API_URL`
//!    `https://api.anthropic.com`, and the claude.ai-OAuth header builder
//!    that sets exactly those two headers. Its utilization is already a
//!    **percentage** (0-100; the CLI multiplies header fractions by 100 to
//!    build the same shape), and it adds per-model weekly windows. It runs
//!    only when the user asks: the first time the card is shown with nothing
//!    current, or on Refresh (`force`), at most once per
//!    [`MIN_FETCH_INTERVAL_SECS`]. The CLI's own call may refresh its OAuth
//!    token on a 401; Hollow's never does (see below).
//!
//! **A window past its `resetsAt` is not shown.** Its number describes the
//! window that just ended (the CLI drops such windows from its own
//! `rate_limit_event` too). When nothing current is left, the next show
//! counts as "nothing observed" and may ask the endpoint once.
//!
//! The token comes from the Keychain item Claude Code keeps
//! (`Claude Code-credentials[-<sha256(dir)[..8]>]`, account `$USER`), read
//! with `/usr/bin/security find-generic-password -a <acct> -w -s <service>`,
//! the same command the CLI uses, through `process_group::run_to_completion`
//! (GUI-owned, settled). **Hollow never refreshes Claude Code's OAuth token**:
//! an expired token hides the card. Refreshing would rotate the refresh token
//! under the CLI's feet.
//!
//! **Hide on any failure (D2b, strict).** Any Keychain or endpoint failure,
//! including "not found", a denied prompt, a timeout, an HTTP error or an
//! unexpected body, hides the card and sets `disabled` for the rest of the
//! process: the snapshot is cleared, stream events are ignored and nothing is
//! retried, so it can never prompt twice. The only retry is restarting
//! Hollow. An alternate-provider environment (API key, Bedrock, Vertex,
//! Foundry, a custom base URL or auth token) hides it too, and so does PR 6's
//! billing probe classifying the account as API. The Keychain is read only
//! when that probe found positive subscription evidence; an unknown account
//! sees stream observations only.
//!
//! **GUI only.** Nothing works until [`register_gui`] runs (lib.rs setup).
//! The MCP subcommands dispatch in `main.rs` before that, so a team-mcp
//! delegate's `run_collect` observing a stream is a no-op and [`get`] can
//! never read the Keychain from a subprocess (the `agent.rs` source-lint also
//! bans `claude_quota::` in the four MCP files).
//!
//! **Memory only.** No vault file, no cache file, no notification: PR 5's
//! host-state attribution (`run_guard`) has nothing to attribute, which is
//! what makes calling [`observe_stream`] from inside a tracked chat, workflow
//! or heartbeat run safe. Locks are leaves outside the crate's lock order: a
//! `std::sync::Mutex` held only to copy the snapshot in or out (never across
//! an `.await`, never with another crate lock), two `AtomicBool`s, and a
//! `tokio::sync::Mutex<()>` that makes the Keychain read → fetch → parse
//! single-flight.
//!
//! **Hygiene.** The token lives in a [`Credential`] whose `Debug` prints
//! nothing and whose `Drop` zeroes its bytes; the `security` stdout, any hex
//! scratch buffer and the `Bearer` header bytes are zeroed after use. The
//! parse is typed (`claudeAiOauth.{accessToken, expiresAt}`, unknown fields
//! ignored), so no JSON tree holds the refresh token. The claim is "not logged
//! or persisted; best-effort zeroing": serde, reqwest and the pipe reader keep
//! short-lived copies this module cannot reach. Error text goes through
//! `claude_runner::redact` before it is logged.

// The Keychain read and the endpoint fetch exist only on macOS; elsewhere the
// stream is the only source and their helpers are unused.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The endpoint's origin and path, as the 2.1.280 bundle builds them.
const USAGE_BASE: &str = "https://api.anthropic.com";
const USAGE_PATH: &str = "/api/oauth/usage";
/// The `anthropic-beta` value the CLI sends with a claude.ai OAuth bearer.
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("Hollow/", env!("CARGO_PKG_VERSION"));
/// The CLI's own timeouts: 2 s for the Keychain read, 5 s for the request.
const KEYCHAIN_LIMIT: Duration = Duration::from_secs(2);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// A body larger than this is not a usage answer.
const MAX_BODY: usize = 256 * 1024;
/// Refresh (and the first fetch) run at most this often.
pub const MIN_FETCH_INTERVAL_SECS: i64 = 60;
/// A token that expires within this margin counts as expired.
const EXPIRY_MARGIN_MS: i64 = 60_000;
/// `security` exits 44 when the item does not exist.
#[cfg(target_os = "macos")]
const SECURITY_NOT_FOUND: i32 = 44;

const SERVICE_BASE: &str = "Claude Code-credentials";
const FALLBACK_ACCOUNT: &str = "claude-code-user";

pub const FIVE_HOUR: &str = "five_hour";
pub const SEVEN_DAY: &str = "seven_day";
pub const SEVEN_DAY_OPUS: &str = "seven_day_opus";
pub const SEVEN_DAY_SONNET: &str = "seven_day_sonnet";
/// Prefix of a per-model weekly row from the endpoint's `limits[]`.
pub const WEEKLY_SCOPED_PREFIX: &str = "weekly_scoped:";
/// The endpoint windows the card shows, in display order.
const ENDPOINT_WINDOWS: [&str; 4] = [FIVE_HOUR, SEVEN_DAY, SEVEN_DAY_OPUS, SEVEN_DAY_SONNET];

/// An API key in Hollow's environment: Claude bills the key, not a plan.
/// PR 6's `usage::claude_env::alt_provider` covers the other alternate
/// providers (Bedrock, Vertex, Foundry, an auth token, a base URL) and leaves
/// the key to its own probe, so it is checked here beside it.
const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

// ── Wire types ──────────────────────────────────────────────────────────────

/// Where a window's numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaSource {
    /// A `rate_limit_event` in a Claude run's stdout.
    Stream,
    /// Claude Code's usage endpoint.
    Endpoint,
}

/// One usage window as the card renders it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeQuotaWindow {
    /// `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet` or
    /// `weekly_scoped:<model>`.
    pub id: String,
    pub label: String,
    /// Share of the window used, in percent. Can exceed 100 on overage; the
    /// card clamps only the meter fill, never the number.
    pub used_percent: f64,
    /// RFC 3339 UTC.
    pub resets_at: Option<String>,
    pub source: QuotaSource,
    /// When Hollow saw this value (RFC 3339 UTC), so the card can show its age.
    pub observed_at: String,
}

/// The card's payload. `None` from [`get`] hides the card.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeQuota {
    /// Source of the most recently observed window.
    pub source: QuotaSource,
    /// The most recent observation (RFC 3339 UTC).
    pub observed_at: String,
    pub windows: Vec<ClaudeQuotaWindow>,
}

/// A window value before it is stamped with a source and a time.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
}

// ── Stream (`rate_limit_event`) ─────────────────────────────────────────────

/// One `unifiedWindows` entry, as printed: a 0-1 fraction and epoch seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamWindow {
    pub utilization: f64,
    pub resets_at: Option<i64>,
}

/// What a `rate_limit_event` says about the plan windows. Carried by
/// `AgentEvent::RateLimit`, hence `Debug + Clone`.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamQuota {
    pub five_hour: Option<StreamWindow>,
    pub seven_day: Option<StreamWindow>,
    /// `allowed` | `allowed_warning` | `rejected`. Parsed for completeness
    /// and pinned by the fixture tests; the card does not show it yet.
    pub status: Option<String>,
    pub using_overage: Option<bool>,
}

impl StreamQuota {
    /// The windows in percent (the stream prints fractions).
    pub fn readings(&self) -> Vec<Reading> {
        [(FIVE_HOUR, self.five_hour), (SEVEN_DAY, self.seven_day)]
            .into_iter()
            .filter_map(|(id, window)| {
                let window = window?;
                Some(Reading {
                    id: id.to_string(),
                    label: label_for(id),
                    used_percent: round2(window.utilization * 100.0),
                    resets_at: window.resets_at.and_then(epoch_to_rfc3339),
                })
            })
            .collect()
    }
}

/// Parse a Claude `rate_limit_event` line (already decoded). `None` when it
/// carries neither the five-hour nor the seven-day window. Unknown windows
/// (`seven_day_overage_included`, future ones) are ignored.
pub fn parse_rate_limit_event(evt: &Value) -> Option<StreamQuota> {
    let info = evt.get("rate_limit_info")?;
    let windows = info.get("unifiedWindows")?;
    let read = |id: &str| -> Option<StreamWindow> {
        let window = windows.get(id)?;
        let utilization = window.get("utilization")?.as_f64()?;
        if !utilization.is_finite() || utilization < 0.0 {
            return None;
        }
        Some(StreamWindow {
            utilization,
            resets_at: window.get("resetsAt").and_then(epoch_secs),
        })
    };
    let quota = StreamQuota {
        five_hour: read(FIVE_HOUR),
        seven_day: read(SEVEN_DAY),
        status: info
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_owned),
        using_overage: info.get("isUsingOverage").and_then(Value::as_bool),
    };
    (quota.five_hour.is_some() || quota.seven_day.is_some()).then_some(quota)
}

// ── Endpoint response ───────────────────────────────────────────────────────

/// Map a usage-endpoint body to readings. The shape follows the 2.1.280
/// bundle's schema: nullish top-level windows `{utilization, resets_at}` with
/// utilization already in percent, plus `limits[]` rows `{kind, percent,
/// resets_at, scope.model.display_name}` of which `kind == "weekly_scoped"`
/// become per-model weekly rows. `None` for a non-object, `{}`, or a body
/// whose windows are all null: the CLI treats a fieldless body as an in-band
/// error.
pub fn parse_usage(body: &Value) -> Option<Vec<Reading>> {
    let object = body.as_object()?;
    let mut out: Vec<Reading> = Vec::new();
    for id in ENDPOINT_WINDOWS {
        let Some(window) = object.get(id).filter(|w| w.is_object()) else {
            continue;
        };
        let Some(percent) = window.get("utilization").and_then(percent_value) else {
            continue;
        };
        out.push(Reading {
            id: id.to_string(),
            label: label_for(id),
            used_percent: percent,
            resets_at: window.get("resets_at").and_then(resets_value),
        });
    }
    if let Some(limits) = object.get("limits").and_then(Value::as_array) {
        for row in limits {
            if row.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
                continue;
            }
            let Some(percent) = row.get("percent").and_then(percent_value) else {
                continue;
            };
            let Some(model) = row
                .get("scope")
                .and_then(|s| s.get("model"))
                .and_then(|m| m.get("display_name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let model: String = model.chars().take(40).collect();
            let id = format!("{WEEKLY_SCOPED_PREFIX}{model}");
            if out.iter().any(|r| r.id == id) {
                continue;
            }
            out.push(Reading {
                label: format!("Weekly · {model}"),
                id,
                used_percent: percent,
                resets_at: row.get("resets_at").and_then(resets_value),
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

fn percent_value(v: &Value) -> Option<f64> {
    let n = v.as_f64()?;
    (n.is_finite() && n >= 0.0).then(|| round2(n))
}

/// `resets_at` is an ISO string on the windows; the CLI also accepts epoch
/// seconds in one projection, so take a number too.
fn resets_value(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|d| {
            d.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        }),
        Value::Number(_) => epoch_secs(v).and_then(epoch_to_rfc3339),
        _ => None,
    }
}

fn epoch_secs(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
}

fn epoch_to_rfc3339(secs: i64) -> Option<String> {
    if secs <= 0 {
        return None;
    }
    // A millisecond value would land tens of thousands of years out.
    let secs = if secs > 100_000_000_000 {
        secs / 1000
    } else {
        secs
    };
    DateTime::<Utc>::from_timestamp(secs, 0).map(rfc3339)
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Two decimals, so `1.01 × 100` reads back as exactly `101`.
fn round2(n: f64) -> f64 {
    (n * 100.0).round() / 100.0
}

fn label_for(id: &str) -> String {
    match id {
        FIVE_HOUR => "5-hour session",
        SEVEN_DAY => "Weekly · all models",
        SEVEN_DAY_OPUS => "Weekly · Opus",
        SEVEN_DAY_SONNET => "Weekly · Sonnet",
        other => other,
    }
    .to_string()
}

fn window_order(id: &str) -> usize {
    ENDPOINT_WINDOWS
        .iter()
        .position(|w| *w == id)
        .unwrap_or(ENDPOINT_WINDOWS.len())
}

// ── Environment, Keychain naming ────────────────────────────────────────────

/// True when the environment says Claude uses something other than a
/// claude.ai plan login, so plan usage does not apply. The same evidence PR
/// 6's billing probe uses (`usage::claude_env`), plus an API key.
pub(crate) fn hidden_by_env(get: &dyn Fn(&str) -> Option<String>) -> bool {
    crate::usage::claude_env::alt_provider(get)
        || get(API_KEY_ENV).is_some_and(|v| !v.trim().is_empty())
}

/// The Keychain service Claude Code stores its login under (2.1.280 `RD`):
/// `Claude Code-credentials`, suffixed with `-` and the first 8 hex digits of
/// sha256(dir) when `CLAUDE_SECURESTORAGE_CONFIG_DIR` is non-empty (dir = it)
/// or, when that is unset, `CLAUDE_CONFIG_DIR` is non-empty. An empty
/// `CLAUDE_SECURESTORAGE_CONFIG_DIR` forces the unsuffixed name.
///
/// `None` when `CLAUDE_CODE_CUSTOM_OAUTH_URL` is set (the CLI then uses a
/// `-custom-oauth` item for a different login) or the dir is not ASCII (the
/// CLI NFC-normalizes it first). Known limitation: with only
/// `CLAUDE_CONFIG_DIR`, the CLI hashes its resolved config dir, which may
/// differ from the raw value (a `~` or relative form); a mismatch reads as
/// "not found" and hides the card.
pub(crate) fn keychain_service(get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if get("CLAUDE_CODE_CUSTOM_OAUTH_URL").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let dir = match get("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        Some(dir) => dir,
        None => get(crate::usage::claude_env::CONFIG_DIR_ENV).unwrap_or_default(),
    };
    if dir.is_empty() {
        return Some(SERVICE_BASE.to_string());
    }
    if !dir.is_ascii() {
        return None;
    }
    let digest = Sha256::digest(dir.as_bytes());
    let prefix: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    Some(format!("{SERVICE_BASE}-{prefix}"))
}

/// The Keychain account, as the CLI picks it (2.1.280 `tA`): `$USER` when
/// non-empty, else the OS login name, kept only if it matches
/// `^[a-zA-Z0-9._-]+$`, else `claude-code-user`.
pub(crate) fn keychain_account(
    get: &dyn Fn(&str) -> Option<String>,
    os_user: &dyn Fn() -> Option<String>,
) -> String {
    let candidate = get("USER").filter(|u| !u.is_empty()).or_else(os_user);
    match candidate {
        Some(name) if valid_account(&name) => name,
        _ => FALLBACK_ACCOUNT.to_string(),
    }
}

fn valid_account(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The login name for the current uid (`getpwuid_r`, thread-safe).
#[cfg(target_os = "macos")]
fn os_login_name() -> Option<String> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer refers to a live local of the right type and
    // `buf.len()` is its real length; on success `pw_name` points into `buf`,
    // which outlives the copy below.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: checked non-null above; getpwuid_r NUL-terminates it in `buf`.
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
    name.to_str().ok().map(str::to_owned)
}

// ── Credential ──────────────────────────────────────────────────────────────

/// Overwrite a buffer so the bytes do not linger in freed memory. Volatile
/// writes plus a fence keep the compiler from eliding them.
pub(crate) fn zero(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
}

/// A claude.ai OAuth access token. Never printed, zeroed on drop.
pub(crate) struct Credential {
    token: Vec<u8>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

impl Drop for Credential {
    fn drop(&mut self) {
        zero(&mut self.token);
    }
}

#[derive(Deserialize)]
struct Stored {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<StoredOauth>,
}

#[derive(Deserialize)]
struct StoredOauth {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// Epoch milliseconds.
    #[serde(rename = "expiresAt")]
    expires_at: Option<f64>,
}

fn decode_hex(s: &[u8]) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) || !s.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    Some(
        s.chunks_exact(2)
            .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
            .collect(),
    )
}

/// Parse `security -w` output (the JSON blob, or its hex form when macOS
/// prints the password as hex). Requires an `sk-ant-oat` access token that
/// is not expired or about to expire. Zeroes `stdout` and any hex scratch
/// buffer before returning, whatever the outcome.
pub(crate) fn parse_credential(stdout: &mut [u8], now_ms: i64) -> Option<Credential> {
    let parsed = {
        let trimmed = stdout.trim_ascii();
        let mut scratch = decode_hex(trimmed);
        let stored = match scratch.as_deref() {
            Some(bytes) => serde_json::from_slice::<Stored>(bytes),
            None => serde_json::from_slice::<Stored>(trimmed),
        };
        if let Some(buf) = scratch.as_mut() {
            zero(buf);
        }
        stored.ok().and_then(|s| s.oauth)
    };
    zero(stdout);
    accept_oauth(parsed?, now_ms)
}

fn accept_oauth(oauth: StoredOauth, now_ms: i64) -> Option<Credential> {
    // Owned by a Credential first, so every rejection below zeroes it.
    let credential = Credential {
        token: oauth.access_token.into_bytes(),
    };
    let expires_at = oauth.expires_at?;
    if !expires_at.is_finite() || expires_at <= now_ms.saturating_add(EXPIRY_MARGIN_MS) as f64 {
        return None;
    }
    if !credential.token.starts_with(b"sk-ant-oat")
        || !credential.token.iter().all(u8::is_ascii_graphic)
    {
        return None;
    }
    Some(credential)
}

/// Read Claude Code's login from the Keychain. Exit 44 is "not found"; any
/// failure is an `Err` that the caller turns into "hidden for the session".
/// `hollow serve` never reads it (M1 PR 3): a headless Mac has no login
/// Keychain to ask, so the card is simply hidden there.
#[cfg(target_os = "macos")]
async fn read_credential(service: &str, account: &str, now_ms: i64) -> Result<Credential, String> {
    if crate::serve::serving() {
        return Err(SERVE_NO_KEYCHAIN.to_string());
    }
    let mut cmd = tokio::process::Command::new("/usr/bin/security");
    cmd.args(["find-generic-password", "-a", account, "-w", "-s", service]);
    let mut out = crate::process_group::run_to_completion(&mut cmd, None, KEYCHAIN_LIMIT, None)
        .await
        .map_err(|e| format!("could not run security: {e}"))?;
    if out.timed_out {
        zero(&mut out.stdout);
        return Err("the Keychain read timed out".to_string());
    }
    match out.status.and_then(|s| s.code()) {
        Some(0) => {}
        Some(SECURITY_NOT_FOUND) => {
            zero(&mut out.stdout);
            return Err("Claude Code's Keychain item was not found".to_string());
        }
        other => {
            zero(&mut out.stdout);
            return Err(format!(
                "security exited {other:?}: {}",
                crate::claude_runner::truncate(out.stderr.trim(), 200)
            ));
        }
    }
    parse_credential(&mut out.stdout, now_ms)
        .ok_or_else(|| "Claude Code's stored login is expired or unreadable".to_string())
}

/// Why `hollow serve` reads no Claude Code login from the Keychain.
pub(crate) const SERVE_NO_KEYCHAIN: &str =
    "hollow serve reads no Keychain; Claude Code's login stays with the CLI";

// ── HTTP ────────────────────────────────────────────────────────────────────

/// HTTPS only. Tests may point at a loopback `http://127.0.0.1:<port>`.
fn base_allowed(base: &str) -> bool {
    if base.starts_with("https://") {
        return true;
    }
    #[cfg(test)]
    {
        if let Some(port) = base.strip_prefix("http://127.0.0.1:") {
            return !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        }
    }
    false
}

fn bearer_header(credential: &Credential) -> Result<reqwest::header::HeaderValue, String> {
    let mut raw = Vec::with_capacity(7 + credential.token.len());
    raw.extend_from_slice(b"Bearer ");
    raw.extend_from_slice(&credential.token);
    let value = reqwest::header::HeaderValue::from_bytes(&raw);
    zero(&mut raw);
    let mut value =
        value.map_err(|_| "the stored access token is not a valid header value".to_string())?;
    value.set_sensitive(true);
    Ok(value)
}

/// `GET {base}/api/oauth/usage`. Redirects are never followed; anything but
/// a 200 with a recognizable body is an `Err`. The error text never contains
/// the token (it is only ever in the sensitive header).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) async fn fetch_usage(
    base: &str,
    credential: &Credential,
    timeout: Duration,
) -> Result<Vec<Reading>, String> {
    use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT as UA};
    if !base_allowed(base) {
        return Err("refusing a usage endpoint that is not HTTPS".to_string());
    }
    let url = format!("{}{USAGE_PATH}", base.trim_end_matches('/'));
    let auth = bearer_header(credential)?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|e| format!("could not build the HTTP client: {e}"))?;
    let response = http
        .get(&url)
        .header(AUTHORIZATION, auth)
        .header("anthropic-beta", OAUTH_BETA)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(UA, USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("usage request failed: {e}"))?;
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(format!("usage endpoint returned HTTP {}", status.as_u16()));
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("usage response could not be read: {e}"))?;
    if body.len() > MAX_BODY {
        return Err("usage response was too large".to_string());
    }
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| "usage response was not JSON".to_string())?;
    parse_usage(&value).ok_or_else(|| "usage response listed no windows".to_string())
}

/// Keychain read → fetch, with names resolved before the first await.
#[cfg(target_os = "macos")]
async fn fetch_via_keychain(
    target: Option<(String, String)>,
    now_ms: i64,
) -> Result<Vec<Reading>, String> {
    let (service, account) = target
        .ok_or_else(|| "Claude Code's login is in a custom or non-ASCII location".to_string())?;
    let credential = read_credential(&service, &account, now_ms).await?;
    fetch_usage(USAGE_BASE, &credential, FETCH_TIMEOUT).await
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Held {
    reading: Reading,
    source: QuotaSource,
    observed: DateTime<Utc>,
}

impl Held {
    /// The window this value describes has reset, so the number is stale.
    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.reading
            .resets_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|resets| resets.with_timezone(&Utc) <= now)
    }
}

#[derive(Debug)]
struct Snapshot {
    held: Vec<Held>,
    /// When the endpoint was last asked (success or not).
    last_fetch: Option<DateTime<Utc>>,
}

impl Snapshot {
    const fn new() -> Self {
        Snapshot {
            held: Vec::new(),
            last_fetch: None,
        }
    }

    /// Replace windows per id. A stream event only carries the five-hour and
    /// seven-day windows, so it leaves endpoint-only rows (Opus, Sonnet,
    /// per-model) alone. A successful endpoint answer is authoritative for
    /// the endpoint rows: one it no longer lists is dropped.
    fn merge(&mut self, readings: Vec<Reading>, source: QuotaSource, observed: DateTime<Utc>) {
        if source == QuotaSource::Endpoint {
            self.held.retain(|held| {
                held.source == QuotaSource::Stream
                    || readings.iter().any(|r| r.id == held.reading.id)
            });
        }
        for reading in readings {
            let fresh = Held {
                reading,
                source,
                observed,
            };
            match self
                .held
                .iter()
                .position(|held| held.reading.id == fresh.reading.id)
            {
                Some(index) => self.held[index] = fresh,
                None => self.held.push(fresh),
            }
        }
        self.held.sort_by(|a, b| {
            window_order(&a.reading.id)
                .cmp(&window_order(&b.reading.id))
                .then_with(|| a.reading.id.cmp(&b.reading.id))
        });
    }

    /// What the card shows at `now`: every held window that has not reset.
    fn view(&self, now: DateTime<Utc>) -> Option<ClaudeQuota> {
        let live: Vec<&Held> = self.held.iter().filter(|held| !held.expired(now)).collect();
        let newest = live.iter().copied().max_by_key(|held| held.observed)?;
        Some(ClaudeQuota {
            source: newest.source,
            observed_at: rfc3339(newest.observed),
            windows: live
                .iter()
                .map(|held| ClaudeQuotaWindow {
                    id: held.reading.id.clone(),
                    label: held.reading.label.clone(),
                    used_percent: held.reading.used_percent,
                    resets_at: held.reading.resets_at.clone(),
                    source: held.source,
                    observed_at: rfc3339(held.observed),
                })
                .collect(),
        })
    }
}

/// Process state. One static instance in production; tests build their own
/// so they never share it.
pub(crate) struct QuotaState {
    registered: AtomicBool,
    disabled: AtomicBool,
    snapshot: Mutex<Snapshot>,
    /// Single-flight gate for Keychain read → fetch → parse.
    fetch_gate: tokio::sync::Mutex<()>,
}

impl QuotaState {
    const fn new() -> Self {
        QuotaState {
            registered: AtomicBool::new(false),
            disabled: AtomicBool::new(false),
            snapshot: Mutex::new(Snapshot::new()),
            fetch_gate: tokio::sync::Mutex::const_new(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn register(&self) {
        self.registered.store(true, Ordering::SeqCst);
    }

    fn is_live(&self) -> bool {
        self.registered.load(Ordering::SeqCst) && !self.disabled.load(Ordering::SeqCst)
    }

    fn observe(&self, quota: &StreamQuota, now: DateTime<Utc>) {
        if !self.is_live() {
            return;
        }
        let readings = quota.readings();
        if readings.is_empty() {
            return;
        }
        let mut snapshot = self.lock();
        // Re-checked under the lock: `disable` sets the flag, then clears.
        if self.disabled.load(Ordering::SeqCst) {
            return;
        }
        snapshot.merge(readings, QuotaSource::Stream, now);
    }

    fn view(&self, now: DateTime<Utc>) -> Option<ClaudeQuota> {
        self.lock().view(now)
    }

    /// Hide the card for the rest of the process.
    fn disable(&self, reason: &str) {
        self.disabled.store(true, Ordering::SeqCst);
        self.lock().held.clear();
        eprintln!(
            "[claude-quota] plan usage hidden until Hollow restarts: {}",
            crate::claude_runner::redact(reason)
        );
    }
}

static STATE: QuotaState = QuotaState::new();

/// Mark this process as the GUI. Called once from lib.rs setup; until then
/// [`observe_stream`] and [`get`] do nothing.
pub fn register_gui() {
    STATE.register();
}

/// Record a `rate_limit_event` from one of Hollow's own Claude runs. Cheap
/// and non-blocking beyond a copy under a leaf lock; safe inside
/// `process_group::drive` callbacks while a run holds the vault lease.
pub fn observe_stream(quota: &StreamQuota) {
    STATE.observe(quota, Utc::now());
}

/// The login's windows as last observed (its runs' streams or the card's
/// fetch), without fetching: `None` outside the GUI or before any reading.
/// The login override's long-work check reads it (`claude_accounts`).
pub fn login_view() -> Option<ClaudeQuota> {
    if !STATE.is_live() {
        return None;
    }
    STATE.view(Utc::now())
}

/// Claude Code's claude.ai login as a sensitive `Authorization` header, read
/// from the Keychain exactly as the plan card reads it. GUI only; the token
/// never leaves the header. The connectors library uses it to list the
/// user's claude.ai connectors and check whether each is signed in.
pub(crate) async fn login_bearer() -> Result<reqwest::header::HeaderValue, String> {
    if !STATE.registered.load(Ordering::SeqCst) {
        return Err("claude.ai sign-in is only read by the Hollow app".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        return Err("claude.ai connectors are only available on macOS".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let env = |key: &str| std::env::var(key).ok();
        let service = keychain_service(&env).ok_or_else(|| {
            "Claude Code's login is in a custom or non-ASCII location".to_string()
        })?;
        let account = keychain_account(&env, &os_login_name);
        let credential = read_credential(&service, &account, Utc::now().timestamp_millis()).await?;
        bearer_header(&credential)
    }
}

/// An HTTP client for Anthropic calls made with the login: no redirects,
/// Hollow's user agent, the given timeout.
pub(crate) fn login_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("could not build the HTTP client: {e}"))
}

/// `GET https://api.anthropic.com{path}` with the login (see [`login_bearer`]).
pub(crate) async fn get_with_login(path: &str, beta: &str) -> Result<Value, String> {
    use reqwest::header::{ACCEPT, AUTHORIZATION};
    let auth = login_bearer().await?;
    let response = login_client(FETCH_TIMEOUT)?
        .get(format!("{USAGE_BASE}{path}"))
        .header(AUTHORIZATION, auth)
        .header("anthropic-beta", beta)
        .header("anthropic-version", "2023-06-01")
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("claude.ai request failed: {e}"))?;
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(format!("claude.ai returned HTTP {}", status.as_u16()));
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("claude.ai response could not be read: {e}"))?;
    if body.len() > 4 * MAX_BODY {
        return Err("claude.ai response was too large".to_string());
    }
    serde_json::from_slice(&body).map_err(|_| "claude.ai response was not JSON".to_string())
}

/// The card's data, or `None` to hide it. `force` is the Refresh button.
///
/// PR 6's billing probe decides what applies: an API-key account hides the
/// card outright, and only positive subscription evidence
/// (`oauthAccount.billingType` in Claude Code's account file) permits the
/// Keychain read, so an API or unknown user is never prompted. An unknown
/// account still shows stream observations from Hollow's own runs.
pub async fn get(force: bool) -> Option<ClaudeQuota> {
    use crate::usage::Billing;
    // Unregistered (an MCP subprocess, a test): nothing, not even the probe.
    if !STATE.is_live() {
        return None;
    }
    let env = |key: &str| std::env::var(key).ok();
    let billing = crate::usage::classify_process_env(crate::agent::AgentKind::Claude, true);
    let hidden = hidden_by_env(&env) || billing == Billing::Api;
    let now = Utc::now();
    // Names are resolved here, before any await; the reader runs only if
    // `get_in` decides to fetch.
    #[cfg(target_os = "macos")]
    let fetch = (billing == Billing::Subscription).then(|| {
        let target =
            keychain_service(&env).map(|service| (service, keychain_account(&env, &os_login_name)));
        let now_ms = now.timestamp_millis();
        move || fetch_via_keychain(target, now_ms)
    });
    #[cfg(not(target_os = "macos"))]
    let fetch = {
        let _ = billing;
        None::<fn() -> std::future::Ready<Result<Vec<Reading>, String>>>
    };
    get_in(&STATE, force, hidden, now, fetch).await
}

/// The decision behind [`get`], with the state, environment verdict, clock
/// and endpoint reader injected. `fetch` is `None` where the endpoint is
/// unavailable (off macOS).
///
/// - Hidden (unregistered, disabled, or an alternate provider): `None`.
/// - Something current (observed, not yet reset) and not forced: that, with
///   no Keychain read.
/// - Otherwise (nothing current, or Refresh): one endpoint fetch under the
///   single-flight gate, at most once per [`MIN_FETCH_INTERVAL_SECS`]; a
///   caller that finds a fetch just happened gets what is held.
/// - Any fetch failure, or an answer with no windows: disabled, `None`.
async fn get_in<F, Fut>(
    state: &QuotaState,
    force: bool,
    hidden: bool,
    now: DateTime<Utc>,
    fetch: Option<F>,
) -> Option<ClaudeQuota>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<Reading>, String>>,
{
    if hidden || !state.is_live() {
        return None;
    }
    let current = state.view(now);
    let Some(fetch) = fetch else {
        return current;
    };
    if current.is_some() && !force {
        return current;
    }
    let _gate = state.fetch_gate.lock().await;
    if !state.is_live() {
        return None;
    }
    let throttled = {
        let mut snapshot = state.lock();
        let recent = snapshot.last_fetch.is_some_and(|last| {
            now.signed_duration_since(last) < chrono::Duration::seconds(MIN_FETCH_INTERVAL_SECS)
        });
        if !recent {
            snapshot.last_fetch = Some(now);
        }
        recent
    };
    if throttled {
        return state.view(now);
    }
    match fetch().await {
        Ok(readings) if !readings.is_empty() => {
            state.lock().merge(readings, QuotaSource::Endpoint, now);
            state.view(now)
        }
        Ok(_) => {
            state.disable("the usage endpoint listed no windows");
            None
        }
        Err(error) => {
            state.disable(&error);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    const ONESHOT: &str = include_str!("../fixtures/usage/claude-oneshot.jsonl");

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    fn reading(id: &str, percent: f64) -> Reading {
        Reading {
            id: id.to_string(),
            label: label_for(id),
            used_percent: percent,
            resets_at: None,
        }
    }

    fn fixture_quota() -> StreamQuota {
        let line = ONESHOT
            .lines()
            .find(|l| l.contains("\"rate_limit_event\""))
            .expect("the recorded run carries a rate_limit_event");
        parse_rate_limit_event(&serde_json::from_str(line).unwrap()).unwrap()
    }

    type Ready = std::future::Ready<Result<Vec<Reading>, String>>;

    /// An endpoint reader that counts its calls.
    fn reader(
        calls: &Arc<AtomicUsize>,
        result: Result<Vec<Reading>, String>,
    ) -> impl FnOnce() -> Ready {
        let calls = Arc::clone(calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(result)
        }
    }

    fn registered() -> QuotaState {
        let state = QuotaState::new();
        state.register();
        state
    }

    // ── Stream ──

    #[test]
    fn recorded_rate_limit_event_yields_both_windows() {
        let quota = fixture_quota();
        assert_eq!(
            quota.five_hour,
            Some(StreamWindow {
                utilization: 1.01,
                resets_at: Some(1_790_374_200)
            })
        );
        assert_eq!(
            quota.seven_day,
            Some(StreamWindow {
                utilization: 0.3,
                resets_at: Some(1_790_665_200)
            })
        );
        assert_eq!(quota.status.as_deref(), Some("rejected"));
        assert_eq!(quota.using_overage, Some(true));
        // Exactly one line of the recorded run is a quota observation.
        let parsed = ONESHOT
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| parse_rate_limit_event(v).is_some())
            .count();
        assert_eq!(parsed, 1);
    }

    #[test]
    fn stream_is_a_fraction_and_the_endpoint_is_a_percentage() {
        let readings = fixture_quota().readings();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].id, FIVE_HOUR);
        assert_eq!(readings[0].used_percent, 101.0, "1.01 is 101 %, not 1.01 %");
        assert_eq!(
            readings[0].resets_at.as_deref(),
            Some("2026-09-25T22:10:00Z")
        );
        assert_eq!(readings[1].id, SEVEN_DAY);
        assert_eq!(readings[1].used_percent, 30.0);
        assert_eq!(
            readings[1].resets_at.as_deref(),
            Some("2026-09-29T07:00:00Z")
        );

        let endpoint = parse_usage(&json!({
            "five_hour": {"utilization": 0.5, "resets_at": null},
            "seven_day": {"utilization": 76, "resets_at": null}
        }))
        .unwrap();
        assert_eq!(
            endpoint[0].used_percent, 0.5,
            "0.5 from the endpoint is 0.5 %"
        );
        assert_eq!(endpoint[1].used_percent, 76.0);
    }

    #[test]
    fn rate_limit_event_needs_a_known_window_under_rate_limit_info() {
        // Top-level unifiedWindows is not where the CLI puts them.
        assert!(parse_rate_limit_event(&json!({
            "type": "rate_limit_event",
            "unifiedWindows": {"five_hour": {"utilization": 0.5}}
        }))
        .is_none());
        // Only unknown windows, or unusable values: nothing to show.
        assert!(parse_rate_limit_event(&json!({
            "type": "rate_limit_event",
            "rate_limit_info": {"unifiedWindows": {
                "seven_day_overage_included": {"utilization": 0.2, "resetsAt": 1},
                "five_hour": {"utilization": -1},
                "seven_day": {"utilization": "high"}
            }}
        }))
        .is_none());
        // One usable window is enough; unknown ones are ignored.
        let quota = parse_rate_limit_event(&json!({
            "type": "rate_limit_event",
            "rate_limit_info": {"status": "allowed", "unifiedWindows": {
                "seven_day_overage_included": {"utilization": 0.2},
                "seven_day": {"utilization": 0.4}
            }}
        }))
        .unwrap();
        assert!(quota.five_hour.is_none());
        assert_eq!(quota.seven_day.unwrap().resets_at, None);
        assert_eq!(quota.readings().len(), 1);
    }

    // ── Environment and Keychain naming ──

    #[test]
    fn keychain_service_matches_claude_codes_naming() {
        let service = |pairs: &[(&str, &str)]| keychain_service(&env_of(pairs));
        assert_eq!(service(&[]).as_deref(), Some("Claude Code-credentials"));
        assert_eq!(
            service(&[("CLAUDE_CONFIG_DIR", "/x")]).as_deref(),
            Some("Claude Code-credentials-b3d1db31")
        );
        assert_eq!(
            service(&[("CLAUDE_CONFIG_DIR", "")]).as_deref(),
            Some("Claude Code-credentials")
        );
        // An empty secure-storage dir forces the unsuffixed name.
        assert_eq!(
            service(&[
                ("CLAUDE_SECURESTORAGE_CONFIG_DIR", ""),
                ("CLAUDE_CONFIG_DIR", "/x")
            ])
            .as_deref(),
            Some("Claude Code-credentials")
        );
        // A secure-storage dir wins over the config dir.
        assert_eq!(
            service(&[
                ("CLAUDE_SECURESTORAGE_CONFIG_DIR", "/y"),
                ("CLAUDE_CONFIG_DIR", "/x")
            ])
            .as_deref(),
            Some("Claude Code-credentials-3c46b538")
        );
        assert_eq!(service(&[("CLAUDE_CONFIG_DIR", "/Üser/.claude")]), None);
        assert_eq!(
            service(&[("CLAUDE_CODE_CUSTOM_OAUTH_URL", "https://example.test")]),
            None
        );
    }

    #[test]
    fn keychain_account_mirrors_the_cli() {
        let account = |pairs: &[(&str, &str)], os: Option<&str>| {
            let os = os.map(str::to_owned);
            keychain_account(&env_of(pairs), &move || os.clone())
        };
        assert_eq!(account(&[("USER", "dev")], Some("other")), "dev");
        assert_eq!(account(&[], Some("dev")), "dev");
        // Empty USER falls through to the OS user, as `||` does in the CLI.
        assert_eq!(account(&[("USER", "")], Some("dev")), "dev");
        assert_eq!(account(&[("USER", "bad name")], None), "claude-code-user");
        assert_eq!(
            account(&[("USER", "bad name")], Some("dev")),
            "claude-code-user"
        );
        assert_eq!(account(&[], None), "claude-code-user");
        assert_eq!(account(&[], Some("dotted.user-1_x")), "dotted.user-1_x");
    }

    #[test]
    fn alternate_providers_hide_plan_usage() {
        assert!(!hidden_by_env(&env_of(&[])));
        assert!(!hidden_by_env(&env_of(&[("CLAUDE_CODE_USE_BEDROCK", "0")])));
        assert!(!hidden_by_env(&env_of(&[("ANTHROPIC_API_KEY", "  ")])));
        for pairs in [
            [("CLAUDE_CODE_USE_BEDROCK", "1")],
            [("CLAUDE_CODE_USE_VERTEX", "true")],
            [("CLAUDE_CODE_USE_FOUNDRY", "yes")],
            [("ANTHROPIC_API_KEY", "key")],
            [("ANTHROPIC_AUTH_TOKEN", "token")],
            [("ANTHROPIC_BASE_URL", "https://proxy.test")],
        ] {
            assert!(hidden_by_env(&env_of(&pairs)), "{pairs:?}");
        }
    }

    // ── Credential ──

    const NOW_MS: i64 = 1_790_000_000_000;
    const TOKEN: &str = "sk-ant-oat01-ACCESSabcdefghijklmnop";

    fn blob(token: &str, expires_at: Value) -> Vec<u8> {
        let mut bytes = json!({
            "claudeAiOauth": {
                "accessToken": token,
                "refreshToken": "sk-ant-ort01-REFRESHabcdefghijklmnop",
                "expiresAt": expires_at,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "max"
            },
            "mcpOAuth": {"server": {"accessToken": "other"}}
        })
        .to_string()
        .into_bytes();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn credential_parsing_rejects_expired_or_foreign_tokens_and_zeroes_its_input() {
        let future = json!(NOW_MS + 3_600_000);

        let mut stdout = blob(TOKEN, future.clone());
        let credential = parse_credential(&mut stdout, NOW_MS).expect("a valid login parses");
        assert_eq!(credential.token, TOKEN.as_bytes());
        assert!(
            stdout.iter().all(|&b| b == 0),
            "stdout is zeroed after parsing"
        );
        let debug = format!("{credential:?}");
        assert_eq!(debug, "Credential(<redacted>)");
        assert!(!debug.contains("sk-ant"));

        // Hex output (how `security -w` prints non-printable data).
        let hex: String = blob(TOKEN, future.clone())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut stdout = format!("{hex}\n").into_bytes();
        assert!(
            parse_credential(&mut stdout, NOW_MS).is_some(),
            "hex decodes"
        );
        assert!(stdout.iter().all(|&b| b == 0));

        for (mut stdout, why) in [
            (blob(TOKEN, json!(NOW_MS - 1)), "expired"),
            (
                blob(TOKEN, json!(NOW_MS + 30_000)),
                "expires inside the margin",
            ),
            (blob(TOKEN, Value::Null), "no expiry"),
            (
                blob("sk-ant-api03-KEYabcdefghijklmnop", future.clone()),
                "API key",
            ),
            (
                blob("gho_foreignabcdefghijklmnop", future.clone()),
                "foreign prefix",
            ),
            (
                blob("sk-ant-oat01-has space", future.clone()),
                "not a header value",
            ),
            (b"{\"other\":{}}\n".to_vec(), "no claudeAiOauth"),
            (b"not json at all".to_vec(), "not JSON"),
            (Vec::new(), "empty"),
        ] {
            assert!(parse_credential(&mut stdout, NOW_MS).is_none(), "{why}");
            assert!(stdout.iter().all(|&b| b == 0), "{why}: stdout is zeroed");
        }
    }

    // ── Endpoint response ──

    #[test]
    fn usage_response_maps_windows_and_hides_on_empty() {
        // SYNTHETIC: built from the 2.1.280 bundle's response schema. There is
        // no recording of this endpoint and none may be made.
        let body = json!({
            "five_hour": {"utilization": 76.0, "resets_at": "2026-09-25T22:10:00.123456+00:00"},
            "seven_day": {"utilization": 18, "resets_at": "2026-09-29T07:00:00Z"},
            "seven_day_oauth_apps": null,
            "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": null, "resets_at": null},
            "cinder_cove": {"utilization": 99, "resets_at": null},
            "extra_usage": {"is_enabled": true, "monthly_limit": 50, "used_credits": 3, "utilization": 6},
            "limits": [
                {"kind": "weekly_scoped", "group": "weekly", "percent": 40, "resets_at": 1790665200,
                 "severity": "normal", "is_active": true, "scope": {"model": {"display_name": "Fable"}}},
                {"kind": "weekly_scoped", "group": "weekly", "percent": 10, "resets_at": null,
                 "severity": "normal", "is_active": true, "scope": {"surface": {"display_name": "Web"}}},
                {"kind": "session", "group": "session", "percent": 76, "resets_at": null,
                 "severity": "normal", "is_active": true, "scope": null}
            ]
        });
        let readings = parse_usage(&body).unwrap();
        let summary: Vec<(&str, f64, Option<&str>)> = readings
            .iter()
            .map(|r| (r.id.as_str(), r.used_percent, r.resets_at.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (FIVE_HOUR, 76.0, Some("2026-09-25T22:10:00Z")),
                (SEVEN_DAY, 18.0, Some("2026-09-29T07:00:00Z")),
                ("weekly_scoped:Fable", 40.0, Some("2026-09-29T07:00:00Z")),
            ]
        );
        assert_eq!(readings[2].label, "Weekly · Fable");

        assert!(parse_usage(&json!({})).is_none());
        assert!(parse_usage(&json!([])).is_none());
        assert!(parse_usage(&json!("error")).is_none());
        assert!(
            parse_usage(&json!({"five_hour": null, "seven_day": {"utilization": null}})).is_none()
        );
        assert!(parse_usage(&json!({"error": {"type": "authentication_error"}})).is_none());
    }

    // ── HTTP (loopback only) ──

    struct Served {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    /// One scripted response per connection; `None` holds the connection
    /// open without answering (a stall).
    fn serve(responses: Vec<Option<String>>) -> Served {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut raw = Vec::new();
                let mut chunk = [0u8; 2048];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw).into_owned());
                match response {
                    Some(text) => {
                        let _ = stream.write_all(text.as_bytes());
                    }
                    None => std::thread::sleep(Duration::from_millis(1500)),
                }
            }
        });
        Served { base, requests }
    }

    fn http(status: &str, extra_headers: &str, body: &str) -> Option<String> {
        Some(format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.len()
        ))
    }

    fn credential() -> Credential {
        Credential {
            token: TOKEN.as_bytes().to_vec(),
        }
    }

    const TIMEOUT: Duration = Duration::from_millis(400);
    const GOOD: &str = r#"{"five_hour":{"utilization":42,"resets_at":null},"seven_day":null}"#;

    #[tokio::test]
    async fn quota_fetch_hides_on_any_failure_sends_no_redirect_and_never_logs_the_token() {
        // 200: headers as the CLI sends them.
        let served = serve(vec![http("200 OK", "", GOOD)]);
        let readings = fetch_usage(&served.base, &credential(), TIMEOUT)
            .await
            .unwrap();
        assert_eq!(readings, vec![reading(FIVE_HOUR, 42.0)]);
        let request = served.requests.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            request.starts_with("get /api/oauth/usage http/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains(&format!(
            "authorization: bearer {}",
            TOKEN.to_ascii_lowercase()
        )));
        assert!(request.contains("anthropic-beta: oauth-2025-04-20"));
        assert!(request.contains("user-agent: hollow/"));

        let mut errors = Vec::new();
        for (response, why) in [
            (
                http("401 Unauthorized", "", r#"{"error":"sk-ant-oat01-ECHOED"}"#),
                "401",
            ),
            (http("500 Internal Server Error", "", "{}"), "500"),
            (http("200 OK", "", "not json"), "bad JSON"),
            (http("200 OK", "", "{}"), "empty body"),
            (None, "stall"),
        ] {
            let served = serve(vec![response]);
            let error = fetch_usage(&served.base, &credential(), TIMEOUT)
                .await
                .expect_err(why);
            errors.push(error);
        }

        // A redirect is refused, and its target never hears from us.
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let location = format!(
            "Location: http://{}/steal\r\n",
            target.local_addr().unwrap()
        );
        let served = serve(vec![http("302 Found", &location, "")]);
        errors.push(
            fetch_usage(&served.base, &credential(), TIMEOUT)
                .await
                .expect_err("302"),
        );
        assert!(
            matches!(target.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the redirect target was contacted"
        );

        // Plain HTTP anywhere but loopback is refused before any connection.
        for base in [
            "http://example.com",
            "http://localhost:1",
            "ftp://127.0.0.1:1",
        ] {
            errors.push(
                fetch_usage(base, &credential(), TIMEOUT)
                    .await
                    .expect_err(base),
            );
        }

        for error in &errors {
            let logged = crate::claude_runner::redact(error);
            assert!(!logged.contains("sk-ant-"), "{logged}");
            assert!(!error.contains(TOKEN), "{error}");
        }
    }

    // ── State machine ──

    #[tokio::test]
    async fn stream_data_is_primary_and_needs_no_keychain_read() {
        let calls = Arc::new(AtomicUsize::new(0));

        // Before register_gui: stream events are ignored and nothing is read.
        let state = QuotaState::new();
        state.observe(&fixture_quota(), at(1_000));
        let ok = Ok(vec![reading(FIVE_HOUR, 1.0)]);
        assert_eq!(
            get_in(
                &state,
                true,
                false,
                at(1_000),
                Some(reader(&calls, ok.clone()))
            )
            .await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Registered, but an alternate provider: hidden, nothing read.
        let state = registered();
        state.observe(&fixture_quota(), at(1_000));
        assert_eq!(
            get_in(
                &state,
                true,
                true,
                at(1_000),
                Some(reader(&calls, ok.clone()))
            )
            .await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Stream data shows without any Keychain read.
        let shown = get_in(
            &state,
            false,
            false,
            at(1_030),
            Some(reader(&calls, ok.clone())),
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(shown.source, QuotaSource::Stream);
        assert_eq!(shown.observed_at, "1970-01-01T00:16:40Z");
        let percents: Vec<f64> = shown.windows.iter().map(|w| w.used_percent).collect();
        assert_eq!(percents, vec![101.0, 30.0]);
        assert!(shown
            .windows
            .iter()
            .all(|w| w.source == QuotaSource::Stream));

        // Off macOS (no endpoint reader) the stream is the only source.
        let off = registered();
        assert_eq!(
            get_in(&off, true, false, at(1_000), None::<fn() -> Ready>).await,
            None
        );
        off.observe(&fixture_quota(), at(1_000));
        assert!(get_in(&off, true, false, at(1_000), None::<fn() -> Ready>)
            .await
            .is_some());
    }

    #[tokio::test]
    async fn refresh_merges_per_window_and_is_throttled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = registered();
        state.observe(&fixture_quota(), at(1_000));

        // Refresh asks the endpoint once; its rows replace the stream's.
        let endpoint = Ok(vec![
            reading(FIVE_HOUR, 50.0),
            reading(SEVEN_DAY, 20.0),
            reading(SEVEN_DAY_OPUS, 5.0),
        ]);
        let view = get_in(
            &state,
            true,
            false,
            at(2_000),
            Some(reader(&calls, endpoint)),
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(view.source, QuotaSource::Endpoint);
        let ids: Vec<&str> = view.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec![FIVE_HOUR, SEVEN_DAY, SEVEN_DAY_OPUS]);

        // A later stream event updates only its two windows.
        state.observe(&fixture_quota(), at(2_010));
        let view = state.view(at(2_010)).unwrap();
        let rows: Vec<(&str, f64, QuotaSource)> = view
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.used_percent, w.source))
            .collect();
        assert_eq!(
            rows,
            vec![
                (FIVE_HOUR, 101.0, QuotaSource::Stream),
                (SEVEN_DAY, 30.0, QuotaSource::Stream),
                (SEVEN_DAY_OPUS, 5.0, QuotaSource::Endpoint),
            ]
        );
        assert_eq!(view.source, QuotaSource::Stream);

        // A second Refresh inside the interval does not read again.
        let again = Ok(vec![reading(FIVE_HOUR, 1.0)]);
        assert!(
            get_in(&state, true, false, at(2_030), Some(reader(&calls, again)))
                .await
                .is_some()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // After it, a successful answer drops endpoint rows it no longer lists.
        let later = Ok(vec![reading(SEVEN_DAY, 25.0)]);
        let view = get_in(&state, true, false, at(2_061), Some(reader(&calls, later)))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let ids: Vec<&str> = view.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec![FIVE_HOUR, SEVEN_DAY]);
        assert_eq!(view.windows[1].source, QuotaSource::Endpoint);
    }

    #[tokio::test]
    async fn any_failure_hides_the_card_for_the_session_even_with_stream_data() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = registered();
        state.observe(&fixture_quota(), at(1_000));
        assert!(state.view(at(1_000)).is_some());

        let failed = Err("Claude Code's Keychain item was not found".to_string());
        assert_eq!(
            get_in(&state, true, false, at(1_100), Some(reader(&calls, failed))).await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Hidden for the session: no read, and stream data no longer shows it.
        let ok = Ok(vec![reading(FIVE_HOUR, 1.0)]);
        assert_eq!(
            get_in(
                &state,
                true,
                false,
                at(9_000),
                Some(reader(&calls, ok.clone()))
            )
            .await,
            None
        );
        assert_eq!(
            get_in(&state, false, false, at(9_000), Some(reader(&calls, ok))).await,
            None
        );
        state.observe(&fixture_quota(), at(9_001));
        assert_eq!(state.view(at(9_001)), None);
        assert_eq!(
            get_in(&state, false, false, at(9_002), None::<fn() -> Ready>).await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // An answer with no windows is a failure too.
        let empty = registered();
        let none = Ok(Vec::new());
        assert_eq!(
            get_in(&empty, false, false, at(1_000), Some(reader(&calls, none))).await,
            None
        );
        empty.observe(&fixture_quota(), at(1_001));
        assert_eq!(empty.view(at(1_001)), None);
    }

    #[tokio::test]
    async fn concurrent_requests_read_the_keychain_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = registered();
        let slow = |calls: &Arc<AtomicUsize>| {
            let calls = Arc::clone(calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<_, String>(vec![reading(FIVE_HOUR, 12.0)])
                }
            }
        };
        let (a, b) = tokio::join!(
            get_in(&state, true, false, at(1_000), Some(slow(&calls))),
            get_in(&state, true, false, at(1_000), Some(slow(&calls))),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(a, b);
        assert_eq!(a.unwrap().windows[0].used_percent, 12.0);
    }

    #[tokio::test]
    async fn the_process_state_is_inert_until_register_gui() {
        // No test registers the process-wide state, and lib.rs setup does not
        // run under `cargo test`, so this never reaches the Keychain.
        observe_stream(&fixture_quota());
        assert_eq!(get(true).await, None);
        assert_eq!(get(false).await, None);
    }

    #[tokio::test]
    async fn a_window_past_its_reset_is_no_longer_shown() {
        let ids = |view: Option<ClaudeQuota>| -> Vec<String> {
            view.map(|v| v.windows.into_iter().map(|w| w.id).collect())
                .unwrap_or_default()
        };
        let state = registered();
        // Recorded windows reset at 1_790_374_200 (5 h) and 1_790_665_200 (7 d).
        state.observe(&fixture_quota(), at(1_790_000_000));
        assert_eq!(
            ids(state.view(at(1_790_374_199))),
            vec![FIVE_HOUR, SEVEN_DAY]
        );
        assert_eq!(ids(state.view(at(1_790_374_200))), vec![SEVEN_DAY]);
        assert_eq!(state.view(at(1_790_665_200)), None);

        // Nothing current: the next show asks the endpoint once, and the
        // reset stream rows stay out of the view.
        let calls = Arc::new(AtomicUsize::new(0));
        let ok = Ok(vec![reading(FIVE_HOUR, 3.0)]);
        let view = get_in(
            &state,
            false,
            false,
            at(1_790_665_200),
            Some(reader(&calls, ok)),
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(ids(view), vec![FIVE_HOUR]);
    }

    #[test]
    fn every_claude_launch_point_hands_rate_limits_to_the_card() {
        // Each production stream loop feeds PR 6's ledger with
        // `ledger.observe(&evt)`, and the ledger routes `RateLimit` to
        // `claude_accounts::observe_stream`, which credits it to the run's
        // account (the login's reaches this card). A loop that called the
        // card directly would credit another account's usage to the login.
        for (name, source) in [
            ("agent.rs", include_str!("agent.rs")),
            // chat_queue.rs is chat.rs's queued-turn half (B14): scan both.
            (
                "chat.rs + chat_queue.rs",
                concat!(include_str!("chat.rs"), include_str!("chat_queue.rs")),
            ),
            ("workflows.rs", include_str!("workflows.rs")),
            ("ingest.rs", include_str!("ingest.rs")),
        ] {
            assert!(
                source.matches("ledger.observe(&evt)").count() >= 1,
                "{name} no longer feeds the ledger here; update this test"
            );
            assert_eq!(
                source
                    .matches(concat!("claude_quota::", "observe_stream("))
                    .count(),
                0,
                "{name}: rate limits go through the ledger, never straight to the card"
            );
        }
        let ledger = include_str!("usage_ledger.rs");
        assert!(ledger.contains("AgentEvent::RateLimit(quota) = evt"));
        assert!(ledger
            .contains("claude_accounts::observe_stream(self.claude_account.as_deref(), quota)"));
        assert!(ledger.contains("self.claude_account = crate::claude_accounts::account_of(cmd)"));
    }

    #[test]
    fn quota_serializes_camel_case_with_lowercase_sources() {
        let state = registered();
        state.observe(&fixture_quota(), at(1_790_000_000));
        let wire = serde_json::to_value(state.view(at(1_790_000_000)).unwrap()).unwrap();
        assert_eq!(wire["source"], "stream");
        assert_eq!(wire["observedAt"], "2026-09-21T14:13:20Z");
        assert_eq!(wire["windows"][0]["usedPercent"], 101.0);
        assert_eq!(wire["windows"][0]["resetsAt"], "2026-09-25T22:10:00Z");
        assert_eq!(wire["windows"][0]["label"], "5-hour session");
        assert_eq!(wire["windows"][1]["observedAt"], "2026-09-21T14:13:20Z");
    }
}
