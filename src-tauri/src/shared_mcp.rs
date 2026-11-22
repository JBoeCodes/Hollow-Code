//! Host-owned HTTP MCP transport. Attachment and broker policy remain outside
//! this module. Every message needs a live, activated attempt capability.

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use rand::RngCore;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

const MAX_GRANTS: usize = 2048;
const MAX_CONNECTIONS: usize = 128;
const BODY_MAX: usize = 1024 * 1024;
const TOKEN_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);
// Past the longest tool timeout, including header/body admission. Also bounds
// a peer that never reads responses while repeatedly sending invalid tokens.
const CONNECTION_LIFETIME: Duration = Duration::from_secs(500);

type Reply = Pin<Box<dyn Future<Output = Option<Value>> + Send>>;
pub(crate) type Handler = Arc<dyn Fn(Value, Permit) -> Reply + Send + Sync>;
type Live = Arc<dyn Fn(&str) -> bool + Send + Sync>;

tokio::task_local! {
    static REQUEST: Option<Permit>;
}

thread_local! {
    static BLOCKING_REQUEST: std::cell::RefCell<Option<Permit>> = const { std::cell::RefCell::new(None) };
}

struct BlockingScope(Option<Permit>);

impl BlockingScope {
    fn enter(permit: Option<Permit>) -> Self {
        Self(BLOCKING_REQUEST.with(|slot| slot.replace(permit)))
    }
}

impl Drop for BlockingScope {
    fn drop(&mut self) {
        BLOCKING_REQUEST.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

fn request_permit() -> Option<Permit> {
    REQUEST
        .try_with(Clone::clone)
        .unwrap_or_else(|_| BLOCKING_REQUEST.with(|slot| slot.borrow().clone()))
}

pub(crate) fn carry<F, R>(f: F) -> impl FnOnce() -> R
where
    F: FnOnce() -> R,
{
    let permit = request_permit();
    move || {
        let _scope = BlockingScope::enter(permit);
        f()
    }
}

/// Carry authority into a blocking worker and restore its prior scope even
/// on panic. Worker Permit ownership retains execution capacity until exit.
pub(crate) fn blocking<F, R>(f: F) -> tauri::async_runtime::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(carry(f))
}

/// The async action keeps its request fence across waits and synchronous
/// broker calls, including calls made through the app action catalogue.
pub(crate) async fn with_request<F: Future>(permit: Option<Permit>, future: F) -> F::Output {
    REQUEST.scope(permit, future).await
}

pub(crate) fn request_current() -> bool {
    request_permit().as_ref().is_none_or(Permit::current)
}

struct Grant {
    server: String,
    run: String,
    expires: Instant,
    active: AtomicBool,
    revoked: AtomicBool,
    changed: tokio::sync::watch::Sender<bool>,
    handler: Handler,
    timeout: Duration,
    live: Live,
}

/// Passed to broker adapters. Check after waits and immediately before effects;
/// dropping an HTTP future cannot by itself stop a blocking broker operation.
#[derive(Clone)]
pub(crate) struct Permit(Arc<Grant>, Option<Arc<RequestState>>);

struct RequestState {
    cancelled: AtomicBool,
    deadline: Instant,
    // Retained by every blocking adapter's Permit clone until it really exits.
    _execution: tokio::sync::OwnedSemaphorePermit,
}

struct RequestGuard(Arc<RequestState>);

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
}

impl Permit {
    pub(crate) fn revoke(&self) {
        self.0.revoked.store(true, Ordering::Release);
        self.0.changed.send_replace(true);
    }
    fn grant_valid(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
            && !self.0.revoked.load(Ordering::Acquire)
            && Instant::now() < self.0.expires
            && (self.0.live)(&self.0.run)
    }

    pub(crate) fn valid(&self) -> bool {
        self.current() && (self.0.live)(&self.0.run)
    }

    /// For a broker already holding History's live-run lock. Reentering its
    /// live predicate there would deadlock; the lock already fences the run.
    pub(crate) fn current(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
            && !self.0.revoked.load(Ordering::Acquire)
            && Instant::now() < self.0.expires
            && self.1.as_ref().is_none_or(|request| {
                !request.cancelled.load(Ordering::Acquire) && Instant::now() < request.deadline
            })
    }
}

/// Approval authority travels with this request, including its cancellation.
#[derive(Clone)]
pub(crate) struct Approval {
    pub core: crate::core_host::Core,
    pub session: String,
    pub run: String,
    pub permit: Permit,
}

impl Approval {
    pub(crate) fn valid(&self) -> bool {
        self.permit.valid()
    }
    pub(crate) fn current(&self) -> bool {
        self.permit.current()
    }

    pub(crate) async fn confirm(
        &self,
        title: &str,
        action: &str,
        kind: &str,
        tool: &str,
        args: &Value,
    ) -> Result<(), String> {
        let request = self.clone();
        let (title, action, kind, tool, args) = (
            title.to_string(),
            action.to_string(),
            kind.to_string(),
            tool.to_string(),
            args.clone(),
        );
        let allowed = tokio::task::spawn_blocking(move || {
            let card = crate::approvals::InProcessCard {
                title: &title,
                action: &action,
                kind: &kind,
                tool: Some(&tool),
                args: &args,
            };
            let verdict = crate::approvals::ask_app_scoped(
                &request.core,
                card,
                &|| !request.valid(),
                &request.session,
                &request.run,
            );
            request.valid() && verdict == crate::approvals::Verdict::Allowed
        })
        .await
        .unwrap_or(false);
        if allowed && self.valid() {
            Ok(())
        } else {
            Err("Not approved or this attempt ended; nothing ran.".into())
        }
    }
}

/// One attempt owns this value from command construction through cleanup.
/// Failed spawn and abandoned construction revoke on Drop, too.
pub(crate) struct Capability {
    token: String,
    grant: Arc<Grant>,
}

impl Capability {
    pub(crate) fn header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    pub(crate) fn activate(&self) {
        if !self.grant.revoked.load(Ordering::Acquire) {
            self.grant.active.store(true, Ordering::Release);
        }
    }

    pub(crate) fn revoke(&self) {
        self.grant.revoked.store(true, Ordering::Release);
        self.grant.changed.send_replace(true);
    }
}

impl Drop for Capability {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub(crate) struct Registry(
    Mutex<HashMap<String, Weak<Grant>>>,
    Arc<tokio::sync::Semaphore>,
);

impl Default for Registry {
    fn default() -> Self {
        Self(
            Mutex::new(HashMap::new()),
            Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)),
        )
    }
}

impl Registry {
    fn request(&self, mut permit: Permit) -> Option<(Permit, RequestGuard)> {
        let execution = self.1.clone().try_acquire_owned().ok()?;
        let state = Arc::new(RequestState {
            cancelled: AtomicBool::new(false),
            deadline: (Instant::now() + permit.0.timeout).min(permit.0.expires),
            _execution: execution,
        });
        permit.1 = Some(state.clone());
        Some((permit, RequestGuard(state)))
    }

    pub(crate) fn register(
        &self,
        server: &str,
        run: &str,
        live: Live,
        timeout: Duration,
        handler: Handler,
    ) -> Option<Capability> {
        if run.is_empty()
            || server.is_empty()
            || !server
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return None;
        }
        let mut grants = self.0.lock().unwrap_or_else(|p| p.into_inner());
        grants.retain(|_, weak| {
            weak.upgrade()
                .is_some_and(|g| !g.revoked.load(Ordering::Acquire) && Instant::now() < g.expires)
        });
        if grants.len() >= MAX_GRANTS {
            return None;
        }
        let mut bytes = [0_u8; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes).ok()?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let (changed, _) = tokio::sync::watch::channel(false);
        let grant = Arc::new(Grant {
            server: server.into(),
            run: run.into(),
            expires: Instant::now() + TOKEN_LIFETIME,
            active: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
            changed,
            handler,
            timeout: timeout.min(Duration::from_secs(480)),
            live,
        });
        grants.insert(token.clone(), Arc::downgrade(&grant));
        Some(Capability { token, grant })
    }

    fn authorize(&self, path: &str, bearer: &str) -> Option<Permit> {
        let token = bearer.strip_prefix("Bearer ")?;
        let grant = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(token)?
            .upgrade()?;
        let permit = Permit(grant, None);
        (path == format!("/mcp/{}", permit.0.server) && permit.valid()).then_some(permit)
    }
}

/// Retain for the host lifetime. Dropping the server ends its listener.
pub(crate) struct Server {
    pub(crate) port: u16,
    stop: tokio::sync::watch::Sender<bool>,
    registry: Arc<Registry>,
}

impl Drop for Server {
    fn drop(&mut self) {
        for grant in self
            .registry
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter_map(Weak::upgrade)
        {
            grant.revoked.store(true, Ordering::Release);
            grant.changed.send_replace(true);
        }
        let _ = self.stop.send(true);
    }
}

pub(crate) async fn start(registry: Arc<Registry>) -> Result<Server, String> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| format!("bind shared MCP: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let serving = registry.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = stopped.changed() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, peer)) = accepted else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                continue;
            };
            let registry = serving.clone();
            let mut closing = stopped.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let mut builder = http1::Builder::new();
                builder
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(5))
                    .max_buf_size(32 * 1024);
                let connection = builder.serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |req| respond(req, registry.clone(), port)),
                );
                tokio::select! {
                    _ = closing.changed() => {},
                    _ = tokio::time::sleep(CONNECTION_LIFETIME) => {},
                    _ = connection => {},
                }
            });
        }
    });
    Ok(Server {
        port,
        stop,
        registry,
    })
}

fn response(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header("Cache-Control", "no-store")
        .body(Full::new(body.into()))
        .expect("static response headers")
}

async fn respond(
    req: Request<Incoming>,
    registry: Arc<Registry>,
    port: u16,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let authority = format!("127.0.0.1:{port}");
    if req.headers().get(HOST).and_then(|h| h.to_str().ok()) != Some(authority.as_str())
        || req.headers().get(ORIGIN).is_some_and(|origin| {
            origin.to_str().ok() != Some(format!("http://{authority}").as_str())
        })
        || req.uri().query().is_some()
    {
        return Ok(response(StatusCode::FORBIDDEN, "{}"));
    }
    let bearer = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let Some(permit) = registry.authorize(req.uri().path(), bearer) else {
        return Ok(response(StatusCode::UNAUTHORIZED, "{}"));
    };
    if req.method() != Method::POST {
        return Ok(response(StatusCode::METHOD_NOT_ALLOWED, "{}"));
    }
    if !req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim() == "application/json")
        })
    {
        return Ok(response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "{}"));
    }
    let body = tokio::time::timeout(
        Duration::from_secs(5),
        Limited::new(req.into_body(), BODY_MAX).collect(),
    )
    .await;
    let bytes = match body {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => return Ok(response(StatusCode::PAYLOAD_TOO_LARGE, "{}")),
        Err(_) => return Ok(response(StatusCode::REQUEST_TIMEOUT, "{}")),
    };
    if !permit.valid() {
        return Ok(response(StatusCode::UNAUTHORIZED, "{}"));
    }
    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(response(StatusCode::BAD_REQUEST, "{}"));
    };
    // One message per request. Batch mutation and notification semantics must
    // not bypass a server's existing checks.
    if !message.is_object() {
        return Ok(response(StatusCode::BAD_REQUEST, "{}"));
    }
    let Some((permit, _request)) = registry.request(permit) else {
        return Ok(response(StatusCode::TOO_MANY_REQUESTS, "{}"));
    };
    let mut revoked = permit.0.changed.subscribe();
    if *revoked.borrow() || !permit.grant_valid() {
        return Ok(response(StatusCode::UNAUTHORIZED, "{}"));
    }
    if !permit.valid() {
        return Ok(response(StatusCode::GATEWAY_TIMEOUT, "{}"));
    }
    let timeout = permit
        .0
        .timeout
        .min(permit.0.expires.saturating_duration_since(Instant::now()));
    let operation = (permit.0.handler)(message, permit.clone());
    let answer = tokio::select! {
        _ = revoked.changed() => return Ok(response(StatusCode::UNAUTHORIZED, "{}")),
        answer = tokio::time::timeout(timeout, operation) => answer,
    };
    if !permit.grant_valid() {
        return Ok(response(StatusCode::UNAUTHORIZED, "{}"));
    }
    if answer.is_err() || !permit.valid() {
        return Ok(response(StatusCode::GATEWAY_TIMEOUT, "{}"));
    }
    Ok(match answer {
        Ok(Some(answer)) => response(StatusCode::OK, answer.to_string()),
        Ok(None) => response(StatusCode::ACCEPTED, Bytes::new()),
        Err(_) => response(StatusCode::GATEWAY_TIMEOUT, "{}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(registry: &Registry, live: Arc<AtomicBool>) -> Capability {
        registry
            .register(
                "hollow_app",
                "same-history-run",
                Arc::new(move |_| live.load(Ordering::Acquire)),
                Duration::from_secs(1),
                Arc::new(|value, permit| {
                    Box::pin(async move {
                        assert!(permit.valid());
                        Some(serde_json::json!({"jsonrpc":"2.0", "id":value["id"], "result":{}}))
                    })
                }),
            )
            .unwrap()
    }

    #[test]
    fn capability_is_attempt_scoped_and_revoked_on_drop() {
        let registry = Registry::default();
        let live = Arc::new(AtomicBool::new(true));
        let first = capability(&registry, live.clone());
        let second = capability(&registry, live.clone());
        let path = "/mcp/hollow_app";
        assert_eq!(first.token.len(), 64);
        assert_ne!(first.token, second.token);
        assert!(registry.authorize(path, &first.header()).is_none());
        first.activate();
        second.activate();
        let permit = registry.authorize(path, &first.header()).unwrap();
        assert!(registry
            .authorize("/mcp/hollow_tasks", &first.header())
            .is_none());
        assert!(registry.authorize(path, "Bearer invalid").is_none());
        first.revoke();
        first.activate();
        assert!(!permit.valid());
        assert!(registry.authorize(path, &second.header()).is_some());
        live.store(false, Ordering::Release);
        assert!(registry.authorize(path, &second.header()).is_none());
        live.store(true, Ordering::Release);
        let header = second.header();
        drop(second);
        assert!(registry.authorize(path, &header).is_none());
    }

    #[test]
    fn dropped_request_cancels_blocking_permits_and_retains_execution_capacity() {
        let registry = Registry::default();
        let capability = capability(&registry, Arc::new(AtomicBool::new(true)));
        capability.activate();
        let permit = registry
            .authorize("/mcp/hollow_app", &capability.header())
            .unwrap();
        let (permit, request) = registry.request(permit).unwrap();
        let blocking_worker = permit.clone();
        assert!(blocking_worker.valid());
        drop(request);
        drop(permit);
        assert!(!blocking_worker.valid());
        assert_eq!(registry.1.available_permits(), MAX_CONNECTIONS - 1);
        drop(blocking_worker);
        assert_eq!(registry.1.available_permits(), MAX_CONNECTIONS);
    }

    #[tokio::test]
    async fn nested_blocking_actions_recheck_revocation_after_waiting() {
        let registry = Registry::default();
        let capability = capability(&registry, Arc::new(AtomicBool::new(true)));
        capability.activate();
        let permit = registry
            .authorize("/mcp/hollow_app", &capability.header())
            .unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = REQUEST.sync_scope(Some(permit), || {
            blocking(move || {
                assert!(request_current());
                started.send(()).unwrap();
                wait.recv().unwrap();
                // This is the same leaf check History makes after its lock.
                request_current()
            })
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        capability.revoke();
        release.send(()).unwrap();
        assert!(!worker.await.unwrap());
        assert!(request_current(), "the calling task has no retained scope");
    }

    #[tokio::test]
    async fn request_timeout_is_not_capability_revocation() {
        for timeout in [Duration::ZERO, Duration::from_millis(10)] {
            let registry = Arc::new(Registry::default());
            let capability = registry
                .register(
                    "hollow_app",
                    "timeout-run",
                    Arc::new(|_| true),
                    timeout,
                    Arc::new(|_, _permit| {
                        Box::pin(async {
                            std::future::pending::<()>().await;
                            None
                        })
                    }),
                )
                .unwrap();
            capability.activate();
            let server = start(registry.clone()).await.unwrap();
            let url = format!("http://127.0.0.1:{}/mcp/hollow_app", server.port);
            for _ in 0..2 {
                let response = reqwest::Client::new()
                    .post(&url)
                    .header(AUTHORIZATION, capability.header())
                    .json(&serde_json::json!({"id":1,"method":"tools/call"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
                assert!(registry
                    .authorize("/mcp/hollow_app", &capability.header())
                    .is_some());
                assert_eq!(registry.1.available_permits(), MAX_CONNECTIONS);
            }
        }
    }

    #[tokio::test]
    async fn every_http_method_requires_live_authority_and_host_origin() {
        let registry = Arc::new(Registry::default());
        let live = Arc::new(AtomicBool::new(true));
        let capability = capability(&registry, live.clone());
        let server = start(registry.clone()).await.unwrap();
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/mcp/hollow_app", server.port);
        let send = |method: &str, header: String, origin: Option<&str>| {
            let mut request = client
                .post(&url)
                .header(AUTHORIZATION, header)
                .json(&serde_json::json!({"jsonrpc":"2.0", "id":1, "method":method}));
            if let Some(origin) = origin {
                request = request.header(ORIGIN, origin);
            }
            request.send()
        };
        assert_eq!(
            send("initialize", capability.header(), None)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        capability.activate();
        for method in ["initialize", "tools/list", "ping", "tools/call", "unknown"] {
            assert_eq!(
                send(method, "Bearer invalid".into(), None)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                send(method, capability.header(), None)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
        assert_eq!(
            send(
                "initialize",
                capability.header(),
                Some("https://foreign.example")
            )
            .await
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(&url)
                .header(HOST, "foreign.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(&url)
                .header(AUTHORIZATION, capability.header())
                .json(&vec![serde_json::json!({})])
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            client
                .post(&url)
                .header(AUTHORIZATION, capability.header())
                .header(CONTENT_TYPE, "application/json")
                .body(" ".repeat(BODY_MAX + 1))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        live.store(false, Ordering::Release);
        assert_eq!(
            send("tools/list", capability.header(), None)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        live.store(true, Ordering::Release);
        drop(server);
        assert!(registry
            .authorize("/mcp/hollow_app", &capability.header())
            .is_none());
    }
}
