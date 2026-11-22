//! Hollow's own Microsoft 365 sign-in, for the built-in Microsoft 365
//! connector (`builtin_connectors.rs`, `m365_mcp.rs`).
//!
//! **Why Hollow signs in itself.** The claude.ai Microsoft 365 connector is
//! signed in per claude.ai account and organization, so a run on another
//! Claude account (or in Codex) can't reach it. This sign-in belongs to
//! Hollow: one Microsoft sign-in serves every agent and every account.
//!
//! **The app.** Register a public-client Azure AD app in your tenant with
//! `http://localhost:53682/callback` as a redirect URI, and supply its client
//! ID and tenant domain via `HOLLOW_M365_CLIENT_ID` and `HOLLOW_M365_TENANT`.
//! The flow is OAuth 2.0 authorization code + PKCE against the tenant
//! authority.
//!
//! **Scopes.** Identity, calendar read and files read only ([`SCOPES`]); with
//! [`Scopes::CalendarOnly`] the files scopes are left out, for a tenant that
//! wants admin approval for them. Hollow has no messaging: no mail or Teams
//! scope is ever requested (mail stays with the claude.ai connector).
//!
//! **Tokens** live in Hollow's one Keychain item (`secret_store`, key
//! [`KEY`]; the retired Calendar-era key `m365` stays retired). Only the GUI
//! process writes them: the store has no cross-process lock, so a connector
//! subprocess that saved would overwrite secrets the GUI changed meanwhile.
//! The GUI refreshes and saves ([`warm`], before attaching the connector and
//! when Settings reads the status); a subprocess refreshes in memory only
//! ([`access_token_readonly`]). Microsoft doesn't revoke a refresh token when
//! it is redeemed, so both can use it. In the GUI every write takes one lock
//! and an auth generation ([`AUTH`]): sign-in and sign-out bump it, and a
//! refresh saves only if the generation it started at is still current, so a
//! sign-out or account switch during the request is never undone.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Placeholder Azure AD client ID; set `HOLLOW_M365_CLIENT_ID` for your tenant.
pub const CLIENT_ID: &str = match option_env!("HOLLOW_M365_CLIENT_ID") {
    Some(id) => id,
    None => "00000000-0000-4000-8000-000000000000",
};
/// Placeholder Microsoft 365 tenant; set `HOLLOW_M365_TENANT` for your tenant.
const TENANT: &str = match option_env!("HOLLOW_M365_TENANT") {
    Some(t) => t,
    None => "example.com",
};
/// The `secret_store` key for this connector's tokens.
pub const KEY: &str = "m365_connector";
/// Fixed: it is the redirect URI registered on the app.
pub const CALLBACK_PORT: u16 = 53682;
const SCOPES: &str = "User.Read Calendars.Read Files.Read.All Sites.Read.All offline_access";
const CALENDAR_SCOPES: &str = "User.Read Calendars.Read offline_access";
/// Refresh this long before the access token expires.
const SLACK_SECS: u64 = 120;
/// How long a sign-in waits for the browser to come back.
const SIGN_IN_WAIT_SECS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scopes {
    /// Calendar and OneDrive / SharePoint files.
    Full,
    /// Calendar only (no files scopes to approve).
    CalendarOnly,
}

impl Scopes {
    fn as_str(self) -> &'static str {
        match self {
            Scopes::Full => SCOPES,
            Scopes::CalendarOnly => CALENDAR_SCOPES,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds since the epoch.
    pub expires_at: u64,
    #[serde(default)]
    pub scope: String,
    /// Who signed in (`userPrincipalName`), for Settings.
    #[serde(default)]
    pub account: String,
    /// Why Microsoft refused the last refresh (`AADSTS50173`, …), kept until
    /// the next sign-in: a refused grant never comes back by itself, so the
    /// connector stops being "ready" (five asks B3: before, a dead grant was
    /// still attached and Settings said it worked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// How a refresh Microsoft refused begins (see [`refused`]).
const NEEDS_SIGN_IN: &str = "Microsoft 365 needs signing in again";
const EXPIRED: &str = "Microsoft 365 sign-in expired";

/// A refresh error that means the grant is over (Microsoft said no, or there
/// is no refresh token), not one worth trying again (network, 5xx).
fn refused(error: &str) -> bool {
    error.starts_with(NEEDS_SIGN_IN) || error.starts_with(EXPIRED)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
}

/// What Settings shows.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub account: Option<String>,
    /// Signed in with the files scopes.
    pub files: bool,
    /// Why the stored sign-in no longer works, if it doesn't.
    pub problem: Option<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn authority(path: &str) -> String {
    format!("https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/{path}")
}

fn callback_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}/callback")
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// True when the granted scope string includes `want` (bare or as a URL).
pub fn has_scope(scopes: &str, want: &str) -> bool {
    scopes
        .split_whitespace()
        .any(|t| t.rsplit('/').next().unwrap_or(t).eq_ignore_ascii_case(want))
}

// ── storage (reads anywhere, writes only in the GUI) ───────────────────────

pub fn load() -> Option<Tokens> {
    Keychain.load()
}

/// Where the GUI keeps the tokens: the Keychain in the app, memory in tests.
trait TokenStore {
    fn load(&self) -> Option<Tokens>;
    fn save(&self, tokens: &Tokens) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

struct Keychain;

impl TokenStore for Keychain {
    fn load(&self) -> Option<Tokens> {
        let raw = crate::secret_store::get(KEY).ok().flatten()?;
        serde_json::from_str(&raw).ok()
    }
    fn save(&self, tokens: &Tokens) -> Result<(), String> {
        let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        crate::secret_store::set(KEY, &raw).map_err(|e| e.to_string())
    }
    fn delete(&self) -> Result<(), String> {
        crate::secret_store::delete(KEY).map_err(|e| e.to_string())
    }
}

/// The GUI's auth generation, and the lock every token write takes. Sign-in,
/// sign-out and account switch (a sign-in as someone else) bump it; a
/// refresh captures it with the tokens it starts from and saves only if it
/// is unchanged, so a refresh that was in flight across one of those can't
/// resurrect the old session (review B).
static AUTH: std::sync::Mutex<u64> = std::sync::Mutex::new(0);

fn auth_lock(auth: &std::sync::Mutex<u64>) -> std::sync::MutexGuard<'_, u64> {
    auth.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The current generation and the tokens stored under it, read together.
fn snapshot(auth: &std::sync::Mutex<u64>, store: &impl TokenStore) -> (u64, Option<Tokens>) {
    let generation = auth_lock(auth);
    (*generation, store.load())
}

/// Sign-in (`Some`) or sign-out (`None`): a new generation, then the write.
fn replace(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    tokens: Option<&Tokens>,
) -> Result<(), String> {
    let mut generation = auth_lock(auth);
    *generation += 1;
    match tokens {
        Some(tokens) => store.save(tokens),
        None => store.delete(),
    }
}

/// Save a refresh that started at generation `captured`, unless the sign-in
/// changed meanwhile. `Ok(false)` when it did and nothing was saved.
fn save_refreshed(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    captured: u64,
    tokens: &Tokens,
) -> Result<bool, String> {
    let generation = auth_lock(auth);
    if *generation != captured {
        return Ok(false);
    }
    store.save(tokens).map(|_| true)
}

/// Save a refusal mark for the sign-in `seen`, unless the sign-in changed
/// meanwhile or another refresh already saved newer tokens: a refusal must
/// never overwrite a refresh that worked (review B3 #2).
fn save_marked(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    captured: u64,
    seen: &Tokens,
    marked: &Tokens,
) -> Result<bool, String> {
    let generation = auth_lock(auth);
    let unchanged = store.load().is_some_and(|now| {
        now.access_token == seen.access_token && now.refresh_token == seen.refresh_token
    });
    if *generation != captured || !unchanged {
        return Ok(false);
    }
    store.save(marked).map(|_| true)
}

pub fn sign_out() -> Result<(), String> {
    crate::agenda::forget_events();
    replace(&AUTH, &Keychain, None)
}

/// Signed in, as far as the stored tokens say (no network), and Microsoft
/// hasn't refused the grant since.
pub fn signed_in() -> bool {
    load().is_some_and(|t| usable(&t))
}

fn usable(t: &Tokens) -> bool {
    t.problem.is_none() && (t.refresh_token.is_some() || t.expires_at > now() + SLACK_SECS)
}

/// Why the stored sign-in no longer works, as last recorded (no network).
pub fn stored_problem() -> Option<String> {
    load().and_then(|t| t.problem)
}

/// The signed-in account (its address), as the stored tokens say (no
/// network, no refresh): meeting prep leaves the person themself out.
pub fn account() -> Option<String> {
    load().map(|t| t.account).filter(|a| !a.trim().is_empty())
}

// ── refresh ────────────────────────────────────────────────────────────────

async fn refresh(tokens: &Tokens) -> Result<Tokens, String> {
    let refresh = tokens.refresh_token.as_deref().ok_or(format!(
        "{EXPIRED}: sign in again in Hollow → Settings → Connectors"
    ))?;
    let scope = if tokens.scope.trim().is_empty() {
        SCOPES
    } else {
        tokens.scope.as_str()
    };
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh),
        ("scope", scope),
    ];
    let res = http()?
        .post(authority("token"))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("couldn't reach Microsoft: {e}"))?;
    if !res.status().is_success() {
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        return Err(if grant_over(status, &body) {
            format!(
                "{NEEDS_SIGN_IN} in Hollow → Settings → Connectors ({})",
                aad_code(&body)
            )
        } else {
            format!(
                "couldn't refresh the Microsoft sign-in (Microsoft answered {status}: {})",
                aad_code(&body)
            )
        });
    }
    let r: TokenResponse = res.json().await.map_err(|e| e.to_string())?;
    Ok(Tokens {
        access_token: r.access_token,
        refresh_token: r.refresh_token.or_else(|| tokens.refresh_token.clone()),
        expires_at: now() + r.expires_in,
        scope: r
            .scope
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| tokens.scope.clone()),
        account: tokens.account.clone(),
        problem: None,
    })
}

/// Whether a failed refresh means the grant is over, so it is remembered
/// until the next sign-in: an OAuth `invalid_grant`, `interaction_required`
/// or `consent_required` on a 400/401 (review B3 #1). Conditional Access by
/// location or device (AADSTS53000/53003), a sign-in loop (AADSTS50196), a
/// proxy's 403/407, 429 and 5xx can clear by themselves and are not.
fn grant_over(status: u16, body: &str) -> bool {
    const PASSING: [&str; 3] = ["AADSTS53000", "AADSTS53003", "AADSTS50196"];
    let error = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_default();
    matches!(status, 400 | 401)
        && matches!(
            error.as_str(),
            "invalid_grant" | "interaction_required" | "consent_required"
        )
        && !PASSING.iter().any(|code| body.contains(code))
}

/// The `AADSTS…` code in a token-endpoint error, or a short excerpt.
fn aad_code(body: &str) -> String {
    body.split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| w.starts_with("AADSTS"))
        .map(str::to_string)
        .unwrap_or_else(|| crate::claude_runner::truncate(body, 120).to_string())
}

/// GUI only: refresh and save when the access token is close to expiry, so
/// connector subprocesses start with a fresh one. A no-op when signed out.
pub async fn warm() -> Result<(), String> {
    warm_in(
        &AUTH,
        &Keychain,
        |tokens| async move { refresh(&tokens).await },
    )
    .await
}

async fn warm_in<F, Fut>(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    refresh: F,
) -> Result<(), String>
where
    F: FnOnce(Tokens) -> Fut,
    Fut: std::future::Future<Output = Result<Tokens, String>>,
{
    warm_with(auth, store, refresh, false).await
}

/// [`warm_in`], or with `retry` (a person's "Check again" in Settings) one
/// more try of a sign-in remembered as refused, so a refusal that was
/// wrong, or has cleared, heals without a new sign-in (review B3 #1).
async fn warm_with<F, Fut>(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    refresh: F,
    retry: bool,
) -> Result<(), String>
where
    F: FnOnce(Tokens) -> Fut,
    Fut: std::future::Future<Output = Result<Tokens, String>>,
{
    let (generation, Some(tokens)) = snapshot(auth, store) else {
        return Ok(());
    };
    match &tokens.problem {
        Some(problem) if !retry => return Err(problem.clone()),
        Some(_) => {}
        None if tokens.expires_at > now() + 10 * 60 => return Ok(()),
        None => {}
    }
    let fresh = match refresh(tokens.clone()).await {
        Ok(fresh) => fresh,
        Err(e) => {
            // Remember a refusal (not a network error) until the next
            // sign-in, unless the sign-in changed meanwhile.
            if refused(&e) {
                let marked = Tokens {
                    problem: Some(e.clone()),
                    ..tokens.clone()
                };
                let _ = save_marked(auth, store, generation, &tokens, &marked);
            }
            return Err(e);
        }
    };
    // A sign-in, sign-out or account switch during the request wins.
    save_refreshed(auth, store, generation, &fresh).map(|_| ())
}

/// For a connector subprocess: a usable access token, refreshed in memory
/// when needed and never saved (see the module docs). `cache` keeps the
/// refreshed one for the life of the process.
pub async fn access_token_readonly(
    cache: &tokio::sync::Mutex<Option<Tokens>>,
) -> Result<Tokens, String> {
    let mut guard = cache.lock().await;
    if let Some(t) = guard.as_ref().filter(|t| t.expires_at > now() + SLACK_SECS) {
        return Ok(t.clone());
    }
    let stored =
        load().ok_or("Microsoft 365 isn't signed in: sign in in Hollow → Settings → Connectors")?;
    if let Some(problem) = stored.problem {
        return Err(problem);
    }
    let fresh = if stored.expires_at > now() + SLACK_SECS {
        stored
    } else {
        refresh(&stored).await?
    };
    *guard = Some(fresh.clone());
    Ok(fresh)
}

/// GUI: the status Settings shows, refreshing (and saving) a stale token.
pub async fn status() -> Status {
    status_in(
        &AUTH,
        &Keychain,
        |tokens| async move { refresh(&tokens).await },
    )
    .await
}

async fn status_in<F, Fut>(
    auth: &std::sync::Mutex<u64>,
    store: &impl TokenStore,
    refresh: F,
) -> Status
where
    F: FnOnce(Tokens) -> Fut,
    Fut: std::future::Future<Output = Result<Tokens, String>>,
{
    let signed_out = Status {
        signed_in: false,
        account: None,
        files: false,
        problem: None,
    };
    if store.load().is_none() {
        return signed_out;
    }
    let problem = warm_with(auth, store, refresh, true).await.err();
    // Read again: a sign-out during the refresh wins. Falling back to the
    // tokens read before it showed a signed-out session as signed in
    // (wave-1 review).
    let Some(tokens) = store.load() else {
        return signed_out;
    };
    Status {
        signed_in: true,
        account: Some(tokens.account.clone()).filter(|a| !a.is_empty()),
        files: has_scope(&tokens.scope, "Files.Read.All"),
        problem,
    }
}

// ── sign-in ────────────────────────────────────────────────────────────────

fn random_string(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The authorize URL, its `state` and the PKCE verifier.
fn authorize_url(scopes: Scopes) -> (String, String, String) {
    let state = random_string(32);
    let verifier = random_string(64);
    let challenge = pkce_challenge(&verifier);
    let callback = callback_uri();
    let url = reqwest::Url::parse_with_params(
        &authority("authorize"),
        [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", callback.as_str()),
            ("scope", scopes.as_str()),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ],
    )
    .map(|u| u.to_string())
    .unwrap_or_default();
    (url, state, verifier)
}

/// `/callback?code=…&state=…` or `?error=…&error_description=…`.
fn parse_callback(target: &str) -> (Option<String>, Option<String>, Option<String>) {
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok();
    let get = |k: &str| {
        url.as_ref().and_then(|u| {
            u.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        })
    };
    let error = match (get("error"), get("error_description")) {
        (Some(e), Some(d)) => Some(format!("{e}: {d}")),
        (e, d) => e.or(d),
    };
    (get("code"), get("state"), error)
}

const DONE_HTML: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title><body style=\"font-family:system-ui;padding:40px\"><h1 style=\"font-weight:500\">Microsoft 365 is connected to Hollow</h1><p>You can close this tab.</p>";

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser's redirect and return the code.
async fn await_code(listener: tokio::net::TcpListener, state: &str) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(SIGN_IN_WAIT_SECS);
    loop {
        let (mut stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| "Microsoft sign-in timed out. If Microsoft asked for admin approval, ask IT to approve the Hollow app's Files.Read.All and Sites.Read.All, or sign in for calendar only.".to_string())?
            .map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("");
        if !target.starts_with("/callback") {
            // A favicon or a stray request: answer and keep waiting.
            reply(&mut stream, "404 Not Found", "").await;
            continue;
        }
        let (code, got_state, error) = parse_callback(target);
        if let Some(error) = error {
            let body = format!("<!doctype html><meta charset=utf-8><body style=\"font-family:system-ui;padding:40px\"><h1>Sign-in failed</h1><p>{}</p>", error.replace('&', "&amp;").replace('<', "&lt;"));
            reply(&mut stream, "400 Bad Request", &body).await;
            return Err(format!("Microsoft sign-in failed: {error}"));
        }
        let Some(code) = code else {
            reply(&mut stream, "400 Bad Request", "No code").await;
            return Err("Microsoft sent no sign-in code".into());
        };
        if got_state.as_deref() != Some(state) {
            reply(&mut stream, "400 Bad Request", "State mismatch").await;
            return Err(
                "Microsoft sign-in was refused: the reply didn't match this sign-in".into(),
            );
        }
        reply(&mut stream, "200 OK", DONE_HTML).await;
        return Ok(code);
    }
}

/// GUI: sign in in the browser and save the tokens. Waits up to five minutes.
pub async fn sign_in(scopes: Scopes) -> Result<Status, String> {
    let (url, state, verifier) = authorize_url(scopes);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| format!("Port {CALLBACK_PORT} is busy, so Microsoft can't return to Hollow ({e}); try again in a moment"))?;
    std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|e| format!("couldn't open the browser: {e}"))?;
    let code = await_code(listener, &state).await?;
    let callback = callback_uri();
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code.as_str()),
        ("redirect_uri", callback.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    let res = http()?
        .post(authority("token"))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("couldn't reach Microsoft: {e}"))?;
    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "Microsoft refused the sign-in ({})",
            aad_code(&body)
        ));
    }
    let r: TokenResponse = res.json().await.map_err(|e| e.to_string())?;
    let account = http()?
        .get("https://graph.microsoft.com/v1.0/me?$select=userPrincipalName,mail")
        .bearer_auth(&r.access_token)
        .send()
        .await
        .ok()
        .and_then(|res| res.status().is_success().then_some(res));
    let account = match account {
        Some(res) => res.json::<serde_json::Value>().await.ok().and_then(|v| {
            v.get("userPrincipalName")
                .or_else(|| v.get("mail"))
                .and_then(|s| s.as_str())
                .map(str::to_string)
        }),
        None => None,
    };
    let tokens = Tokens {
        access_token: r.access_token,
        refresh_token: r.refresh_token,
        expires_at: now() + r.expires_in,
        scope: r.scope.unwrap_or_else(|| scopes.as_str().to_string()),
        account: account.unwrap_or_default(),
        problem: None,
    };
    replace(&AUTH, &Keychain, Some(&tokens))?;
    // Another account's calendar must not be reused.
    crate::agenda::forget_events();
    Ok(Status {
        signed_in: true,
        account: Some(tokens.account.clone()).filter(|a| !a.is_empty()),
        files: has_scope(&tokens.scope, "Files.Read.All"),
        problem: None,
    })
}

// ── IPC ────────────────────────────────────────────────────────────────────

/// Sign in to Microsoft 365 for the built-in connector (opens the browser).
#[tauri::command]
pub async fn m365_sign_in(calendar_only: Option<bool>) -> Result<Status, String> {
    sign_in(if calendar_only.unwrap_or(false) {
        Scopes::CalendarOnly
    } else {
        Scopes::Full
    })
    .await
}

/// Forget the built-in connector's Microsoft sign-in.
#[tauri::command]
pub async fn m365_sign_out() -> Result<(), String> {
    tokio::task::spawn_blocking(sign_out)
        .await
        .map_err(|e| e.to_string())?
}

/// Whether the built-in connector is signed in, and as whom.
#[tauri::command]
pub async fn m365_status() -> Result<Status, String> {
    Ok(status().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here may touch the Keychain (load/save): a real item exists on
    // the developer's Mac and a read would prompt.

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_uses_the_tenant_pkce_and_no_messaging_scope() {
        let (url, state, _) = authorize_url(Scopes::Full);
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("login.microsoftonline.com"));
        assert!(parsed.path().starts_with("/example.com/"), "{url}");
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], CLIENT_ID);
        assert_eq!(q["redirect_uri"], "http://localhost:53682/callback");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], state);
        for scope in [SCOPES, CALENDAR_SCOPES] {
            for banned in ["Mail", "Chat", "Channel", "Team"] {
                assert!(
                    !scope.split_whitespace().any(|s| s.starts_with(banned)),
                    "{scope}"
                );
            }
        }
        let (url, _, _) = authorize_url(Scopes::CalendarOnly);
        assert!(!url.contains("Files.Read.All"));
    }

    #[test]
    fn callback_parsing_and_scope_checks() {
        assert_eq!(
            parse_callback("/callback?code=abc&state=xyz"),
            (Some("abc".into()), Some("xyz".into()), None)
        );
        let (code, _, error) =
            parse_callback("/callback?error=access_denied&error_description=user%20cancelled");
        assert_eq!(
            (code, error.as_deref()),
            (None, Some("access_denied: user cancelled"))
        );
        assert!(has_scope(
            "https://graph.microsoft.com/Files.Read.All User.Read",
            "files.read.all"
        ));
        assert!(!has_scope("Files.ReadWrite.All", "Files.Read.All"));
        assert_eq!(
            aad_code(r#"{"error_description":"AADSTS700082: The refresh token has expired"}"#),
            "AADSTS700082"
        );
    }

    /// In-memory token store, so the refresh race is testable off the Keychain.
    #[derive(Default)]
    struct Memory(std::sync::Mutex<Option<Tokens>>);

    impl TokenStore for Memory {
        fn load(&self) -> Option<Tokens> {
            self.0.lock().unwrap().clone()
        }
        fn save(&self, tokens: &Tokens) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(tokens.clone());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    fn tokens(account: &str, access: &str, expires_at: u64) -> Tokens {
        Tokens {
            access_token: access.into(),
            refresh_token: Some(format!("{account}-refresh")),
            expires_at,
            scope: SCOPES.into(),
            account: account.into(),
            problem: None,
        }
    }

    #[tokio::test]
    async fn a_sign_out_during_a_refresh_saves_no_token() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("old@example.com", "stale", 0))).unwrap();
        warm_in(&auth, &store, |t| {
            // The user signs out while the token request is in flight.
            replace(&auth, &store, None).unwrap();
            async move {
                Ok(Tokens {
                    access_token: "refreshed".into(),
                    expires_at: now() + 3600,
                    ..t
                })
            }
        })
        .await
        .unwrap();
        assert!(
            store.load().is_none(),
            "the refresh resurrected a signed-out session: {:?}",
            store.load()
        );
    }

    #[tokio::test]
    async fn an_account_switch_during_a_refresh_leaves_the_new_account_alone() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("old@example.com", "stale", 0))).unwrap();
        let new = tokens("new@example.com", "new-access", now() + 3600);
        warm_in(&auth, &store, |t| {
            // The user signs in as someone else while the request is in flight.
            replace(&auth, &store, Some(&new)).unwrap();
            async move {
                Ok(Tokens {
                    access_token: "old-refreshed".into(),
                    expires_at: now() + 3600,
                    ..t
                })
            }
        })
        .await
        .unwrap();
        let stored = store.load().expect("the new account's tokens");
        assert_eq!(
            (stored.account.as_str(), stored.access_token.as_str()),
            ("new@example.com", "new-access")
        );
    }

    #[tokio::test]
    async fn a_sign_out_during_status_shows_signed_out() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("old@example.com", "stale", 0))).unwrap();
        let status = status_in(&auth, &store, |t| {
            // The user signs out while status refreshes the stale token.
            replace(&auth, &store, None).unwrap();
            async move {
                Ok(Tokens {
                    access_token: "refreshed".into(),
                    expires_at: now() + 3600,
                    ..t
                })
            }
        })
        .await;
        assert_eq!(
            status,
            Status {
                signed_in: false,
                account: None,
                files: false,
                problem: None,
            }
        );
    }

    #[tokio::test]
    async fn status_reports_the_stored_account_after_a_refresh() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("me@example.com", "stale", 0))).unwrap();
        let status = status_in(&auth, &store, |t| async move {
            Ok(Tokens {
                access_token: "refreshed".into(),
                expires_at: now() + 3600,
                ..t
            })
        })
        .await;
        assert!(status.signed_in && status.problem.is_none(), "{status:?}");
        assert_eq!(status.account.as_deref(), Some("me@example.com"));
        assert_eq!(store.load().unwrap().access_token, "refreshed");
    }

    #[tokio::test]
    async fn an_undisturbed_refresh_is_saved() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("me@example.com", "stale", 0))).unwrap();
        warm_in(&auth, &store, |t| async move {
            Ok(Tokens {
                access_token: "refreshed".into(),
                expires_at: now() + 3600,
                ..t
            })
        })
        .await
        .unwrap();
        assert_eq!(store.load().unwrap().access_token, "refreshed");
    }

    #[tokio::test]
    async fn a_refused_refresh_is_remembered_until_the_next_sign_in() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("me@example.com", "stale", 0))).unwrap();
        let refusal = format!("{NEEDS_SIGN_IN} in Hollow → Settings → Connectors (AADSTS50173)");
        let r = refusal.clone();
        let err = warm_in(&auth, &store, |_| async move { Err(r) })
            .await
            .unwrap_err();
        assert_eq!(err, refusal);
        let stored = store.load().unwrap();
        assert_eq!(stored.problem.as_deref(), Some(refusal.as_str()));
        assert!(!usable(&stored), "a refused grant is not ready");
        // No second request: the refusal is answered from the store.
        let again = warm_in(&auth, &store, |_| async {
            panic!("no request after a refusal")
        })
        .await;
        assert_eq!(again.unwrap_err(), refusal);
        // "Check again" tries once more; still refused, still remembered.
        let r = refusal.clone();
        let status = status_in(&auth, &store, |_| async move { Err(r) }).await;
        assert_eq!(status.problem.as_deref(), Some(refusal.as_str()));
        assert!(store.load().unwrap().problem.is_some());
        // And once Microsoft accepts it again, the mark is gone.
        let status = status_in(&auth, &store, |t| async move {
            Ok(Tokens {
                access_token: "back".into(),
                expires_at: now() + 3600,
                problem: None,
                ..t
            })
        })
        .await;
        assert!(status.problem.is_none());
        assert!(usable(&store.load().unwrap()));
        // A sign-in replaces the tokens and the problem with them.
        replace(
            &auth,
            &store,
            Some(&tokens("me@example.com", "new", now() + 3600)),
        )
        .unwrap();
        assert!(usable(&store.load().unwrap()));
    }

    #[tokio::test]
    async fn a_network_failure_is_not_remembered() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("me@example.com", "stale", 0))).unwrap();
        let err = warm_in(&auth, &store, |_| async {
            Err("couldn't reach Microsoft: timeout".to_string())
        })
        .await
        .unwrap_err();
        assert!(err.contains("couldn't reach"));
        assert!(store.load().unwrap().problem.is_none());
        assert!(refused(&format!("{EXPIRED}: sign in again")));
        assert!(!refused(
            "couldn't refresh the Microsoft sign-in (Microsoft answered 503)"
        ));
    }

    #[test]
    fn only_a_real_oauth_refusal_ends_the_grant() {
        let body = |error: &str, code: &str| {
            format!(r#"{{"error":"{error}","error_description":"{code}: x"}}"#)
        };
        assert!(grant_over(400, &body("invalid_grant", "AADSTS50173")));
        assert!(grant_over(
            400,
            &body("interaction_required", "AADSTS50076")
        ));
        assert!(grant_over(401, &body("consent_required", "AADSTS65001")));
        // Conditional Access by place or device, and a sign-in loop, pass.
        assert!(!grant_over(400, &body("invalid_grant", "AADSTS53003")));
        assert!(!grant_over(
            400,
            &body("interaction_required", "AADSTS53000")
        ));
        assert!(!grant_over(400, &body("invalid_grant", "AADSTS50196")));
        assert!(!grant_over(
            400,
            &body("temporarily_unavailable", "AADSTS1")
        ));
        assert!(!grant_over(403, "Forbidden by proxy"));
        assert!(!grant_over(407, ""));
        assert!(!grant_over(429, &body("invalid_grant", "x")));
        assert!(!grant_over(503, ""));
    }

    #[tokio::test]
    async fn a_refusal_never_overwrites_a_refresh_that_worked() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        let start = tokens("me@example.com", "stale", 0);
        replace(&auth, &store, Some(&start)).unwrap();
        // Another refresh saved newer tokens while this one was refused.
        let newer = Tokens {
            access_token: "newer".into(),
            refresh_token: Some("r2".into()),
            expires_at: now() + 3600,
            ..start.clone()
        };
        let refusal = format!("{NEEDS_SIGN_IN} (AADSTS50173)");
        let r = refusal.clone();
        let saved = &store;
        let n = newer.clone();
        let err = warm_in(&auth, &store, |_| async move {
            saved.save(&n).unwrap();
            Err(r)
        })
        .await
        .unwrap_err();
        assert_eq!(err, refusal);
        let now_stored = store.load().unwrap();
        assert_eq!(now_stored.access_token, "newer");
        assert!(now_stored.problem.is_none());
    }

    #[tokio::test]
    async fn a_refusal_after_a_new_sign_in_is_not_saved() {
        let auth = std::sync::Mutex::new(0);
        let store = Memory::default();
        replace(&auth, &store, Some(&tokens("me@example.com", "stale", 0))).unwrap();
        let a = &auth;
        let st = &store;
        let refusal = format!("{NEEDS_SIGN_IN} (AADSTS50173)");
        let r = refusal.clone();
        let _ = warm_in(&auth, &store, |_| async move {
            replace(
                a,
                st,
                Some(&tokens("other@example.com", "fresh", now() + 3600)),
            )
            .unwrap();
            Err(r)
        })
        .await;
        let stored = store.load().unwrap();
        assert_eq!(stored.account, "other@example.com");
        assert!(stored.problem.is_none());
    }
}
