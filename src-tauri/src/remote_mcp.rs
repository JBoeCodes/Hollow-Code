//! `hollow remote-mcp --id <id> --url <url> [--auth-dir <dir>]`: a remote MCP
//! server on Hollow's own sign-in, as a stdio server any agent can start
//! (five asks B2, 2026-10-10; sign-in in `remote_auth.rs`).
//!
//! Each JSON-RPC line from the agent is POSTed to the server (MCP Streamable
//! HTTP) with the connector's bearer token, its `Mcp-Session-Id` and the
//! negotiated `MCP-Protocol-Version`; the answer (JSON, or an SSE stream read
//! until the request's response) goes back as lines. A 401 refreshes the
//! token once (under `remote_auth`'s lock, so a token another process just
//! rotated is reused) and retries. `initialize` is forwarded before anything
//! else is read, so the session id is known; later messages run concurrently.
//! The token is sent only to the URL it was granted for, and redirects are
//! never followed. Not supported: the optional server-initiated GET stream
//! (servers must answer requests on the POST, which is all agents need).
//!
//! **Mail asks first (10-11 B4).** For an approved mail connector
//! (`remote_auth::approved_mail`, decided by the URL, so no library edit can
//! turn it off) every `tools/call` that isn't a read ([`is_read_tool`]) waits
//! for the person on Hollow's approval card (`approvals::confirm_tool`: the
//! chat's card, or the native dialog when the app channel is down; a deny or
//! a timeout is no). A refused call never reaches the server; the agent gets
//! an error result saying nothing was sent.
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub struct Cfg {
    pub id: String,
    pub url: String,
    /// Ask on Hollow's card before any tool call that isn't a read (an
    /// approved mail connector; set from the URL by [`serve`]).
    pub confirm_writes: bool,
    pub app: PathBuf,
    /// For tool calls: long, since a tool may take minutes.
    pub http: reqwest::Client,
    /// For token refreshes.
    pub auth_http: reqwest::Client,
}

#[derive(Default)]
struct Session {
    id: Option<String>,
    protocol: Option<String>,
}

struct Ctx<W> {
    cfg: Cfg,
    session: Arc<Mutex<Session>>,
    out: tokio::sync::Mutex<W>,
    /// The agent's `initialize`, kept to start a new session when the
    /// server ends one (404).
    init: Arc<Mutex<Option<Value>>>,
    /// One re-initialization at a time.
    reinit: Arc<tokio::sync::Mutex<()>>,
    /// Request ids the agent cancelled (`notifications/cancelled`): a mail
    /// write the person allows after its agent gave up is never sent.
    cancelled: Arc<Mutex<std::collections::HashSet<String>>>,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub fn serve() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(id), Some(url)) = (arg(&args, "--id"), arg(&args, "--url")) else {
        eprintln!("usage: hollow remote-mcp --id <id> --url <url> [--auth-dir <dir>]");
        std::process::exit(2);
    };
    let app = match arg(&args, "--auth-dir").map(PathBuf::from) {
        Some(dir) => dir,
        None => match crate::agent_settings::app_data_dir() {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("[remote-mcp] {e}");
                std::process::exit(2);
            }
        },
    };
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client");
    let auth_http = crate::remote_auth::http().expect("http client");
    // A mail bridge trusts the approval socket only when the Hollow app that
    // started this run answers on it; anything else (an agent's own listener
    // that would say yes) is dropped, and the native dialog asks instead.
    // Before the runtime starts, while this process has one thread.
    if crate::remote_auth::approved_mail(&url) {
        if let Ok(sock) = std::env::var(crate::approvals::ENV_SOCK) {
            if !crate::approvals::broker_is_ancestor(&sock) {
                eprintln!("[remote-mcp {id}] the approval socket isn't Hollow's own; mail writes ask on the native dialog");
                std::env::remove_var(crate::approvals::ENV_SOCK);
            }
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let cfg = Cfg {
            confirm_writes: crate::remote_auth::approved_mail(&url),
            id,
            url,
            app,
            http,
            auth_http,
        };
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        bridge(cfg, input, tokio::io::stdout()).await;
    });
}

/// Forward every line of `input` until it ends; returns the writer.
pub async fn bridge<R, W>(cfg: Cfg, input: R, output: W) -> W
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let ctx = Arc::new(Ctx {
        cfg,
        session: Arc::new(Mutex::new(Session::default())),
        out: tokio::sync::Mutex::new(output),
        init: Arc::new(Mutex::new(None)),
        reinit: Arc::new(tokio::sync::Mutex::new(())),
        cancelled: Arc::new(Mutex::new(std::collections::HashSet::new())),
    });
    let mut lines = input.lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(raw)) = lines.next_line().await {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => {
                emit(&ctx, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})).await;
                continue;
            }
        };
        let method = msg.get("method").and_then(Value::as_str);
        if ctx.cfg.confirm_writes {
            // A batch could carry a write past the gate, and a `tools/call`
            // with no id would run without an answer to hold: neither is
            // forwarded for a mail connector.
            if msg.is_array() {
                emit(&ctx, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Batches aren't forwarded to a mail connector; send one request at a time."}})).await;
                continue;
            }
            if method == Some("tools/call") && msg.get("id").is_none() {
                eprintln!(
                    "[remote-mcp {}] dropped a tools/call with no id",
                    ctx.cfg.id
                );
                continue;
            }
            if method == Some("notifications/cancelled") {
                if let Some(id) = msg.pointer("/params/requestId") {
                    ctx.cancelled
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(id.to_string());
                }
            }
        }
        if method == Some("initialize") {
            *ctx.init
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(msg.clone());
        }
        // Requests run side by side; `initialize`, notifications and the
        // agent's answers to the server go in order, so
        // `notifications/initialized` always reaches the server before the
        // next request (review B2 #2).
        if method.is_some() && msg.get("id").is_some() && method != Some("initialize") {
            let ctx = ctx.clone();
            tasks.spawn(async move {
                if let Some(refusal) = gate(&ctx.cfg, &msg).await {
                    return emit(&ctx, &refusal).await;
                }
                // Allowed after the agent cancelled it: the agent has moved
                // on (it may already have asked again), so nothing is sent.
                let id = msg.get("id").map(Value::to_string).unwrap_or_default();
                if ctx.cfg.confirm_writes
                    && ctx
                        .cancelled
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&id)
                {
                    return eprintln!(
                        "[remote-mcp {}] request {id} was cancelled; not sent",
                        ctx.cfg.id
                    );
                }
                forward(&ctx, msg).await
            });
        } else {
            forward(&ctx, msg).await;
        }
    }
    // The agent closed stdin: let answers in flight finish, briefly.
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    end_session(&ctx).await;
    match Arc::try_unwrap(ctx) {
        Ok(ctx) => ctx.out.into_inner(),
        Err(_) => unreachable!("every task has ended"),
    }
}

/// Superhuman Mail's read tools (its help pages, 10-11 B4). Any other tool
/// of an approved mail connector asks first.
const MAIL_READS: [&str; 9] = [
    "list_threads",
    "get_thread",
    "get_message",
    "get_attachment",
    "list_labels",
    "list_splits",
    "get_read_statuses",
    "get_availability",
    "query_email_and_calendar",
];

/// A tool that only reads: exactly one of [`MAIL_READS`]. Everything else
/// asks, a tool the server adds later included, until it is listed here.
pub fn is_read_tool(name: &str) -> bool {
    MAIL_READS.contains(&name)
}

/// For a mail connector, the card before a write: `None` lets the call go
/// on; `Some` is the error answer for a call the person didn't allow (or a
/// call Hollow couldn't ask about), which never reaches the server.
pub async fn gate(cfg: &Cfg, msg: &Value) -> Option<Value> {
    gate_with(cfg, msg, |title, action| async move {
        let tool = msg.pointer("/params/name").and_then(Value::as_str);
        let args = msg.pointer("/params/arguments");
        crate::approvals::confirm_tool(&title, &action, "mail", tool, args).await
    })
    .await
}

/// [`gate`] with its card as a seam (tests answer it without a dialog).
pub async fn gate_with<F, Fut>(cfg: &Cfg, msg: &Value, confirm: F) -> Option<Value>
where
    F: FnOnce(String, String) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if !cfg.confirm_writes || msg.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    let name = msg
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !name.is_empty() && is_read_tool(name) {
        return None;
    }
    let args = msg
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or(Value::Null);
    let shown = crate::claude_runner::truncate(&args.to_string(), 600);
    let action = format!(
        "{} wants to run `{}`, which can send, draft or change mail.\n\nWith: {shown}",
        connector_name(&cfg.url),
        if name.is_empty() {
            "(no tool name)"
        } else {
            name
        },
    );
    let verdict = confirm(connector_name(&cfg.url), action).await;
    let why = match verdict {
        Ok(()) => return None,
        Err(e) => e,
    };
    Some(
        json!({"jsonrpc": "2.0", "id": msg.get("id").cloned().unwrap_or(Value::Null), "result": {
            "isError": true,
            "content": [{"type": "text", "text": format!("The person didn't allow `{name}` in Hollow ({why}). Nothing was sent to the mail server. Don't retry on your own: tell the user, and ask before trying again.")}],
        }}),
    )
}

/// The card's title for a mail connector's address.
fn connector_name(url: &str) -> String {
    if crate::remote_auth::approved_mail(url) {
        "Superhuman Mail".into()
    } else {
        "Connector".into()
    }
}

async fn emit<W: AsyncWrite + Unpin>(ctx: &Ctx<W>, v: &Value) {
    let mut line = v.to_string();
    line.push('\n');
    let mut out = ctx.out.lock().await;
    let _ = out.write_all(line.as_bytes()).await;
    let _ = out.flush().await;
}

/// A request's error answer; a notification's failure only goes to stderr.
async fn fail<W: AsyncWrite + Unpin>(ctx: &Ctx<W>, id: Option<&Value>, message: String) {
    match id {
        Some(id) => {
            emit(
                ctx,
                &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": message}}),
            )
            .await
        }
        None => eprintln!("[remote-mcp {}] {message}", ctx.cfg.id),
    }
}

/// The server's answer to `msg`, forwarded line by line.
async fn forward<W: AsyncWrite + Unpin>(ctx: &Ctx<W>, msg: Value) {
    forward_checked(ctx, msg, None).await
}

async fn forward_checked<W: AsyncWrite + Unpin>(
    ctx: &Ctx<W>,
    msg: Value,
    permit: Option<&crate::shared_mcp::Permit>,
) {
    let id = msg.get("method").and(msg.get("id")).cloned();
    let initialize = msg.get("method").and_then(Value::as_str) == Some("initialize");
    let mut rejected: Option<String> = None;
    let mut refreshed = false;
    let mut reinitialized = false;
    loop {
        if permit.is_some_and(|p| !p.valid()) {
            return;
        }
        let token = match crate::remote_auth::access_token(
            &ctx.cfg.app,
            &ctx.cfg.id,
            &ctx.cfg.url,
            &ctx.cfg.auth_http,
            rejected.as_deref(),
        )
        .await
        {
            Ok(t) => t,
            Err(e) => return fail(ctx, id.as_ref(), e).await,
        };
        if permit.is_some_and(|p| !p.valid()) {
            return;
        }
        let (session, protocol) = {
            let s = ctx
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (s.id.clone(), s.protocol.clone())
        };
        let mut req = ctx
            .cfg
            .http
            .post(&ctx.cfg.url)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .json(&msg);
        if let Some(s) = &session {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(p) = &protocol {
            req = req.header("MCP-Protocol-Version", p);
        }
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                return fail(ctx, id.as_ref(), format!("Couldn't reach the server: {e}")).await
            }
        };
        let status = res.status();
        if status.as_u16() == 401 {
            if !refreshed {
                refreshed = true;
                rejected = Some(token);
                continue;
            }
            return fail(ctx, id.as_ref(), "The server refused Hollow's sign-in: sign in again in Hollow → Settings → Connectors.".into()).await;
        }
        if status.as_u16() == 404 && session.is_some() {
            // The server ended the session: start a new one with the
            // agent's own `initialize`, then send this again (review B2 #5).
            if !initialize
                && !reinitialized
                && reinitialize(ctx, &token, session.as_deref(), permit).await
            {
                reinitialized = true;
                continue;
            }
            ctx.session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .id = None;
            return fail(
                ctx,
                id.as_ref(),
                "The server ended this session and a new one couldn't start.".into(),
            )
            .await;
        }
        if let Some(s) = res
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty() && s.len() <= 256)
        {
            ctx.session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .id = Some(s.to_string());
        }
        if !status.is_success() {
            let body: String = res
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            return fail(
                ctx,
                id.as_ref(),
                format!("The server answered {status}: {body}"),
            )
            .await;
        }
        if status.as_u16() == 202 || status.as_u16() == 204 {
            return;
        }
        let sse = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let got = if sse {
            stream_events(ctx, res, id.as_ref(), initialize).await
        } else {
            match res.json::<Value>().await {
                Ok(Value::Array(items)) => {
                    for v in &items {
                        deliver(ctx, v, initialize).await;
                    }
                    true
                }
                Ok(v) => {
                    deliver(ctx, &v, initialize).await;
                    true
                }
                Err(e) => {
                    return fail(
                        ctx,
                        id.as_ref(),
                        format!("The server sent an unreadable answer: {e}"),
                    )
                    .await
                }
            }
        };
        if !got && id.is_some() {
            fail(
                ctx,
                id.as_ref(),
                "The server's stream ended without an answer.".into(),
            )
            .await;
        }
        return;
    }
}

/// Start a new session after the server ended `stale`: the agent's
/// `initialize` again (its answer isn't forwarded; the agent already has
/// one) and `notifications/initialized`. True when the session is usable,
/// including when another request already restarted it.
async fn reinitialize<W>(
    ctx: &Ctx<W>,
    token: &str,
    stale: Option<&str>,
    permit: Option<&crate::shared_mcp::Permit>,
) -> bool {
    let _one = ctx.reinit.lock().await;
    if permit.is_some_and(|p| !p.valid()) {
        return false;
    }
    let current = ctx
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .id
        .clone();
    if current.as_deref() != stale {
        return true;
    }
    let Some(init) = ctx
        .init
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    else {
        return false;
    };
    let post = |msg: &Value, session: Option<&str>| {
        let mut req = ctx
            .cfg
            .http
            .post(&ctx.cfg.url)
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(msg);
        if let Some(s) = session {
            req = req.header("Mcp-Session-Id", s);
        }
        req.send()
    };
    let Ok(res) = post(&init, None).await else {
        return false;
    };
    if !res.status().is_success() {
        return false;
    }
    let session = res
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_string);
    drop(res);
    ctx.session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .id = session.clone();
    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    if permit.is_some_and(|p| !p.valid()) {
        return false;
    }
    post(&initialized, session.as_deref())
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// One protocol session per attempt and selected OAuth connector. The host
/// reuses the URL-bound refresh code, without sharing protocol/cancel state.
pub(crate) fn shared_handler(entry: &Value) -> Option<crate::shared_mcp::Handler> {
    let args: Vec<String> = entry
        .get("args")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect::<Option<_>>()?;
    if args.first().map(String::as_str) != Some("remote-mcp") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if entry.get("command")?.as_str()? != exe.to_str()? {
        return None;
    }
    let id = arg(&args, "--id")?;
    let url = arg(&args, "--url")?;
    let app = arg(&args, "--auth-dir")
        .map(PathBuf::from)
        .or_else(|| crate::agent_settings::app_data_dir().ok())?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(480))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let auth_http = crate::remote_auth::http().ok()?;
    let bridge = Arc::new(SharedBridge {
        cfg: Cfg {
            confirm_writes: crate::remote_auth::approved_mail(&url),
            id,
            url,
            app,
            http,
            auth_http,
        },
        session: Arc::new(Mutex::new(Session::default())),
        init: Arc::new(Mutex::new(None)),
        reinit: Arc::new(tokio::sync::Mutex::new(())),
        cancelled: Arc::new(Mutex::new(std::collections::HashSet::new())),
    });
    // Mail still needs its existing connector-specific approval context.
    // Keep that selected entry on stdio until the caller supplies this gate.
    if bridge.cfg.confirm_writes {
        return None;
    }
    Some(Arc::new(move |msg, permit| {
        let bridge = bridge.clone();
        Box::pin(async move { bridge.call(msg, permit).await })
    }))
}

struct SharedBridge {
    cfg: Cfg,
    session: Arc<Mutex<Session>>,
    init: Arc<Mutex<Option<Value>>>,
    reinit: Arc<tokio::sync::Mutex<()>>,
    cancelled: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl SharedBridge {
    fn context<W>(&self, out: W) -> Ctx<W> {
        Ctx {
            cfg: Cfg {
                id: self.cfg.id.clone(),
                url: self.cfg.url.clone(),
                confirm_writes: self.cfg.confirm_writes,
                app: self.cfg.app.clone(),
                http: self.cfg.http.clone(),
                auth_http: self.cfg.auth_http.clone(),
            },
            session: self.session.clone(),
            init: self.init.clone(),
            reinit: self.reinit.clone(),
            cancelled: self.cancelled.clone(),
            out: tokio::sync::Mutex::new(out),
        }
    }

    async fn call(&self, msg: Value, permit: crate::shared_mcp::Permit) -> Option<Value> {
        use tokio::io::AsyncReadExt;
        if !permit.valid() {
            return None;
        }
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "notifications/cancelled" {
            if let Some(id) = msg.pointer("/params/requestId") {
                let mut cancelled = self.cancelled.lock().unwrap_or_else(|p| p.into_inner());
                if cancelled.len() >= 1024 {
                    permit.revoke();
                    return None;
                }
                cancelled.insert(id.to_string());
            }
        }
        if method == "initialize" {
            *self.init.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg.clone());
        }
        let id = msg.get("id").cloned();
        let (writer, mut reader) = tokio::io::duplex(65536);
        let ctx = self.context(writer);
        let forward = async {
            forward_checked(&ctx, msg, Some(&permit)).await;
            let _ = ctx.out.lock().await.shutdown().await;
        };
        let collect = async {
            let mut bytes = Vec::new();
            let _ = (&mut reader)
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await;
            bytes
        };
        let cancelled = async {
            loop {
                if !permit.valid()
                    || id.as_ref().is_some_and(|id| {
                        self.cancelled
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .contains(&id.to_string())
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        let bytes = tokio::select! {
            (_, bytes) = async { tokio::join!(forward,collect) } => bytes,
            _ = cancelled => return None,
        };
        if !permit.valid() || bytes.len() > 16 * 1024 * 1024 {
            return None;
        }
        let id = id?;
        String::from_utf8(bytes)
            .ok()?
            .lines()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .find(|v| v.get("id") == Some(&id))
    }
}

impl Drop for SharedBridge {
    fn drop(&mut self) {
        let ctx = self.context(tokio::io::sink());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                end_session(&ctx).await;
            });
        }
    }
}

/// Write one message, keeping the protocol version `initialize` settled on.
async fn deliver<W: AsyncWrite + Unpin>(ctx: &Ctx<W>, v: &Value, initialize: bool) {
    if initialize {
        if let Some(p) = v.pointer("/result/protocolVersion").and_then(Value::as_str) {
            ctx.session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .protocol = Some(p.to_string());
        }
    }
    emit(ctx, v).await;
}

/// Read an SSE answer, forwarding each event's message, until the response
/// to `id` arrives (or, for a notification, the stream ends). Whether it
/// did.
async fn stream_events<W: AsyncWrite + Unpin>(
    ctx: &Ctx<W>,
    mut res: reqwest::Response,
    id: Option<&Value>,
    initialize: bool,
) -> bool {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        while let Some(end) = buf.windows(2).position(|w| w == b"\n\n") {
            let event: Vec<u8> = buf.drain(..end + 2).collect();
            let text = String::from_utf8_lossy(&event);
            let data: Vec<&str> = text
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if data.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&data.join("\n")) else {
                continue;
            };
            deliver(ctx, &v, initialize).await;
            let answered = id.is_some_and(|id| {
                v.get("id") == Some(id) && (v.get("result").is_some() || v.get("error").is_some())
            });
            if answered {
                return true;
            }
        }
        match res.chunk().await {
            Ok(Some(chunk)) => buf.extend(chunk.iter().filter(|b| **b != b'\r')),
            _ => return id.is_none(),
        }
    }
}

/// Tell the server the session is over (best effort).
async fn end_session<W>(ctx: &Ctx<W>) {
    let session = ctx
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .id
        .take();
    let Some(session) = session else { return };
    let Ok(token) = crate::remote_auth::access_token(
        &ctx.cfg.app,
        &ctx.cfg.id,
        &ctx.cfg.url,
        &ctx.cfg.auth_http,
        None,
    )
    .await
    else {
        return;
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        ctx.cfg
            .http
            .delete(&ctx.cfg.url)
            .bearer_auth(token)
            .header("Mcp-Session-Id", session)
            .send(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_auth::fake::{serve, Req, Resp};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn save_tokens(app: &std::path::Path, url: &str, token_endpoint: &str, access: &str) {
        let t = crate::remote_auth::Tokens {
            access_token: access.into(),
            refresh_token: Some("r1".into()),
            expires_at: 0,
            client_id: "c".into(),
            client_secret: None,
            client_auth: "none".into(),
            token_endpoint: token_endpoint.into(),
            resource: url.into(),
            audience: String::new(),
            scope: String::new(),
            problem: None,
        };
        std::fs::create_dir_all(app.join("RemoteAuth")).unwrap();
        std::fs::write(
            app.join("RemoteAuth/z.json"),
            serde_json::to_vec(&t).unwrap(),
        )
        .unwrap();
    }

    fn cfg(app: &std::path::Path, url: &str) -> Cfg {
        Cfg {
            confirm_writes: false,
            id: "z".into(),
            url: url.into(),
            app: app.to_path_buf(),
            http: reqwest::Client::new(),
            auth_http: crate::remote_auth::http().unwrap(),
        }
    }

    async fn run(app: &std::path::Path, url: &str, input: &str) -> Vec<Value> {
        let out = bridge(
            cfg(app, url),
            tokio::io::BufReader::new(input.as_bytes()),
            Vec::new(),
        )
        .await;
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// An MCP server that takes only the token `good`: initialize answers in
    /// JSON with a session, tools/list over SSE (a progress notification
    /// first), notifications get 202. Records every request.
    fn mcp(seen: Arc<Mutex<Vec<Req>>>) -> impl Fn(Req, &str) -> Resp + Send + Sync {
        move |req: Req, _base: &str| {
            seen.lock().unwrap().push(req.clone());
            if req.path() == "/token" {
                return Resp::json(200, json!({"access_token": "good", "expires_in": 3600}));
            }
            if req.method == "DELETE" {
                return Resp::empty(200);
            }
            if req.header("authorization") != Some("Bearer good") {
                return Resp::empty(401);
            }
            let msg: Value = serde_json::from_str(&req.body).unwrap();
            match msg["method"].as_str() {
                Some("initialize") => Resp::json(200, json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"protocolVersion": "2025-06-18", "capabilities": {}}}))
                    .with("Mcp-Session-Id", "s1"),
                Some("notifications/initialized") => Resp::empty(202),
                Some("tools/list") => Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/event-stream".into())],
                    body: format!(
                        "event: message\r\ndata: {}\r\n\r\ndata: {}\r\n\r\n",
                        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}),
                        json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": [{"name": "list_meetings"}]}}),
                    ),
                },
                _ => Resp::json(200, json!({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "nope"}})),
            }
        }
    }

    #[tokio::test]
    async fn bridges_json_sse_and_notifications_with_the_session() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(mcp(seen.clone())).await;
        let url = format!("{base}/mcp");
        let app = tempfile::tempdir().unwrap();
        save_tokens(app.path(), &url, &format!("{base}/token"), "good");
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let out = run(app.path(), &url, &input).await;
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
        assert!(out.iter().any(|v| v["method"] == "notifications/progress"));
        let tools = out.iter().find(|v| v["id"] == 2).unwrap();
        assert_eq!(tools["result"]["tools"][0]["name"], "list_meetings");
        assert_eq!(out.len(), 3, "{out:?}");
        let seen = seen.lock().unwrap();
        let list = seen.iter().find(|r| r.body.contains("tools/list")).unwrap();
        assert_eq!(list.header("mcp-session-id"), Some("s1"));
        assert_eq!(list.header("mcp-protocol-version"), Some("2025-06-18"));
        assert!(seen
            .iter()
            .any(|r| r.method == "DELETE" && r.header("mcp-session-id") == Some("s1")));
    }

    #[tokio::test]
    async fn shared_oauth_protocol_state_is_isolated_per_attempt() {
        let next = Arc::new(AtomicUsize::new(0));
        let base = serve(move |req: Req,_| {
            let msg: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
            if msg["method"] == "initialize" {
                let session = format!("session-{}",next.fetch_add(1,Ordering::SeqCst));
                return Resp::json(200,json!({"jsonrpc":"2.0","id":msg["id"],"result":{"protocolVersion":"2025-06-18"}})).with("Mcp-Session-Id",&session);
            }
            Resp::json(200,json!({"jsonrpc":"2.0","id":msg["id"],"result":{"session":req.header("mcp-session-id")}}))
        }).await;
        let url = format!("{base}/mcp");
        let dir = tempfile::tempdir().unwrap();
        save_tokens(dir.path(), &url, &format!("{base}/token"), "good");
        let entry = json!({"command":std::env::current_exe().unwrap(),"args":["remote-mcp","--id","z","--url",url,"--auth-dir",dir.path()]});
        let registry = Arc::new(crate::shared_mcp::Registry::default());
        let host = crate::shared_mcp::start(registry.clone()).await.unwrap();
        let first = registry
            .register(
                "z",
                "one",
                Arc::new(|_| true),
                Duration::from_secs(5),
                shared_handler(&entry).unwrap(),
            )
            .unwrap();
        let second = registry
            .register(
                "z",
                "two",
                Arc::new(|_| true),
                Duration::from_secs(5),
                shared_handler(&entry).unwrap(),
            )
            .unwrap();
        first.activate();
        second.activate();
        let client = reqwest::Client::new();
        let call = |grant: &crate::shared_mcp::Capability, method: &str| {
            client
                .post(format!("http://127.0.0.1:{}/mcp/z", host.port))
                .header("Authorization", grant.header())
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method}))
        };
        call(&first, "initialize")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        call(&second, "initialize")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let a: Value = call(&first, "tools/list")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let b: Value = call(&second, "tools/list")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(a["result"]["session"], "session-0");
        assert_eq!(b["result"]["session"], "session-1");
        first.revoke();
        assert_eq!(
            call(&first, "tools/list").send().await.unwrap().status(),
            401
        );
        assert_eq!(
            call(&second, "tools/list").send().await.unwrap().status(),
            200
        );
    }

    #[tokio::test]
    async fn a_401_refreshes_once_and_retries() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(mcp(seen.clone())).await;
        let url = format!("{base}/mcp");
        let app = tempfile::tempdir().unwrap();
        save_tokens(app.path(), &url, &format!("{base}/token"), "stale");
        let out = run(
            app.path(),
            &url,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string(),
        )
        .await;
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|r| r.path() == "/token")
                .count(),
            1
        );
        assert_eq!(
            crate::remote_auth::load(app.path(), "z")
                .unwrap()
                .access_token,
            "good"
        );
    }

    #[tokio::test]
    async fn not_signed_in_answers_each_request_with_an_error() {
        let app = tempfile::tempdir().unwrap();
        let out = run(
            app.path(),
            "https://mcp.example.com/mcp",
            &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"}).to_string(),
        )
        .await;
        assert_eq!(out[0]["id"], 7);
        assert!(out[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Settings → Connectors"));
    }

    #[tokio::test]
    async fn a_bad_line_gets_a_parse_error_and_the_bridge_goes_on() {
        let app = tempfile::tempdir().unwrap();
        let out = run(app.path(), "https://mcp.example.com/mcp", "not json\n").await;
        assert_eq!(out[0]["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn notifications_reach_the_server_before_the_next_request() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = serve(mcp(seen.clone())).await;
        let url = format!("{base}/mcp");
        let app = tempfile::tempdir().unwrap();
        save_tokens(app.path(), &url, &format!("{base}/token"), "good");
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        for _ in 0..5 {
            seen.lock().unwrap().clear();
            run(app.path(), &url, &input).await;
            let order: Vec<String> = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .map(|r| r.body.clone())
                .collect();
            let at = |m: &str| order.iter().position(|b| b.contains(m)).unwrap();
            assert!(
                at("notifications/initialized") < at("tools/list"),
                "{order:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_ended_session_is_started_again_and_the_request_retried() {
        let seen = Arc::new(Mutex::new(Vec::<Req>::new()));
        let log = seen.clone();
        let sessions = Arc::new(AtomicUsize::new(0));
        let base = serve(move |req: Req, _base: &str| {
            log.lock().unwrap().push(req.clone());
            if req.method == "DELETE" {
                return Resp::empty(200);
            }
            let msg: Value = serde_json::from_str(&req.body).unwrap();
            match msg["method"].as_str() {
                Some("initialize") => {
                    let n = sessions.fetch_add(1, Ordering::SeqCst) + 1;
                    Resp::json(200, json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"protocolVersion": "2025-06-18"}}))
                        .with("Mcp-Session-Id", &format!("s{n}"))
                }
                Some("notifications/initialized") => Resp::empty(202),
                // s1 has ended on the server; s2 works.
                _ if req.header("mcp-session-id") == Some("s1") => Resp::empty(404),
                // A batch answer: a notification and the response.
                _ => Resp::json(200, json!([
                    {"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info"}},
                    {"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": []}},
                ])),
            }
        })
        .await;
        let url = format!("{base}/mcp");
        let app = tempfile::tempdir().unwrap();
        save_tokens(app.path(), &url, &format!("{base}/token"), "good");
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let out = run(app.path(), &url, &input).await;
        // One initialize answer for the agent (the restart's is kept back),
        // the batch's notification and the retried request's answer.
        assert_eq!(out.iter().filter(|v| v["id"] == 1).count(), 1, "{out:?}");
        assert!(out.iter().any(|v| v["method"] == "notifications/message"));
        assert_eq!(
            out.iter().find(|v| v["id"] == 2).unwrap()["result"]["tools"],
            json!([])
        );
        let seen = seen.lock().unwrap();
        let count = |m: &str| seen.iter().filter(|r| r.body.contains(m)).count();
        assert_eq!(count("\"initialize\""), 2);
        assert_eq!(count("notifications/initialized"), 2);
        let retried = seen
            .iter()
            .rev()
            .find(|r| r.body.contains("tools/list"))
            .unwrap();
        assert_eq!(retried.header("mcp-session-id"), Some("s2"));
    }

    #[test]
    fn arguments_are_read_by_name() {
        let args: Vec<String> = [
            "hollow",
            "remote-mcp",
            "--id",
            "zoom",
            "--url",
            "https://mcp.zoom.us/mcp",
            "--auth-dir",
            "/app",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(arg(&args, "--id").as_deref(), Some("zoom"));
        assert_eq!(
            arg(&args, "--url").as_deref(),
            Some("https://mcp.zoom.us/mcp")
        );
        assert_eq!(arg(&args, "--auth-dir").as_deref(), Some("/app"));
        assert_eq!(arg(&args, "--missing"), None);
        assert_eq!(
            arg(&args[..7], "--auth-dir"),
            None,
            "a flag without its value"
        );
    }

    // ── 10-11 B4: an approved mail connector asks before writes ─────────

    fn mail_cfg(app: &std::path::Path) -> Cfg {
        Cfg {
            confirm_writes: true,
            ..cfg(app, "https://mcp.mail.superhuman.com/mcp")
        }
    }

    fn call(name: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": {"draft_id": "d1"}}})
    }

    #[test]
    fn reads_go_and_anything_else_asks() {
        for read in MAIL_READS {
            assert!(is_read_tool(read), "{read}");
        }
        for write in [
            "send_draft",
            "create_or_update_draft",
            "discard_draft",
            "undo_send",
            "trash_thread",
            "unsubscribe",
            "create_or_update_event",
            "get_and_send",
            "list_and_archive",
            "mark_read",
            "new_tool_we_never_saw",
            "",
        ] {
            assert!(!is_read_tool(write), "{write} asks");
        }
        // Only the listed reads: a read-sounding name that acts still asks.
        for unlisted in [
            "search_threads",
            "find_and_respond",
            "list_and_accept_invites",
            "get_and_rsvp",
        ] {
            assert!(!is_read_tool(unlisted), "{unlisted} asks");
        }
    }

    #[tokio::test]
    async fn a_mail_write_waits_for_the_card_and_a_no_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mail_cfg(dir.path());
        let asked = std::sync::Mutex::new(Vec::new());
        // Allowed: the call goes on.
        let out = gate_with(&cfg, &call("send_draft"), |title, action| {
            asked.lock().unwrap().push((title, action));
            async { Ok(()) }
        })
        .await;
        assert!(out.is_none());
        let (title, action) = asked.lock().unwrap()[0].clone();
        assert_eq!(title, "Superhuman Mail");
        assert!(
            action.contains("`send_draft`") && action.contains("d1"),
            "{action}"
        );
        // Denied (or timed out): an error result, never forwarded.
        let out = gate_with(&cfg, &call("trash_thread"), |_, _| async {
            Err("denied".to_string())
        })
        .await
        .expect("refused");
        assert_eq!(out["id"], 7);
        assert_eq!(out["result"]["isError"], true);
        assert!(out["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Nothing was sent"));
    }

    #[tokio::test]
    async fn reads_and_other_connectors_never_ask() {
        let dir = tempfile::tempdir().unwrap();
        let never = |_: String, _: String| async { panic!("asked") };
        assert!(
            gate_with(&mail_cfg(dir.path()), &call("list_threads"), never)
                .await
                .is_none()
        );
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        assert!(gate_with(&mail_cfg(dir.path()), &list, never)
            .await
            .is_none());
        let zoom = cfg(dir.path(), "https://mcp.zoom.us/mcp");
        assert!(gate_with(&zoom, &call("send_draft"), never).await.is_none());
    }

    #[test]
    fn the_approved_mail_hosts_match_the_frontend() {
        let ts = include_str!("../../src/connectorsLibrary.ts");
        for host in crate::remote_auth::APPROVED_MAIL_HOSTS {
            assert!(
                ts.contains(&format!("APPROVED_MAIL_HOSTS = ['{host}']")),
                "{host}"
            );
        }
    }

    #[tokio::test]
    async fn a_mail_bridge_forwards_no_batch_and_no_id_less_call() {
        let dir = tempfile::tempdir().unwrap();
        let input = [
            r#"[{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send_draft","arguments":{}}}]"#,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"send_draft","arguments":{}}}"#,
        ]
        .join("\n");
        let out = bridge(
            mail_cfg(dir.path()),
            tokio::io::BufReader::new(input.as_bytes()),
            Vec::new(),
        )
        .await;
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // One refusal for the batch; the id-less call is dropped silently.
        // Neither reached the server (there is none at this address in a
        // test, and no sign-in: a forward would answer with an error).
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["error"]["code"], -32600);
    }
}
