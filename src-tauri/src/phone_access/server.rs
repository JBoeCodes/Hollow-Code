//! The phone API's HTTP/1.1 server, on hyper.
//!
//! One listener on `127.0.0.1` (never another address: `tailscale serve`
//! is the only way in from the tailnet). Connections are served
//! concurrently, at most [`MAX_CONNECTIONS`] at once, with keep-alive; each
//! request's headers must arrive within [`HEADER_WAIT`] (which also closes
//! an idle kept-alive connection) and its body within [`BODY_WAIT`], at most
//! [`routes::BODY_MAX`] bytes (413 past it). A write that waits on the peer
//! for [`WRITE_STALL`] without a byte going out closes the connection, so a
//! peer that stopped reading frees its slot. Every request passes the Host
//! check (421), the tailnet identity check (403: never Funnel, and only this
//! Mac's own Tailscale user unless it came straight from this Mac) and,
//! except `health` and `pair`, the device token (401) when its headers
//! arrive and again once its body has (a phone revoked while its body was
//! still coming gets the same 401) before its route.
//! Every response says `Cache-Control: no-store`.
//!
//! M2: another Mac comes in over an SSH tunnel, which lands on loopback, so
//! machine pairing and a machine token are taken only straight from this
//! Mac (no `Tailscale-User-Login`, no proxy's `X-Forwarded-*`): a tailnet
//! request is refused 403 before a code is tried or a token used.

use super::attachments;
use super::routes::{self, ApiError, Ctx, Reply};
use super::store::{Device, DeviceKind};
use super::tailscale::{HostName, Identity};
use http_body_util::{combinators::BoxBody, BodyExt, Full, Limited};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{HeaderMap, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The phone API's port on loopback. `tailscale serve` names it, so there
/// is no fallback.
pub(crate) const PORT: u16 = 47680;
pub(crate) const MAX_CONNECTIONS: usize = 64;
pub(crate) const HEADER_WAIT: Duration = Duration::from_secs(30);
pub(crate) const BODY_WAIT: Duration = Duration::from_secs(30);
/// What a refusal that comes ahead of an upload's body reads and drops first.
const DRAIN_MAX: usize = attachments::FILE_MAX + 1024 * 1024;
const DRAIN_WAIT: Duration = Duration::from_secs(15);
/// A write that has waited on the peer this long without a byte going out
/// closes its connection (second review, finding 5, 2026-11-04: with no
/// write timeout a peer that stopped reading kept one of the
/// [`MAX_CONNECTIONS`] slots, after the hub had dropped its stream, until
/// TCP gave up). A slow peer that keeps taking bytes is never cut off.
pub(crate) const WRITE_STALL: Duration = Duration::from_secs(60);
/// Bytes buffered per connection while reading a request.
const READ_BUFFER: usize = 64 * 1024;

/// How long a write may wait on the peer, and how often a connection looks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stall {
    pub limit: Duration,
    pub check: Duration,
}

impl Stall {
    pub(crate) const SERVER: Stall = Stall {
        limit: WRITE_STALL,
        check: Duration::from_secs(1),
    };
}

/// How a connection's serving ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The connection ended (the peer or hyper closed it).
    Done,
    /// The server stopped.
    Stopped,
    /// A write waited on the peer for [`Stall::limit`].
    Stalled,
}

/// A connection's socket, noting since when a write has waited on the peer
/// (`None` while nothing waits).
struct Watched<T> {
    io: T,
    waiting: Arc<Mutex<Option<Instant>>>,
}

impl<T> Watched<T> {
    /// A write, flush or shutdown polled: `Pending` waits on the peer (from
    /// the first time, not each poll); anything else moved.
    fn note<R>(&self, polled: &Poll<R>) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        if polled.is_pending() {
            waiting.get_or_insert_with(Instant::now);
        } else {
            *waiting = None;
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Watched<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Watched<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_write(cx, buf);
        this.note(&polled);
        polled
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_write_vectored(cx, bufs);
        this.note(&polled);
        polled
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_flush(cx);
        this.note(&polled);
        polled
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_shutdown(cx);
        this.note(&polled);
        polled
    }
}

/// Resolves once a write has waited on the peer for `stall.limit`.
async fn stalled(waiting: &Mutex<Option<Instant>>, stall: Stall) {
    loop {
        tokio::time::sleep(stall.check).await;
        let since = *waiting.lock().unwrap_or_else(|e| e.into_inner());
        if since.is_some_and(|at| at.elapsed() >= stall.limit) {
            return;
        }
    }
}

/// Serve one connection until it ends, the server stops (`closing`), or a
/// write has waited on the peer for `stall.limit`; then it closes, freeing
/// its slot.
pub(crate) async fn serve_one<T>(
    io: T,
    ctx: Arc<Ctx>,
    gate: Arc<Gate>,
    mut closing: tokio::sync::watch::Receiver<bool>,
    stall: Stall,
) -> Ended
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let waiting = Arc::new(Mutex::new(None));
    let io = Watched {
        io,
        waiting: waiting.clone(),
    };
    let service = service_fn(move |request| {
        let (ctx, gate) = (ctx.clone(), gate.clone());
        async move { Ok::<_, Infallible>(answer(&ctx, &gate, request).await) }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_WAIT)
        .keep_alive(true)
        .max_buf_size(READ_BUFFER);
    let connection = builder.serve_connection(TokioIo::new(io), service);
    tokio::select! {
        _ = connection => Ended::Done,
        _ = closing.changed() => Ended::Stopped,
        _ = stalled(&waiting, stall) => Ended::Stalled,
    }
}

type Body = BoxBody<Bytes, Infallible>;

/// An event stream's body: frames from the live hub until it lets go.
struct Events {
    rx: super::live::FrameRx,
}

impl hyper::body::Body for Events {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.rx
            .poll_recv(cx)
            .map(|frame| frame.map(|bytes| Ok(Frame::data(bytes))))
    }
}

/// A running server: dropping it (or [`Server::stop`]) closes the listener
/// and every connection.
pub(crate) struct Server {
    pub addr: SocketAddr,
    stop: tokio::sync::watch::Sender<bool>,
}

impl Server {
    /// Bind `127.0.0.1:port` (0 picks a free port: tests).
    pub(crate) async fn bind(port: u16) -> Result<tokio::net::TcpListener, String> {
        tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("Port {port} on this Mac is already in use, so the iPhone app can't listen. Quit what uses it, then turn the iPhone app off and on.")
                } else {
                    format!("Hollow couldn't listen on 127.0.0.1:{port}: {e}")
                }
            })
    }

    /// Serve `listener` until stopped.
    pub(crate) fn start(
        listener: tokio::net::TcpListener,
        ctx: Arc<Ctx>,
        identity: Arc<Identity>,
    ) -> Result<Server, String> {
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let permits = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        let gate = Arc::new(Gate::new(identity, addr.port()));
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stopped.changed() => return,
                    accepted = listener.accept() => accepted,
                };
                let (stream, peer) = match accepted {
                    Ok(pair) => pair,
                    Err(_) => {
                        // Out of descriptors or similar: wait, then go on.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                // Bound to loopback, so only this Mac connects; checked anyway.
                if !peer.ip().is_loopback() {
                    continue;
                }
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let (ctx, gate) = (ctx.clone(), gate.clone());
                let closing = stopped.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    serve_one(stream, ctx, gate, closing, Stall::SERVER).await;
                });
            }
        });
        Ok(Server { addr, stop })
    }

    pub(crate) fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// Changes once the server stops (or is dropped).
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<bool> {
        self.stop.subscribe()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Who may call: the Host and tailnet identity checks.
pub(crate) struct Gate {
    identity: Arc<Identity>,
    port: u16,
}

impl Gate {
    /// The checks for a server listening on loopback `port`.
    pub(crate) fn new(identity: Arc<Identity>, port: u16) -> Self {
        Self { identity, port }
    }
}

fn header(headers: &HeaderMap, name: impl hyper::header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The bearer token, if the request carries one.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = header(headers, AUTHORIZATION)?.trim();
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

async fn answer(ctx: &Arc<Ctx>, gate: &Gate, request: Request<Incoming>) -> Response<Body> {
    respond(reply(ctx, gate, request).await)
}

/// Whether a proxy passed the request on: `tailscale serve` sets
/// `X-Forwarded-For` (the tailnet address) and `X-Forwarded-Host` on
/// everything it proxies, whatever `Host` the caller sent.
fn forwarded(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for") || headers.contains_key("x-forwarded-host")
}

/// The Host check (421) and who may call (403): never Funnel's traffic
/// (`Tailscale-Funnel-Request`, the internet rather than the tailnet), and
/// from the tailnet only this Mac's own user. `tailscale serve` names the
/// user on every request it passes on from one (`Tailscale-User-Login`) and
/// leaves the header out for a tagged node and for Funnel, so a request
/// without it is refused unless it came straight from this Mac: addressed
/// to the loopback name and passed on by no proxy. A raw `tailscale serve
/// --tcp` forward would pass as that, since it passes on nothing of the
/// caller, so Settings shows one as a problem: Stop publishing removes a
/// saved one, and Settings says how to stop a foreground or service one
/// (`tailscale::ServeInfo`).
fn who_may_call(gate: &Gate, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(name) =
        header(headers, HOST).and_then(|host| gate.identity.host_name(host, gate.port))
    else {
        return Err(ApiError::misdirected());
    };
    if headers.contains_key("tailscale-funnel-request") {
        return Err(ApiError::funnel());
    }
    match headers.get("tailscale-user-login") {
        Some(login) if gate.identity.login_ok(login.to_str().unwrap_or("")) => Ok(()),
        Some(_) => Err(ApiError::forbidden()),
        None if name == HostName::Loopback && !forwarded(headers) => Ok(()),
        None => Err(ApiError::forbidden()),
    }
}

/// M2: the request came straight from this Mac to its loopback name (an
/// SSH tunnel's end), not through `tailscale serve` or another proxy.
fn straight_from_this_mac(headers: &HeaderMap) -> bool {
    !forwarded(headers)
        && !headers.contains_key("tailscale-user-login")
        && !headers.contains_key("tailscale-funnel-request")
}

async fn reply(ctx: &Arc<Ctx>, gate: &Gate, request: Request<Incoming>) -> Reply {
    let headers = request.headers();
    if let Err(error) = who_may_call(gate, headers) {
        return error.into();
    }
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let mut query = request.uri().query().unwrap_or("").to_string();
    // AG5: a stream resumes from `Last-Event-ID`; it reaches the route as the
    // first `lastEventId` in the query, so the header beats a query value.
    if method == "GET" && path.trim_end_matches('/') == "/v1/events" {
        if let Some(id) = header(headers, "last-event-id") {
            // An id is hex digits and dashes; anything else resumes nothing.
            let id: String =
                if id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
                    id.to_string()
                } else {
                    "bad".into()
                };
            query = format!("lastEventId={id}&{query}");
        }
    }
    let machine_pair = method == "POST" && path.trim_end_matches('/') == "/v1/machines/pair";
    if machine_pair && !straight_from_this_mac(headers) {
        return ApiError::loopback_only().into();
    }
    let open = machine_pair
        || matches!(
            (method.as_str(), path.trim_end_matches('/')),
            ("GET", "/v1/health") | ("POST", "/v1/pair")
        );
    let (device, token) = if open {
        (None, None)
    } else {
        let Some(token) = bearer(headers) else {
            return ApiError::unauthorized().into();
        };
        match ctx.store.device_for_token(token) {
            Ok(Some(device)) => {
                if device.kind == DeviceKind::Machine && !straight_from_this_mac(headers) {
                    return ApiError::loopback_only().into();
                }
                if device.kind == DeviceKind::Phone && ctx.phones_off.load(Ordering::SeqCst) {
                    return ApiError::phones_off().into();
                }
                let (store, id) = (ctx.store.clone(), device.device_id.clone());
                tokio::task::spawn_blocking(move || store.touch(&id, chrono::Utc::now()));
                (Some(device), Some(token.to_string()))
            }
            Ok(None) => return ApiError::unauthorized().into(),
            Err(error) => return ApiError::unavailable(error).into(),
        }
    };
    // M2: who may call this route is settled before any body is read or an
    // upload place taken, so a Mac's POST to a route it may not use never
    // holds a place or buffers a file; its body is drained, as the other
    // refusals ahead of the body do. `dispatch` checks again.
    if let Some(device) = &device {
        let parts: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
        if let Err(refused) = routes::may_call(device.kind, &method, &parts) {
            drain(request.into_body()).await;
            return refused.into();
        }
    }
    let declared = header(headers, CONTENT_LENGTH).and_then(|n| n.trim().parse::<u64>().ok());
    // v1.2: an upload's body is a file (up to `attachments::FILE_MAX`), held
    // in memory while it arrives and while it is saved, so Hollow takes a few
    // at a time. The permit is taken after the token check and goes with the
    // file's bytes once they have arrived (`attachments::with_place`): it is
    // given back when the save ends, not when the phone's connection closes
    // (hyper drops this handler then, and the save runs on in its own task).
    // A file whose declared size is over the limit is refused before it takes
    // a place, and a phone that finds Hollow full waits up to
    // `attachments::PLACE_WAIT` for one. A refusal that comes before the body
    // is read drains the body first ([`drain`]): a reply sent over bytes the
    // phone is still sending can be lost to the connection's reset.
    let upload = routes::is_upload(&method, &path);
    let place = if upload {
        if declared.is_some_and(|n| n > attachments::FILE_MAX as u64) {
            drain(request.into_body()).await;
            return attachments::too_large().into();
        }
        let place = ctx.uploading.clone().acquire_owned();
        match tokio::time::timeout(attachments::PLACE_WAIT, place).await {
            Ok(Ok(permit)) => Some(permit),
            _ => {
                drain(request.into_body()).await;
                return ApiError::unavailable(
                    "Hollow is taking other files. Try again in a moment.",
                )
                .into();
            }
        }
    } else {
        None
    };
    let body = if method == "POST" || method == "PUT" || method == "PATCH" {
        match read_body(request.into_body(), declared, upload).await {
            Ok(body) => body,
            Err(error) => return error.into(),
        }
    } else {
        Bytes::new()
    };
    // The token was checked when the headers came, and a body may take up to
    // `BODY_WAIT` (an upload `attachments::UPLOAD_WAIT`) to follow: a phone
    // revoked meanwhile must not have its send queued or its file saved, so
    // the pairing is checked again once the body is in, before any route
    // runs (Q-R20).
    let device = match (device, token) {
        (Some(device), Some(token)) => match still_paired(ctx, &token, &device.device_id) {
            Ok(device) => Some(device),
            Err(error) => return error.into(),
        },
        _ => None,
    };
    let body = match place {
        Some(place) => attachments::with_place(body, place),
        None => body,
    };
    routes::dispatch(ctx, device.as_ref(), &method, &path, &query, body).await
}

/// The phone `token` names, checked again after its request's body arrived:
/// still paired, and still the same phone. A revoked one (or a token now
/// naming another phone) gets the same 401 as a token Hollow never issued.
fn still_paired(ctx: &Ctx, token: &str, device_id: &str) -> Result<Device, ApiError> {
    match ctx.store.device_for_token(token) {
        Ok(Some(device)) if device.device_id == device_id => Ok(device),
        Ok(_) => Err(ApiError::unauthorized()),
        Err(error) => Err(ApiError::unavailable(error)),
    }
}

/// What is left of a body is read and dropped, within [`DRAIN_MAX`] bytes and
/// [`DRAIN_WAIT`], before a refusal that comes ahead of the body.
async fn drain(body: Incoming) {
    let mut body = Limited::new(body, DRAIN_MAX);
    let _ = tokio::time::timeout(DRAIN_WAIT, async {
        while let Some(frame) = body.frame().await {
            if frame.is_err() {
                break;
            }
        }
    })
    .await;
}

/// An upload's file: at most [`attachments::FILE_MAX`] bytes, within
/// [`attachments::UPLOAD_WAIT`] in all and with no more than
/// [`attachments::UPLOAD_IDLE`] between two chunks, so a stalled upload frees
/// its place.
async fn read_upload(body: Incoming) -> Result<Bytes, ApiError> {
    let mut body = Limited::new(body, attachments::FILE_MAX);
    let mut data: Vec<u8> = Vec::new();
    let read = async {
        loop {
            match tokio::time::timeout(attachments::UPLOAD_IDLE, body.frame()).await {
                Err(_) => return Err(ApiError::bad_request("The file stopped arriving.")),
                Ok(None) => return Ok(()),
                Ok(Some(Err(error))) => {
                    return Err(
                        if error
                            .downcast_ref::<http_body_util::LengthLimitError>()
                            .is_some()
                        {
                            attachments::too_large()
                        } else {
                            ApiError::bad_request("The request body couldn't be read.")
                        },
                    )
                }
                Ok(Some(Ok(frame))) => {
                    if let Ok(chunk) = frame.into_data() {
                        data.extend_from_slice(&chunk);
                    }
                }
            }
        }
    };
    match tokio::time::timeout(attachments::UPLOAD_WAIT, read).await {
        Err(_) => Err(ApiError::bad_request(
            "The request body didn't arrive in time.",
        )),
        Ok(result) => result.map(|()| Bytes::from(data)),
    }
}

/// A request body: at most [`routes::BODY_MAX`] bytes within [`BODY_WAIT`], or
/// for an upload a file ([`read_upload`]).
async fn read_body(body: Incoming, declared: Option<u64>, upload: bool) -> Result<Bytes, ApiError> {
    if upload {
        return read_upload(body).await;
    }
    let (max, wait) = (routes::BODY_MAX, BODY_WAIT);
    let too_large = || ApiError::too_large("That request is larger than Hollow takes (256 KiB).");
    if declared.is_some_and(|n| n > max as u64) {
        return Err(too_large());
    }
    match tokio::time::timeout(wait, Limited::new(body, max).collect()).await {
        Err(_) => Err(ApiError::bad_request(
            "The request body didn't arrive in time.",
        )),
        Ok(Err(error)) => {
            if error
                .downcast_ref::<http_body_util::LengthLimitError>()
                .is_some()
            {
                Err(too_large())
            } else {
                Err(ApiError::bad_request("The request body couldn't be read."))
            }
        }
        Ok(Ok(collected)) => Ok(collected.to_bytes()),
    }
}

fn respond(reply: Reply) -> Response<Body> {
    let builder = Response::builder().header(CACHE_CONTROL, "no-store");
    let response = match reply {
        Reply::Json { status, body } => builder
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(body).boxed()),
        Reply::Events(rx) => builder
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header("x-accel-buffering", "no")
            .body(Events { rx }.boxed()),
        Reply::Empty(status) => builder
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::NO_CONTENT))
            .body(Full::new(Bytes::new()).boxed()),
        // v1.2: an attachment's bytes. The type is one Hollow worked out
        // (never the client's), nothing may reinterpret it, and only a picture
        // is shown in place: any other file is a download, and a sandboxed
        // policy with no sources keeps a document a browser did open (an
        // `.xml` that is really an SVG) from running anything.
        Reply::Bytes {
            status,
            content_type,
            filename,
            inline,
            body,
        } => builder
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
            .header(CONTENT_TYPE, content_type)
            .header(
                "content-disposition",
                format!(
                    "{}; filename=\"{filename}\"",
                    if inline { "inline" } else { "attachment" }
                ),
            )
            .header("x-content-type-options", "nosniff")
            .header("content-security-policy", "default-src 'none'; sandbox")
            .body(Full::new(body).boxed()),
    };
    response.unwrap_or_else(|_| {
        let mut fallback = Response::new(Full::new(Bytes::new()).boxed());
        *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    })
}
