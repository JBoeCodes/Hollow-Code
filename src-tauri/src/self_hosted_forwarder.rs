//! Self-hosted models, Q2 (2026-11-03): Hollow's local forwarder. One
//! loopback listener per enabled endpoint; a run's Claude Code (or, since
//! Q9, Codex, on `/v1/responses`) talks to it with a per-run key
//! (`self_hosted::RunTarget::run_key`) and the forwarder adds the
//! endpoint's own bearer token and streams the answer back. Any path goes
//! up as sent; a refusal's body has Anthropic's shape, whose `error.type`
//! and `error.message` are where OpenAI's clients read them too.
//!
//! - **Loopback only.** The listener binds `127.0.0.1` on a free port.
//! - **Per-run keys.** [`Forwarder::mint`] makes a random key with an
//!   expiry (wall clock, so a sleeping Mac doesn't stretch it); a request
//!   with no key, a wrong one, an expired or a revoked one gets 401 right
//!   after its headers, before its body is read, and never reaches the
//!   endpoint. Keys live in memory only.
//! - **Token command.** The endpoint's token comes from a command's stdout
//!   (Qwen: `gcloud auth print-identity-token`), cached in memory for at most
//!   [`TOKEN_MAX_AGE`]; an upstream 401 drops it (unless another request
//!   already replaced it), fetches a fresh one and retries once. The token
//!   is never written to disk, logs or argv, and never follows a redirect.
//! - **Streaming.** Each upstream chunk is written to the client as it
//!   arrives, one write per chunk frame with Nagle off, so Server-Sent
//!   Events keep their timing.
//! - **Timeouts and limits** ([`Limits`]): a client has 30 s to send its
//!   headers and 5 minutes for its body; at most 64 connections at once.
//!   Upstream headers may take 15 minutes (a cold start is 4 to 6; Qwen's
//!   took 272 s in Q0), a stalled body 10 between chunks. A client that
//!   leaves while the endpoint is still thinking ends the upstream call.
//! - **Logs** carry method, path, status, sizes and timing, never bodies,
//!   header values, keys or tokens; what a client sent is escaped first.
//! - **Shutdown.** [`Forwarder::shutdown`] (and [`stop_all`] at quit) stops
//!   the listener and every connection it is serving, and closes the
//!   forwarder: an in-flight [`Forwarder::get`] (a Test's health check or
//!   model list, and the token command under it) ends at once with
//!   [`STOPPED`], and no further upstream call starts. Retiring a replaced
//!   forwarder is not a shutdown, so a run's proxied stream outlives an edit
//!   (Q4 review).
//! - **Stops win.** [`stop`] counts per entry ([`stops`]); [`ensure_since`]
//!   refuses, and shuts down what it just started, when a stop came after
//!   the caller's reading, so a remove or switch-off during a start never
//!   leaves a forwarder registered (Q4 review).
//! - **Hardened after review** (PR #1112): checked chunk arithmetic (a
//!   crafted size can't panic the app), the key checked before the body,
//!   read timeouts, a connection cap, no redirects.
//! - **When it last answered** (Q10): the entry's current forwarder notes
//!   the wall-clock time of each 2xx from the endpoint (a run's call or
//!   Hollow's own GET, at the response headers) in [`last_answered`] and
//!   wakes [`answered_signal`]'s waiter, so background jobs know the model
//!   is awake. A stop, or a new forwarder for a changed address, forgets it.

// Q4's Test (`self_hosted_registry::test_entry`) starts forwarders and mints
// keys; Q5's run path will too. Until it lands, some of the rest is used by
// the tests only.
#![cfg_attr(not(test), allow(dead_code))]
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// How long a fetched endpoint token is reused (Google identity tokens last
/// an hour).
pub const TOKEN_MAX_AGE: Duration = Duration::from_secs(50 * 60);
/// How long the token command may run.
pub const TOKEN_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest a run key lives, whatever its run asked for.
pub const KEY_MAX_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// What a call through a stopped forwarder gets.
pub const STOPPED: &str =
    "Hollow stopped this model's local forwarder: it was switched off, removed or changed.";
/// Request limits: header block, chunk-size line and body.
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_CHUNK_LINE: usize = 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// Header names passed through to the upstream (lowercase). Everything
/// else, the client's `authorization` and `x-api-key` included, stays here.
const FORWARDED_REQUEST_HEADERS: [&str; 4] = [
    "content-type",
    "accept",
    "anthropic-version",
    "anthropic-beta",
];
/// Response header names passed back to the client (lowercase).
const FORWARDED_RESPONSE_HEADERS: [&str; 4] = [
    "content-type",
    "cache-control",
    "x-request-id",
    "request-id",
];

/// The forwarder's timeouts and caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// A client's time to send its request line and headers.
    pub client_headers: Duration,
    /// A client's time to send its body.
    pub client_body: Duration,
    /// The upstream's time to send response headers (a cold start).
    pub upstream_headers: Duration,
    /// The longest an upstream body may stall between chunks.
    pub upstream_idle: Duration,
    /// Connections served at once; more get 503.
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            client_headers: Duration::from_secs(30),
            client_body: Duration::from_secs(5 * 60),
            upstream_headers: Duration::from_secs(15 * 60),
            upstream_idle: Duration::from_secs(10 * 60),
            max_connections: 64,
        }
    }
}

/// How the forwarder authenticates to the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// An open endpoint (a model on this Mac): no token.
    None,
    /// A command whose stdout is a bearer token (`program` and its
    /// arguments, run without a shell).
    TokenCommand { program: String, args: Vec<String> },
}

/// The endpoint a forwarder serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// The registry entry's id, for logs.
    pub entry: String,
    /// `https://…` or `http://` loopback, no trailing slash.
    pub base_url: String,
    pub auth: Auth,
}

impl Upstream {
    /// The same upstream with its base URL trimmed and without a trailing
    /// slash, so two spellings of one address compare equal.
    pub fn normalized(&self) -> Upstream {
        Upstream {
            base_url: self.base_url.trim().trim_end_matches('/').to_string(),
            ..self.clone()
        }
    }
}

/// Why `url` can't be an upstream base: only `https://` or `http://` to
/// this Mac (`localhost` allowed here, unlike a run's base URL), so a token
/// never travels in the clear; a host, and no user name, password, query or
/// fragment, since a path is appended to it (Q4 review).
pub fn upstream_url_problem(url: &str) -> Option<String> {
    const SCHEME: &str =
        "The address must start with https://, or http:// for this Mac (localhost or 127.0.0.1).";
    let url = url.trim();
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return Some(SCHEME.into());
    };
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some("The address can't contain spaces or control characters.".into());
    }
    if rest.is_empty() || rest.starts_with('/') {
        return Some("The address needs a host name after the scheme.".into());
    }
    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(e) => return Some(format!("The address isn't a valid URL ({e}).")),
    };
    if !matches!(parsed.host_str(), Some(host) if !host.is_empty()) {
        return Some("The address needs a host name after the scheme.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Some("The address can't hold a user name or password (user@host).".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Some("The address can't have a query (?) or a fragment (#).".into());
    }
    if url.starts_with("https://") {
        return None;
    }
    // Plain http only for this Mac, read from the parsed host (so `[::1]` with
    // no port is the loopback address it is, and a port never hides a host).
    if matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        return None;
    }
    Some(SCHEME.into())
}

/// A log sink, so tests can read what the forwarder logs.
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

fn default_log() -> Log {
    Arc::new(|line: &str| eprintln!("[self-hosted] {line}"))
}

/// The endpoint token, in memory only.
#[derive(Default)]
struct TokenCache {
    token: Option<(String, Instant)>,
}

struct Keys {
    /// Run key → expiry (wall clock).
    live: HashMap<String, SystemTime>,
}

/// Equal byte strings, compared without an early exit.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Keys {
    fn check(&mut self, offered: &[&str], now: SystemTime) -> bool {
        self.live.retain(|_, expiry| *expiry > now);
        // Compare every offered key with every live one, each in constant time.
        let mut ok = false;
        for key in self.live.keys() {
            for candidate in offered {
                ok |= same_bytes(key.as_bytes(), candidate.as_bytes());
            }
        }
        ok
    }
}

struct Conns {
    handles: Vec<JoinHandle<()>>,
    closed: bool,
}

struct Shared {
    upstream: Upstream,
    limits: Limits,
    keys: Mutex<Keys>,
    token: tokio::sync::Mutex<TokenCache>,
    client: reqwest::Client,
    log: Log,
    /// Connections being served, aborted at shutdown.
    conns: Mutex<Conns>,
    /// True once [`Forwarder::shutdown`] ran: [`Forwarder::get`] ends and
    /// [`send_upstream`] starts nothing more (Q4 review).
    closed: tokio::sync::watch::Sender<bool>,
}

impl Shared {
    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }
}

/// A running forwarder for one endpoint.
pub struct Forwarder {
    shared: Arc<Shared>,
    port: u16,
    accept: JoinHandle<()>,
}

impl Forwarder {
    /// Start a forwarder for `upstream` on a free loopback port.
    pub async fn start(upstream: Upstream) -> Result<Forwarder, String> {
        Self::start_with(upstream, default_log(), Limits::default()).await
    }

    pub(crate) async fn start_with(
        upstream: Upstream,
        log: Log,
        limits: Limits,
    ) -> Result<Forwarder, String> {
        if let Some(problem) = upstream_url_problem(&upstream.base_url) {
            return Err(problem);
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("The local forwarder couldn't start: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("The local forwarder couldn't start: {e}"))?
            .port();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            // A redirect could carry the token to another scheme or host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("The local forwarder couldn't start: {e}"))?;
        let shared = Arc::new(Shared {
            upstream: upstream.normalized(),
            limits,
            keys: Mutex::new(Keys {
                live: HashMap::new(),
            }),
            token: tokio::sync::Mutex::new(TokenCache::default()),
            client,
            log,
            conns: Mutex::new(Conns {
                handles: Vec::new(),
                closed: false,
            }),
            closed: tokio::sync::watch::Sender::new(false),
        });
        let serving = shared.clone();
        let accept = tokio::spawn(async move {
            loop {
                let mut stream = match listener.accept().await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        // A client that reset before the accept, or no file
                        // descriptors left: keep serving (delta review).
                        if serving.conns.lock().map(|c| c.closed).unwrap_or(true) {
                            return;
                        }
                        (serving.log)(&format!("{}: accept failed: {e}", serving.upstream.entry));
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                let mut conns = match serving.conns.lock() {
                    Ok(conns) => conns,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if conns.closed {
                    return;
                }
                conns.handles.retain(|h| !h.is_finished());
                if conns.handles.len() >= serving.limits.max_connections {
                    drop(conns);
                    tokio::spawn(async move {
                        refuse_and_close(
                            &mut stream,
                            503,
                            "overloaded_error",
                            "Too many requests at once.",
                        )
                        .await;
                    });
                    continue;
                }
                let conn = serving.clone();
                conns.handles.push(tokio::spawn(async move {
                    serve(conn, stream).await;
                }));
            }
        });
        (shared.log)(&format!(
            "{}: forwarder on 127.0.0.1:{port}",
            shared.upstream.entry
        ));
        Ok(Forwarder {
            shared,
            port,
            accept,
        })
    }

    /// `http://127.0.0.1:<port>`: a run's `ANTHROPIC_BASE_URL`.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn upstream(&self) -> &Upstream {
        &self.shared.upstream
    }

    /// Whether the forwarder still accepts connections.
    pub fn is_serving(&self) -> bool {
        !self.is_closed() && !self.accept.is_finished()
    }

    /// Whether [`Forwarder::shutdown`] ran (a stop, never a retirement).
    pub fn is_closed(&self) -> bool {
        self.shared.is_closed()
    }

    /// A new run key, valid for `ttl` (at most [`KEY_MAX_TTL`]).
    pub fn mint(&self, ttl: Duration) -> crate::self_hosted::RunKey {
        let key = new_key();
        let expiry = SystemTime::now() + ttl.min(KEY_MAX_TTL);
        if let Ok(mut keys) = self.shared.keys.lock() {
            keys.live.insert(key.clone(), expiry);
        }
        crate::self_hosted::RunKey::new(key)
    }

    /// End a run key now (its run ended).
    pub fn revoke(&self, key: &str) {
        if let Ok(mut keys) = self.shared.keys.lock() {
            keys.live.remove(key);
        }
    }

    /// How many keys are live (tests, diagnostics, retiring).
    pub fn live_keys(&self) -> usize {
        let now = SystemTime::now();
        self.shared
            .keys
            .lock()
            .map(|mut k| {
                k.live.retain(|_, e| *e > now);
                k.live.len()
            })
            .unwrap_or(0)
    }

    /// A GET to the endpoint itself (a health ping or the model list), with
    /// its token: Hollow's own calls, which need no run key. Returns the
    /// status and at most `max_bytes` of the body. A shutdown ends it at
    /// once with [`STOPPED`], the token command included (Q4 review: a
    /// switch-off during a Test must not keep waking a billed model).
    pub async fn get(&self, path: &str, max_bytes: usize) -> Result<(u16, String), String> {
        let mut closed = self.shared.closed.subscribe();
        tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => Err(STOPPED.into()),
            got = self.get_open(path, max_bytes) => got,
        }
    }

    async fn get_open(&self, path: &str, max_bytes: usize) -> Result<(u16, String), String> {
        let started = Instant::now();
        let mut resp = send_upstream(&self.shared, "GET", path, Vec::new(), &[]).await?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            note_answered(&self.shared);
        }
        let mut body = Vec::new();
        while let Ok(Ok(Some(chunk))) =
            tokio::time::timeout(self.shared.limits.upstream_idle, resp.chunk()).await
        {
            body.extend_from_slice(&chunk);
            if body.len() >= max_bytes {
                body.truncate(max_bytes);
                break;
            }
        }
        (self.shared.log)(&format!(
            "{}: GET {} {} in {} ms (Hollow)",
            self.shared.upstream.entry,
            loggable(path_only(path)),
            status,
            started.elapsed().as_millis()
        ));
        Ok((status, String::from_utf8_lossy(&body).into_owned()))
    }

    /// Stop listening, end every connection being served and close the
    /// forwarder (in-flight Hollow calls end with [`STOPPED`]).
    pub fn shutdown(&self) {
        self.shared.closed.send_replace(true);
        self.accept.abort();
        if let Ok(mut conns) = self.shared.conns.lock() {
            conns.closed = true;
            for conn in conns.handles.drain(..) {
                conn.abort();
            }
        }
        if let Ok(mut keys) = self.shared.keys.lock() {
            keys.live.clear();
        }
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A random 32-byte key, hex, with a recognisable prefix.
fn new_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("hsr_{hex}")
}

/// The path without its query, for logs.
fn path_only(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

/// Client bytes made safe for a log line: printable ASCII only, at most
/// 120 characters.
fn loggable(text: &str) -> String {
    text.chars()
        .take(120)
        .map(|c| if c.is_ascii_graphic() { c } else { '?' })
        .collect()
}

/// The endpoint token: cached, or fetched by the command. `None` for an
/// open endpoint. `rejected` is a token the endpoint just refused: the
/// cache is refreshed only while it still holds that one, so many requests
/// refused at once run the command once.
async fn token(shared: &Shared, rejected: Option<&str>) -> Result<Option<String>, String> {
    let Auth::TokenCommand { program, args } = &shared.upstream.auth else {
        return Ok(None);
    };
    let mut cache = shared.token.lock().await;
    if let Some((token, at)) = cache.token.as_ref() {
        let refused = rejected.is_some_and(|r| r == token);
        if !refused && at.elapsed() < TOKEN_MAX_AGE {
            return Ok(Some(token.clone()));
        }
    }
    cache.token = None;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .env("PATH", crate::agent::cli_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(TOKEN_COMMAND_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "The token command took more than a minute.".to_string())?
        .map_err(|e| format!("The token command couldn't start ({program}): {e}"))?;
    if !out.status.success() {
        // Its stderr names the problem (not signed in, no project); it never
        // holds the token, which only stdout carries.
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        return Err(format!(
            "The token command failed ({}){}",
            out.status,
            if why.is_empty() {
                String::new()
            } else {
                format!(": {}", truncate(why, 200))
            }
        ));
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err("The token command printed no single token.".into());
    }
    cache.token = Some((token.clone(), Instant::now()));
    Ok(Some(token))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Send one request upstream with the endpoint token; on a 401, fetch a
/// fresh token and send it once more.
async fn send_upstream(
    shared: &Shared,
    method: &str,
    path: &str,
    body: Vec<u8>,
    headers: &[(String, String)],
) -> Result<reqwest::Response, String> {
    let url = format!("{}{}", shared.upstream.base_url, path);
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| "unsupported method".to_string())?;
    let mut rejected: Option<String> = None;
    loop {
        // A stopped forwarder starts no token command and no call.
        if shared.is_closed() {
            return Err(STOPPED.into());
        }
        let bearer = token(shared, rejected.as_deref()).await?;
        if shared.is_closed() {
            return Err(STOPPED.into());
        }
        let mut req = shared.client.request(method.clone(), &url);
        for (name, value) in headers {
            req = req.header(name.as_str(), value.as_str());
        }
        if let Some(bearer) = bearer.as_deref() {
            req = req.bearer_auth(bearer);
        }
        if !body.is_empty() {
            req = req.body(body.clone());
        }
        let resp = tokio::time::timeout(shared.limits.upstream_headers, req.send())
            .await
            .map_err(|_| {
                format!(
                    "The model didn't answer within {} minutes.",
                    shared.limits.upstream_headers.as_secs() / 60
                )
            })?
            .map_err(|e| format!("The model couldn't be reached: {}", why(&e)))?;
        if resp.status().as_u16() == 401 && bearer.is_some() && rejected.is_none() {
            rejected = bearer;
            continue;
        }
        return Ok(resp);
    }
}

/// A reqwest error and its causes, without its URL (never secret, but
/// long): reqwest's own text leaves the cause out.
fn why(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    if let Some(url) = e.url() {
        text = text.replace(url.as_str(), "the endpoint");
    }
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    truncate(&text, 400)
}

/// A parsed request head, before its body.
struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    /// What was read past the blank line.
    rest: Vec<u8>,
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// The run keys a request offers: `Authorization: Bearer <key>` and
/// `x-api-key: <key>`; either may match.
fn offered_keys(headers: &[(String, String)]) -> Vec<&str> {
    let mut keys = Vec::new();
    if let Some(key) = header(headers, "authorization").and_then(|a| a.strip_prefix("Bearer ")) {
        keys.push(key.trim());
    }
    if let Some(key) = header(headers, "x-api-key") {
        keys.push(key.trim());
    }
    keys
}

type Refusal = (u16, &'static str);

async fn read_head(stream: &mut TcpStream) -> Result<Head, Refusal> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(at) = find(&buf, b"\r\n\r\n") {
            break at;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err((431, "headers too large"));
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| (400, "read failed"))?;
        if n == 0 {
            return Err((400, "connection closed"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    if head_end > MAX_HEADER_BYTES {
        return Err((431, "headers too large"));
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    if !path.starts_with('/') || method.is_empty() {
        return Err((400, "bad request line"));
    }
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok(Head {
        method,
        path,
        headers,
        rest: buf[head_end + 4..].to_vec(),
    })
}

/// The body after `head`: by `Content-Length`, or chunked.
async fn read_body(stream: &mut TcpStream, head: &mut Head) -> Result<Vec<u8>, Refusal> {
    let rest = std::mem::take(&mut head.rest);
    let chunked = head
        .headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        return read_chunked(stream, rest).await;
    }
    let length: usize = match header(&head.headers, "content-length") {
        None => 0,
        Some(v) => v.trim().parse().map_err(|_| (400, "bad content-length"))?,
    };
    if length > MAX_BODY_BYTES {
        return Err((413, "body too large"));
    }
    let mut body = rest;
    let mut chunk = [0u8; 8192];
    while body.len() < length {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| (400, "read failed"))?;
        if n == 0 {
            return Err((400, "connection closed"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(body)
}

/// A chunked request body, given what was already read past the headers.
/// Every size is checked before it is used (review: a crafted size must not
/// wrap or index out of range).
async fn read_chunked(stream: &mut TcpStream, mut buf: Vec<u8>) -> Result<Vec<u8>, Refusal> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut at = 0usize;
    loop {
        let line_end = loop {
            if let Some(i) = find(&buf[at..], b"\r\n") {
                break at + i;
            }
            if buf.len() - at > MAX_CHUNK_LINE {
                return Err((400, "bad chunk size"));
            }
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|_| (400, "read failed"))?;
            if n == 0 {
                return Err((400, "connection closed"));
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        if line_end - at > MAX_CHUNK_LINE {
            return Err((400, "bad chunk size"));
        }
        let size_text = String::from_utf8_lossy(&buf[at..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| (400, "bad chunk size"))?;
        at = line_end + 2;
        if size == 0 {
            return Ok(body);
        }
        if size > MAX_BODY_BYTES.saturating_sub(body.len()) {
            return Err((413, "body too large"));
        }
        // Both fit in usize: `at` is within a buffer and `size` is capped.
        let need = at + size + 2;
        while buf.len() < need {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|_| (400, "read failed"))?;
            if n == 0 {
                return Err((400, "connection closed"));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        body.extend_from_slice(&buf[at..at + size]);
        // Drop what was consumed, so the raw buffer stays near the body.
        buf.drain(..need);
        at = 0;
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// An Anthropic-shaped error answer, so Claude Code shows the message.
async fn write_error(stream: &mut TcpStream, status: u16, kind: &str, message: &str) {
    let body = serde_json::json!({
        "type": "error",
        "error": {"type": kind, "message": message},
    })
    .to_string();
    let reply = format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.flush().await;
}

/// An error answer to a request whose bytes may still be arriving: close
/// our side, then read (and drop) what is left for a moment, so the client
/// gets the message rather than a connection reset (delta review).
async fn refuse_and_close(stream: &mut TcpStream, status: u16, kind: &str, message: &str) {
    write_error(stream, status, kind, message).await;
    let _ = stream.shutdown().await;
    let mut sink = [0u8; 8192];
    let mut drained = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while drained < 64 * 1024 {
        match tokio::time::timeout_at(deadline, stream.read(&mut sink)).await {
            Ok(Ok(n)) if n > 0 => drained += n,
            _ => break,
        }
    }
}

/// Resolves when the client closed its side (it has sent its whole request,
/// so anything readable now is the end).
async fn client_gone(stream: &TcpStream) {
    loop {
        if stream.readable().await.is_err() {
            return;
        }
        let mut probe = [0u8; 1];
        match stream.try_read(&mut probe) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
    }
}

/// Serve one connection: one request, then close.
async fn serve(shared: Arc<Shared>, mut stream: TcpStream) {
    let started = Instant::now();
    let entry = shared.upstream.entry.clone();
    let limits = shared.limits;
    let refuse = |status: u16, why: &str| {
        (shared.log)(&format!("{entry}: refused a request: {status} {why}"));
    };
    let mut head = match tokio::time::timeout(limits.client_headers, read_head(&mut stream)).await {
        Ok(Ok(head)) => head,
        Ok(Err((status, why))) => {
            refuse(status, why);
            write_error(&mut stream, status, "invalid_request_error", why).await;
            return;
        }
        Err(_) => {
            refuse(408, "headers too slow");
            write_error(
                &mut stream,
                408,
                "invalid_request_error",
                "headers too slow",
            )
            .await;
            return;
        }
    };
    let method = loggable(&head.method);
    let path = loggable(path_only(&head.path));
    // The key before the body: a request without one costs no more reading.
    let authorised = shared
        .keys
        .lock()
        .map(|mut k| k.check(&offered_keys(&head.headers), SystemTime::now()))
        .unwrap_or(false);
    if !authorised {
        (shared.log)(&format!("{entry}: {method} {path} 401 (no valid run key)"));
        refuse_and_close(
            &mut stream,
            401,
            "authentication_error",
            "Hollow's forwarder needs this run's key.",
        )
        .await;
        return;
    }
    if head.method != "GET" && head.method != "POST" {
        refuse_and_close(
            &mut stream,
            405,
            "invalid_request_error",
            "Only GET and POST.",
        )
        .await;
        return;
    }
    let body =
        match tokio::time::timeout(limits.client_body, read_body(&mut stream, &mut head)).await {
            Ok(Ok(body)) => body,
            Ok(Err((status, why))) => {
                refuse(status, why);
                refuse_and_close(&mut stream, status, "invalid_request_error", why).await;
                return;
            }
            Err(_) => {
                refuse(408, "body too slow");
                write_error(&mut stream, 408, "invalid_request_error", "body too slow").await;
                return;
            }
        };
    let forwarded: Vec<(String, String)> = head
        .headers
        .iter()
        .filter(|(n, _)| FORWARDED_REQUEST_HEADERS.contains(&n.as_str()))
        .cloned()
        .collect();
    let sent_bytes = body.len();
    // A client that leaves during a long wait (a cold start) ends the call.
    let sent = tokio::select! {
        sent = send_upstream(&shared, &head.method, &head.path, body, &forwarded) => Some(sent),
        _ = client_gone(&stream) => None,
    };
    let resp = match sent {
        Some(Ok(resp)) => resp,
        Some(Err(why)) => {
            (shared.log)(&format!(
                "{entry}: {method} {path} 502 after {} ms: {why}",
                started.elapsed().as_millis()
            ));
            write_error(&mut stream, 502, "api_error", &why).await;
            return;
        }
        None => {
            (shared.log)(&format!(
                "{entry}: {method} {path}: the client left after {} ms, before the model answered",
                started.elapsed().as_millis()
            ));
            return;
        }
    };
    let status = resp.status().as_u16();
    let first_byte_ms = started.elapsed().as_millis();
    if (200..300).contains(&status) {
        note_answered(&shared);
    }
    let mut head_out = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in resp.headers() {
        let name = name.as_str().to_ascii_lowercase();
        if FORWARDED_RESPONSE_HEADERS.contains(&name.as_str()) {
            if let Ok(value) = value.to_str() {
                head_out.push_str(&format!("{name}: {value}\r\n"));
            }
        }
    }
    head_out.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    if stream.write_all(head_out.as_bytes()).await.is_err() {
        return;
    }
    let mut resp = resp;
    let mut received = 0usize;
    let mut outcome = "done";
    loop {
        match tokio::time::timeout(limits.upstream_idle, resp.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                received += chunk.len();
                // One write per frame (with Nagle off) so an event isn't held.
                let mut frame = Vec::with_capacity(chunk.len() + 12);
                frame.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                frame.extend_from_slice(&chunk);
                frame.extend_from_slice(b"\r\n");
                if stream.write_all(&frame).await.is_err() || stream.flush().await.is_err() {
                    outcome = "client left";
                    break;
                }
            }
            Ok(Ok(None)) => {
                let _ = stream.write_all(b"0\r\n\r\n").await;
                let _ = stream.flush().await;
                break;
            }
            Ok(Err(_)) => {
                outcome = "upstream broke off";
                break;
            }
            Err(_) => {
                outcome = "upstream stalled";
                break;
            }
        }
    }
    (shared.log)(&format!(
        "{entry}: {method} {path} {status}, {sent_bytes} B sent, {received} B back, first byte {first_byte_ms} ms, {} ms in all ({outcome})",
        started.elapsed().as_millis()
    ));
}

// ── The app's forwarders ────────────────────────────────────────────────

struct Running {
    current: HashMap<String, Arc<Forwarder>>,
    /// Forwarders an edited entry replaced, kept until their last run key
    /// ends so a running turn isn't cut off.
    retired: Vec<Arc<Forwarder>>,
    /// How many times [`stop`] ran for each entry.
    stops: HashMap<String, u64>,
}

impl Running {
    fn stops(&self, entry: &str) -> u64 {
        self.stops.get(entry).copied().unwrap_or(0)
    }
}

fn running() -> std::sync::MutexGuard<'static, Running> {
    static RUNNING: OnceLock<Mutex<Running>> = OnceLock::new();
    RUNNING
        .get_or_init(|| {
            Mutex::new(Running {
                current: HashMap::new(),
                retired: Vec::new(),
                stops: HashMap::new(),
            })
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One start at a time, so two callers can't both start a forwarder.
fn starting() -> &'static tokio::sync::Mutex<()> {
    static STARTING: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    STARTING.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// How many times [`stop`] has run for `entry`: read it before deciding to
/// start a forwarder, and hand it to [`ensure_since`].
pub fn stops(entry: &str) -> u64 {
    running().stops(entry)
}

/// The forwarder for `upstream.entry`, started if it isn't running or its
/// upstream changed. A replaced forwarder keeps serving the keys it minted
/// until they end (or the app quits).
pub async fn ensure(upstream: Upstream) -> Result<Arc<Forwarder>, String> {
    let since = stops(&upstream.entry);
    ensure_since(upstream, since).await
}

/// [`ensure`], refused with [`STOPPED`] when [`stop`] ran for the entry
/// after `since` (a [`stops`] reading): the entry was switched off or
/// removed meanwhile. Checked again when the new forwarder is registered,
/// under the same lock `stop` takes, so a stop during the start shuts the
/// new one down instead of leaving it registered (Q4 review).
pub async fn ensure_since(upstream: Upstream, since: u64) -> Result<Arc<Forwarder>, String> {
    let upstream = upstream.normalized();
    let _one = starting().lock().await;
    {
        let map = running();
        if map.stops(&upstream.entry) != since {
            return Err(STOPPED.into());
        }
        if let Some(fwd) = map.current.get(&upstream.entry) {
            if *fwd.upstream() == upstream && fwd.is_serving() {
                return Ok(fwd.clone());
            }
        }
    }
    let fwd = Arc::new(Forwarder::start(upstream.clone()).await?);
    let mut map = running();
    if map.stops(&upstream.entry) != since {
        drop(map);
        fwd.shutdown();
        return Err(STOPPED.into());
    }
    map.retired.retain(|old| old.live_keys() > 0);
    // Q10: a new address hasn't answered yet (a restart of the same one keeps
    // what the old forwarder saw).
    if map
        .current
        .get(&upstream.entry)
        .is_none_or(|old| *old.upstream() != upstream)
    {
        answers().remove(&upstream.entry);
    }
    if let Some(old) = map.current.insert(upstream.entry.clone(), fwd.clone()) {
        if old.live_keys() > 0 && old.is_serving() {
            map.retired.push(old);
        } else {
            old.shutdown();
        }
    }
    Ok(fwd)
}

/// The running forwarder for `entry`, if any.
pub fn get(entry: &str) -> Option<Arc<Forwarder>> {
    running().current.get(entry).cloned()
}

/// Stop the forwarder for `entry` and any it replaced (the entry was
/// removed or disabled), and count the stop so a start in flight for it
/// gives up ([`ensure_since`]).
pub fn stop(entry: &str) {
    let mut map = running();
    *map.stops.entry(entry.to_string()).or_insert(0) += 1;
    answers().remove(entry);
    if let Some(fwd) = map.current.remove(entry) {
        fwd.shutdown();
    }
    map.retired.retain(|old| {
        let keep = old.upstream().entry != entry && old.live_keys() > 0;
        if !keep {
            old.shutdown();
        }
        keep
    });
}

// ── When each entry last answered (Q10) ─────────────────────────────────

fn answers() -> std::sync::MutexGuard<'static, HashMap<String, SystemTime>> {
    static ANSWERS: OnceLock<Mutex<HashMap<String, SystemTime>>> = OnceLock::new();
    ANSWERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Woken each time an entry's endpoint answers 2xx ([`last_answered`]).
pub fn answered_signal() -> &'static tokio::sync::Notify {
    static SIGNAL: OnceLock<tokio::sync::Notify> = OnceLock::new();
    SIGNAL.get_or_init(tokio::sync::Notify::new)
}

/// Note that `shared`'s endpoint answered 2xx now, when `shared` is its
/// entry's current forwarder (a retired one serves an old address).
fn note_answered(shared: &Arc<Shared>) {
    let entry = &shared.upstream.entry;
    let current = running()
        .current
        .get(entry)
        .is_some_and(|fwd| Arc::ptr_eq(&fwd.shared, shared));
    if current {
        answers().insert(entry.clone(), SystemTime::now());
        answered_signal().notify_one();
    }
}

/// Q10: when `entry`'s endpoint last answered 2xx through its current
/// forwarder (wall clock, so a sleeping Mac doesn't count as awake), if it
/// has since the forwarder started at this address.
pub fn last_answered(entry: &str) -> Option<SystemTime> {
    answers().get(entry).copied()
}

/// Set (or clear) when `entry` last answered (tests).
#[cfg(test)]
pub(crate) fn set_last_answered(entry: &str, at: Option<SystemTime>) {
    match at {
        Some(at) => answers().insert(entry.to_string(), at),
        None => answers().remove(entry),
    };
}

/// Stop every forwarder (the app is quitting).
pub fn stop_all() {
    let mut map = running();
    for (_, fwd) in map.current.drain() {
        fwd.shutdown();
    }
    for fwd in map.retired.drain(..) {
        fwd.shutdown();
    }
}

#[cfg(test)]
#[path = "self_hosted_forwarder_tests.rs"]
mod tests;
