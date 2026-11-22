//! The Preview pane's web mode (2026-09-30 as the side panel's browser; the
//! app-wide Preview pane's since run 15 P1). Each browser tab is a native
//! child webview of the main window, laid over a placeholder `BrowserPane.tsx`
//! measures, so any site loads (an iframe is refused by most of the web) and
//! a tab keeps its page, scroll and sign-ins while it is hidden.
//!
//! Security: Tauri treats a page as *local*, and lets it call every app
//! command, when its URL is the app's own protocol, the dev URL, or a custom
//! scheme Hollow registered (`artifact:`). `classify` therefore lets a tab's
//! main frame and subframes reach only the web (http/https) plus the
//! document-internal `about:`/`data:`/`blob:`, and never those hosts. wry
//! consults the navigation handler for every frame, not just the top one.
//! Another app's link (run 15 P2) is handed to that app after a question,
//! never loaded in the tab.
//!
//! The webview owns the layout; Rust only creates, places, hides and closes
//! webviews, and reports what they load as `browser-event`. WebKit reports a
//! load only once the server answered (wry has no failed-load hook), so a
//! server on this Mac is probed before a tab opens it (`not_answering`) and
//! the pane gives any other page a time limit (`browserReview.ts`).

use serde::Serialize;
use std::collections::BTreeMap;
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

mod agent_cursor;

const LABEL_PREFIX: &str = "browser-";
/// Live tabs; each is a WKWebView with its own process, so keep it modest.
pub const MAX_TABS: usize = 8;
/// WKWebView's default user agent omits the `Version/… Safari/…` tail, and
/// some sites (Google sign-in among them) serve a degraded page without it.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Hand to the default app (mail, phone), never load in the tab.
    External,
    /// Another app's link (an editor, an app's sign-in callback): hand it to
    /// that app only after the person says yes to the exact address.
    Ask,
    Deny,
}

/// Schemes no page may hand anywhere: scripts, files, and Hollow's own
/// protocols (a tab on one would be treated as the app itself).
const NEVER_HANDED_OFF: &[&str] = &[
    "javascript",
    "vbscript",
    "file",
    "tauri",
    "asset",
    "ipc",
    "artifact",
    "hollow-media",
];

/// What a tab may load. `dev` is the dev-server origin, local only in dev.
pub fn classify(url: &Url, dev: Option<&Url>) -> Verdict {
    match url.scheme() {
        "http" | "https" => {
            let host = url.host_str().unwrap_or("");
            // Tauri's own hosts (the Windows/Android form of its protocols).
            if matches!(
                host,
                "tauri.localhost" | "ipc.localhost" | "asset.localhost" | "artifact.localhost"
            ) {
                return Verdict::Deny;
            }
            if dev.is_some_and(|d| d.origin() == url.origin()) {
                return Verdict::Deny;
            }
            Verdict::Allow
        }
        "about" | "data" | "blob" => Verdict::Allow,
        "mailto" | "tel" => Verdict::External,
        s if NEVER_HANDED_OFF.contains(&s) => Verdict::Deny,
        _ => Verdict::Ask,
    }
}

/// Why an address can't open in a tab, in words; None when it may.
fn refusal(url: &Url, dev: Option<&Url>) -> Option<String> {
    let web = matches!(url.scheme(), "http" | "https");
    match classify(url, dev) {
        Verdict::Allow if web || url.scheme() == "about" => None,
        Verdict::Deny if web && dev.is_some_and(|d| d.origin() == url.origin()) => Some(
            "That address is Hollow's own development server, which its browser never opens."
                .into(),
        ),
        Verdict::Deny if web => {
            Some("That address belongs to Hollow itself, so its browser never opens it.".into())
        }
        _ => Some(format!(
            "The browser opens web pages only (http or https), not {}.",
            url.scheme()
        )),
    }
}

/// Parse what the user typed into the address field (the webview normalizes
/// bare hosts and searches first) and check it may load.
fn parse_allowed(app: &AppHandle, raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("Not a web address: {e}"))?;
    match refusal(&url, dev_origin(app).as_ref()) {
        None => Ok(url),
        Some(why) => Err(why),
    }
}

/// How long a server on this Mac gets to accept a connection. A closed port
/// refuses at once; this bounds only a port that drops the connection.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// A page served by this Mac (localhost, 127.x, [::1], 0.0.0.0): the host to
/// probe and its port. Public hosts are never probed: a system proxy can
/// make a direct connection fail while the page itself loads.
fn loopback(url: &Url) -> Option<(String, u16)> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let port = url.port_or_known_default()?;
    let host = url
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return Some(("localhost".into(), port));
    }
    match host.parse::<IpAddr>().ok()? {
        IpAddr::V4(ip) if ip.is_unspecified() => Some(("127.0.0.1".into(), port)),
        IpAddr::V6(ip) if ip.is_unspecified() => Some(("::1".into(), port)),
        ip if ip.is_loopback() => Some((ip.to_string(), port)),
        _ => None,
    }
}

/// Whether anything accepts a connection at `host:port` (each address it
/// resolves to, `timeout` each).
fn answers(host: &str, port: u16, timeout: Duration) -> bool {
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

/// The whole probe's limit (name resolution included): past it the page is
/// left to the pane's own time limit.
pub const PROBE_BOUND: Duration = Duration::from_secs(2);

/// `not_answering` off the main thread (a sync command would run there and
/// stall the app while it waits), bounded by `PROBE_BOUND`.
async fn probe(url: &Url) -> Option<String> {
    let url = url.clone();
    let task = tauri::async_runtime::spawn_blocking(move || not_answering(&url));
    tokio::time::timeout(PROBE_BOUND, task)
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten()
}

/// Each tab's newest `browser_open`: one that a later open of the same tab,
/// or its close, replaced while it probed does nothing, so a slow probe can
/// never navigate back to an older address or bring a closed tab back.
static OPENS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());
static OPEN_SEQ: AtomicU64 = AtomicU64::new(0);

fn claim_open(tab: &str) -> u64 {
    let n = OPEN_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut opens) = OPENS.lock() {
        opens.insert(tab.to_string(), n);
    }
    n
}

fn newest_open(tab: &str, n: u64) -> bool {
    OPENS.lock().is_ok_and(|opens| opens.get(tab) == Some(&n))
}

fn forget_opens(tab: &str) {
    agent_cursor::forget(tab);
    for map in [&OPENS, &NAVIGATED] {
        if let Ok(mut m) = map.lock() {
            m.remove(tab);
        }
    }
}

/// The open or reload whose navigation each tab's webview runs now. Every
/// `browser-event` carries it as `gen`, and those commands return it, so the
/// pane applies exactly that load's events: an older page's never count, and
/// a fast page's that beat its command's reply wait for it (run 15 P2 review).
static NAVIGATED: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

fn mark_navigated(tab: &str, n: u64) {
    if let Ok(mut m) = NAVIGATED.lock() {
        m.insert(tab.to_string(), n);
    }
}

fn navigated(tab: &str) -> u64 {
    NAVIGATED
        .lock()
        .ok()
        .and_then(|m| m.get(tab).copied())
        .unwrap_or(0)
}

/// The words for a page on this Mac whose server isn't running, or None.
/// WebKit reports no failed load (wry has no hook for one), so without this
/// a stopped dev server is a blank page that never finishes loading.
fn not_answering(url: &Url) -> Option<String> {
    let (host, port) = loopback(url)?;
    if answers(&host, port, PROBE_TIMEOUT) {
        return None;
    }
    let shown = url.host_str().unwrap_or(&host);
    Some(format!(
        "Nothing is answering at {shown}:{port}. Start the server there, then try again."
    ))
}

/// At most this many links a minute go from pages to other apps: a page can
/// fire them in a loop, and a click can't be told from a script here.
pub const HAND_OFF_LIMIT: usize = 3;
pub const HAND_OFF_WINDOW: Duration = Duration::from_secs(60);
/// The longest other app's link the question shows whole (and so may open).
pub const HAND_OFF_MAX_CHARS: usize = 1000;

pub struct HandOffGate {
    opened: Vec<Instant>,
}

impl HandOffGate {
    pub const fn new() -> Self {
        Self { opened: Vec::new() }
    }
    pub fn admit(&mut self, now: Instant) -> bool {
        self.opened
            .retain(|t| now.saturating_duration_since(*t) < HAND_OFF_WINDOW);
        if self.opened.len() >= HAND_OFF_LIMIT {
            return false;
        }
        self.opened.push(now);
        true
    }
}

static HAND_OFFS: Mutex<HandOffGate> = Mutex::new(HandOffGate::new());
/// One question at a time; a page asking again meanwhile is dropped.
static ASKING: AtomicBool = AtomicBool::new(false);

/// Give a link a page asked for to the app that handles it: mail and phone
/// links at once (as before), any other app's only after a native question
/// naming the exact address. Never a scheme in `NEVER_HANDED_OFF`.
fn hand_off(app: &AppHandle, url: &Url, ask: bool) {
    use tauri_plugin_opener::OpenerExt;
    if !HAND_OFFS
        .lock()
        .map(|mut g| g.admit(Instant::now()))
        .unwrap_or(false)
    {
        return;
    }
    let target = url.to_string();
    if !ask {
        let _ = app.opener().open_url(target, None::<&str>);
        return;
    }
    // The question shows the whole address it asks about, so one too long to
    // show is never handed off.
    if target.chars().count() > HAND_OFF_MAX_CHARS || ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let opener = app.clone();
    app.dialog()
        .message(format!(
            "A page in Hollow's browser wants to open another app with this link:\n\n{target}"
        ))
        .title("Open another app?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Open".into(),
            "Cancel".into(),
        ))
        .show(move |ok| {
            ASKING.store(false, Ordering::SeqCst);
            if ok {
                let _ = opener.opener().open_url(target, None::<&str>);
            }
        });
}

fn dev_origin(app: &AppHandle) -> Option<Url> {
    if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    }
}

fn label(tab_id: &str) -> Result<String, String> {
    if tab_id.is_empty()
        || tab_id.len() > 40
        || !tab_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("bad browser tab id".into());
    }
    Ok(format!("{LABEL_PREFIX}{tab_id}"))
}

fn tab(app: &AppHandle, tab_id: &str) -> Result<Webview, String> {
    app.get_webview(&label(tab_id)?)
        .ok_or_else(|| "That browser tab is closed.".into())
}

fn tabs(app: &AppHandle) -> Vec<Webview> {
    app.webviews()
        .into_iter()
        .filter(|(l, _)| l.starts_with(LABEL_PREFIX))
        .map(|(_, w)| w)
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserEvent {
    tab_id: String,
    /// `loading` | `loaded` | `title` | `new-tab` | `download`
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
}

#[derive(Clone, Serialize)]
struct Tagged {
    #[serde(flatten)]
    ev: BrowserEvent,
    /// `navigated` when it was sent (on the main thread, like the change).
    gen: u64,
}

fn emit(app: &AppHandle, ev: BrowserEvent) {
    let gen = navigated(&ev.tab_id);
    let _ = app.emit_to("main", "browser-event", Tagged { ev, gen });
}

fn tab_id_of(webview: &Webview) -> String {
    webview.label().trim_start_matches(LABEL_PREFIX).to_string()
}

/// `~/Downloads/<name>`, numbered instead of overwriting an existing file.
fn download_target(dir: &Path, url: &Url) -> PathBuf {
    let raw = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download");
    let decoded = urlencoding::decode(raw)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| raw.to_string());
    let name: String = decoded
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = if name.starts_with('.') {
        format!("download{name}")
    } else {
        name
    };
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (1..1000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[derive(serde::Deserialize, Clone, Copy)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Bounds {
    fn size(&self) -> LogicalSize<f64> {
        LogicalSize::new(self.width.max(1.0), self.height.max(1.0))
    }
    fn position(&self) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x, self.y)
    }
}

/// Run `f` on the main thread, where `browser_close` runs too, and wait for
/// it: an ownership check (`newest_open`) and the change it guards happen
/// there in one step, so no close or newer open can come between them.
async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "The browser tab closed while it opened.".to_string())?
}

/// Open `url` in tab `tab_id`: create its webview at `bounds` (hidden until
/// the pane shows it) or navigate the existing one. Async, so the probe of a
/// server on this Mac never runs on the main thread (run 15 P2 review); a
/// later open of this tab, a Stop, a reload or its close owns the tab
/// instead (`claim_open`), and then this one does nothing (returns 0).
/// Returns the load's `gen` (`NAVIGATED`).
#[tauri::command]
pub async fn browser_open(
    app: AppHandle,
    tab_id: String,
    url: String,
    bounds: Bounds,
) -> Result<u64, String> {
    let target = parse_allowed(&app, &url)?;
    let open = claim_open(&tab_id);
    let refused = probe(&target).await;
    let main = app.clone();
    on_main(&app, move || {
        if !newest_open(&tab_id, open) {
            return Ok(0);
        }
        if let Some(why) = refused {
            return Err(why);
        }
        open_now(&main, &tab_id, target, bounds, open).map(|()| open)
    })
    .await
}

/// `browser_open`'s change, on the main thread once it owns the tab. The
/// load is `n` from here on: every event the navigation sends comes later.
fn open_now(
    app: &AppHandle,
    tab_id: &str,
    target: Url,
    bounds: Bounds,
    n: u64,
) -> Result<(), String> {
    let app = app.clone();
    let tab_id = tab_id.to_string();
    if let Some(existing) = app.get_webview(&label(&tab_id)?) {
        mark_navigated(&tab_id, n);
        return existing.navigate(target).map_err(|e| e.to_string());
    }
    // The pane closes its least recently shown tab first (browserReview.ts
    // `lruVictim`); this stays the backstop.
    if tabs(&app).len() >= MAX_TABS {
        return Err(format!("Close a browser tab first (at most {MAX_TABS})."));
    }
    let window = app.get_window("main").ok_or("The main window is closed.")?;
    let dev = dev_origin(&app);
    let new_dev = dev.clone();
    let nav_app = app.clone();
    let new_app = app.clone();
    let id = tab_id.clone();
    let builder = WebviewBuilder::new(label(&tab_id)?, WebviewUrl::External(target))
        .user_agent(USER_AGENT)
        .on_navigation(move |u| match classify(u, dev.as_ref()) {
            Verdict::Allow => true,
            Verdict::External => {
                hand_off(&nav_app, u, false);
                false
            }
            Verdict::Ask => {
                hand_off(&nav_app, u, true);
                false
            }
            Verdict::Deny => false,
        })
        .on_page_load(|webview, payload| {
            let kind = match payload.event() {
                PageLoadEvent::Started => "loading",
                PageLoadEvent::Finished => "loaded",
            };
            emit(
                webview.app_handle(),
                BrowserEvent {
                    tab_id: tab_id_of(&webview),
                    kind,
                    url: Some(payload.url().to_string()),
                    title: None,
                    path: None,
                    ok: None,
                },
            );
        })
        .on_document_title_changed(|webview, title| {
            emit(
                webview.app_handle(),
                BrowserEvent {
                    tab_id: tab_id_of(&webview),
                    kind: "title",
                    url: None,
                    title: Some(title),
                    path: None,
                    ok: None,
                },
            );
        })
        // A link or script asking for a new window opens a new tab in the
        // pane instead (the webview decides; a popup can't escape it).
        // Another app's link goes as a navigation's would.
        .on_new_window(move |u, _features| {
            match classify(&u, new_dev.as_ref()) {
                Verdict::Allow if matches!(u.scheme(), "http" | "https") => emit(
                    &new_app,
                    BrowserEvent {
                        tab_id: id.clone(),
                        kind: "new-tab",
                        url: Some(u.to_string()),
                        title: None,
                        path: None,
                        ok: None,
                    },
                ),
                Verdict::External => hand_off(&new_app, &u, false),
                Verdict::Ask => hand_off(&new_app, &u, true),
                _ => {}
            }
            NewWindowResponse::Deny
        })
        .on_download(|webview, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let Some(dir) = dirs::download_dir() else {
                        return false;
                    };
                    *destination = download_target(&dir, &url);
                    emit(
                        webview.app_handle(),
                        BrowserEvent {
                            tab_id: tab_id_of(&webview),
                            kind: "download",
                            url: Some(url.to_string()),
                            title: None,
                            path: Some(destination.display().to_string()),
                            ok: None,
                        },
                    );
                }
                DownloadEvent::Finished { url, success, .. } => {
                    emit(
                        webview.app_handle(),
                        BrowserEvent {
                            tab_id: tab_id_of(&webview),
                            kind: "download",
                            url: Some(url.to_string()),
                            title: None,
                            path: None,
                            ok: Some(success),
                        },
                    );
                }
                _ => {}
            }
            true
        });
    mark_navigated(&tab_id, n);
    let webview = window
        .add_child(builder, bounds.position(), bounds.size())
        .map_err(|e| e.to_string())?;
    webview.hide().map_err(|e| e.to_string())
}

/// Place tab `tab_id` and show it; every other tab is hidden. `None` hides
/// all of them (the pane closed, another view, or a menu over the page).
#[tauri::command]
pub fn browser_show(
    app: AppHandle,
    tab_id: Option<String>,
    bounds: Option<Bounds>,
    cursor: Option<agent_cursor::Style>,
) -> Result<(), String> {
    let keep = tab_id.as_deref().map(label).transpose()?;
    for webview in tabs(&app) {
        if Some(webview.label()) != keep.as_deref() {
            agent_cursor::render(
                &webview,
                webview.label().strip_prefix(LABEL_PREFIX).unwrap_or(""),
                None,
            );
            let _ = webview.hide();
        }
    }
    let (Some(id), Some(bounds)) = (tab_id, bounds) else {
        return Ok(());
    };
    let webview = tab(&app, &id)?;
    webview
        .set_position(bounds.position())
        .map_err(|e| e.to_string())?;
    webview.set_size(bounds.size()).map_err(|e| e.to_string())?;
    webview.show().map_err(|e| e.to_string())?;
    agent_cursor::render(&webview, &id, cursor.as_ref());
    Ok(())
}

/// `back` | `forward` | `reload` | `stop` for tab `tab_id`. A reload of a
/// page on this Mac whose server stopped says so instead of doing nothing.
/// Reload and Stop own the tab like an open (`claim_open`): a pending open
/// or reload then does nothing, and a later one makes this reload do nothing.
/// A reload returns its load's `gen` (as `browser_open`); the rest return 0.
#[tauri::command]
pub async fn browser_go(app: AppHandle, tab_id: String, action: String) -> Result<u64, String> {
    if !matches!(action.as_str(), "back" | "forward" | "reload" | "stop") {
        return Err(format!("unknown browser action {action}"));
    }
    label(&tab_id)?;
    // Owned before the lookup, so a Stop while a tab's first open still
    // probes (no webview yet) cancels that open.
    let owns = matches!(action.as_str(), "reload" | "stop").then(|| claim_open(&tab_id));
    let webview = match tab(&app, &tab_id) {
        Ok(webview) => webview,
        Err(_) if action == "stop" => return Ok(0),
        Err(e) => return Err(e),
    };
    let refused = match (action == "reload").then(|| webview.url()) {
        Some(Ok(url)) => probe(&url).await,
        _ => None,
    };
    on_main(&app, move || {
        if owns.is_some_and(|n| !newest_open(&tab_id, n)) {
            return Ok(0);
        }
        if let Some(why) = refused {
            return Err(why);
        }
        let reload = owns.filter(|_| action == "reload");
        if let Some(n) = reload {
            mark_navigated(&tab_id, n);
        }
        match action.as_str() {
            "back" => webview.eval("history.back()"),
            "forward" => webview.eval("history.forward()"),
            "reload" => webview.reload(),
            _ => webview.eval("window.stop()"),
        }
        .map(|()| reload.unwrap_or(0))
        .map_err(|e| e.to_string())
    })
    .await
}

/// The tab's current address. Page-load events miss a single-page app's
/// `pushState`, so the pane polls this for the tab it shows.
#[tauri::command]
pub fn browser_url(app: AppHandle, tab_id: String) -> Result<String, String> {
    tab(&app, &tab_id)?
        .url()
        .map(|u| u.to_string())
        .map_err(|e| e.to_string())
}

/// Annotate, step 1: a PNG of what tab `tab_id` shows now (base64), taken by
/// WebKit itself, so it needs no screen-recording permission and sees only
/// the page. The pane calls it while the page is still on show.
#[tauri::command]
pub async fn browser_snapshot(app: AppHandle, tab_id: String) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let webview = tab(&app, &tab_id)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<u8>, String>>();
    snapshot::take(&webview, tx)?;
    let png = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "The page took too long to capture.".to_string())?
        .map_err(|_| "The page closed while it was captured.".to_string())??;
    Ok(STANDARD.encode(png))
}

#[cfg(target_os = "macos")]
mod snapshot {
    use crate::webkit_capture::png;
    use block2::RcBlock;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSError;
    use objc2_web_kit::WKWebView;
    use std::sync::Mutex;
    use tokio::sync::oneshot::Sender;

    type Reply = Sender<Result<Vec<u8>, String>>;

    pub fn take(webview: &tauri::Webview, tx: Reply) -> Result<(), String> {
        webview
            .with_webview(move |platform| {
                let reply = Mutex::new(Some(tx));
                // WebKit calls this once, on the main thread.
                let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                    let Some(tx) = reply.lock().ok().and_then(|mut r| r.take()) else {
                        return;
                    };
                    // SAFETY: WebKit passes a valid NSImage or NSError (or null) for the call.
                    let result = match unsafe { image.as_ref() } {
                        Some(image) => png(image),
                        None => Err(unsafe { error.as_ref() }
                            .map(|e| e.localizedDescription().to_string())
                            .unwrap_or_else(|| "WebKit returned no image.".into())),
                    };
                    let _ = tx.send(result);
                });
                // SAFETY: on macOS `inner()` is this tab's WKWebView, alive
                // for the duration of this main-thread callback.
                unsafe {
                    let wk = &*(platform.inner() as *const WKWebView);
                    wk.takeSnapshotWithConfiguration_completionHandler(None, &done);
                }
            })
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod snapshot {
    pub fn take(
        _: &tauri::Webview,
        _: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
    ) -> Result<(), String> {
        Err("Annotate needs macOS.".into())
    }
}

/// What the page reports is under one annotation mark. Page-controlled, so
/// every field is capped before it reaches the chat.
#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageElement {
    tag: String,
    id: String,
    classes: String,
    text: String,
    label: String,
    href: String,
    selector: String,
}

#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageInspection {
    url: String,
    title: String,
    elements: Vec<Option<PageElement>>,
}

const MAX_POINTS: usize = 40;

/// Runs in the page: the element at each viewport point, described.
const INSPECT_JS: &str = r#"(() => {
  const pts = __POINTS__;
  const clip = (s, n) => String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const path = el => {
    const parts = [];
    for (let e = el; e && e.nodeType === 1 && parts.length < 5; e = e.parentElement) {
      let p = e.tagName.toLowerCase();
      if (e.id) { parts.unshift(p + '#' + e.id); break; }
      const c = Array.from(e.classList).slice(0, 2);
      if (c.length) p += '.' + c.join('.');
      parts.unshift(p);
    }
    return parts.join(' > ');
  };
  return JSON.stringify({ url: location.href, title: document.title, elements: pts.map(([x, y]) => {
    const el = document.elementFromPoint(x, y);
    if (!el) return null;
    const a = el.closest('a');
    return { tag: el.tagName.toLowerCase(), id: el.id || '', classes: Array.from(el.classList).slice(0, 4).join(' '),
      text: clip(el.innerText || el.textContent, 160), label: clip(el.getAttribute('aria-label') || el.getAttribute('alt') || el.getAttribute('title') || el.getAttribute('placeholder'), 100),
      href: a ? a.href : '', selector: path(el) };
  }) });
})()"#;

fn cap(v: &serde_json::Value, key: &str, max: usize) -> String {
    let s = v.get(key).and_then(|x| x.as_str()).unwrap_or("");
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

/// Parse the page's answer (a JSON string; one quoted once more, as wry's
/// `eval_with_callback` used to deliver it, still reads), keeping at most
/// `n` elements.
pub fn parse_inspection(raw: &str, n: usize) -> Result<PageInspection, String> {
    let mut v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    if let Some(inner) = v.as_str() {
        v = serde_json::from_str(inner)
            .map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    }
    let elements = v
        .get("elements")
        .and_then(|e| e.as_array())
        .map(|list| {
            list.iter()
                .take(n)
                .map(|e| {
                    e.is_object().then(|| PageElement {
                        tag: cap(e, "tag", 20),
                        id: cap(e, "id", 60),
                        classes: cap(e, "classes", 120),
                        text: cap(e, "text", 160),
                        label: cap(e, "label", 100),
                        href: cap(e, "href", 300),
                        selector: cap(e, "selector", 200),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(PageInspection {
        url: cap(&v, "url", 500),
        title: cap(&v, "title", 200),
        elements,
    })
}

/// Annotate, step 2: the page's element at each mark (viewport CSS px), so
/// the agent can find what was marked in the code, not only in the picture.
#[tauri::command]
pub async fn browser_inspect(
    app: AppHandle,
    tab_id: String,
    points: Vec<(f64, f64)>,
) -> Result<PageInspection, String> {
    let webview = tab(&app, &tab_id)?;
    let points: Vec<(f64, f64)> = points
        .into_iter()
        .take(MAX_POINTS)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    let n = points.len();
    let js = INSPECT_JS.replace(
        "__POINTS__",
        &serde_json::to_string(&points).map_err(|e| e.to_string())?,
    );
    // In WebKit's client world, like Include page text (review of #1052):
    // wry's eval would serialise whatever the page's patched JSON returns.
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    isolated::eval(&webview, js, PAGE_ANSWER_MAX, tx)?;
    let raw = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .map_err(|_| "The page didn't answer.".to_string())?
        .map_err(|_| "The page closed.".to_string())??;
    parse_inspection(&raw, n)
}

/// Annotate → Include page text (repo ideas 2026-10-27 #3): at most this
/// many bytes (UTF-8) of a page's text go to the chat.
pub const PAGE_TEXT_BYTES: usize = 24 * 1024;
/// `ui.read_tab`'s numbered outline of a page (CD2): at most this many bytes
/// (UTF-8) of formatted outline lines go to the agent, beside the text.
pub const PAGE_OUTLINE_BYTES: usize = 8 * 1024;
/// `ui.read_tab`'s timing read (CD3, `include: ["timing"]`): at most this
/// many bytes (UTF-8) of timing lines go to the agent.
pub const PAGE_TIMING_BYTES: usize = 4 * 1024;
/// The slowest resources listed, and the broken images, at most.
const TIMING_RESOURCES: usize = 15;
const TIMING_BROKEN: usize = 20;
/// Said with every timing read: WebKit's Resource Timing has no
/// `responseStatus` (checked on this Mac's WebKit, 2026-11-18: a 404 image
/// and a 500 fetch read the same), so only images that failed to load can
/// be named as failed.
pub(crate) const TIMING_NO_STATUS: &str = "Status codes: not available in this browser (WebKit's Resource Timing has no response status), so failed requests other than images can't be named.";
/// More outline entries than this in the page's answer are ignored.
const OUTLINE_ITEMS_MAX: usize = 1000;
/// One outline entry's text, in characters, and its reduced link, in bytes.
const OUTLINE_TEXT_CHARS: usize = 100;
const OUTLINE_HREF_BYTES: usize = 300;
/// The longest answer (UTF-16 units) the extraction may give; anything
/// longer is refused before it is copied out of WebKit.
const PAGE_ANSWER_MAX: usize = 256 * 1024;

/// What Include page text attaches: the address of the document it read
/// (without any user name or password), its title, and the rendered text of
/// its main region, cut at [`PAGE_TEXT_BYTES`].
#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageText {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) text: String,
    /// The page had more text than was kept.
    pub(crate) truncated: bool,
    /// `ui.read_tab` only (never sent to the webview): the numbered outline,
    /// one line per landmark, heading, link, button and labelled field, in
    /// page order, at most [`PAGE_OUTLINE_BYTES`]. Empty when not asked for
    /// or the page has none.
    #[serde(skip)]
    pub(crate) outline: String,
    /// The page had more outline entries than were kept.
    #[serde(skip)]
    pub(crate) outline_truncated: bool,
    /// How many entries `outline` holds (numbered 1 to this).
    #[serde(skip)]
    pub(crate) outline_items: usize,
    /// `ui.read_tab` with `include: ["timing"]` only (CD3): load timing,
    /// the slowest resources and the images that failed to load, at most
    /// [`PAGE_TIMING_BYTES`]. Empty when not asked for.
    #[serde(skip)]
    pub(crate) timing: String,
    /// The timing read had more than was kept.
    #[serde(skip)]
    pub(crate) timing_truncated: bool,
}

/// Runs in WebKit's own client world ([`isolated::eval`]), main frame only:
/// the page's scripts can't see or patch its globals or the DOM getters it
/// uses. `innerText` of the main region (`main`, `[role=main]` or `article`
/// when a rendered one holds a real amount of text, else the body), so only
/// rendered text comes back (no `display: none` or `visibility: hidden`
/// elements, form values, scripts or other frames) and tables arrive as
/// tab-separated rows. An element that isn't rendered is never read: its
/// `innerText` would be its hidden text content.
const PAGE_TEXT_JS: &str = r#"(() => {
  const max = __MAX__;
  // Named elements shadow the document's query functions and a form's own
  // properties, so the DOM is read through the prototypes' own functions and
  // getters, never through the page's object.
  const DP = Document.prototype;
  const pick = sel => DP.querySelector.call(document, sel);
  const docGet = name => Object.getOwnPropertyDescriptor(DP, name).get.call(document);
  const rectsOf = Element.prototype.getClientRects;
  const shown = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'innerText').get;
  // A non-HTML element (MathML) has no rendered text of this kind: empty, not a throw.
  const innerOf = el => { try { return shown.call(el); } catch (e) { return ''; } };
  const rendered = el => !!el && rectsOf.call(el).length > 0;
  const enough = el => rendered(el) && String(innerOf(el) || '').trim().length >= 200;
  const main = [pick('main'), pick('[role="main"]'), pick('article')].find(enough);
  const root = main || [docGet('body'), docGet('documentElement')].find(rendered);
  const text = String((root && innerOf(root)) || '').replace(/[ \xa0]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
  // Never half a surrogate pair at the cut (serde refuses a lone one).
  let kept = text.slice(0, max);
  if (/[\ud800-\udbff]$/.test(kept)) kept = kept.slice(0, -1);
  const out = { href: String(location.href).slice(0, 4000), title: String(docGet('title')).slice(0, 400), text: kept, more: text.length > kept.length };
  __OUTLINE__
  __TIMING__
  return JSON.stringify(out);
})()"#;

/// The timing step spliced into [`PAGE_TEXT_JS`] in place of `__TIMING__`
/// for `ui.read_tab` with `include: ["timing"]` (CD3), in the same client
/// world: the navigation entry's phases, the recorded resource count and the
/// slowest resources (address, kind, duration, bytes over the wire; 0 bytes
/// can mean a cache hit or a cross-origin resource without
/// `Timing-Allow-Origin`), and images that finished without loading. The
/// client world doesn't stop a page from shadowing `document.images` with a
/// named element (`<img name="images">`), so the image list comes from
/// `Document.prototype`'s own getter, as the outline calls `Element.prototype`.
/// Addresses leave as origin and path (or a scheme alone), so the answer stays
/// far under [`PAGE_ANSWER_MAX`]. Each half has its own `try`: a failure is
/// reported as "could not be read", never as none, and never costs the read.
const TIMING_JS: &str = r#"try {
    const r = x => Math.max(0, Math.round(Number(x) || 0));
    const short = u => {
      const s = String(u || '');
      const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(s);
      if (!m) return '';
      if (!/^https?$/i.test(m[1])) return m[1].slice(0, 20) + ':';
      return s.split(/[?#]/)[0].slice(0, 400);
    };
    const t = {};
    try {
      const nav = performance.getEntriesByType('navigation')[0];
      const all = performance.getEntriesByType('resource');
      t.nav = nav ? { firstByte: r(nav.responseStart), domReady: r(nav.domContentLoadedEventEnd), load: r(nav.loadEventEnd), bytes: r(nav.transferSize) } : null;
      t.resources = all.length;
      t.slow = Array.from(all, e => ({ url: short(e.name), type: String(e.initiatorType).slice(0, 30), ms: r(e.duration), bytes: r(e.transferSize) }))
        .sort((a, b) => b.ms - a.ms).slice(0, __RES__);
    } catch (e) { t.navFailed = true; }
    try {
      const imagesOf = Object.getOwnPropertyDescriptor(Document.prototype, 'images').get;
      const attr = (el, name) => Element.prototype.getAttribute.call(el, name);
      const failed = Array.prototype.filter.call(imagesOf.call(document), i => {
        const src = attr(i, 'src');
        if (src === null || !src.trim()) return false;
        // An SVG can have no intrinsic size without failing.
        if (/^data:image\/svg|\.svg([?#]|$)/i.test(src)) return false;
        return i.complete && i.naturalWidth === 0;
      });
      t.broken = failed.slice(0, __BROKEN__).map(i => short(i.currentSrc || i.src));
      t.failedImages = failed.length;
    } catch (e) { t.imagesFailed = true; }
    out.timing = t;
  } catch (e) { out.timing = null; }"#;

/// The outline step spliced into [`PAGE_TEXT_JS`] in place of `__OUTLINE__`
/// for `ui.read_tab` (CD2), in the same client world and main frame, so it
/// has the same guarantees: only elements that are rendered (and not
/// `visibility: hidden`) are listed, in document order, as landmarks,
/// headings, links, buttons and labelled form fields. Entry text is built by
/// walking the LIVE element (`shownText`), skipping any subtree that is not
/// rendered, is `visibility: hidden` or is a form control, a button, script,
/// style or editable region (a detached clone has no layout, so `textContent`
/// of one would hand out hidden text). A field gives its label and type,
/// nothing the person typed or chose; a link its raw address (Rust drops the
/// query and fragment); a button its `aria-label`, then its text, then
/// `title`, and a submit, reset or plain input button a fixed name only when
/// it has none of those. Built-ins are called through `Element.prototype`
/// because a form's named controls shadow its properties, and each element
/// is described inside a `try`, so one odd element costs one entry, never the
/// read. At most 300 entries, 20 000 nodes walked and about 40 000 UTF-16
/// units, so the answer stays far under [`PAGE_ANSWER_MAX`] whatever the page
/// holds.
const OUTLINE_JS: &str = r#"const EP = Element.prototype;
  const attr = (el, name) => EP.getAttribute.call(el, name);
  const tagOf = Object.getOwnPropertyDescriptor(EP, 'localName').get;
  const NP = Node.prototype;
  const parentOf = Object.getOwnPropertyDescriptor(NP, 'parentElement').get;
  const typeOf = Object.getOwnPropertyDescriptor(NP, 'nodeType').get;
  const firstOf = Object.getOwnPropertyDescriptor(NP, 'firstChild').get;
  const nextOf = Object.getOwnPropertyDescriptor(NP, 'nextSibling').get;
  const dataOf = Object.getOwnPropertyDescriptor(CharacterData.prototype, 'data').get;
  const editableOf = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'isContentEditable').get;
  // The getter throws on a non-HTML element (MathML): that is not editable.
  const editable = n => { try { return !!editableOf.call(n); } catch (e) { return false; } };
  const cut = (s, n) => {
    let t = String(s || '');
    t = t.toWellFormed ? t.toWellFormed() : t.replace(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g, '�');
    t = t.replace(/\s+/g, ' ').trim();
    if (t.length <= n) return t;
    let k = t.slice(0, n);
    if (/[\ud800-\udbff]$/.test(k)) k = k.slice(0, -1);
    return k;
  };
  const LANDMARKS = ['banner', 'navigation', 'main', 'complementary', 'contentinfo', 'search', 'form', 'region'];
  const FIELD_ROLES = ['textbox', 'searchbox', 'combobox', 'checkbox', 'radio', 'switch', 'slider', 'spinbutton'];
  const NO_FIELD = ['hidden', 'submit', 'button', 'reset', 'image'];
  const BUTTONS = new Map([['submit', 'Submit'], ['reset', 'Reset'], ['button', 'Button'], ['image', 'Button']]);
  const NATIVE = new Map([['nav', 'navigation'], ['main', 'main'], ['aside', 'complementary'], ['form', 'form']]);
  const SKIP_TAGS = new Set(['input', 'textarea', 'select', 'option', 'button', 'script', 'style', 'noscript', 'template', 'svg', 'iframe']);
  let walked = 0;
  const hiddenNode = n => {
    const cs = getComputedStyle(n);
    if (cs.display === 'none' || cs.visibility !== 'visible') return true;
    return cs.display !== 'contents' && !rendered(n);
  };
  const visible = el => rendered(el) && getComputedStyle(el).visibility === 'visible';
  const shownText = (root, max) => {
    let acc = '';
    const walk = (node, top) => {
      if (acc.length >= max * 2 || walked > 20000) return;
      walked++;
      const type = typeOf.call(node);
      if (type === 3) { acc += dataOf.call(node); return; }
      if (type !== 1) return;
      if (!top && (SKIP_TAGS.has(tagOf.call(node)) || editable(node) || hiddenNode(node))) return;
      for (let c = firstOf.call(node); c; c = nextOf.call(c)) walk(c, false);
      if (!top && !getComputedStyle(node).display.startsWith('inline')) acc += ' ';
    };
    walk(root, true);
    return cut(acc, max);
  };
  const fieldLabel = el => {
    const named = cut(attr(el, 'aria-label'), 100);
    if (named) return named;
    for (const l of Array.from(el.labels || [])) {
      if (!visible(l)) continue;
      const t = shownText(l, 100);
      if (t) return t;
    }
    return cut(attr(el, 'placeholder') || attr(el, 'title'), 100);
  };
  const describe = el => {
    const tag = tagOf.call(el);
    const role = String(attr(el, 'role') || '').trim().split(/\s+/)[0].toLowerCase();
    const named = cut(attr(el, 'aria-label'), 100);
    const shown = () => {
      if (named) return named;
      const own = shownText(el, 100);
      if (own) return own;
      const title = cut(attr(el, 'title'), 100);
      if (title) return title;
      const img = EP.querySelector.call(el, 'img[alt]');
      return img && visible(img) ? cut(attr(img, 'alt'), 100) : '';
    };
    const up = parentOf.call(el);
    const inside = !!up && !!EP.closest.call(up, 'article,aside,main,nav,section');
    let landmark = '';
    if (LANDMARKS.includes(role)) landmark = role;
    else if (!role) {
      landmark = NATIVE.get(tag) || '';
      if (!landmark && !inside && tag === 'header') landmark = 'banner';
      if (!landmark && !inside && tag === 'footer') landmark = 'contentinfo';
      if (!landmark && tag === 'section' && named) landmark = 'region';
    }
    if (landmark) return { k: 'landmark', y: landmark, t: named };
    if (role === 'heading' || (!role && /^h[1-6]$/.test(tag))) {
      const level = role === 'heading' ? Number(attr(el, 'aria-level')) || 2 : Number(tag.slice(1));
      const t = named || shownText(el, 100);
      return t ? { k: 'heading', n: Math.min(Math.max(level, 1), 6), t } : null;
    }
    if ((tag === 'a' && EP.hasAttribute.call(el, 'href')) || role === 'link') {
      const t = shown();
      return t ? { k: 'link', t, h: typeof el.href === 'string' ? el.href.slice(0, 2000) : '' } : null;
    }
    if (tag === 'button' || tag === 'summary' || role === 'button' || (tag === 'input' && BUTTONS.has(el.type))) {
      const t = tag === 'input' ? (named || cut(attr(el, 'title'), 100) || BUTTONS.get(el.type)) : shown();
      return t ? { k: 'button', t } : null;
    }
    if ((tag === 'input' && !NO_FIELD.includes(el.type)) || tag === 'select' || tag === 'textarea' || FIELD_ROLES.includes(role)) {
      const t = fieldLabel(el);
      const y = tag === 'input' ? el.type : (tag === 'select' || tag === 'textarea') ? tag : role;
      return t ? { k: 'field', y: String(y), t } : null;
    }
    return null;
  };
  const items = [];
  let more = false, used = 0, scanned = 0;
  const candidates = DP.querySelectorAll.call(document, 'header,nav,main,aside,footer,form,section,[role],h1,h2,h3,h4,h5,h6,a[href],button,summary,input,select,textarea');
  for (const el of candidates) {
    if (++scanned > 5000 || walked > 20000) { more = true; break; }
    let item = null;
    try {
      if (!visible(el)) continue;
      item = describe(el);
    } catch (e) {
      continue;
    }
    if (!item) continue;
    used += item.t.length + (item.h ? item.h.length : 0) + 24;
    if (items.length >= 300 || used > 40000) { more = true; break; }
    items.push(item);
  }
  out.outline = items;
  out.outlineMore = more;"#;

/// Parse the extraction's answer: the document's address (user name and
/// password dropped), control characters other than line breaks and tabs
/// dropped, the text cut at a character boundary within
/// [`PAGE_TEXT_BYTES`].
pub fn parse_page_text(raw: &str) -> Result<PageText, String> {
    if raw.len() > PAGE_ANSWER_MAX * 3 {
        return Err("The page's answer was too large.".into());
    }
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    let mut url = v
        .get("href")
        .and_then(|h| h.as_str())
        .and_then(|h| Url::parse(h).ok())
        .ok_or("The page's answer had no address.")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let text: String = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Err("This page has no readable text.".into());
    }
    let mut end = text.len().min(PAGE_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let more = v.get("more").and_then(|m| m.as_bool()).unwrap_or(false);
    Ok(PageText {
        url: url.to_string(),
        title: cap(&v, "title", 200),
        text: text[..end].trim_end().to_string(),
        truncated: more || end < text.len(),
        ..Default::default()
    })
}

/// A word with no punctuation: a landmark role or a field type, as the page
/// named it, reduced to lower-case letters and digits (at most 20).
fn outline_word(raw: Option<&serde_json::Value>) -> String {
    raw.and_then(|w| w.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(20)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Invisible Unicode format characters (category Cf and its neighbours:
/// zero-width and joiner marks, bidi controls and overrides, the soft hyphen,
/// the tag block that can hide text from a reader) that never belong in an
/// outline entry.
fn outline_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{08E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E007F}')
}

/// An outline entry's text: control characters turned into spaces, invisible
/// format characters dropped ([`outline_invisible`]), runs of white space
/// made one, double quotes made single (the line quotes it), cut at
/// [`OUTLINE_TEXT_CHARS`] characters.
fn outline_text(raw: Option<&serde_json::Value>) -> String {
    let flat: String = raw
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| !outline_invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'")
        .chars()
        .take(OUTLINE_TEXT_CHARS)
        .collect()
}

/// A link's address as the outline shows it: origin and path only (a path,
/// `;params` included, is kept on purpose, so a token in it such as
/// `/reset/<token>` passes: that is the stated rule). The query and fragment
/// (they can carry tokens), a user name and a password are dropped. Another scheme (`mailto:`, `tel:`, `javascript:`) shows as the
/// scheme alone; an address that doesn't parse shows as nothing.
fn outline_href(raw: Option<&serde_json::Value>) -> String {
    let Some(raw) = raw.and_then(|h| h.as_str()).filter(|h| h.len() <= 4000) else {
        return String::new();
    };
    let Ok(mut url) = Url::parse(raw) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return format!("{}:", url.scheme());
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let mut shown = url.to_string();
    if shown.len() > OUTLINE_HREF_BYTES {
        let mut end = OUTLINE_HREF_BYTES;
        while !shown.is_char_boundary(end) {
            end -= 1;
        }
        shown.truncate(end);
        shown.push('\u{2026}');
    }
    shown
}

/// One outline line, `n` its number: `1 landmark navigation "Main menu"`,
/// `2 h1 "Pricing"`, `3 link "Pricing" -> https://example.com/pricing`,
/// `4 button "Sign in"`, `5 field email "Email address"`. None for an entry
/// that is malformed, of an unknown kind, or (other than a landmark) has no
/// text.
fn outline_line(n: usize, item: &serde_json::Value) -> Option<String> {
    let text = outline_text(item.get("t"));
    let quoted = if text.is_empty() {
        String::new()
    } else {
        format!(" \"{text}\"")
    };
    match item.get("k")?.as_str()? {
        "landmark" => {
            let role = outline_word(item.get("y"));
            (!role.is_empty()).then(|| format!("{n} landmark {role}{quoted}"))
        }
        "heading" if !text.is_empty() => {
            let level = item
                .get("n")
                .and_then(|l| l.as_u64())
                .unwrap_or(2)
                .clamp(1, 6);
            Some(format!("{n} h{level}{quoted}"))
        }
        "link" if !text.is_empty() => {
            let href = outline_href(item.get("h"));
            Some(if href.is_empty() {
                format!("{n} link{quoted}")
            } else {
                format!("{n} link{quoted} -> {href}")
            })
        }
        "button" if !text.is_empty() => Some(format!("{n} button{quoted}")),
        "field" if !text.is_empty() => {
            let ty = outline_word(item.get("y"));
            let ty = if ty.is_empty() { "text" } else { &ty };
            Some(format!("{n} field {ty}{quoted}"))
        }
        _ => None,
    }
}

/// The page's outline, numbered 1 to N in the order given, as lines of at
/// most `cap` bytes in all (each line, newline included, whole or left out:
/// a cut line would be half an entry, and the numbers of the kept ones never
/// move). Returns the lines, how many were kept, and whether any entry was
/// left out (`more` is the page's own flag that it listed fewer than it has).
fn outline_from(items: &[serde_json::Value], more: bool, cap: usize) -> (String, usize, bool) {
    let mut out = String::new();
    let mut kept = 0;
    let mut cut = more || items.len() > OUTLINE_ITEMS_MAX;
    for item in items.iter().take(OUTLINE_ITEMS_MAX) {
        let Some(line) = outline_line(kept + 1, item) else {
            continue;
        };
        if out.len() + line.len() + 1 > cap {
            cut = true;
            break;
        }
        out.push_str(&line);
        out.push('\n');
        kept += 1;
    }
    (out.trim_end().to_string(), kept, cut)
}

/// Parse an answer that carries an outline ([`PageText`] with the outline
/// fields filled in): [`parse_page_text`] plus the numbered outline, a missing
/// or malformed one reading as none.
pub fn parse_page_read(raw: &str) -> Result<PageText, String> {
    let mut page = parse_page_text(raw)?;
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    let items = v
        .get("outline")
        .and_then(|o| o.as_array())
        .map_or(&[][..], |a| a.as_slice());
    let more = v
        .get("outlineMore")
        .and_then(|m| m.as_bool())
        .unwrap_or(false);
    (page.outline, page.outline_items, page.outline_truncated) =
        outline_from(items, more, PAGE_OUTLINE_BYTES);
    if let Some(timing) = v.get("timing") {
        (page.timing, page.timing_truncated) = timing_from(timing, PAGE_TIMING_BYTES);
    }
    Ok(page)
}

/// A non-negative whole number from the page's answer, capped at a day of
/// milliseconds (anything else reads as 0).
fn whole(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|x| x.as_u64()).unwrap_or(0).min(86_400_000)
}

/// Bytes as a reader says them.
fn byte_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} B")
    }
}

/// The timing lines (CD3) from the page's `timing` answer, at most `max`
/// bytes, [`TIMING_NO_STATUS`] always last; whether lines were left out.
/// Addresses as the outline shows links ([`outline_href`]: no query,
/// fragment, user name or password) or a scheme alone. A step that failed in
/// the page reads "could not be read", never "none".
fn timing_from(v: &serde_json::Value, max: usize) -> (String, bool) {
    if !v.is_object() {
        return (
            format!("Timing: could not be read\n{TIMING_NO_STATUS}"),
            false,
        );
    }
    let flag = |k: &str| v.get(k).and_then(|f| f.as_bool()).unwrap_or(false);
    let mut lines = Vec::new();
    if flag("navFailed") {
        lines.push("Page and resources: timing could not be read".to_string());
    } else {
        lines.push(match v.get("nav").filter(|n| n.is_object()) {
            Some(nav) => {
                let load = whole(nav.get("load"));
                let load = if load == 0 {
                    "load not finished".to_string()
                } else {
                    format!("load {load} ms")
                };
                format!(
                    "Page: first byte {} ms, DOM ready {} ms, {load}, {} over the wire",
                    whole(nav.get("firstByte")),
                    whole(nav.get("domReady")),
                    byte_size(whole(nav.get("bytes")))
                )
            }
            None => "Page: no navigation timing".to_string(),
        });
        let empty = Vec::new();
        let slow = v.get("slow").and_then(|s| s.as_array()).unwrap_or(&empty);
        let count = whole(v.get("resources"));
        let shown = slow.len().min(TIMING_RESOURCES);
        // The page's timing buffer (250 entries by default, and the page can
        // clear it): "recorded", not "loaded".
        lines.push(if shown == 0 {
            format!("Resources recorded: {count}")
        } else {
            format!("Resources recorded: {count}; the {shown} slowest:")
        });
        timing_rows(slow, &mut lines);
    }
    let more = if flag("imagesFailed") {
        lines.push("Failed images: could not be read".to_string());
        false
    } else {
        failed_images(v, &mut lines)
    };
    finish_timing(lines, more, max)
}

/// The slowest resources' lines.
fn timing_rows(slow: &[serde_json::Value], lines: &mut Vec<String>) {
    for (i, res) in slow.iter().take(TIMING_RESOURCES).enumerate() {
        let kind: String = res
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(20)
            .collect();
        lines.push(format!(
            "{}. {} ms {} {} {}",
            i + 1,
            whole(res.get("ms")),
            if kind.is_empty() { "other" } else { &kind },
            byte_size(whole(res.get("bytes"))),
            timing_address(res.get("url"))
        ));
    }
}

/// An address as the timing read shows it: [`outline_href`]'s, or a scheme
/// alone (`data:`, `blob:`) as the page's step already cut it.
fn timing_address(raw: Option<&serde_json::Value>) -> String {
    let shown = outline_href(raw);
    if !shown.is_empty() {
        return shown;
    }
    raw.and_then(|r| r.as_str())
        .filter(|r| r.len() <= 21 && r.len() > 1 && r.ends_with(':'))
        .filter(|r| {
            r[..r.len() - 1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        })
        .unwrap_or("")
        .to_string()
}

/// The failed images' lines: how many, then the first [`TIMING_BROKEN`].
/// Whether there were more than were listed.
fn failed_images(v: &serde_json::Value, lines: &mut Vec<String>) -> bool {
    let empty = Vec::new();
    let broken = v.get("broken").and_then(|b| b.as_array()).unwrap_or(&empty);
    let listed = broken.len().min(TIMING_BROKEN);
    let total = whole(v.get("failedImages")).max(listed as u64);
    if total == 0 {
        lines.push("Failed images: none".to_string());
        return false;
    }
    lines.push(if total > listed as u64 {
        format!("Failed images: {total} (the first {listed} listed)")
    } else {
        format!("Failed images: {total}")
    });
    for url in broken.iter().take(TIMING_BROKEN) {
        lines.push(format!("- {}", timing_address(Some(url))));
    }
    total > listed as u64
}

/// `lines` joined within `max` bytes at whole lines, [`TIMING_NO_STATUS`]
/// last; cut when a line was left out or `more` was not listed.
fn finish_timing(lines: Vec<String>, more: bool, max: usize) -> (String, bool) {
    let mut cut = more;
    let room = max.saturating_sub(TIMING_NO_STATUS.len() + 1);
    let mut out = String::new();
    for line in lines {
        if out.len() + line.len() + 1 > room {
            cut = true;
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(TIMING_NO_STATUS);
    (out, cut)
}

/// Annotate → Include page text: the readable text of what tab `tab_id`
/// shows now, read only when the person ticks Include page text and adds
/// the capture to the chat (main frame only; never cookies, storage or a
/// way to fetch more). Not an agent action.
#[tauri::command]
pub async fn browser_page_text(app: AppHandle, tab_id: String) -> Result<PageText, String> {
    page_read(app, tab_id, false, false).await
}

/// `ui.read_tab`: [`browser_page_text`]'s read plus the numbered outline of
/// the page (CD2), and with `timing` its load timing and failed images
/// (CD3), in the same single pass. Not a Tauri command: only the action
/// that checks the person's share calls it.
pub async fn browser_page_read(
    app: AppHandle,
    tab_id: String,
    timing: bool,
) -> Result<PageText, String> {
    page_read(app, tab_id, true, timing).await
}

/// [`PAGE_TEXT_JS`] with its steps spliced in.
fn page_script(outline: bool, timing: bool) -> String {
    let timing_js = TIMING_JS
        .replace("__RES__", &TIMING_RESOURCES.to_string())
        .replace("__BROKEN__", &TIMING_BROKEN.to_string());
    PAGE_TEXT_JS
        .replace("__MAX__", &PAGE_TEXT_BYTES.to_string())
        .replace("__OUTLINE__", if outline { OUTLINE_JS } else { "" })
        .replace("__TIMING__", if timing { &timing_js } else { "" })
}

async fn page_read(
    app: AppHandle,
    tab_id: String,
    outline: bool,
    timing: bool,
) -> Result<PageText, String> {
    let webview = tab(&app, &tab_id)?;
    let js = page_script(outline, timing);
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    isolated::eval(&webview, js, PAGE_ANSWER_MAX, tx)?;
    let raw = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .map_err(|_| "The page didn't answer.".to_string())?
        .map_err(|_| "The page closed.".to_string())??;
    if outline {
        parse_page_read(&raw)
    } else {
        parse_page_text(&raw)
    }
}

/// `ui.inspect_tab` (GU1): at most this many matches of a selector are
/// described, and the answer is cut to this many bytes.
const SELECTOR_MATCHES: usize = 5;
const SELECTOR_MAX_LEN: usize = 300;
const SELECTOR_ANSWER_BYTES: usize = 6 * 1024;

/// Runs in the client world: describes the elements a CSS selector matches.
/// Read only: it measures and reads computed style, and never touches a
/// control's value, storage or cookies. Every read of a page object goes
/// through a prototype method taken first (a form's named controls shadow
/// properties, see `PAGE_TEXT_JS`).
const SELECTOR_JS: &str = r#"(() => {
  const SEL = __SELECTOR__;
  const MAX = __MAX__;
  const D = Document.prototype, E = Element.prototype;
  const qsa = D.querySelectorAll, rect = E.getBoundingClientRect, attr = E.getAttribute;
  const closest = E.closest, contains = Node.prototype.contains, efp = D.elementFromPoint;
  const tagGet = Object.getOwnPropertyDescriptor(E, 'tagName').get;
  const clip = (s, n) => String(s == null ? '' : s).replace(/[\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, n);
  const innerGet = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'innerText').get;
  const titleGet = Object.getOwnPropertyDescriptor(D, 'title').get;
  const NOTEXT = ['script', 'style', 'template', 'noscript', 'textarea', 'input', 'select', 'option'];
  let list;
  try { list = qsa.call(document, SEL); } catch (e) { return JSON.stringify({ error: 'invalid selector' }); }
  const cs = window.getComputedStyle;
  const out = [];
  for (let i = 0; i < list.length && out.length < MAX; i++) {
    const el = list[i];
    try {
      const r = rect.call(el);
      const st = cs.call(window, el);
      const get = p => st.getPropertyValue(p);
      const shown = r.width > 0 && r.height > 0 && get('visibility') !== 'hidden' && get('display') !== 'none' && Number(get('opacity')) > 0;
      let covered = false;
      if (shown) {
        const x = Math.min(Math.max(r.left + r.width / 2, 0), window.innerWidth - 1);
        const y = Math.min(Math.max(r.top + r.height / 2, 0), window.innerHeight - 1);
        const top = efp.call(document, x, y);
        covered = !!top && top !== el && !contains.call(el, top);
      }
      const dis = closest.call(el, ':disabled') !== null || attr.call(el, 'aria-disabled') === 'true';
      const tag = String(tagGet.call(el)).toLowerCase();
      // Rendered text only, like ui.read_tab: never script or style bodies, hidden subtrees or fields.
      let text = '';
      if (shown && NOTEXT.indexOf(tag) < 0) { try { text = innerGet.call(el); } catch (e) { text = ''; } }
      out.push({
        tag, id: clip(attr.call(el, 'id'), 60), classes: clip(attr.call(el, 'class'), 120),
        role: clip(attr.call(el, 'role'), 40), label: clip(attr.call(el, 'aria-label') || attr.call(el, 'title') || attr.call(el, 'alt'), 100),
        text: clip(text, 120),
        box: { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) },
        visible: shown, covered, disabled: dis,
        style: { display: get('display'), position: get('position'), color: get('color'), background: get('background-color'),
          fontSize: get('font-size'), fontFamily: clip(get('font-family'), 80), fontWeight: get('font-weight'),
          opacity: get('opacity'), pointerEvents: get('pointer-events'), overflow: get('overflow') },
      });
    } catch (e) { out.push({ error: 'unreadable' }); }
  }
  return JSON.stringify({ url: location.href, title: clip(titleGet.call(document), 200), total: list.length,
    viewport: { w: window.innerWidth, h: window.innerHeight, scrollX: Math.round(window.scrollX), scrollY: Math.round(window.scrollY) },
    matches: out });
})()"#;

/// What `ui.inspect_tab` hands the agent (page-controlled: every string is
/// capped and stripped of control characters).
#[derive(Serialize, Default, Debug, PartialEq)]
pub struct SelectorRead {
    pub url: String,
    pub title: String,
    pub total: u64,
    pub viewport: serde_json::Value,
    pub matches: Vec<serde_json::Value>,
    #[serde(skip)]
    navigation: u64,
}

fn selector_ok(selector: &str) -> Result<&str, String> {
    let s = selector.trim();
    if s.is_empty() {
        return Err(
            "ui.inspect_tab needs a CSS selector, for example `button.save` or `#nav a`.".into(),
        );
    }
    if s.len() > SELECTOR_MAX_LEN || s.chars().any(char::is_control) {
        return Err(format!(
            "ui.inspect_tab: the selector must be one line of at most {SELECTOR_MAX_LEN} bytes."
        ));
    }
    Ok(s)
}

fn style_field(v: &serde_json::Value, key: &str, max: usize) -> serde_json::Value {
    cap(v, key, max).into()
}

pub fn parse_selector_read(raw: &str) -> Result<SelectorRead, String> {
    let mut v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    if let Some(inner) = v.as_str() {
        v = serde_json::from_str(inner)
            .map_err(|e| format!("The page's answer was not JSON: {e}"))?;
    }
    if v.get("error").and_then(|e| e.as_str()) == Some("invalid selector") {
        return Err("ui.inspect_tab: that is not a valid CSS selector.".into());
    }
    let num = |x: Option<&serde_json::Value>| {
        x.and_then(|n| n.as_i64())
            .unwrap_or(0)
            .clamp(-1_000_000, 1_000_000)
    };
    let boolean =
        |x: &serde_json::Value, k: &str| x.get(k).and_then(|b| b.as_bool()).unwrap_or(false);
    let matches = v
        .get("matches")
        .and_then(|m| m.as_array())
        .map(|list| {
            list.iter()
                .take(SELECTOR_MATCHES)
                .map(|m| {
                    if m.get("tag").is_none() {
                        return serde_json::json!({ "unreadable": true });
                    }
                    let b = m.get("box");
                    let st = m.get("style").cloned().unwrap_or_default();
                    serde_json::json!({
                        "tag": cap(m, "tag", 20),
                        "id": cap(m, "id", 60),
                        "classes": cap(m, "classes", 120),
                        "role": cap(m, "role", 40),
                        "label": cap(m, "label", 100),
                        "text": cap(m, "text", 120),
                        "box": {
                            "x": num(b.and_then(|b| b.get("x"))),
                            "y": num(b.and_then(|b| b.get("y"))),
                            "w": num(b.and_then(|b| b.get("w"))),
                            "h": num(b.and_then(|b| b.get("h"))),
                        },
                        "visible": boolean(m, "visible"),
                        "covered": boolean(m, "covered"),
                        "disabled": boolean(m, "disabled"),
                        "style": {
                            "display": style_field(&st, "display", 30),
                            "position": style_field(&st, "position", 30),
                            "color": style_field(&st, "color", 60),
                            "background": style_field(&st, "background", 60),
                            "fontSize": style_field(&st, "fontSize", 20),
                            "fontFamily": style_field(&st, "fontFamily", 80),
                            "fontWeight": style_field(&st, "fontWeight", 20),
                            "opacity": style_field(&st, "opacity", 20),
                            "pointerEvents": style_field(&st, "pointerEvents", 20),
                            "overflow": style_field(&st, "overflow", 30),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let vp = v.get("viewport");
    Ok(SelectorRead {
        url: cap(&v, "url", 500),
        title: cap(&v, "title", 200),
        total: v
            .get("total")
            .and_then(|t| t.as_u64())
            .unwrap_or(0)
            .min(1_000_000),
        viewport: serde_json::json!({
            "w": num(vp.and_then(|b| b.get("w"))),
            "h": num(vp.and_then(|b| b.get("h"))),
            "scrollX": num(vp.and_then(|b| b.get("scrollX"))),
            "scrollY": num(vp.and_then(|b| b.get("scrollY"))),
        }),
        matches,
        navigation: 0,
    })
}

/// `ui.inspect_tab`: what a CSS selector matches in tab `tab_id` (box,
/// visible, covered, disabled, a few computed styles). Not a Tauri command:
/// only the action that checks the person's share calls it.
pub async fn browser_inspect_selector(
    app: AppHandle,
    tab_id: String,
    selector: String,
) -> Result<SelectorRead, String> {
    let selector = selector_ok(&selector)?.to_string();
    let webview = tab(&app, &tab_id)?;
    let gen = navigated(&tab_id);
    let js = SELECTOR_JS
        .replace(
            "__SELECTOR__",
            &serde_json::to_string(&selector).map_err(|e| e.to_string())?,
        )
        .replace("__MAX__", &SELECTOR_MATCHES.to_string());
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    isolated::eval(&webview, js, SELECTOR_ANSWER_BYTES, tx)?;
    let raw = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .map_err(|_| "The page didn't answer.".to_string())?
        .map_err(|_| "The page closed.".to_string())??;
    let mut read = parse_selector_read(&raw)?;
    read.navigation = gen;
    Ok(read)
}

/// Called only after the action validates its original share and host scope.
/// A revoked reader can never borrow a replacement share's identity.
pub(crate) fn note_agent_inspection(
    app: &AppHandle,
    scope: &str,
    run: &str,
    tab_id: &str,
    generation: u64,
    read: &SelectorRead,
) {
    if let Some(share) =
        agent_cursor::original_share(crate::tab_grants::list(), scope, run, tab_id, generation)
    {
        agent_cursor::remember(app, share, read.navigation, read);
    }
}

/// A script run in WebKit's own client world (`WKContentWorld
/// defaultClientWorld`) in the main frame, whose answer must be a string
/// of at most `max` UTF-16 units: Annotate's element inspection and Include
/// page text. Not wry's `eval_with_callback`: that runs in the page's world
/// and JSON-serialises any answer with an `unwrap`, so a page that patched
/// `JSON.stringify` to return `NaN` would abort Hollow (review of #1052).
#[cfg(target_os = "macos")]
mod isolated {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKContentWorld, WKWebView};
    use std::sync::Mutex;
    use tokio::sync::oneshot::Sender;

    pub fn eval(
        webview: &tauri::Webview,
        js: String,
        max: usize,
        tx: Sender<Result<String, String>>,
    ) -> Result<(), String> {
        webview
            .with_webview(move |platform| {
                let reply = Mutex::new(Some(tx));
                // WebKit calls this once, on the main thread.
                let done = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                    let Some(tx) = reply.lock().ok().and_then(|mut r| r.take()) else {
                        return;
                    };
                    // SAFETY: WebKit passes the script's value or an NSError (or null).
                    let result = match unsafe { value.as_ref() } {
                        Some(value) => match value.downcast_ref::<NSString>() {
                            Some(s) if s.length() <= max => Ok(s.to_string()),
                            Some(_) => Err("The page's answer was too large.".to_string()),
                            None => Err("The page's answer was not text.".to_string()),
                        },
                        None => Err(unsafe { error.as_ref() }
                            .map(|e| e.localizedDescription().to_string())
                            .unwrap_or_else(|| "The page gave no answer.".into())),
                    };
                    let _ = tx.send(result);
                });
                // `with_webview` runs this on the main thread; without the
                // marker, `done` is dropped and the caller hears "closed".
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                // SAFETY: on macOS `inner()` is this tab's WKWebView, alive
                // for the duration of this main-thread callback; a nil frame
                // is the main frame.
                unsafe {
                    let wk = &*(platform.inner() as *const WKWebView);
                    let world = WKContentWorld::defaultClientWorld(mtm);
                    wk.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                        &NSString::from_str(&js),
                        None,
                        &world,
                        Some(&done),
                    );
                }
            })
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod isolated {
    pub fn eval(
        _: &tauri::Webview,
        _: String,
        _: usize,
        _: tokio::sync::oneshot::Sender<Result<String, String>>,
    ) -> Result<(), String> {
        Err("Annotate needs macOS.".into())
    }
}

/// Repo ideas 2026-10-27 #5: share tab `tab_id` with chat `session_key`
/// (the person's click in the Preview toolbar; main window only), so that
/// chat's agent may read its text during one turn: the one running, else
/// its next (`tab_grants`).
#[tauri::command]
pub fn browser_grant_tab(
    app: AppHandle,
    webview: tauri::Webview,
    tab_id: String,
    session_key: String,
    title: String,
) -> Result<crate::tab_grants::Grant, String> {
    if webview.label() != "main" {
        return Err("Share a tab from Hollow's main window.".into());
    }
    tab(&app, &tab_id)?;
    crate::tab_grants::grant(&tab_id, &session_key, &title)
}

/// Stop sharing tab `tab_id` (its strip's Stop; main window only).
#[tauri::command]
pub fn browser_revoke_tab(webview: tauri::Webview, tab_id: String) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Stop sharing from Hollow's main window.".into());
    }
    crate::tab_grants::revoke(&tab_id);
    agent_cursor::forget(&tab_id);
    if let Ok(page) = tab(webview.app_handle(), &tab_id) {
        agent_cursor::render(&page, &tab_id, None);
    }
    Ok(())
}

/// The tabs shared with a chat now, for their strips.
#[tauri::command]
pub fn browser_tab_grants() -> Vec<crate::tab_grants::Grant> {
    crate::tab_grants::list()
}

#[tauri::command]
pub fn browser_close(app: AppHandle, tab_id: String) -> Result<(), String> {
    forget_opens(&tab_id);
    // A closed tab is shared with no chat (repo ideas 10-27 #5).
    crate::tab_grants::revoke(&tab_id);
    match app.get_webview(&label(&tab_id)?) {
        Some(webview) => webview.close().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn web_pages_load_and_app_origins_never_do() {
        let dev = u("http://localhost:1420");
        assert_eq!(classify(&u("https://example.com/a"), None), Verdict::Allow);
        assert_eq!(
            classify(&u("http://localhost:5173/"), Some(&dev)),
            Verdict::Allow
        );
        assert_eq!(
            classify(&u("http://localhost:1420/index.html"), Some(&dev)),
            Verdict::Deny
        );
        assert_eq!(
            classify(&u("http://localhost:1420/"), None),
            Verdict::Allow,
            "the dev origin is only local in dev"
        );
        for s in [
            "tauri://localhost/",
            "artifact://localhost/abc",
            "ipc://localhost/x",
            "file:///etc/passwd",
            "https://tauri.localhost/",
            "http://ipc.localhost/",
            "http://artifact.localhost/x",
            "asset://localhost/x",
            "javascript:alert(1)",
        ] {
            assert_eq!(classify(&u(s), Some(&dev)), Verdict::Deny, "{s}");
        }
        for s in [
            "about:blank",
            "about:srcdoc",
            "data:text/html,hi",
            "blob:https://example.com/1",
        ] {
            assert_eq!(classify(&u(s), None), Verdict::Allow, "{s}");
        }
        assert_eq!(classify(&u("mailto:a@b.c"), None), Verdict::External);
    }

    #[test]
    fn other_apps_links_ask_and_app_schemes_never_leave() {
        // An app's sign-in callback or an editor link is handed to that app
        // after a question; Hollow's own schemes and scripts never are.
        for s in [
            "vscode://file/Users/me/a.ts:3",
            "myapp://oauth/callback?code=1",
            "slack://open",
        ] {
            assert_eq!(classify(&u(s), None), Verdict::Ask, "{s}");
        }
        for s in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "tauri://localhost/",
            "asset://localhost/x",
            "ipc://localhost/x",
            "artifact://localhost/abc",
            "hollow-media://localhost/a.mp4",
        ] {
            assert_eq!(classify(&u(s), None), Verdict::Deny, "{s}");
        }
        assert_eq!(classify(&u("tel:+15551234"), None), Verdict::External);
    }

    #[test]
    fn hand_offs_are_rate_limited() {
        let mut gate = HandOffGate::new();
        let t0 = std::time::Instant::now();
        assert!((0..HAND_OFF_LIMIT).all(|_| gate.admit(t0)));
        assert!(!gate.admit(t0), "a page firing links in a loop is cut off");
        assert!(gate.admit(t0 + HAND_OFF_WINDOW), "and allowed again later");
    }

    #[test]
    fn refusals_say_why() {
        let dev = u("http://localhost:1420");
        assert_eq!(refusal(&u("https://example.com/"), Some(&dev)), None);
        assert_eq!(refusal(&u("about:blank"), None), None);
        let own = refusal(&u("http://localhost:1420/"), Some(&dev)).unwrap();
        assert!(own.contains("Hollow's own development server"), "{own}");
        let app = refusal(&u("https://tauri.localhost/"), None).unwrap();
        assert!(app.contains("belongs to Hollow"), "{app}");
        let other = refusal(&u("ftp://example.com/a"), None).unwrap();
        assert!(other.contains("not ftp"), "{other}");
    }

    #[test]
    fn only_this_macs_servers_are_probed() {
        let probe = |s: &str| loopback(&u(s));
        assert_eq!(
            probe("http://localhost:5173/app"),
            Some(("localhost".into(), 5173))
        );
        assert_eq!(
            probe("http://127.0.0.1:8080"),
            Some(("127.0.0.1".into(), 8080))
        );
        assert_eq!(probe("http://[::1]:3000/"), Some(("::1".into(), 3000)));
        assert_eq!(
            probe("http://0.0.0.0:4321/"),
            Some(("127.0.0.1".into(), 4321)),
            "0.0.0.0 means this Mac"
        );
        assert_eq!(probe("https://LOCALHOST/"), Some(("localhost".into(), 443)));
        assert_eq!(
            probe("https://example.com/"),
            None,
            "public hosts load as they are"
        );
        assert_eq!(probe("http://printer.local/"), None);
        assert_eq!(probe("about:blank"), None);
    }

    #[test]
    fn a_closed_local_port_does_not_answer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(answers("127.0.0.1", port, PROBE_TIMEOUT));
        drop(listener);
        assert!(!answers("127.0.0.1", port, PROBE_TIMEOUT));
        let refused = not_answering(&u(&format!("http://127.0.0.1:{port}/")));
        assert!(
            refused.is_some_and(|m| m.contains(&format!("127.0.0.1:{port}"))),
            "names the address"
        );
    }

    #[test]
    fn a_later_open_or_a_close_supersedes_a_probing_one() {
        let first = claim_open("t-race");
        let second = claim_open("t-race");
        assert!(!newest_open("t-race", first), "the older open does nothing");
        assert!(newest_open("t-race", second));
        forget_opens("t-race");
        assert!(!newest_open("t-race", second), "a closed tab is never made");
    }

    #[test]
    fn events_carry_the_load_that_sent_them() {
        assert_eq!(navigated("t-gen"), 0, "no load yet");
        mark_navigated("t-gen", 7);
        assert_eq!(navigated("t-gen"), 7);
        forget_opens("t-gen");
        assert_eq!(navigated("t-gen"), 0, "a closed tab's loads are forgotten");
        let ev = Tagged {
            ev: BrowserEvent {
                tab_id: "t-gen".into(),
                kind: "loaded",
                url: Some("http://localhost:5173/".into()),
                title: None,
                path: None,
                ok: None,
            },
            gen: 7,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["gen"], 7);
        assert_eq!(json["tabId"], "t-gen", "flattened beside the event");
    }

    #[test]
    fn web_content_may_load_plain_http_and_the_app_keeps_ats() {
        let plist = include_str!("../Info.plist");
        let key = "<key>NSAllowsArbitraryLoadsInWebContent</key>";
        let at = plist.find(key).expect("ATS exception for web views");
        assert!(plist[at + key.len()..].trim_start().starts_with("<true/>"));
        assert!(
            !plist.contains("<key>NSAllowsArbitraryLoads</key>"),
            "never the app-wide exception"
        );
    }

    #[test]
    fn page_answers_are_capped_and_unquoted() {
        let long = "x".repeat(500);
        let page = serde_json::json!({ "url": "http://localhost:5173/", "title": "Site\u{7}", "elements": [
            { "tag": "button", "id": "", "classes": "btn primary", "text": long, "label": "Refresh", "href": "", "selector": "main > button.btn.primary" },
            null, 7, { "tag": "a" }
        ] }).to_string();
        // eval_with_callback JSON-encodes the returned string once more.
        let quoted = serde_json::to_string(&page).unwrap();
        let got = parse_inspection(&quoted, 3).unwrap();
        assert_eq!(got.url, "http://localhost:5173/");
        assert_eq!(got.title, "Site", "control characters dropped");
        assert_eq!(got.elements.len(), 3, "at most one per mark");
        let first = got.elements[0].as_ref().unwrap();
        assert_eq!(first.tag, "button");
        assert_eq!(first.text.chars().count(), 161, "160 + an ellipsis");
        assert_eq!(first.selector, "main > button.btn.primary");
        assert!(got.elements[1].is_none() && got.elements[2].is_none());
        assert_eq!(
            parse_inspection(&page, 10).unwrap().elements.len(),
            4,
            "an unquoted answer works too"
        );
        assert!(parse_inspection("not json", 1).is_err());
    }

    #[test]
    fn page_text_is_capped_cleaned_and_named_by_its_document() {
        let page = serde_json::json!({ "href": "https://me:secret@example.com/report#totals", "title": "Q3\u{7} report", "text": "Revenue\tCost\n12\t8\u{0}\n\n", "more": false })
            .to_string();
        let got = parse_page_text(&page).unwrap();
        assert_eq!(
            got.url, "https://example.com/report#totals",
            "the document's address, never its user name or password"
        );
        assert_eq!(got.title, "Q3 report");
        assert_eq!(
            got.text, "Revenue\tCost\n12\t8",
            "tabs and line breaks kept, other control characters dropped"
        );
        assert!(!got.truncated);
        let with = |text: &str, more: bool| {
            serde_json::json!({ "href": "https://example.com/", "title": "", "text": text, "more": more }).to_string()
        };
        // Cut within the byte cap, at a character boundary.
        let got = parse_page_text(&with(&"é".repeat(PAGE_TEXT_BYTES), false)).unwrap();
        assert!(got.truncated);
        assert_eq!(
            got.text.len(),
            PAGE_TEXT_BYTES,
            "2-byte characters fill it exactly"
        );
        let got = parse_page_text(&with(
            &format!("{}é", "x".repeat(PAGE_TEXT_BYTES - 1)),
            false,
        ))
        .unwrap();
        assert_eq!(
            got.text.len(),
            PAGE_TEXT_BYTES - 1,
            "never half a character"
        );
        assert!(
            parse_page_text(&with("short", true)).unwrap().truncated,
            "the page had more"
        );
        for empty in ["", "  \n\t "] {
            assert!(parse_page_text(&with(empty, false))
                .unwrap_err()
                .contains("no readable text"));
        }
        for no_address in [
            serde_json::json!({ "title": "x", "text": "words" }).to_string(),
            serde_json::json!({ "href": "not a url", "text": "words" }).to_string(),
        ] {
            assert!(parse_page_text(&no_address)
                .unwrap_err()
                .contains("no address"));
        }
        assert!(parse_page_text(&"x".repeat(PAGE_ANSWER_MAX * 3 + 1))
            .unwrap_err()
            .contains("too large"));
        assert!(parse_page_text("not json").is_err());
        // The extraction reads rendered text only, never what a hidden
        // region holds, and never cookies or storage.
        assert!(PAGE_TEXT_JS.contains("rectsOf.call(el).length > 0"));
        assert!(!PAGE_TEXT_JS.contains("cookie") && !PAGE_TEXT_JS.contains("Storage"));
        assert!(
            !PAGE_TEXT_JS.contains('\u{a0}'),
            "escapes, not invisible characters"
        );
    }

    fn outline_answer(items: serde_json::Value, more: bool) -> String {
        serde_json::json!({ "href": "https://example.com/", "title": "T", "text": "words", "more": false, "outline": items, "outlineMore": more }).to_string()
    }

    #[test]
    fn the_outline_is_numbered_in_page_order_and_names_what_it_lists() {
        let page = parse_page_read(&outline_answer(
            serde_json::json!([
                { "k": "landmark", "y": "Navigation", "t": "Main \"menu\"" },
                { "k": "landmark", "y": "main", "t": "" },
                { "k": "heading", "n": 1, "t": "Pricing" },
                { "k": "heading", "n": 9, "t": "Deep" },
                { "k": "link", "t": "Pricing\n plans", "h": "https://u:p@example.com/pricing?token=abc#frag" },
                { "k": "link", "t": "Mail us", "h": "mailto:a@example.com?subject=x" },
                { "k": "link", "t": "Broken", "h": "not a url" },
                { "k": "button", "t": "Sign in" },
                { "k": "field", "y": "email", "t": "Email address" },
                { "k": "field", "t": "Search" }
            ]),
            false,
        ))
        .unwrap();
        assert_eq!(
            page.outline,
            [
                "1 landmark navigation \"Main 'menu'\"",
                "2 landmark main",
                "3 h1 \"Pricing\"",
                "4 h6 \"Deep\"",
                "5 link \"Pricing plans\" -> https://example.com/pricing",
                "6 link \"Mail us\" -> mailto:",
                "7 link \"Broken\"",
                "8 button \"Sign in\"",
                "9 field email \"Email address\"",
                "10 field text \"Search\"",
            ]
            .join("\n"),
            "document order, 1 to N, query, fragment and password dropped"
        );
        assert_eq!(page.outline_items, 10);
        assert!(!page.outline_truncated);
        assert!(
            !page.outline.contains("token") && !page.outline.contains("secret"),
            "nothing a link's query carried"
        );
        // The text read is the same as without the outline.
        assert_eq!(page.text, "words");
    }

    #[test]
    fn the_outline_skips_what_it_cannot_name_without_a_gap_in_the_numbers() {
        let page = parse_page_read(&outline_answer(
            serde_json::json!([
                { "k": "heading", "t": "" },
                { "k": "link", "t": "", "h": "https://example.com/" },
                { "k": "script", "t": "alert(1)" },
                { "t": "no kind" },
                "not an object",
                { "k": "button", "t": "Go\u{7}\u{0} now" },
                { "k": "landmark", "y": "", "t": "no role" },
                { "k": "field", "y": "pass word!", "t": "Password" }
            ]),
            false,
        ))
        .unwrap();
        assert_eq!(
            page.outline,
            "1 button \"Go now\"\n2 field password \"Password\""
        );
        assert_eq!(page.outline_items, 2);
        // No outline in the answer, or none asked for: nothing, no error.
        for none in [
            outline_answer(serde_json::json!([]), false),
            serde_json::json!({ "href": "https://example.com/", "text": "words" }).to_string(),
            serde_json::json!({ "href": "https://example.com/", "text": "words", "outline": "nope" }).to_string(),
        ] {
            let page = parse_page_read(&none).unwrap();
            assert!(page.outline.is_empty() && page.outline_items == 0 && !page.outline_truncated);
        }
        assert!(parse_page_text(&outline_answer(
            serde_json::json!([{ "k": "button", "t": "Go" }]),
            false
        ))
        .unwrap()
        .outline
        .is_empty());
        assert!(parse_page_read("not json").is_err());
        assert!(
            serde_json::to_value(&page)
                .unwrap()
                .get("outline")
                .is_none(),
            "Annotate's payload to the webview carries no outline"
        );
    }

    #[test]
    fn the_outline_is_cut_at_whole_lines_within_its_byte_cap() {
        let items: Vec<_> = (0..40)
            .map(|i| serde_json::json!({ "k": "button", "t": format!("Button é{i:02}") }))
            .collect();
        let line = |n: usize| format!("{n} button \"Button é{:02}\"", n - 1);
        // Each line is its text plus a newline, and a multi-byte letter counts as its bytes.
        let all: String = (1..=40).map(|n| line(n) + "\n").collect();
        let exact = all.len();
        let (text, kept, cut) = outline_from(&items, false, exact);
        assert_eq!((kept, cut), (40, false), "exactly the cap fits");
        assert_eq!(text.len(), exact - 1, "no trailing newline");
        let (text, kept, cut) = outline_from(&items, false, exact - 1);
        assert_eq!((kept, cut), (39, true), "one byte short drops the last");
        assert!(text.ends_with(&line(39)) && text.len() < exact);
        assert!(text.is_char_boundary(text.len()));
        let (text, kept, cut) = outline_from(&items, false, 10);
        assert_eq!(
            (text.as_str(), kept, cut),
            ("", 0, true),
            "not even one line fits"
        );
        // The real cap, with entries that overflow it: the kept ones keep their numbers.
        let many: Vec<_> = (0..600)
            .map(|i| serde_json::json!({ "k": "link", "t": format!("Link number {i} é"), "h": format!("https://example.com/{}", "p".repeat(80)) }))
            .collect();
        let page = parse_page_read(&outline_answer(serde_json::json!(many), false)).unwrap();
        assert!(page.outline_truncated);
        assert!(page.outline.len() < PAGE_OUTLINE_BYTES);
        assert!(
            page.outline.len() + 200 > PAGE_OUTLINE_BYTES,
            "filled to within a line"
        );
        assert!(page.outline_items < 600);
        let last = page.outline.lines().last().unwrap();
        assert!(last.starts_with(&format!("{} link ", page.outline_items)));
        assert_eq!(page.outline.lines().count(), page.outline_items);
        // The page's own "there was more" flag counts, as does a huge list.
        let (_, _, cut) = outline_from(&items[..1], true, PAGE_OUTLINE_BYTES);
        assert!(cut);
        let huge: Vec<_> = (0..OUTLINE_ITEMS_MAX + 5)
            .map(|_| serde_json::json!({ "k": "button", "t": "x" }))
            .collect();
        let (_, kept, cut) = outline_from(&huge, false, usize::MAX);
        assert_eq!((kept, cut), (OUTLINE_ITEMS_MAX, true));
        // One long entry is cut by characters, never in half a character.
        let long = serde_json::json!({ "k": "heading", "n": 2, "t": "é".repeat(500) });
        let (text, _, _) = outline_from(&[long], false, PAGE_OUTLINE_BYTES);
        assert_eq!(
            text.chars().filter(|c| *c == 'é').count(),
            OUTLINE_TEXT_CHARS
        );
        // A long address is cut at a boundary with a mark.
        let href = outline_href(Some(&serde_json::json!(format!(
            "https://example.com/{}",
            "ü".repeat(400)
        ))));
        assert!(href.len() <= OUTLINE_HREF_BYTES + 3 && href.ends_with('\u{2026}'));
        assert_eq!(
            outline_href(Some(&serde_json::json!("https://example.com/a/b?x=1#y"))),
            "https://example.com/a/b"
        );
    }

    #[test]
    fn the_outline_drops_invisible_format_characters_and_keeps_replacement_marks() {
        let page = parse_page_read(&outline_answer(
            serde_json::json!([
                { "k": "button", "t": "Pay\u{202E}ment\u{200B}\u{FEFF} \u{E0041}\u{E007F}now" },
                { "k": "link", "t": "a\u{2066}b\u{2069}\u{00AD}c", "h": "https://example.com/" },
                { "k": "heading", "n": 2, "t": "Bad \u{fffd} piece" },
                { "k": "field", "y": "text", "t": "\u{200B}\u{E0041}" }
            ]),
            false,
        ))
        .unwrap();
        assert_eq!(
            page.outline,
            "1 button \"Payment now\"\n2 link \"abc\" -> https://example.com/\n3 h2 \"Bad \u{fffd} piece\""
        );
        assert_eq!(
            page.outline_items, 3,
            "an entry of only invisible text is dropped"
        );
        assert!(page.outline.chars().all(|c| !outline_invisible(c)));
    }

    #[test]
    fn the_outline_script_reads_only_what_is_rendered_and_never_what_was_typed() {
        for js in [PAGE_TEXT_JS, OUTLINE_JS] {
            let lower = js.to_ascii_lowercase();
            assert!(
                !lower.contains("cookie") && !lower.contains("storage"),
                "no cookies or storage"
            );
            assert!(!js.contains('\u{a0}'), "escapes, not invisible characters");
        }
        // No field is ever read for what it holds, and no clone is read for
        // hidden text: no `.value`, `defaultValue`, `valueAsNumber`,
        // `.options`, `.files`, `.dataset`, `value` attribute, `clone`,
        // `textContent` or `innerText` (the text comes from `shownText`'s walk
        // of the live element), no `innerHTML`, no fetching.
        let lower = OUTLINE_JS.to_ascii_lowercase();
        for banned in [
            ".value",
            "'value'",
            "\"value\"",
            "defaultvalue",
            "valueasnumber",
            "valueasdate",
            ".options",
            ".files",
            ".dataset",
            "clonenode",
            "textcontent",
            "innertext",
            "innerhtml",
            "outerhtml",
            "fetch(",
            "xmlhttp",
            "selectedoptions",
            "checked",
        ] {
            assert!(!lower.contains(banned), "{banned}");
        }
        // The walk skips what is not rendered or is visibility: hidden, and
        // never enters a control, a button, an editable region or a script.
        for must in [
            "const hiddenNode = n =>",
            "cs.display === 'none' || cs.visibility !== 'visible'",
            "!rendered(n)",
            "hiddenNode(node)",
            "editable(node)",
            "const SKIP_TAGS = new Set(['input', 'textarea', 'select', 'option', 'button', 'script', 'style'",
            "if (!visible(l)) continue;",
            "if (!visible(el)) continue;",
            "const visible = el => rendered(el) && getComputedStyle(el).visibility === 'visible';",
            "} catch (e) {",
            "toWellFormed",
            "walked > 20000",
        ] {
            assert!(OUTLINE_JS.contains(must), "{must}");
        }
        assert!(
            OUTLINE_JS.contains("'hidden', 'submit'"),
            "hidden inputs are not fields"
        );
        // Spliced in, it runs after the text and before the answer is made.
        assert!(PAGE_TEXT_JS.contains("__OUTLINE__"));
        let spliced = PAGE_TEXT_JS.replace("__OUTLINE__", OUTLINE_JS);
        assert!(!spliced.contains("__OUTLINE__"));
        assert!(
            spliced.find("const out =").unwrap() < spliced.find("out.outline = items").unwrap()
                && spliced.find("out.outline = items").unwrap()
                    < spliced.find("return JSON.stringify(out)").unwrap()
        );
    }

    /// CD2 hardening: a named element (`<img name=querySelector>`) or a
    /// control inside a `<form>` (`<input name=firstChild>`) shadows the
    /// page's own `document.querySelector` and the form's `nodeType`,
    /// `firstChild`, `nextSibling` and `isContentEditable`, so none of them
    /// may be read through the page's object: the text walk uses the
    /// prototypes' getters and both scripts call the query functions through
    /// `Document.prototype` and `Element.prototype`.
    #[test]
    fn the_read_scripts_never_read_the_dom_through_a_page_object() {
        let spliced = PAGE_TEXT_JS.replace("__OUTLINE__", OUTLINE_JS);
        for banned in [
            "document.querySelector",
            "document.body",
            "document.documentElement",
            ".nodeType",
            ".firstChild",
            ".nextSibling",
            ".isContentEditable",
            "node.data",
            "el.getClientRects",
            "el.innerText",
            "root.innerText",
            "document.title",
        ] {
            assert!(!spliced.contains(banned), "{banned} is shadowable");
        }
        for must in [
            "DP.querySelector.call(document, sel)",
            "DP.querySelectorAll.call(document, ",
            "Object.getOwnPropertyDescriptor(NP, 'nodeType').get",
            "Object.getOwnPropertyDescriptor(NP, 'firstChild').get",
            "Object.getOwnPropertyDescriptor(NP, 'nextSibling').get",
            "Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'isContentEditable').get",
            "Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'innerText').get",
            "Object.getOwnPropertyDescriptor(CharacterData.prototype, 'data').get",
            "rectsOf.call(el)",
            "docGet('title')",
            "typeOf.call(node)",
            "firstOf.call(node)",
            "nextOf.call(c)",
            "editable(node)",
        ] {
            assert!(spliced.contains(must), "{must}");
        }
        // The getters are taken from the prototypes before the walk runs.
        assert!(
            spliced.find("const NP = Node.prototype").unwrap()
                < spliced.find("const walk =").unwrap()
        );
    }

    #[test]
    fn tab_ids_are_plain_labels() {
        assert_eq!(label("t-1a").unwrap(), "browser-t-1a");
        for bad in ["", "../main", "a b", "main/x", &"x".repeat(41)] {
            assert!(label(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn downloads_never_overwrite_or_escape_the_folder() {
        let dir = crate::test_temp::process_dir("hollow-browser-dl-");
        std::fs::create_dir_all(&dir).unwrap();
        let first = download_target(&dir, &u("https://x.test/files/report%20q3.pdf?x=1"));
        assert_eq!(first, dir.join("report q3.pdf"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            download_target(&dir, &u("https://x.test/files/report%20q3.pdf")),
            dir.join("report q3 (1).pdf")
        );
        assert_eq!(
            download_target(&dir, &u("https://x.test/a%2F..%2F..%2Fevil")),
            dir.join("a_.._.._evil")
        );
        assert_eq!(
            download_target(&dir, &u("https://x.test/")),
            dir.join("download")
        );
        assert_eq!(
            download_target(&dir, &u("https://x.test/.bashrc")),
            dir.join("download.bashrc")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// CD3: the timing read lists the page's phases, the slowest resources
    /// (addresses without query or fragment) and the failed images, and
    /// always says status codes are not available.
    #[test]
    fn the_timing_read_names_phases_slow_resources_and_failed_images() {
        let answer = serde_json::json!({
            "nav": { "firstByte": 120, "domReady": 340, "load": 905, "bytes": 46_000 },
            "resources": 37,
            "slow": [
                { "url": "https://app.test/main.js?token=abc#x", "type": "script", "ms": 820, "bytes": 12_400 },
                { "url": "https://u:p@cdn.test/a.css", "type": "link<bad>", "ms": 90, "bytes": 0 },
            ],
            "broken": ["https://app.test/missing.png?sig=1"],
            "failedImages": 1,
        });
        let (text, cut) = timing_from(&answer, PAGE_TIMING_BYTES);
        assert!(!cut);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "Page: first byte 120 ms, DOM ready 340 ms, load 905 ms, 44 KB over the wire"
        );
        assert_eq!(lines[1], "Resources recorded: 37; the 2 slowest:");
        assert_eq!(lines[2], "1. 820 ms script 12 KB https://app.test/main.js");
        assert_eq!(lines[3], "2. 90 ms linkbad 0 B https://cdn.test/a.css");
        assert_eq!(lines[4], "Failed images: 1");
        assert_eq!(lines[5], "- https://app.test/missing.png");
        assert_eq!(lines[6], TIMING_NO_STATUS);
        assert!(!text.contains("token") && !text.contains("sig=") && !text.contains("u:p"));
    }

    #[test]
    fn the_timing_read_stays_within_its_cap_and_reads_nonsense_as_nothing() {
        let slow: Vec<_> = (0..40)
            .map(|i| serde_json::json!({ "url": format!("https://x.test/{}", "p".repeat(280)), "type": "img", "ms": 1000 - i, "bytes": 5 }))
            .collect();
        let broken: Vec<_> = (0..40)
            .map(|i| serde_json::json!(format!("https://x.test/b{i}.png")))
            .collect();
        let answer = serde_json::json!({ "nav": null, "resources": 40, "slow": slow, "broken": broken, "failedImages": 40 });
        let (text, cut) = timing_from(&answer, PAGE_TIMING_BYTES);
        assert!(cut);
        assert!(text.len() <= PAGE_TIMING_BYTES, "{}", text.len());
        assert!(text.starts_with("Page: no navigation timing\n"));
        assert!(text.ends_with(TIMING_NO_STATUS));
        assert!(text.lines().filter(|l| l.contains(" ms img ")).count() <= TIMING_RESOURCES);
        // A malformed answer: no lines invented, the status note still said.
        let (text, cut) = timing_from(
            &serde_json::json!({ "slow": "x", "nav": 3, "resources": -4 }),
            PAGE_TIMING_BYTES,
        );
        assert!(!cut);
        assert_eq!(
            text,
            format!(
                "Page: no navigation timing\nResources recorded: 0\nFailed images: none\n{TIMING_NO_STATUS}"
            )
        );
    }

    #[test]
    fn the_timing_step_is_spliced_only_when_asked_and_parsed_from_the_answer() {
        let plain = page_script(true, false);
        assert!(!plain.contains("__TIMING__") && !plain.contains("performance."));
        let timed = page_script(true, true);
        for hole in [
            "__TIMING__",
            "__OUTLINE__",
            "__MAX__",
            "__RES__",
            "__BROKEN__",
        ] {
            assert!(!timed.contains(hole), "{hole}");
        }
        assert!(timed.contains("getEntriesByType('resource')"));
        let raw = serde_json::json!({
            "href": "https://app.test/", "title": "T", "text": "hello", "more": false,
            "timing": { "nav": { "firstByte": 5, "domReady": 6, "load": 7, "bytes": 8 }, "resources": 0, "slow": [], "broken": [] },
        })
        .to_string();
        let page = parse_page_read(&raw).unwrap();
        assert!(
            page.timing.starts_with("Page: first byte 5 ms"),
            "{}",
            page.timing
        );
        assert!(!page.timing_truncated);
        // A read without timing keeps it empty.
        let raw = serde_json::json!({ "href": "https://app.test/", "title": "T", "text": "hello", "more": false }).to_string();
        assert!(parse_page_read(&raw).unwrap().timing.is_empty());
    }

    /// CD3 review: a step that failed in the page is never read as "none".
    #[test]
    fn a_timing_step_that_failed_reads_as_unknown_never_as_none() {
        let (text, _) = timing_from(&serde_json::Value::Null, PAGE_TIMING_BYTES);
        assert_eq!(
            text,
            format!("Timing: could not be read\n{TIMING_NO_STATUS}")
        );
        let answer = serde_json::json!({ "navFailed": true, "imagesFailed": true });
        let (text, _) = timing_from(&answer, PAGE_TIMING_BYTES);
        assert!(
            text.contains("Page and resources: timing could not be read"),
            "{text}"
        );
        assert!(text.contains("Failed images: could not be read"), "{text}");
        assert!(!text.contains("none"), "{text}");
        // Still loading; more failed images than listed; a data: image.
        let answer = serde_json::json!({
            "nav": { "firstByte": 5, "domReady": 0, "load": 0, "bytes": 0 },
            "resources": 0, "slow": [],
            "broken": ["data:", "https://a.test/x.png"], "failedImages": 25,
        });
        let (text, cut) = timing_from(&answer, PAGE_TIMING_BYTES);
        assert!(text.contains("load not finished"), "{text}");
        assert!(
            text.contains(
                "Failed images: 25 (the first 2 listed)\n- data:\n- https://a.test/x.png\n"
            ),
            "{text}"
        );
        assert!(cut);
        let (_, cut) = timing_from(
            &serde_json::json!({ "broken": ["https://a.test/x.png"], "failedImages": 1 }),
            PAGE_TIMING_BYTES,
        );
        assert!(!cut);
    }

    /// CD3 review: a page can shadow `document.images` with a named element,
    /// so the step reads the list through `Document.prototype`'s getter.
    #[test]
    fn the_timing_step_reads_images_through_the_prototype_getter() {
        assert!(
            TIMING_JS.contains("Object.getOwnPropertyDescriptor(Document.prototype, 'images').get")
        );
        assert!(!TIMING_JS.contains("document.images"));
        assert!(!TIMING_JS.contains("Array.from(document"));
        let page = PageText::default();
        let result = crate::tab_grants::page_result(&page);
        assert!(result.get("timing").is_none() && result.get("timing_truncated").is_none());
        let page = PageText {
            timing: "Page: x".into(),
            ..PageText::default()
        };
        assert_eq!(crate::tab_grants::page_result(&page)["timing"], "Page: x");
    }

    /// GU1: the selector read is read only (no value, storage, click or
    /// input) and reads the page through prototype methods.
    #[test]
    fn the_selector_script_reads_and_never_acts() {
        for banned in [
            ".click(",
            ".focus(",
            ".dispatchEvent",
            ".value",
            "localStorage",
            "sessionStorage",
            "document.cookie",
            "fetch(",
            "XMLHttpRequest",
            ".submit(",
            "eval(",
            "innerHTML",
            "document.querySelector",
            "el.textContent",
            "el.tagName",
            "textContent",
            "document.title",
        ] {
            assert!(
                !SELECTOR_JS.contains(banned),
                "the selector script must not use {banned}"
            );
        }
        assert!(SELECTOR_JS.contains("qsa.call(document, SEL)"));
    }

    #[test]
    fn a_selector_is_one_short_line() {
        assert!(selector_ok("  button.save ").is_ok());
        assert!(selector_ok("   ").is_err());
        assert!(selector_ok("a\nb").is_err());
        assert!(selector_ok(&"a".repeat(SELECTOR_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn the_selector_answer_is_capped_and_never_invents() {
        let long = "x".repeat(900);
        let one = serde_json::json!({
            "tag": "button", "id": "go", "classes": long, "text": "Save\u{7}",
            "box": { "x": 1, "y": 2, "w": 30, "h": 10 }, "visible": true, "covered": true, "disabled": false,
            "style": { "color": "rgb(0, 0, 0)", "fontFamily": long }
        });
        let many: Vec<_> = (0..9).map(|_| one.clone()).collect();
        let raw = serde_json::json!({ "url": "https://x.test/", "title": "T", "total": 9,
            "viewport": { "w": 800, "h": 600, "scrollX": 0, "scrollY": 40 }, "matches": many })
        .to_string();
        let read = parse_selector_read(&raw).unwrap();
        assert_eq!(read.total, 9);
        assert_eq!(read.matches.len(), SELECTOR_MATCHES);
        let m = &read.matches[0];
        assert_eq!(m["classes"].as_str().unwrap().chars().count(), 121);
        assert_eq!(m["text"], "Save");
        assert_eq!(m["covered"], true);
        assert_eq!(m["box"]["w"], 30);
        assert_eq!(m["style"]["display"], "");
        assert_eq!(read.viewport["scrollY"], 40);
        // Quoted once more still reads; an invalid selector says so; junk is refused.
        assert!(parse_selector_read(&serde_json::to_string(&raw).unwrap()).is_ok());
        assert!(parse_selector_read(r#"{"error":"invalid selector"}"#)
            .unwrap_err()
            .contains("valid CSS"));
        assert!(parse_selector_read("nope").is_err());
        assert_eq!(
            parse_selector_read(r#"{"matches":[{"error":"unreadable"}]}"#)
                .unwrap()
                .matches[0]["unreadable"],
            true
        );
    }
}
