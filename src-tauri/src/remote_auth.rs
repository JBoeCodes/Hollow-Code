//! Hollow's own sign-in for remote MCP connectors (five asks B2, 2026-10-10).
//!
//! **Why.** A claude.ai connector (Zoom, Wispr Flow, …) is signed in per
//! claude.ai account and organization, so a run that uses one is pinned to
//! Claude Code's login and Codex never gets it (`connectors.rs`). A remote
//! connector in Hollow's library (`Transport::Http`) with `oauth` on is signed
//! in by Hollow instead, once, and then serves every Claude account and Codex
//! through `hollow remote-mcp` (`remote_mcp.rs`), which bridges stdio to the
//! server and adds the token.
//!
//! **The flow** follows the MCP authorization spec (OAuth 2.1): the server's
//! protected-resource metadata (RFC 9728: from its 401's `WWW-Authenticate`
//! `resource_metadata`, else the well-known paths) names its authorization
//! server; that server's metadata (RFC 8414, then OpenID discovery) gives the
//! endpoints; Hollow registers itself (RFC 7591 dynamic client registration)
//! unless the connector names a client id; then authorization code + PKCE
//! (S256) with a loopback redirect, and `resource` (RFC 8707) on every
//! request so the token is only good for this server. A server with no
//! protected-resource metadata gets the 2025-03-26 fallback: its own origin
//! as the authorization server, then `/authorize`, `/token`, `/register`.
//! Every endpoint must be https (http only on loopback, for tests).
//!
//! **Tokens** live in one owner-only file per connector,
//! `<app data>/RemoteAuth/<id>.json`, not in `secret_store`'s Keychain item:
//! OAuth 2.1 rotates a public client's refresh token on use, so the connector
//! subprocess must save what it refreshes, and the Keychain item has no
//! cross-process lock (a subprocess save would overwrite secrets the GUI
//! changed meanwhile; `m365_auth.rs`). Every refresh and sign-in takes
//! `<id>.lock` (`flock`), re-reads the file and refreshes only if the stored
//! token still needs it, so two processes never redeem one refresh token.
//! The token is bound to the URL it was granted for (`resource`): a library
//! edit that points the connector elsewhere reads as signed out instead of
//! sending the token to another server.
//!
//! **Never for messaging** (guardrail: Hollow has no mail, Slack or Teams):
//! [`messaging_host`] refuses such servers, and Anthropic-hosted connector
//! addresses ([`anthropic_hosted`]) only work through Anthropic's proxy.
//! The one exception is a mail connector the person approved
//! ([`approved_mail`]: Superhuman Mail's MCP server, 10-11 B4). It is a
//! connector, not an inbox: no mail sync, no inbox UI, and every tool that
//! sends, drafts or changes mail asks on Hollow's card (`remote_mcp`).
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Refresh this long before the access token expires.
const SLACK_SECS: u64 = 120;
/// How long a sign-in waits for the browser to come back.
pub const SIGN_IN_WAIT: Duration = Duration::from_secs(5 * 60);
/// The loopback port for a connector with its own client id: that app's
/// redirect URI must be `http://127.0.0.1:53683/callback`. A self-registered
/// client gets any free port (it registers the one it got).
pub const FIXED_PORT: u16 = 53683;
const DIR: &str = "RemoteAuth";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds since the epoch; 0 when the server gave no lifetime.
    #[serde(default)]
    pub expires_at: u64,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// `none`, `client_secret_post` or `client_secret_basic`.
    #[serde(default)]
    pub client_auth: String,
    pub token_endpoint: String,
    /// The server URL the grant is for.
    pub resource: String,
    /// The `resource` value the server's metadata asked for (RFC 8707);
    /// empty means `resource`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub audience: String,
    #[serde(default)]
    pub scope: String,
    /// Why the stored sign-in stopped working (a refused refresh), if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// What Settings shows for one remote connector (never a token).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub problem: Option<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

// ── guards ─────────────────────────────────────────────────────────────────

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// Mail servers Alex approved as Hollow connectors (10-11 B4: "I want
/// superhuman to be a hollow connector"), by exact host.
pub const APPROVED_MAIL_HOSTS: [&str; 1] = ["mcp.mail.superhuman.com"];

/// An approved mail connector: Hollow signs in to it, and its bridge asks on
/// Hollow's card before any tool that isn't a read (`remote_mcp`).
pub fn approved_mail(url: &str) -> bool {
    url.trim_start()
        .to_ascii_lowercase()
        .starts_with("https://")
        && host_of(url).is_some_and(|h| APPROVED_MAIL_HOSTS.contains(&h.as_str()))
}

/// A mail, Slack or Teams server: Hollow never signs in to one (guardrail),
/// except an [`approved_mail`] connector.
pub fn messaging_host(url: &str) -> bool {
    const WORDS: [&str; 6] = ["mail", "gmail", "outlook", "slack", "teams", "superhuman"];
    !approved_mail(url)
        && host_of(url).is_some_and(|h| h.split(['.', '-']).any(|label| WORDS.contains(&label)))
}

/// The OpenID scopes an approved mail connector asks for when its server
/// names none: `offline_access` is what earns a refresh token, so the sign-in
/// lasts (Superhuman's resource metadata lists no scopes; its authorization
/// server lists these).
const MAIL_SCOPES: [&str; 4] = ["openid", "profile", "email", "offline_access"];

/// An address only Anthropic's connector proxy can use
/// (`microsoft365.mcp.claude.com`).
pub fn anthropic_hosted(url: &str) -> bool {
    host_of(url).is_some_and(|h| {
        h == "claude.ai"
            || h.ends_with(".claude.ai")
            || h == "claude.com"
            || h.ends_with(".claude.com")
            || h.ends_with(".anthropic.com")
    })
}

fn loopback(u: &reqwest::Url) -> bool {
    matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

/// https, or http on loopback only.
/// https; plain http only to loopback and only in tests and debug builds,
/// so a remote server can't point a release build at a local service
/// (review B2 #7).
fn endpoint_ok(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            || (cfg!(any(test, debug_assertions)) && u.scheme() == "http" && loopback(&u))
    })
}

/// Scheme, host, port and path, without a trailing slash, query or fragment.
fn normal(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    Some(format!(
        "{}://{}:{}{}",
        u.scheme(),
        u.host_str()?.to_ascii_lowercase(),
        u.port_or_known_default()?,
        u.path().trim_end_matches('/')
    ))
}

/// `resource` names `url`'s server: same scheme, host and port, and a path
/// at or above `url`'s (whole segments).
fn covers(resource: &str, url: &str) -> bool {
    let (Ok(r), Ok(u)) = (reqwest::Url::parse(resource), reqwest::Url::parse(url)) else {
        return false;
    };
    let rp = r.path().trim_end_matches('/');
    let up = u.path().trim_end_matches('/');
    r.scheme() == u.scheme()
        && r.host_str().map(str::to_ascii_lowercase) == u.host_str().map(str::to_ascii_lowercase)
        && r.port_or_known_default() == u.port_or_known_default()
        && (rp.is_empty() || up == rp || up.starts_with(&format!("{rp}/")))
}

/// Whether a grant for `resource` may be sent to `url`.
pub fn same_resource(resource: &str, url: &str) -> bool {
    matches!((normal(resource), normal(url)), (Some(a), Some(b)) if a == b)
}

// ── storage ────────────────────────────────────────────────────────────────

/// Connector ids are `[a-z0-9_-]` (`connectors::valid_id`); anything else
/// can't name a file here.
fn safe_id(id: &str) -> Result<&str, String> {
    if !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        Ok(id)
    } else {
        Err(format!("Not a connector id: {id}"))
    }
}

fn token_path(app: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(app.join(DIR).join(format!("{}.json", safe_id(id)?)))
}

pub fn load(app: &Path, id: &str) -> Option<Tokens> {
    let raw = std::fs::read_to_string(token_path(app, id).ok()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save(app: &Path, id: &str, tokens: &Tokens) -> Result<(), String> {
    let dir = app.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let raw = serde_json::to_vec_pretty(tokens).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&token_path(app, id)?, &raw)
}

/// Forget a connector's sign-in (sign-out, or the connector was removed).
/// Callers hold its lock, so a refresh in flight can't write it back.
pub fn delete(app: &Path, id: &str) -> Result<(), String> {
    match std::fs::remove_file(token_path(app, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The token still works for a while (or never expires).
fn fresh(t: &Tokens) -> bool {
    t.expires_at == 0 || t.expires_at > now() + SLACK_SECS
}

/// Signed in for `url`, as far as the stored file says (no network): the
/// grant is for this URL, no refresh was refused, and either the token is
/// fresh or there is a refresh token. Decides whether the connector is
/// attached to a run.
pub fn ready(app: &Path, id: &str, url: &str) -> bool {
    load(app, id).is_some_and(|t| {
        same_resource(&t.resource, url)
            && t.problem.is_none()
            && (fresh(&t) || t.refresh_token.is_some())
    })
}

pub fn status(app: &Path, id: &str, url: &str) -> Status {
    match load(app, id) {
        None => Status {
            signed_in: false,
            problem: None,
        },
        Some(t) if !same_resource(&t.resource, url) => Status {
            signed_in: false,
            problem: Some("Signed in for another address; sign in again for this one.".into()),
        },
        Some(t) if t.problem.is_some() => Status {
            signed_in: false,
            problem: t.problem,
        },
        Some(t) if !fresh(&t) && t.refresh_token.is_none() => Status {
            signed_in: false,
            problem: Some("The sign-in expired; sign in again.".into()),
        },
        Some(_) => Status {
            signed_in: true,
            problem: None,
        },
    }
}

/// An exclusive `flock` on `<id>.lock`, held until the file is dropped.
async fn lock(app: &Path, id: &str) -> Result<std::fs::File, String> {
    let app = app.to_path_buf();
    let id = id.to_string();
    tokio::task::spawn_blocking(move || lock_blocking(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// [`lock`] for synchronous callers (removing a connector).
pub fn lock_blocking(app: &Path, id: &str) -> Result<std::fs::File, String> {
    let dir = app.join(DIR);
    let path = dir.join(format!("{}.lock", safe_id(id)?));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: a valid descriptor this function owns.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!(
                "couldn't lock the sign-in: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(file)
}

// ── tokens ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

fn with_client(
    req: reqwest::RequestBuilder,
    form: &mut Vec<(&'static str, String)>,
    client_id: &str,
    secret: Option<&str>,
    auth: &str,
) -> reqwest::RequestBuilder {
    match (secret, auth) {
        (Some(s), "client_secret_basic") => req.basic_auth(client_id, Some(s)),
        (Some(s), _) => {
            form.push(("client_id", client_id.into()));
            form.push(("client_secret", s.into()));
            req
        }
        (None, _) => {
            form.push(("client_id", client_id.into()));
            req
        }
    }
}

/// The error a token endpoint gave (`error`, then its description), short.
fn token_error(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v.get("error").and_then(Value::as_str).unwrap_or("");
    let what = v
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let text = match (code.is_empty(), what.is_empty()) {
        (false, false) => format!("{code}: {what}"),
        (false, true) => code.to_string(),
        _ => body.chars().take(160).collect(),
    };
    text.chars().take(240).collect()
}

async fn redeem_refresh(http: &reqwest::Client, t: &Tokens) -> Result<Tokens, RefreshError> {
    let Some(refresh) = t.refresh_token.clone() else {
        return Err(RefreshError::Refused(
            "the sign-in expired and has no refresh token".into(),
        ));
    };
    let mut form: Vec<(&'static str, String)> = vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.clone()),
        (
            "resource",
            if t.audience.is_empty() {
                t.resource.clone()
            } else {
                t.audience.clone()
            },
        ),
    ];
    let req = with_client(
        http.post(&t.token_endpoint),
        &mut form,
        &t.client_id,
        t.client_secret.as_deref(),
        &t.client_auth,
    );
    let res = req
        .form(&form)
        .send()
        .await
        .map_err(|e| RefreshError::Unreachable(e.to_string()))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.as_u16() == 400 || status.as_u16() == 401 {
        return Err(RefreshError::Refused(token_error(&body)));
    }
    if !status.is_success() {
        return Err(RefreshError::Unreachable(format!(
            "the token endpoint answered {status}"
        )));
    }
    let r: TokenResponse =
        serde_json::from_str(&body).map_err(|e| RefreshError::Unreachable(e.to_string()))?;
    Ok(Tokens {
        access_token: r.access_token,
        // A server that doesn't rotate keeps the old refresh token valid.
        refresh_token: r.refresh_token.or(Some(refresh)),
        expires_at: r.expires_in.map(|s| now() + s).unwrap_or(0),
        scope: r.scope.unwrap_or_else(|| t.scope.clone()),
        problem: None,
        ..t.clone()
    })
}

enum RefreshError {
    /// The server said no (`invalid_grant`, …): the sign-in is over.
    Refused(String),
    /// Couldn't ask (network, 5xx): try again later, keep the sign-in.
    Unreachable(String),
}

/// An access token for connector `id` at `url`, refreshed (and saved) when
/// it is about to expire, or when `rejected` is the token the server just
/// refused. Under the connector's lock, so the GUI and every connector
/// subprocess share one refresh; a token another process refreshed
/// meanwhile is simply reused.
pub async fn access_token(
    app: &Path,
    id: &str,
    url: &str,
    http: &reqwest::Client,
    rejected: Option<&str>,
) -> Result<String, String> {
    let usable = |t: &Tokens| fresh(t) && rejected.is_none_or(|r| r != t.access_token);
    // Every read is checked the same way: a sign-in for another address
    // (the connector was edited and signed in again meanwhile) is never
    // refreshed or sent here (review B2 #1).
    let read = || -> Result<Tokens, String> {
        let t = load(app, id).ok_or_else(|| {
            "Not signed in: sign in once in Hollow → Settings → Connectors.".to_string()
        })?;
        if !same_resource(&t.resource, url) {
            return Err(
                "This sign-in is for another address: sign in again in Hollow → Settings → Connectors."
                    .into(),
            );
        }
        if let Some(p) = &t.problem {
            return Err(format!(
                "The sign-in stopped working ({p}): sign in again in Hollow → Settings → Connectors."
            ));
        }
        Ok(t)
    };
    let t = read()?;
    if usable(&t) {
        return Ok(t.access_token);
    }
    let _guard = lock(app, id).await?;
    let t = read()?;
    if usable(&t) {
        return Ok(t.access_token);
    }
    match redeem_refresh(http, &t).await {
        Ok(next) => {
            save(app, id, &next)?;
            Ok(next.access_token)
        }
        Err(RefreshError::Refused(why)) => {
            let _ = save(
                app,
                id,
                &Tokens {
                    problem: Some(why.clone()),
                    ..t
                },
            );
            Err(format!("The sign-in stopped working ({why}): sign in again in Hollow → Settings → Connectors."))
        }
        Err(RefreshError::Unreachable(why)) => Err(format!("Couldn't refresh the sign-in: {why}")),
    }
}

// ── discovery ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAuth {
    /// The `resource` value sent with every request: the protected-resource
    /// metadata's own `resource` when it publishes one, else the URL.
    pub audience: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    /// The resource's own `scopes_supported`; empty asks for none.
    pub scopes: Vec<String>,
    pub auth_methods: Vec<String>,
}

/// `resource_metadata="…"` from a `WWW-Authenticate: Bearer …` header.
fn resource_metadata(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &header[at..];
    let value = match rest.strip_prefix('"') {
        Some(q) => &q[..q.find('"')?],
        None => rest.split([',', ' ']).next()?,
    };
    Some(value.to_string()).filter(|v| !v.is_empty())
}

fn origin(u: &reqwest::Url) -> String {
    u.origin().ascii_serialization()
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    if !endpoint_ok(url) {
        return None;
    }
    let res = http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Value>().await.ok().filter(Value::is_object)
}

/// The server's protected-resource metadata: from its 401 challenge, then
/// the RFC 9728 well-known paths (with the URL's path, then without).
async fn protected_resource(http: &reqwest::Client, url: &reqwest::Url) -> Option<Value> {
    let probe = http
        .post(url.as_str())
        .header("Accept", "application/json, text/event-stream")
        .json(
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "Hollow", "version": env!("CARGO_PKG_VERSION")}}}),
        )
        .send()
        .await
        .ok();
    let mut candidates = Vec::new();
    if let Some(res) = probe.filter(|r| r.status().as_u16() == 401) {
        if let Some(m) = res
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(resource_metadata)
        {
            candidates.push(m);
        }
    }
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        candidates.push(format!(
            "{}/.well-known/oauth-protected-resource{path}",
            origin(url)
        ));
    }
    candidates.push(format!(
        "{}/.well-known/oauth-protected-resource",
        origin(url)
    ));
    for c in candidates {
        if let Some(v) = get_json(http, &c).await {
            if v.get("authorization_servers").is_some_and(Value::is_array) {
                return Some(v);
            }
        }
    }
    None
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// RFC 8414 / OpenID discovery for `issuer`, path-inserted when it has one.
async fn authorization_server(http: &reqwest::Client, issuer: &str) -> Option<Value> {
    let u = reqwest::Url::parse(issuer).ok()?;
    let path = u.path().trim_end_matches('/');
    let o = origin(&u);
    let candidates = if path.is_empty() {
        vec![
            format!("{o}/.well-known/oauth-authorization-server"),
            format!("{o}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{o}/.well-known/oauth-authorization-server{path}"),
            format!("{o}/.well-known/openid-configuration{path}"),
            format!("{o}{path}/.well-known/openid-configuration"),
        ]
    };
    for c in candidates {
        if let Some(v) = get_json(http, &c).await {
            if v.get("authorization_endpoint").is_some() && v.get("token_endpoint").is_some() {
                return Some(v);
            }
        }
    }
    None
}

/// How to sign in to the MCP server at `url`.
pub async fn discover(http: &reqwest::Client, url: &str) -> Result<ServerAuth, String> {
    let u = reqwest::Url::parse(url).map_err(|_| "Not a URL".to_string())?;
    if !endpoint_ok(url) {
        return Err("Hollow signs in only over https".into());
    }
    let prm = protected_resource(http, &u).await;
    // RFC 9728 §3.3: the metadata must describe this server. Its `resource`
    // may name the server's origin or a path above the URL, never another.
    let audience = match prm
        .as_ref()
        .and_then(|p| p.get("resource"))
        .and_then(Value::as_str)
    {
        Some(r) if covers(r, url) => r.to_string(),
        Some(_) => return Err("The server's metadata describes another address".into()),
        None => url.to_string(),
    };
    let scopes = prm
        .as_ref()
        .map(|p| strings(p, "scopes_supported"))
        .unwrap_or_default();
    let issuer = prm
        .as_ref()
        .and_then(|p| strings(p, "authorization_servers").into_iter().next())
        .unwrap_or_else(|| origin(&u));
    if !endpoint_ok(&issuer) {
        return Err("The server named an authorization server that isn't https".into());
    }
    let meta = authorization_server(http, &issuer).await;
    // RFC 8414 §3.3: the metadata's issuer is the one asked for.
    if let Some(named) = meta
        .as_ref()
        .and_then(|m| m.get("issuer"))
        .and_then(Value::as_str)
    {
        if normal(named) != normal(&issuer) {
            return Err("The authorization server's metadata names another issuer".into());
        }
    }
    // An approved mail server whose resource names no scopes asks for the
    // standard ones its authorization server offers (a refresh token).
    let scopes = match &meta {
        Some(m) if scopes.is_empty() && approved_mail(url) => strings(m, "scopes_supported")
            .into_iter()
            .filter(|s| MAIL_SCOPES.contains(&s.as_str()))
            .collect(),
        _ => scopes,
    };
    let auth = match meta {
        Some(m) => ServerAuth {
            audience: audience.clone(),
            authorization_endpoint: m["authorization_endpoint"].as_str().unwrap_or("").into(),
            token_endpoint: m["token_endpoint"].as_str().unwrap_or("").into(),
            registration_endpoint: m
                .get("registration_endpoint")
                .and_then(Value::as_str)
                .map(str::to_string),
            scopes,
            auth_methods: strings(&m, "token_endpoint_auth_methods_supported"),
        },
        // The 2025-03-26 fallback: only for a server with no metadata at all.
        None if prm.is_none() => {
            let o = origin(&u);
            ServerAuth {
                audience,
                authorization_endpoint: format!("{o}/authorize"),
                token_endpoint: format!("{o}/token"),
                registration_endpoint: Some(format!("{o}/register")),
                scopes,
                auth_methods: Vec::new(),
            }
        }
        None => return Err("The server's authorization server publishes no metadata".into()),
    };
    let endpoints = [
        Some(auth.authorization_endpoint.as_str()),
        Some(auth.token_endpoint.as_str()),
        auth.registration_endpoint.as_deref(),
    ];
    if endpoints.into_iter().flatten().any(|e| !endpoint_ok(e)) {
        return Err("The server's sign-in endpoints must be https".into());
    }
    Ok(auth)
}

/// Dynamic client registration: the client id (and secret, if the server
/// issued one) and how to present it.
async fn register(
    http: &reqwest::Client,
    endpoint: &str,
    redirect: &str,
) -> Result<(String, Option<String>, String), String> {
    let res = http
        .post(endpoint)
        .json(&json!({
            "client_name": "Hollow",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .map_err(|e| format!("couldn't register Hollow with the server: {e}"))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "the server refused to register Hollow ({})",
            token_error(&body)
        ));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let id = v
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("the server registered Hollow without a client id")?
        .to_string();
    let secret = v
        .get("client_secret")
        .and_then(Value::as_str)
        .map(str::to_string);
    let method = v
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if secret.is_some() {
                "client_secret_basic".into()
            } else {
                "none".into()
            }
        });
    Ok((id, secret, method))
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

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser's redirect to `/callback` and return the code.
async fn await_code(
    listener: tokio::net::TcpListener,
    state: &str,
    name: &str,
    wait: Duration,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let (mut stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| format!("The {name} sign-in timed out; try again."))?
            .map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("")
            .to_string();
        if !target.starts_with("/callback") {
            reply(&mut stream, "404 Not Found", "").await;
            continue;
        }
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok();
        let get = |k: &str| {
            url.as_ref().and_then(|u| {
                u.query_pairs()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.into_owned())
            })
        };
        // The state first: a reply without ours, error or code, comes from
        // someone else (any page can reach the loopback port) and can't end
        // the sign-in (review B2 #6).
        if get("state").as_deref() != Some(state) {
            reply(&mut stream, "400 Bad Request", "State mismatch").await;
            continue;
        }
        if let Some(error) = get("error") {
            let what = get("error_description")
                .map(|d| format!("{error}: {d}"))
                .unwrap_or(error);
            let what: String = what.chars().take(200).collect();
            let body = format!("<!doctype html><meta charset=utf-8><body style=\"font-family:system-ui;padding:40px\"><h1>Sign-in failed</h1><p>{}</p>", escape(&what));
            reply(&mut stream, "400 Bad Request", &body).await;
            return Err(format!("The {name} sign-in failed: {what}"));
        }
        let Some(code) = get("code") else {
            reply(&mut stream, "400 Bad Request", "No code").await;
            return Err(format!("{name} sent no sign-in code"));
        };
        let body = format!("<!doctype html><meta charset=utf-8><title>Signed in</title><body style=\"font-family:system-ui;padding:40px\"><h1 style=\"font-weight:500\">{} is connected to Hollow</h1><p>You can close this tab.</p>", escape(name));
        reply(&mut stream, "200 OK", &body).await;
        return Ok(code);
    }
}

/// One sign-in: which connector, its name for messages, the server URL and
/// the client id of an app the person registered, if the server doesn't let
/// Hollow register itself, and that app's secret when it is a confidential
/// client (Zoom Marketplace apps are; overnight 10-11 Y6).
pub struct SignIn<'a> {
    pub app: &'a Path,
    pub id: &'a str,
    pub name: &'a str,
    pub url: &'a str,
    pub client_id: Option<&'a str>,
    pub client_secret: Option<&'a str>,
    /// Run under the token lock just before the grant is saved: an error
    /// (the connector's credentials or server changed while the person was
    /// in the browser) saves nothing, so a removed secret can't come back
    /// with a late sign-in (B2, #537 review).
    pub still_current: Option<&'a (dyn Fn() -> Result<(), String> + Send + Sync)>,
}

/// How a confidential client presents its secret at the token endpoint:
/// what the server says it takes, `client_secret_basic` first (RFC 6749's
/// default, and Zoom's), else `client_secret_post`.
pub fn secret_method(supported: &[String]) -> &'static str {
    let has = |m: &str| supported.iter().any(|s| s == m);
    if supported.is_empty() || has("client_secret_basic") || !has("client_secret_post") {
        "client_secret_basic"
    } else {
        "client_secret_post"
    }
}

/// Sign in in the browser (`open` gets the authorize URL) and save the
/// tokens. Waits up to `wait` for the browser to come back.
pub async fn sign_in<O>(
    req: SignIn<'_>,
    http: &reqwest::Client,
    open: O,
    wait: Duration,
) -> Result<Status, String>
where
    O: FnOnce(&str) -> Result<(), String>,
{
    safe_id(req.id)?;
    if messaging_host(req.url) {
        return Err("Hollow doesn't connect to mail, Slack or Teams.".into());
    }
    if anthropic_hosted(req.url) {
        return Err(
            "This address only works through claude.ai; Hollow can't sign in to it.".into(),
        );
    }
    let client_id = req.client_id.map(str::trim).filter(|s| !s.is_empty());
    // Tests take any free port: the fixed one may be in use on the machine.
    let fixed = if cfg!(test) { 0 } else { FIXED_PORT };
    let port = if client_id.is_some() { fixed } else { 0 };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("Port {port} is busy, so the sign-in can't return to Hollow ({e}); try again in a moment"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let auth = discover(http, req.url).await?;
    let client_secret = req
        .client_secret
        .map(str::trim)
        .filter(|s| !s.is_empty() && client_id.is_some());
    let (client_id, secret, method) = match client_id {
        Some(id) => match client_secret {
            Some(s) => (
                id.to_string(),
                Some(s.to_string()),
                secret_method(&auth.auth_methods).to_string(),
            ),
            None => (id.to_string(), None, "none".to_string()),
        },
        None => match &auth.registration_endpoint {
            Some(endpoint) => register(http, endpoint, &redirect).await?,
            None => {
                return Err(format!(
                    "{} doesn't let apps register themselves. Register an app with it (redirect http://127.0.0.1:{FIXED_PORT}/callback) and add its client id to the connector.",
                    req.name
                ))
            }
        },
    };
    let state = random_string(32);
    let verifier = random_string(64);
    let challenge = pkce_challenge(&verifier);
    let mut params = vec![
        ("response_type", "code".to_string()),
        ("client_id", client_id.clone()),
        ("redirect_uri", redirect.clone()),
        ("state", state.clone()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256".into()),
        ("resource", auth.audience.clone()),
    ];
    if !auth.scopes.is_empty() {
        params.push(("scope", auth.scopes.join(" ")));
    }
    let authorize = reqwest::Url::parse_with_params(&auth.authorization_endpoint, &params)
        .map_err(|e| e.to_string())?;
    open(authorize.as_str())?;
    let code = await_code(listener, &state, req.name, wait).await?;
    let mut form: Vec<(&'static str, String)> = vec![
        ("grant_type", "authorization_code".into()),
        ("code", code),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
        ("resource", auth.audience.clone()),
    ];
    let request = with_client(
        http.post(&auth.token_endpoint),
        &mut form,
        &client_id,
        secret.as_deref(),
        &method,
    );
    let res = request
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("couldn't reach {}: {e}", req.name))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{} refused the sign-in ({})",
            req.name,
            token_error(&body)
        ));
    }
    let r: TokenResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tokens = Tokens {
        access_token: r.access_token,
        refresh_token: r.refresh_token,
        expires_at: r.expires_in.map(|s| now() + s).unwrap_or(0),
        client_id,
        client_secret: secret,
        client_auth: method,
        token_endpoint: auth.token_endpoint,
        resource: req.url.to_string(),
        audience: auth.audience.clone(),
        scope: r.scope.unwrap_or_else(|| auth.scopes.join(" ")),
        problem: None,
    };
    let _guard = lock(req.app, req.id).await?;
    if let Some(check) = req.still_current {
        check()?;
    }
    save(req.app, req.id, &tokens)?;
    Ok(Status {
        signed_in: true,
        problem: None,
    })
}

/// Sign a connector out (forget its tokens), under its lock.
pub async fn sign_out(app: &Path, id: &str) -> Result<(), String> {
    let _guard = lock(app, id).await?;
    delete(app, id)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A loopback HTTP/1.1 server for tests: one handler, no keep-alive.
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone, Debug)]
    pub struct Req {
        pub method: String,
        /// Path and query.
        pub target: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Req {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
        pub fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }
        /// A form or query value.
        pub fn param(&self, key: &str) -> Option<String> {
            let q = if self.method == "GET" {
                self.target.split_once('?').map(|(_, q)| q).unwrap_or("")
            } else {
                self.body.as_str()
            };
            let u = reqwest::Url::parse(&format!("http://x/?{q}")).ok()?;
            let found = u
                .query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned());
            found
        }
    }

    pub struct Resp {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Resp {
        pub fn json(status: u16, v: serde_json::Value) -> Self {
            Self {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: v.to_string(),
            }
        }
        pub fn empty(status: u16) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: String::new(),
            }
        }
        pub fn with(mut self, k: &str, v: &str) -> Self {
            self.headers.push((k.into(), v.into()));
            self
        }
    }

    /// Serve `handler` on a free loopback port; returns `http://127.0.0.1:<port>`.
    pub async fn serve<F>(handler: F) -> String
    where
        F: Fn(Req, &str) -> Resp + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let handler = Arc::new(handler);
        let b = base.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                let base = b.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let head_end = loop {
                        let n = s.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let mut lines = head.split("\r\n");
                    let first: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                        .collect();
                    let len: usize = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.parse().ok())
                        .unwrap_or(0);
                    while buf.len() < head_end + len {
                        let n = s.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let req = Req {
                        method: first.first().unwrap_or(&"").to_string(),
                        target: first.get(1).unwrap_or(&"").to_string(),
                        headers,
                        body,
                    };
                    let resp = handler(req, &base);
                    let mut out = format!("HTTP/1.1 {} X\r\n", resp.status);
                    for (k, v) in &resp.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp.body.len(),
                        resp.body
                    ));
                    let _ = s.write_all(out.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        base
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{serve, Req, Resp};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn app() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn tokens(resource: &str, token_endpoint: &str, expires_at: u64) -> Tokens {
        Tokens {
            access_token: "old".into(),
            refresh_token: Some("r1".into()),
            expires_at,
            client_id: "c1".into(),
            client_secret: None,
            client_auth: "none".into(),
            token_endpoint: token_endpoint.into(),
            resource: resource.into(),
            audience: String::new(),
            scope: String::new(),
            problem: None,
        }
    }

    /// An MCP server with RFC 9728 metadata behind its 401, a path-inserted
    /// RFC 8414 authorization server, registration and a token endpoint.
    /// Records what the token endpoint received.
    fn oauth_server(seen: Arc<Mutex<Vec<Req>>>) -> impl Fn(Req, &str) -> Resp + Send + Sync {
        move |req: Req, base: &str| {
            seen.lock().unwrap().push(req.clone());
            match (req.method.as_str(), req.path()) {
                ("POST", "/mcp") => Resp::empty(401).with(
                    "WWW-Authenticate",
                    &format!(r#"Bearer resource_metadata="{base}/meta/prm", scope="x""#),
                ),
                ("GET", "/meta/prm") => Resp::json(
                    200,
                    json!({
                        "resource": format!("{base}/mcp"),
                        "authorization_servers": [format!("{base}/tenant")],
                        "scopes_supported": ["meetings:read", "recordings:read"],
                    }),
                ),
                ("GET", "/.well-known/oauth-authorization-server/tenant") => Resp::json(
                    200,
                    json!({
                        "issuer": format!("{base}/tenant"),
                        "authorization_endpoint": format!("{base}/tenant/authorize"),
                        "token_endpoint": format!("{base}/tenant/token"),
                        "registration_endpoint": format!("{base}/tenant/register"),
                    }),
                ),
                ("POST", "/tenant/register") => Resp::json(201, json!({"client_id": "dyn-1"})),
                ("POST", "/tenant/token") => Resp::json(
                    200,
                    json!({
                        "access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600,
                    }),
                ),
                _ => Resp::empty(404),
            }
        }
    }

    #[test]
    fn messaging_and_anthropic_hosts_are_recognised() {
        // 10-11 B4: Superhuman's MCP server is an approved mail connector;
        // any other Superhuman or mail host stays refused.
        assert!(!messaging_host("https://mcp.mail.superhuman.com/mcp"));
        assert!(approved_mail("https://mcp.mail.superhuman.com/mcp"));
        assert!(messaging_host("https://api.superhuman.com/mcp"));
        assert!(messaging_host(
            "https://mcp.mail.superhuman.com.evil.example/mcp"
        ));
        assert!(!approved_mail(
            "http://mcp.mail.superhuman.com.evil.example/mcp"
        ));
        assert!(
            !approved_mail("http://mcp.mail.superhuman.com/mcp"),
            "https only"
        );
        assert!(messaging_host("https://mcp.slack.com/x"));
        assert!(messaging_host("https://teams-mcp.example.com/x"));
        assert!(!messaging_host("https://mcp.zoom.us/mcp/zoom/streamable"));
        assert!(!messaging_host("https://api.wisprflow.ai/connect/mcp"));
        assert!(anthropic_hosted("https://microsoft365.mcp.claude.com/mcp"));
        assert!(!anthropic_hosted("https://mcp.zoom.us/mcp"));
    }

    #[test]
    fn a_grant_is_only_for_its_own_address() {
        assert!(same_resource(
            "https://mcp.zoom.us/mcp/",
            "https://MCP.zoom.us:443/mcp"
        ));
        assert!(!same_resource(
            "https://mcp.zoom.us/mcp",
            "https://evil.example/mcp"
        ));
        assert!(!same_resource(
            "https://mcp.zoom.us/mcp",
            "https://mcp.zoom.us/other"
        ));
        assert!(!same_resource(
            "https://mcp.zoom.us/mcp",
            "http://mcp.zoom.us/mcp"
        ));
    }

    #[test]
    fn the_challenge_header_names_the_metadata() {
        assert_eq!(
            resource_metadata(r#"Bearer realm="x", resource_metadata="https://a.example/prm""#)
                .as_deref(),
            Some("https://a.example/prm")
        );
        assert_eq!(
            resource_metadata("Bearer resource_metadata=https://a.example/prm, error=x").as_deref(),
            Some("https://a.example/prm")
        );
        assert_eq!(resource_metadata("Bearer realm=x"), None);
    }

    #[tokio::test]
    async fn discovery_follows_the_challenge_to_path_inserted_metadata() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen)).await;
        let auth = discover(&http().unwrap(), &format!("{base}/mcp"))
            .await
            .unwrap();
        assert_eq!(
            auth.authorization_endpoint,
            format!("{base}/tenant/authorize")
        );
        assert_eq!(auth.token_endpoint, format!("{base}/tenant/token"));
        assert_eq!(
            auth.registration_endpoint,
            Some(format!("{base}/tenant/register"))
        );
        assert_eq!(auth.scopes, vec!["meetings:read", "recordings:read"]);
    }

    #[tokio::test]
    async fn a_server_without_metadata_gets_the_origin_fallback() {
        let base = serve(|_req, _base| Resp::empty(404)).await;
        let auth = discover(&http().unwrap(), &format!("{base}/mcp"))
            .await
            .unwrap();
        assert_eq!(auth.authorization_endpoint, format!("{base}/authorize"));
        assert_eq!(auth.token_endpoint, format!("{base}/token"));
        assert_eq!(auth.registration_endpoint, Some(format!("{base}/register")));
    }

    #[tokio::test]
    async fn plain_http_endpoints_off_loopback_are_refused() {
        let base = serve(|req, base| match req.path() {
            "/.well-known/oauth-protected-resource/mcp" => Resp::json(
                200,
                json!({
                    "authorization_servers": [base.to_string()],
                }),
            ),
            "/.well-known/oauth-authorization-server" => Resp::json(
                200,
                json!({
                    "authorization_endpoint": "http://login.example.com/authorize",
                    "token_endpoint": format!("{base}/token"),
                }),
            ),
            _ => Resp::empty(404),
        })
        .await;
        let err = discover(&http().unwrap(), &format!("{base}/mcp"))
            .await
            .unwrap_err();
        assert!(err.contains("https"), "{err}");
    }

    /// The browser hop: follow the authorize URL's redirect with a code and
    /// the state it carried, as the authorization server would.
    fn browser(
        captured: Arc<Mutex<String>>,
        state_override: Option<&'static str>,
    ) -> impl FnOnce(&str) -> Result<(), String> {
        move |url: &str| {
            *captured.lock().unwrap() = url.to_string();
            let u = reqwest::Url::parse(url).unwrap();
            let q = |k: &str| {
                u.query_pairs()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.into_owned())
                    .unwrap()
            };
            let redirect = q("redirect_uri");
            let state = state_override
                .map(str::to_string)
                .unwrap_or_else(|| q("state"));
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                if state_override.is_some() {
                    // A forged reply first, then (never) the real one.
                    let _ = client
                        .get(format!("{redirect}?code=evil&state={state}"))
                        .send()
                        .await;
                } else {
                    let _ = client
                        .get(format!("{redirect}?code=the-code&state={state}"))
                        .send()
                        .await;
                }
            });
            Ok(())
        }
    }

    #[test]
    fn a_confidential_client_presents_its_secret_the_way_the_server_takes() {
        let m = |v: &[&str]| secret_method(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(m(&[]), "client_secret_basic", "RFC 6749's default");
        assert_eq!(m(&["client_secret_post"]), "client_secret_post");
        assert_eq!(
            m(&["client_secret_post", "client_secret_basic"]),
            "client_secret_basic"
        );
        assert_eq!(m(&["none", "private_key_jwt"]), "client_secret_basic");
    }

    /// Overnight 10-11 Y6: a Zoom Marketplace app is a confidential client.
    /// Its secret goes to the token endpoint (HTTP Basic here, no metadata
    /// list), never to the browser, and is kept for refreshes.
    #[tokio::test]
    async fn sign_in_with_a_registered_app_sends_its_secret_to_the_token_endpoint_only() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen.clone())).await;
        let dir = app();
        let url = format!("{base}/mcp");
        let captured = Arc::new(Mutex::new(String::new()));
        sign_in(
            SignIn {
                app: dir.path(),
                id: "zoom",
                name: "Zoom",
                url: &url,
                client_id: Some("app-1"),
                client_secret: Some(" s3cret "),
                still_current: Some(&|| Ok(())),
            },
            &http().unwrap(),
            browser(captured.clone(), None),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(
            !captured.lock().unwrap().contains("s3cret"),
            "never in the browser"
        );
        let seen = seen.lock().unwrap();
        assert!(
            !seen.iter().any(|r| r.path() == "/tenant/register"),
            "a registered app doesn't register again"
        );
        let token = seen.iter().find(|r| r.path() == "/tenant/token").unwrap();
        let basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("app-1:s3cret")
        );
        assert_eq!(token.header("authorization"), Some(basic.as_str()));
        assert_eq!(
            token.param("client_secret"),
            None,
            "not in the form as well"
        );
        let t = load(dir.path(), "zoom").unwrap();
        assert_eq!(
            t.client_secret.as_deref(),
            Some("s3cret"),
            "kept for refreshes"
        );
        assert_eq!(t.client_auth, "client_secret_basic");
        assert_eq!(t.client_id, "app-1");
    }

    /// B2 (#537 review): a grant that arrives after its connector changed is
    /// not saved; `still_current` runs under the token lock first.
    #[tokio::test]
    async fn a_late_grant_is_not_saved_when_its_connector_changed() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen.clone())).await;
        let dir = app();
        let url = format!("{base}/mcp");
        let changed = || Err::<(), String>("Zoom changed while you were signing in".into());
        let err = sign_in(
            SignIn {
                app: dir.path(),
                id: "zoom",
                name: "Zoom",
                url: &url,
                client_id: Some("app-1"),
                client_secret: Some("s3cret"),
                still_current: Some(&changed),
            },
            &http().unwrap(),
            browser(Arc::new(Mutex::new(String::new())), None),
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert!(err.contains("changed while you were signing in"), "{err}");
        assert!(load(dir.path(), "zoom").is_none(), "nothing saved");
    }

    #[tokio::test]
    async fn sign_in_registers_uses_pkce_and_binds_the_resource() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen.clone())).await;
        let dir = app();
        let url = format!("{base}/mcp");
        let captured = Arc::new(Mutex::new(String::new()));
        let status = sign_in(
            SignIn {
                app: dir.path(),
                id: "zoom",
                name: "Zoom",
                url: &url,
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            browser(captured.clone(), None),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(status.signed_in);
        let authorize = reqwest::Url::parse(&captured.lock().unwrap()).unwrap();
        let q = |k: &str| {
            authorize
                .query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(q("client_id").as_deref(), Some("dyn-1"));
        assert_eq!(q("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(q("resource").as_deref(), Some(url.as_str()));
        assert_eq!(q("scope").as_deref(), Some("meetings:read recordings:read"));
        let seen = seen.lock().unwrap();
        let register = seen
            .iter()
            .find(|r| r.path() == "/tenant/register")
            .unwrap();
        let reg: Value = serde_json::from_str(&register.body).unwrap();
        assert_eq!(reg["redirect_uris"][0], q("redirect_uri").unwrap());
        assert_eq!(reg["token_endpoint_auth_method"], "none");
        let token = seen.iter().find(|r| r.path() == "/tenant/token").unwrap();
        assert_eq!(token.param("code").as_deref(), Some("the-code"));
        assert_eq!(token.param("resource").as_deref(), Some(url.as_str()));
        let verifier = token.param("code_verifier").unwrap();
        assert_eq!(pkce_challenge(&verifier), q("code_challenge").unwrap());
        let stored = load(dir.path(), "zoom").unwrap();
        assert_eq!(stored.access_token, "at-1");
        assert_eq!(stored.resource, url);
        assert!(ready(dir.path(), "zoom", &url));
        assert!(!ready(dir.path(), "zoom", "https://elsewhere.example/mcp"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(dir.path(), "zoom").unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn a_reply_with_the_wrong_state_is_ignored() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen.clone())).await;
        let dir = app();
        let url = format!("{base}/mcp");
        let err = sign_in(
            SignIn {
                app: dir.path(),
                id: "zoom",
                name: "Zoom",
                url: &url,
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            browser(Arc::new(Mutex::new(String::new())), Some("forged")),
            Duration::from_millis(800),
        )
        .await
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.path() == "/tenant/token"));
        assert!(load(dir.path(), "zoom").is_none());
    }

    #[tokio::test]
    async fn messaging_servers_are_never_signed_in() {
        let dir = app();
        let err = sign_in(
            SignIn {
                app: dir.path(),
                id: "mail",
                name: "Mail",
                url: "https://api.superhuman.com/mcp",
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            |_| panic!("no browser for a mail server"),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(err.contains("mail"), "{err}");
    }

    #[tokio::test]
    async fn a_server_without_registration_needs_a_client_id() {
        let base = serve(|req, base| match req.path() {
            "/.well-known/oauth-protected-resource/mcp" => {
                Resp::json(200, json!({"authorization_servers": [base.to_string()]}))
            }
            "/.well-known/oauth-authorization-server" => Resp::json(
                200,
                json!({
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                }),
            ),
            _ => Resp::empty(404),
        })
        .await;
        let dir = app();
        let err = sign_in(
            SignIn {
                app: dir.path(),
                id: "x",
                name: "X",
                url: &format!("{base}/mcp"),
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            |_| panic!("no browser without a client"),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(err.contains("client id"), "{err}");
    }

    #[tokio::test]
    async fn concurrent_refreshes_redeem_the_refresh_token_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let base = serve(move |req, _| {
            if req.path() == "/token" {
                let n = c.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
                // Rotation: only r1 is good, once.
                if req.param("refresh_token").as_deref() != Some("r1") || n > 0 {
                    return Resp::json(400, json!({"error": "invalid_grant"}));
                }
                return Resp::json(
                    200,
                    json!({"access_token": "new", "refresh_token": "r2", "expires_in": 3600}),
                );
            }
            Resp::empty(404)
        })
        .await;
        let dir = app();
        let url = format!("{base}/mcp");
        save(dir.path(), "z", &tokens(&url, &format!("{base}/token"), 1)).unwrap();
        let client = http().unwrap();
        let (a, b) = tokio::join!(
            access_token(dir.path(), "z", &url, &client, None),
            access_token(dir.path(), "z", &url, &client, None),
        );
        assert_eq!(a.unwrap(), "new");
        assert_eq!(b.unwrap(), "new");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            load(dir.path(), "z").unwrap().refresh_token.as_deref(),
            Some("r2")
        );
    }

    #[tokio::test]
    async fn a_refused_refresh_signs_the_connector_out_with_its_reason() {
        let base = serve(|req, _| match req.path() {
            "/token" => Resp::json(
                400,
                json!({"error": "invalid_grant", "error_description": "revoked"}),
            ),
            _ => Resp::empty(404),
        })
        .await;
        let dir = app();
        let url = format!("{base}/mcp");
        save(dir.path(), "z", &tokens(&url, &format!("{base}/token"), 1)).unwrap();
        let err = access_token(dir.path(), "z", &url, &http().unwrap(), None)
            .await
            .unwrap_err();
        assert!(err.contains("invalid_grant: revoked"), "{err}");
        assert!(!ready(dir.path(), "z", &url));
        assert_eq!(
            status(dir.path(), "z", &url).problem.as_deref(),
            Some("invalid_grant: revoked")
        );
    }

    #[tokio::test]
    async fn an_unreachable_token_endpoint_keeps_the_sign_in() {
        let dir = app();
        let url = "https://mcp.example.com/mcp";
        save(dir.path(), "z", &tokens(url, "http://127.0.0.1:9/token", 1)).unwrap();
        assert!(access_token(dir.path(), "z", url, &http().unwrap(), None)
            .await
            .is_err());
        assert!(ready(dir.path(), "z", url));
    }

    #[tokio::test]
    async fn a_rejected_token_is_refreshed_even_before_it_expires() {
        let base = serve(|req, _| match req.path() {
            "/token" => Resp::json(200, json!({"access_token": "new", "expires_in": 3600})),
            _ => Resp::empty(404),
        })
        .await;
        let dir = app();
        let url = format!("{base}/mcp");
        save(
            dir.path(),
            "z",
            &tokens(&url, &format!("{base}/token"), now() + 3600),
        )
        .unwrap();
        let client = http().unwrap();
        assert_eq!(
            access_token(dir.path(), "z", &url, &client, None)
                .await
                .unwrap(),
            "old"
        );
        assert_eq!(
            access_token(dir.path(), "z", &url, &client, Some("old"))
                .await
                .unwrap(),
            "new"
        );
        // The old refresh token is kept when the server doesn't rotate.
        assert_eq!(
            load(dir.path(), "z").unwrap().refresh_token.as_deref(),
            Some("r1")
        );
    }

    #[tokio::test]
    async fn a_token_for_another_address_is_never_sent() {
        let dir = app();
        save(
            dir.path(),
            "z",
            &tokens(
                "https://mcp.zoom.us/mcp",
                "https://zoom.us/token",
                now() + 3600,
            ),
        )
        .unwrap();
        let err = access_token(
            dir.path(),
            "z",
            "https://evil.example/mcp",
            &http().unwrap(),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("another address"), "{err}");
        assert!(!status(dir.path(), "z", "https://evil.example/mcp").signed_in);
    }

    #[test]
    fn ids_that_could_escape_the_folder_are_refused() {
        let dir = app();
        assert!(token_path(dir.path(), "../x").is_err());
        assert!(token_path(dir.path(), "a/b").is_err());
        assert!(load(dir.path(), "..").is_none());
    }

    /// Mail/Slack/Teams cases shared with `connectorsLibrary.test.ts`
    /// (`messagingHost`), so the two guards can't drift apart.
    pub(crate) const MESSAGING_CASES: [(&str, bool); 8] = [
        ("https://mcp.mail.superhuman.com/mcp", false),
        ("https://api.superhuman.com/mcp", true),
        ("https://mcp.slack.com/mcp", true),
        ("https://teams-mcp.example.com/x", true),
        ("https://gmail.example.com/mcp", true),
        ("https://outlook-mcp.example.com/mcp", true),
        ("https://mcp.zoom.us/mcp/zoom/streamable", false),
        ("https://api.wisprflow.ai/connect/mcp", false),
    ];

    #[test]
    fn messaging_cases_match_the_frontend_list() {
        for (url, want) in MESSAGING_CASES {
            assert_eq!(messaging_host(url), want, "{url}");
        }
        let ts = include_str!("../../src/connectorsLibrary.test.ts");
        for (url, want) in MESSAGING_CASES {
            assert!(
                ts.contains(&format!("['{url}', {want}]")),
                "connectorsLibrary.test.ts lacks {url}"
            );
        }
    }

    #[test]
    fn metadata_may_name_the_server_or_a_path_above_it_only() {
        assert!(covers(
            "https://mcp.zoom.us",
            "https://mcp.zoom.us/mcp/zoom/streamable"
        ));
        assert!(covers(
            "https://mcp.zoom.us/mcp/",
            "https://mcp.zoom.us/mcp/zoom"
        ));
        assert!(!covers(
            "https://mcp.zoom.us/mc",
            "https://mcp.zoom.us/mcp/zoom"
        ));
        assert!(!covers(
            "https://other.example/mcp",
            "https://mcp.zoom.us/mcp"
        ));
        assert!(!covers("http://mcp.zoom.us/mcp", "https://mcp.zoom.us/mcp"));
    }

    #[tokio::test]
    async fn a_sign_in_swapped_in_during_the_lock_wait_is_never_refreshed_or_sent() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let base = serve(move |_req, _| {
            c.fetch_add(1, Ordering::SeqCst);
            Resp::json(200, json!({"access_token": "leaked", "expires_in": 3600}))
        })
        .await;
        let dir = app();
        let a = "https://a.example.com/mcp";
        save(dir.path(), "z", &tokens(a, &format!("{base}/token"), 1)).unwrap();
        let held = lock(dir.path(), "z").await.unwrap();
        let client = http().unwrap();
        let path = dir.path().to_path_buf();
        let waiting = tokio::spawn(async move { access_token(&path, "z", a, &client, None).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        // The connector moved to B and signed in again while A's bridge waited.
        save(
            dir.path(),
            "z",
            &tokens("https://b.example.com/mcp", &format!("{base}/token"), 1),
        )
        .unwrap();
        drop(held);
        let err = waiting.await.unwrap().unwrap_err();
        assert!(err.contains("another address"), "{err}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "B's grant was never refreshed"
        );
        assert!(load(dir.path(), "z").unwrap().problem.is_none());
    }

    #[tokio::test]
    async fn an_error_reply_without_our_state_cannot_end_the_sign_in() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(oauth_server(seen)).await;
        let dir = app();
        let url = format!("{base}/mcp");
        let open = |authorize: &str| {
            let u = reqwest::Url::parse(authorize).unwrap();
            let q = |k: &str| {
                u.query_pairs()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.into_owned())
                    .unwrap()
            };
            let (redirect, state) = (q("redirect_uri"), q("state"));
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                // A page aborting the sign-in, then the real reply.
                let _ = client
                    .get(format!("{redirect}?error=access_denied&state=forged"))
                    .send()
                    .await;
                let _ = client
                    .get(format!("{redirect}?code=the-code&state={state}"))
                    .send()
                    .await;
            });
            Ok(())
        };
        let status = sign_in(
            SignIn {
                app: dir.path(),
                id: "zoom",
                name: "Zoom",
                url: &url,
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            open,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(status.signed_in);
    }

    #[tokio::test]
    async fn metadata_for_another_resource_or_issuer_is_refused() {
        let other_resource = serve(|req, base| match req.path() {
            "/.well-known/oauth-protected-resource/mcp" => Resp::json(200, json!({
                "resource": format!("{base}/elsewhere"), "authorization_servers": [base.to_string()],
            })),
            _ => Resp::empty(404),
        })
        .await;
        let err = discover(&http().unwrap(), &format!("{other_resource}/mcp"))
            .await
            .unwrap_err();
        assert!(err.contains("another address"), "{err}");
        let other_issuer = serve(|req, base| match req.path() {
            "/.well-known/oauth-protected-resource/mcp" => {
                Resp::json(200, json!({"authorization_servers": [base.to_string()]}))
            }
            "/.well-known/oauth-authorization-server" => Resp::json(
                200,
                json!({
                    "issuer": "https://evil.example.com",
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                }),
            ),
            _ => Resp::empty(404),
        })
        .await;
        let err = discover(&http().unwrap(), &format!("{other_issuer}/mcp"))
            .await
            .unwrap_err();
        assert!(err.contains("issuer"), "{err}");
    }

    #[tokio::test]
    async fn the_metadatas_resource_is_the_audience_on_every_request() {
        let seen = Arc::new(Mutex::new(Vec::<Req>::new()));
        let log = seen.clone();
        let base = serve(move |req, base| {
            log.lock().unwrap().push(req.clone());
            match req.path() {
                "/.well-known/oauth-protected-resource/mcp" => Resp::json(
                    200,
                    json!({
                        "resource": base.to_string(), "authorization_servers": [base.to_string()],
                    }),
                ),
                "/.well-known/oauth-authorization-server" => Resp::json(
                    200,
                    json!({
                        "issuer": base.to_string(),
                        "authorization_endpoint": format!("{base}/authorize"),
                        "token_endpoint": format!("{base}/token"),
                        "registration_endpoint": format!("{base}/register"),
                    }),
                ),
                "/register" => Resp::json(201, json!({"client_id": "c"})),
                "/token" => Resp::json(
                    200,
                    json!({"access_token": "a", "refresh_token": "r", "expires_in": 1}),
                ),
                _ => Resp::empty(404),
            }
        })
        .await;
        let dir = app();
        let url = format!("{base}/mcp");
        let captured = Arc::new(Mutex::new(String::new()));
        sign_in(
            SignIn {
                app: dir.path(),
                id: "z",
                name: "Z",
                url: &url,
                client_id: None,
                client_secret: None,
                still_current: None,
            },
            &http().unwrap(),
            browser(captured.clone(), None),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let authorize = reqwest::Url::parse(&captured.lock().unwrap()).unwrap();
        let resource = authorize
            .query_pairs()
            .find(|(k, _)| k == "resource")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(resource, base);
        // The token expired at once: the refresh carries the same audience.
        access_token(dir.path(), "z", &url, &http().unwrap(), None)
            .await
            .unwrap();
        let tokens: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path() == "/token")
            .filter_map(|r| r.param("resource"))
            .collect();
        assert_eq!(tokens, vec![base.clone(), base.clone()]);
        // And the grant still binds to the connector's own URL.
        assert!(ready(dir.path(), "z", &url));
    }
}
