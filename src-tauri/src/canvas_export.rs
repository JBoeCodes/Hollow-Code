//! PNG/PDF/SVG export, isolated from the main document in a sandboxed capture
//! child. No remote loads or pinned live data. The person starts it from the
//! canvas; the COO's `canvases.export_canvas` runs the same inner export for a
//! file in a workspace's `outputs/exports` (never an attachment or the clipboard).

use crate::{canvas_fence, commands::AppState, run_guard, webkit_capture, workspaces};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
use tauri::{AppHandle, Manager, State, Webview, WebviewBuilder, WebviewUrl};
use tokio::sync::oneshot;

static SERIAL: AtomicU64 = AtomicU64::new(1);
const MAX_BYTES: usize = 64 * 1024 * 1024;
type Ready = oneshot::Sender<Result<u32, String>>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    workspace_id: Option<String>,
    source: String,
    revision: Option<u32>,
    state: Option<Value>,
    format: String,
    preset: String,
    look: String,
    copy: bool,
    target: String,
    scheme: String,
    theme: Vec<(String, String)>,
    theme_name: Option<String>,
    svg: Option<String>,
    flow_index: Option<u32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureJob {
    label: String,
    request: ExportRequest,
    width: u32,
    /// Repo ideas 2026-10-27 #4: a Measure on the picture capture instead of
    /// an export (`canvas_measure`).
    #[serde(skip_serializing_if = "Option::is_none")]
    measure: Option<MeasurePass>,
}

/// How a Measure on the picture capture draws the canvas: as the reply's
/// frame shows it (texture as it was allowed there, so motion isn't
/// reduced), its animation frozen, with its text as it is, transparent,
/// black or white (`text`; the capture host's `measureCss`).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurePass {
    text: &'static str,
    texture: bool,
}

/// The four passes, in the order the main window scores them.
const MEASURE_TEXT: [&str; 4] = ["shown", "hidden", "black", "white"];
/// The most device pixels one Measure on the picture picture may have
/// (about 48 MB once decoded; `measurePicture.ts` holds the same limit).
const MEASURE_MAX_PIXELS: u64 = 12_000_000;
struct Active {
    job: CaptureJob,
    reply: Option<Ready>,
    document_url: Option<tauri::Url>,
}
static ACTIVE: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
fn active() -> &'static Mutex<Option<Active>> {
    ACTIVE.get_or_init(|| Mutex::new(None))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedFile {
    rel: Option<String>,
    path: Option<String>,
    png_base64: Option<String>,
    width: u32,
    height: u32,
    scale: f64,
    copied: bool,
    warning: Option<String>,
}

fn width(preset: &str) -> Result<u32, String> {
    match preset {
        "email" => Ok(600),
        "doc" => Ok(800),
        "slide" => Ok(1280),
        _ => Err("Choose an export size.".into()),
    }
}
fn height(h: f64, format: &str, preset: &str) -> Result<u32, String> {
    let limit = if format == "pdf" { 14400.0 } else { 8000.0 };
    if !h.is_finite() || h < 1.0 || h > limit {
        return Err(if format == "pdf" {
            "This canvas is too tall for PDF (14,400 pixels)."
        } else {
            "This canvas is too tall for PNG (8,000 pixels). Try PDF."
        }
        .into());
    }
    Ok((h.ceil() as u32).max(if preset == "slide" { 720 } else { 80 }))
}
fn validate(request: &ExportRequest) -> Result<(String, u32), String> {
    let parsed = canvas_fence::validate(&request.source)
        .map_err(|_| "This canvas is incomplete or invalid.".to_string())?;
    let w = width(&request.preset)?;
    if request
        .theme_name
        .as_ref()
        .is_some_and(|name| name.is_empty() || name.len() > 256)
        || (request.theme_name.is_some() && (request.look != "match" || request.theme.is_empty()))
    {
        return Err("Invalid export theme name or palette.".into());
    }
    if request.revision == Some(0) || (request.target == "file" && request.revision.is_none()) {
        return Err("Invalid canvas revision.".into());
    }
    if !["png", "pdf", "svg"].contains(&request.format.as_str())
        || !["light", "dark", "match"].contains(&request.look.as_str())
        || !["light", "dark"].contains(&request.scheme.as_str())
        || !["file", "attachment"].contains(&request.target.as_str())
    {
        return Err("Invalid canvas export options.".into());
    }
    if request.target == "file" && request.workspace_id.as_deref().is_none_or(str::is_empty) {
        return Err("Save this canvas in a workspace or Chats first.".into());
    }
    if request.target == "attachment" && request.format != "png" {
        return Err("A picture attachment must be PNG.".into());
    }
    if request.copy && request.format != "png" {
        return Err("Only PNG can be copied as an image.".into());
    }
    if request.format == "svg" {
        if !request.flow_index.is_some_and(|n| (1..=4096).contains(&n)) {
            return Err("Choose a flow to export.".into());
        }
        validate_svg(
            request
                .svg
                .as_deref()
                .ok_or("The flow drawing is missing.")?,
        )?;
    } else if request.svg.is_some() || request.flow_index.is_some() {
        return Err("Only SVG exports accept a flow drawing.".into());
    }
    if request
        .state
        .as_ref()
        .is_some_and(|v| !v.is_null() && (!v.is_object() || v.to_string().len() > 16384))
    {
        return Err("The canvas state is invalid or too large.".into());
    }
    if request.theme.len() > 64
        || request
            .theme
            .iter()
            .any(|(k, v)| !k.starts_with("--app-") || k.len() > 64 || v.len() > 256)
    {
        return Err("The export theme is too large.".into());
    }
    Ok((parsed.root.id, w))
}

fn theme_suffix(name: Option<&str>) -> String {
    let Some(name) = name else {
        return String::new();
    };
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(48).collect();
    let slug = slug.trim_end_matches('-');
    format!("-{}", if slug.is_empty() { "theme" } else { slug })
}

/// Accept only the static subset emitted by flowSvg, not arbitrary web content.
fn validate_svg(svg: &str) -> Result<(u32, u32), String> {
    use quick_xml::{events::Event, Reader};
    let invalid = || "The flow drawing is not a safe standalone SVG.".to_string();
    if svg.len() > 2 * 1024 * 1024 || !svg.starts_with("<svg ") {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(svg);
    let mut stack: Vec<String> = Vec::new();
    let mut root_seen = false;
    let mut dimensions = (0, 0);
    loop {
        match reader.read_event().map_err(|_| invalid())? {
            Event::Start(e) | Event::Empty(e) => {
                let name = std::str::from_utf8(e.name().into_inner()).map_err(|_| invalid())?;
                // A style element must contain only checked text. Nested XML
                // could otherwise concatenate unchecked descendant text into CSS.
                if stack.last().is_some_and(|parent| {
                    ["style", "title", "desc", "tspan"].contains(&parent.as_str())
                        || (parent == "text" && name != "tspan")
                }) {
                    return Err(invalid());
                }
                if ![
                    "svg", "g", "defs", "marker", "path", "rect", "polygon", "text", "tspan",
                    "title", "desc", "style",
                ]
                .contains(&name)
                    || (stack.is_empty() && name != "svg")
                    || (name == "svg" && root_seen)
                    || stack.len() > 16
                {
                    return Err(invalid());
                }
                if name == "svg" {
                    root_seen = true;
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| invalid())?;
                    let key = std::str::from_utf8(attr.key.into_inner()).map_err(|_| invalid())?;
                    let value = attr.unescape_value().map_err(|_| invalid())?;
                    if ![
                        "xmlns",
                        "width",
                        "height",
                        "viewBox",
                        "role",
                        "aria-labelledby",
                        "id",
                        "class",
                        "x",
                        "y",
                        "rx",
                        "dy",
                        "d",
                        "points",
                        "fill",
                        "stroke",
                        "stroke-width",
                        "stroke-dasharray",
                        "text-anchor",
                        "transform",
                        "refX",
                        "refY",
                        "markerWidth",
                        "markerHeight",
                        "orient",
                        "marker-end",
                    ]
                    .contains(&key)
                    {
                        return Err(invalid());
                    }
                    if (key == "xmlns" && value != "http://www.w3.org/2000/svg")
                        || (key == "marker-end" && value != "url(#arrow)")
                        || (key != "xmlns"
                            && key != "marker-end"
                            && (value.to_ascii_lowercase().contains("url")
                                || value.contains(['\\', '&', '<', '>', '@', ':'])))
                    {
                        return Err(invalid());
                    }
                    if name == "svg" && ["width", "height"].contains(&key) {
                        let n: u32 = value.parse().map_err(|_| invalid())?;
                        if !(1..=100_000).contains(&n) {
                            return Err(invalid());
                        }
                        if key == "width" {
                            dimensions.0 = n;
                        } else {
                            dimensions.1 = n;
                        }
                    }
                }
                // Empty elements have '/>' in the event's original source.
                let end = reader.buffer_position() as usize;
                if !svg[..end].ends_with("/>") {
                    stack.push(name.to_string());
                }
            }
            Event::End(e) => {
                if stack.pop().as_deref().map(str::as_bytes) != Some(e.name().as_ref()) {
                    return Err(invalid());
                }
            }
            Event::Text(e) => {
                let text = std::str::from_utf8(e.as_ref()).map_err(|_| invalid())?;
                if stack.last().is_some_and(|n| n == "style") {
                    if text.to_ascii_lowercase().contains("url")
                        || !text
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || " .{},:;#()%\n\r\t-".contains(c))
                    {
                        return Err(invalid());
                    }
                } else if stack.is_empty() && !text.trim().is_empty() {
                    return Err(invalid());
                }
            }
            Event::GeneralRef(e) => {
                // Entities are text only; a reference must not assemble CSS.
                if !stack
                    .last()
                    .is_some_and(|n| ["title", "desc", "text", "tspan"].contains(&n.as_str()))
                    || ![b"amp".as_slice(), b"lt", b"gt", b"quot", b"apos"].contains(&e.as_ref())
                {
                    return Err(invalid());
                }
            }
            Event::Eof => break,
            _ => return Err(invalid()),
        }
    }
    if !root_seen || !stack.is_empty() || dimensions.0 == 0 || dimensions.1 == 0 {
        return Err(invalid());
    }
    Ok(dimensions)
}
fn own_label(label: &str, expected: &str) -> Result<(), String> {
    if label == expected {
        Ok(())
    } else {
        Err("This capture belongs to another view.".into())
    }
}

#[tauri::command]
pub fn canvas_capture_job(
    webview: Webview,
    document_url: Option<String>,
) -> Result<CaptureJob, String> {
    let mut guard = active().lock().map_err(|_| "Capture lock failed.")?;
    let a = guard.as_mut().ok_or("This capture has ended.")?;
    own_label(webview.label(), &a.job.label)?;
    if let Some(raw) = document_url {
        let url = tauri::Url::parse(&raw).map_err(|_| "Invalid capture document.")?;
        if url.scheme() != "artifact"
            || url.host_str() != Some("localhost")
            || url.query().is_some()
            || url.fragment().is_some()
            || !crate::artifacts::is_canvas_document(url.path().trim_start_matches('/'))
        {
            return Err("The capture needs a local Canvas-mode document.".into());
        }
        if a.document_url.as_ref().is_some_and(|old| old != &url) {
            return Err("This capture already has a document.".into());
        }
        a.document_url = Some(url);
    }
    Ok(a.job.clone())
}
#[tauri::command]
pub fn canvas_capture_ready(webview: Webview, height: f64) -> Result<(), String> {
    let mut guard = active().lock().map_err(|_| "Capture lock failed.")?;
    let a = guard.as_mut().ok_or("This capture has ended.")?;
    own_label(webview.label(), &a.job.label)?;
    let result = self::height(height, &a.job.request.format, &a.job.request.preset);
    if let Some(tx) = a.reply.take() {
        let _ = tx.send(result);
    }
    Ok(())
}
#[tauri::command]
pub fn canvas_capture_failed(webview: Webview) -> Result<(), String> {
    if let Some(a) = active()
        .lock()
        .map_err(|_| "Capture lock failed.")?
        .as_mut()
    {
        own_label(webview.label(), &a.job.label)?;
        if let Some(tx) = a.reply.take() {
            let _ = tx.send(Err(
                "The canvas reported an error while drawing. Nothing was saved.".into(),
            ));
        }
    }
    Ok(())
}

struct Cleanup(AppHandle, String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(mut a) = active().lock() {
            if a.as_ref().is_some_and(|a| a.job.label == self.1) {
                *a = None;
            }
        }
        if let Some(view) = self.0.get_webview(&self.1) {
            let _ = view.close();
        }
    }
}
fn allowed_navigation(
    url: &tauri::Url,
    expected: &tauri::Url,
    document: Option<&tauri::Url>,
) -> bool {
    url == expected || document == Some(url)
}

async fn capture(
    app: &AppHandle,
    job: CaptureJob,
    rx: oneshot::Receiver<Result<u32, String>>,
) -> Result<(Vec<u8>, u32, f64), String> {
    let app2 = app.clone();
    let w = job.width;
    let label = job.label.clone();
    let (created, created_rx) = oneshot::channel::<Result<Webview, String>>();
    app.run_on_main_thread(move || {
        let result = (|| {
            if active()
                .lock()
                .map_err(|_| "Capture lock failed.")?
                .as_ref()
                .is_none_or(|a| a.job.label != label)
            {
                return Err("This capture has ended.".into());
            }
            let parent = app2
                .get_window("main")
                .ok_or("Open Hollow to export a canvas.")?;
            let main = app2
                .get_webview("main")
                .ok_or("Open Hollow to export a canvas.")?;
            let mut expected = main.url().map_err(|e| e.to_string())?;
            expected.set_path("/index.html");
            expected.set_query(Some("window=canvas-capture"));
            expected.set_fragment(None);
            let nav_label = label.clone();
            let builder = WebviewBuilder::new(
                &label,
                WebviewUrl::App("index.html?window=canvas-capture".into()),
            )
            .on_navigation(move |url| {
                // Wry checks every frame and gives no main-frame bit. Admit only
                // our host and this job's registered sandbox document. The iframe
                // lacks allow-top-navigation, allow-same-origin and allow-popups.
                active()
                    .lock()
                    .ok()
                    .and_then(|a| {
                        a.as_ref()
                            .filter(|a| a.job.label == nav_label)
                            .map(|a| allowed_navigation(url, &expected, a.document_url.as_ref()))
                    })
                    .unwrap_or(false)
            });
            let view = parent
                .add_child(
                    builder,
                    tauri::LogicalPosition::new(-20000.0, 0.0),
                    tauri::LogicalSize::new(w as f64, 80.0),
                )
                .map_err(|e| e.to_string())?;
            if active()
                .lock()
                .map_err(|_| "Capture lock failed.")?
                .as_ref()
                .is_none_or(|a| a.job.label != label)
            {
                let _ = view.close();
                return Err("This capture has ended.".into());
            }
            Ok(view)
        })();
        // A timeout can drop the receiver after the active-job check above.
        // In that race the view was never handed to the async capture owner.
        if let Err(Ok(view)) = created.send(result) {
            let _ = view.close();
        }
    })
    .map_err(|e| e.to_string())?;
    let view = created_rx.await.map_err(|_| "The capture view closed.")??;
    let (enabled, enabled_rx) = oneshot::channel();
    webkit_capture::enable_frames(&view, enabled)?;
    enabled_rx.await.map_err(|_| "The capture view closed.")??;
    let h = rx
        .await
        .map_err(|_| "The canvas closed before drawing.")??;
    view.set_size(tauri::LogicalSize::new(w as f64, h as f64))
        .map_err(|e| e.to_string())?;
    let (tx, rx) = oneshot::channel();
    webkit_capture::take(&view, job.request.format == "pdf", tx)?;
    let bytes = rx.await.map_err(|_| "The capture view closed.")??;
    if bytes.len() > MAX_BYTES {
        return Err("The exported file is too large (64 MB). Try a smaller size.".into());
    }
    let scale = capture_scale(&bytes, &job.request.format, w)?;
    Ok((bytes, h, scale))
}
fn capture_scale(bytes: &[u8], format: &str, width: u32) -> Result<f64, String> {
    if format == "pdf" {
        return if bytes.starts_with(b"%PDF-") {
            Ok(1.0)
        } else {
            Err("WebKit returned an invalid PDF.".into())
        };
    }
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("WebKit returned an invalid PNG.".into());
    }
    let px = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    if px == 0 {
        return Err("The image had no pixels.".into());
    }
    Ok(px as f64 / width as f64)
}

fn output_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    let ws = workspaces::folder(root, id)?;
    let base = ws
        .canonicalize()
        .map_err(|_| "The workspace folder is unavailable.")?;
    let mut dir = ws;
    for name in ["outputs", "exports"] {
        dir.push(name);
        if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Export folders cannot be symbolic links.".into());
        }
        let rel = dir
            .strip_prefix(root)
            .map_err(|_| "Export leaves the vault.")?
            .to_string_lossy();
        if run_guard::is_protected_target(root, &rel) {
            return Err("This export target is protected.".into());
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if !dir
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&base)
        {
            return Err("Export leaves the workspace.".into());
        }
    }
    Ok(dir)
}
fn save(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    for n in 1..=50 {
        let suffix = if n == 1 {
            String::new()
        } else {
            format!("-{n}")
        };
        let path = dir.join(format!("{stem}{suffix}.{ext}"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
                    let _ = fs::remove_file(&path);
                    return Err(format!("The export could not be saved: {e}"));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("The export could not be saved: {e}")),
        }
    }
    Err("There are already 50 exports with this name. Rename one and try again.".into())
}

/// One capture: register the job (one at a time), draw it in its own
/// offscreen child view within 15 s, and close that view whatever happens.
async fn run_capture(
    app: &AppHandle,
    request: ExportRequest,
    width: u32,
    measure: Option<MeasurePass>,
) -> Result<(Vec<u8>, u32, f64), String> {
    let job = CaptureJob {
        label: format!("canvas-capture-{}", SERIAL.fetch_add(1, Ordering::Relaxed)),
        request,
        width,
        measure,
    };
    let (tx, rx) = oneshot::channel();
    {
        let mut a = active().lock().map_err(|_| "Capture lock failed.")?;
        if a.is_some() {
            return Err("Another canvas is being exported. Try again when it finishes.".into());
        }
        *a = Some(Active {
            job: job.clone(),
            reply: Some(tx),
            document_url: None,
        });
    }
    let _cleanup = Cleanup(app.clone(), job.label.clone());
    tokio::time::timeout(std::time::Duration::from_secs(15), capture(app, job, rx))
        .await
        .map_err(|_| "The canvas did not finish drawing within 15 seconds. Nothing was saved.")?
}

/// Repo ideas 2026-10-27 #4: what Measure on the picture asks for: the
/// canvas as the reply's frame drew it (its source and state, the app's
/// scheme and theme, the frame's width, whether texture was allowed).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureRequest {
    source: String,
    state: Option<Value>,
    scheme: String,
    theme: Vec<(String, String)>,
    width: u32,
    texture: bool,
}

/// The four pictures (PNG, base64), all the same size: text as it is,
/// transparent, black and white; `scale` is device pixels per CSS pixel.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureCaptures {
    shown: String,
    hidden: String,
    black: String,
    white: String,
    width: u32,
    height: u32,
    scale: f64,
}

/// A PNG's size from its header.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let at = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    Some((at(16), at(20)))
}

/// The narrowest and widest frame Measure on the picture draws (CSS px).
const MEASURE_WIDTHS: std::ops::RangeInclusive<u32> = 240..=2400;

fn measure_export(request: &MeasureRequest) -> Result<ExportRequest, String> {
    if !MEASURE_WIDTHS.contains(&request.width) {
        return Err("The canvas frame's width is out of range.".into());
    }
    let export = ExportRequest {
        workspace_id: None,
        source: request.source.clone(),
        revision: None,
        state: request.state.clone(),
        format: "png".into(),
        preset: "doc".into(),
        look: "match".into(),
        copy: false,
        target: "attachment".into(),
        scheme: request.scheme.clone(),
        theme: request.theme.clone(),
        theme_name: None,
        svg: None,
        flow_index: None,
    };
    validate(&export)?;
    Ok(export)
}

/// Repo ideas 2026-10-27 #4, Measure on the picture: the person's click on a
/// reply canvas's unresolved design findings (text over a gradient, image or
/// texture). Draws the canvas offscreen four times, as its frame showed it
/// (text as it is, transparent, black, white), each picture at most
/// `MEASURE_MAX_PIXELS`, and returns them for the main window to score each
/// text's glyph pixels against what lies under them. Nothing is saved; main
/// window only; never an agent action.
#[tauri::command]
pub async fn canvas_measure(
    app: AppHandle,
    webview: Webview,
    request: MeasureRequest,
) -> Result<MeasureCaptures, String> {
    if webview.label() != "main" {
        return Err("Measure a canvas from Hollow's main window.".into());
    }
    let export = measure_export(&request)?;
    let mut pictures: Vec<Vec<u8>> = Vec::new();
    let mut size: Option<(u32, f64, (u32, u32))> = None;
    for text in MEASURE_TEXT {
        let pass = Some(MeasurePass {
            text,
            texture: request.texture,
        });
        let (bytes, h, scale) = run_capture(&app, export.clone(), request.width, pass).await?;
        // Checked after every pass, so a tall canvas stops after the first.
        let px = png_size(&bytes).ok_or("WebKit returned an invalid PNG.")?;
        if u64::from(px.0) * u64::from(px.1) > MEASURE_MAX_PIXELS {
            return Err("The canvas is too large to measure on the picture.".into());
        }
        match size {
            None => size = Some((h, scale, px)),
            Some(first) if first.0 == h && (first.1 - scale).abs() < f64::EPSILON && first.2 == px => {}
            Some(_) => return Err("The canvas drew differently from one picture to the next; it can't be measured on the picture.".into()),
        }
        pictures.push(bytes);
    }
    if pictures.iter().map(Vec::len).sum::<usize>() > 32 * 1024 * 1024 {
        return Err("The canvas is too large to measure on the picture.".into());
    }
    let (height, scale, _) = size.ok_or("Nothing was drawn.")?;
    let mut encoded = pictures.iter().map(|b| STANDARD.encode(b));
    let mut next = || encoded.next().unwrap_or_default();
    Ok(MeasureCaptures {
        shown: next(),
        hidden: next(),
        black: next(),
        white: next(),
        width: request.width,
        height,
        scale,
    })
}

#[tauri::command]
pub async fn canvas_export(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<ExportedFile, String> {
    if webview.label() != "main" {
        return Err("Export a canvas from Hollow's main window.".into());
    }
    export_request(&app, state.config.vault_path.clone(), request).await
}

/// The body of `canvas_export`: validate, capture (PNG and PDF draw in an
/// offscreen child of the main window, so it must exist; SVG is written
/// from the flow drawing alone) and save under the workspace's
/// `outputs/exports`.
pub(crate) async fn export_request(
    app: &AppHandle,
    root: PathBuf,
    request: ExportRequest,
) -> Result<ExportedFile, String> {
    let (id, w) = validate(&request)?;
    // Validate the destination before creating a capture, but create no folders
    // until capture succeeds (a timeout must write nothing).
    if request.target == "file" {
        workspaces::summary(&root, request.workspace_id.as_deref().unwrap())?;
    }
    if request.format == "svg" {
        let svg = request.svg.as_deref().unwrap();
        let (width, height) = validate_svg(svg)?;
        let dir = output_dir(&root, request.workspace_id.as_deref().unwrap())?;
        let stem = format!(
            "{id}-v{}-{}{}{}-flow-{}",
            request.revision.unwrap(),
            request.preset,
            if request.scheme == "dark" {
                "-dark"
            } else {
                ""
            },
            theme_suffix(request.theme_name.as_deref()),
            request.flow_index.unwrap()
        );
        let path = save(&dir, &stem, "svg", svg.as_bytes())?;
        return Ok(ExportedFile {
            rel: Some(format!(
                "outputs/exports/{}",
                path.file_name().unwrap().to_string_lossy()
            )),
            path: Some(path.to_string_lossy().into_owned()),
            png_base64: None,
            width,
            height,
            scale: 1.0,
            copied: false,
            warning: None,
        });
    }
    let (bytes, h, scale) = run_capture(app, request.clone(), w, None).await?;
    let mut result = ExportedFile {
        rel: None,
        path: None,
        png_base64: None,
        width: w,
        height: h,
        scale,
        copied: false,
        warning: None,
    };
    if request.target == "attachment" {
        if bytes.len() > 6 * 1024 * 1024 {
            return Err(
                "The picture attachment is too large (8 MB encoded). Try a smaller size.".into(),
            );
        }
        result.png_base64 = Some(STANDARD.encode(&bytes));
    } else {
        let ws_id = request.workspace_id.as_deref().unwrap();
        let dir = output_dir(&root, ws_id)?;
        let stem = format!(
            "{id}-v{}-{}{}{}",
            request.revision.unwrap_or(1),
            request.preset,
            if request.scheme == "dark" {
                "-dark"
            } else {
                ""
            },
            theme_suffix(request.theme_name.as_deref())
        );
        let path = save(&dir, &stem, &request.format, &bytes)?;
        result.rel = Some(format!(
            "outputs/exports/{}",
            path.file_name().unwrap().to_string_lossy()
        ));
        result.path = Some(path.to_string_lossy().into_owned());
    }
    if request.copy {
        let (tx, rx) = oneshot::channel();
        let copied = match webkit_capture::copy(app, bytes, tx) {
            Ok(()) => rx
                .await
                .unwrap_or_else(|_| Err("The clipboard closed before copying.".into())),
            Err(e) => Err(e),
        };
        match copied {
            Ok(()) => result.copied = true,
            Err(e) => result.warning = Some(e),
        }
    }
    Ok(result)
}

/// The export request a COO call means: always a file in the workspace's
/// `outputs/exports` (never an attachment or the clipboard), light or dark,
/// no custom theme.
pub(crate) fn agent_request(args: &Value) -> Result<ExportRequest, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
    let number = |key: &str| args.get(key).and_then(Value::as_u64).map(|n| n as u32);
    let scheme = if text("look").as_deref() == Some("dark") {
        "dark"
    } else {
        "light"
    };
    let request: ExportRequest = serde_json::from_value(serde_json::json!({
        "workspaceId": text("workspaceId"),
        "source": text("source").unwrap_or_default(),
        "revision": number("revision").unwrap_or(1),
        "state": null,
        "format": text("format").unwrap_or_else(|| "png".into()),
        "preset": text("preset").unwrap_or_else(|| "doc".into()),
        "look": scheme,
        "copy": false,
        "target": "file",
        "scheme": scheme,
        "theme": [],
        "themeName": null,
        "svg": text("svg"),
        "flowIndex": number("flowIndex"),
    }))
    .map_err(|e| format!("the export options don't fit: {e}. Nothing was saved."))?;
    validate(&request)?;
    Ok(request)
}

/// `canvases.export_canvas`: export a canvas to a file in a workspace. PNG
/// and PDF need Hollow's window (the capture is an offscreen child of it) and
/// are refused with a clear message while it is closed; SVG is written from
/// the flow drawing alone and works without it.
pub(crate) async fn agent_export(
    app: &AppHandle,
    root: PathBuf,
    args: &Value,
) -> Result<Value, String> {
    let request = agent_request(args)?;
    if request.format != "svg" && app.get_window("main").is_none() {
        return Err("Hollow's window is closed, so a PNG or PDF can't be drawn. Open Hollow and try again, or export an SVG flow drawing, which needs no window. Nothing was saved.".into());
    }
    let format = request.format.clone();
    let file = export_request(app, root, request).await?;
    Ok(serde_json::json!({
        "format": format,
        "rel": file.rel,
        "path": file.path,
        "width": file.width,
        "height": file.height,
        "scale": file.scale,
        "warning": file.warning,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ExportRequest {
        serde_json::from_value(serde_json::json!({"workspaceId":"test", "source":"<hc-canvas id=\"chart\" title=\"Chart\">Hi</hc-canvas>", "revision":2, "state":null,"format":"png","preset":"doc","look":"light","copy":false,"target":"file","scheme":"light","theme":[]})).unwrap()
    }
    /// Repo ideas 2026-10-27 #4: Measure on the picture draws a PNG for the
    /// main window only, at the reply frame's width, never saved or copied.
    #[test]
    fn a_measure_request_is_an_unsaved_png_at_the_frame_width() {
        let m: MeasureRequest = serde_json::from_value(serde_json::json!({"source":"<hc-canvas id=\"chart\" title=\"Chart\">Hi</hc-canvas>","state":null,"scheme":"dark","theme":[],"width":640,"texture":true})).unwrap();
        let e = measure_export(&m).unwrap();
        assert_eq!(
            (
                e.format.as_str(),
                e.target.as_str(),
                e.copy,
                e.workspace_id.is_none()
            ),
            ("png", "attachment", false, true)
        );
        for width in [239, 2401] {
            let bad = MeasureRequest { width, ..m.clone() };
            assert!(measure_export(&bad).err().unwrap().contains("width"));
        }
        assert!(measure_export(&MeasureRequest {
            scheme: "sepia".into(),
            ..m.clone()
        })
        .is_err());
        assert!(measure_export(&MeasureRequest {
            source: "not a canvas".into(),
            ..m.clone()
        })
        .is_err());
        assert!(serde_json::from_value::<MeasureRequest>(serde_json::json!({"source":"x","state":null,"scheme":"dark","theme":[],"width":640,"texture":true,"workspaceId":"w"})).is_err());
        // The capture host learns which pass it draws; an export has none.
        let job = |measure| CaptureJob {
            label: "x".into(),
            request: e.clone(),
            width: 640,
            measure,
        };
        let pass = serde_json::to_value(job(Some(MeasurePass {
            text: "hidden",
            texture: false,
        })))
        .unwrap();
        assert_eq!(
            pass["measure"],
            serde_json::json!({"text": "hidden", "texture": false})
        );
        assert_eq!(MEASURE_TEXT, ["shown", "hidden", "black", "white"]);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(3000u32.to_be_bytes());
        png.extend(4000u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((3000, 4000)));
        assert_eq!(
            MEASURE_MAX_PIXELS, 12_000_000,
            "measurePicture.ts holds the same limit"
        );
        assert_eq!(png_size(b"not a png at all, really"), None);
        assert!(serde_json::to_value(job(None))
            .unwrap()
            .get("measure")
            .is_none());
    }

    #[test]
    fn options_fail_before_capture() {
        let mut r = request();
        assert_eq!(validate(&r).unwrap(), ("chart".into(), 800));
        r.revision = Some(0);
        assert!(validate(&r).unwrap_err().contains("revision"));
        r.revision = None;
        assert!(validate(&r).unwrap_err().contains("revision"));
        r.revision = Some(1);
        r.workspace_id = None;
        assert!(validate(&r).is_err());
        r.target = "attachment".into();
        assert!(validate(&r).is_ok());
        r.source = "<script>bad()</script>".into();
        assert!(validate(&r).is_err());
    }
    #[test]
    fn named_palette_is_bounded_and_names_are_safe() {
        assert_eq!(
            theme_suffix(Some("../Solarized Dark / A")),
            "-solarized-dark-a"
        );
        assert_eq!(theme_suffix(Some("日本語")), "-theme");
        assert_eq!(theme_suffix(Some(&"x".repeat(100))).len(), 49);
        assert_eq!(
            theme_suffix(Some(&("a".repeat(47) + " b"))),
            "-".to_owned() + &"a".repeat(47)
        );
        let mut r = request();
        r.theme_name = Some("Paper".into());
        assert!(validate(&r).is_err());
        r.look = "match".into();
        r.theme = vec![("--app-canvas".into(), "#fef0d0".into())];
        assert!(validate(&r).is_ok());
        r.theme_name = Some("x".repeat(257));
        assert!(validate(&r).is_err());
    }
    #[test]
    fn svg_is_static_bounded_and_well_formed() {
        assert!(validate_svg(include_str!("../../src/canvas/flowSvg.golden.svg")).is_ok());
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="200"><style>text{fill:#ffffff}</style><title>A &amp; B</title><path d="M0,0 L1,1" marker-end="url(#arrow)"/></svg>"##;
        assert_eq!(validate_svg(svg).unwrap(), (800, 200));
        for bad in [
            svg.replace("<title>", "<script>"),
            svg.replace("<title>", "<foreignObject>"),
            svg.replace("<path ", "<path onload=\"bad()\" "),
            svg.replace("<path ", "<path href='https://bad' "),
            svg.replace("#ffffff", "url(https://bad)"),
            svg.replace("#ffffff", "u&#114;l(data:bad)"),
            svg.replace("#ffffff", "@import 'bad'"),
            svg.replace("text{fill:#ffffff}", "<g>text{fill:url(https://bad)}</g>"),
            svg.replace("</svg>", "</svg><svg width=\"1\" height=\"1\"/>"),
            svg.replace("height=\"200\"", "height=\"0\""),
            svg.replace("</svg>", ""),
            format!("<!DOCTYPE svg>{svg}"),
            format!("<svg {}", "x".repeat(2 * 1024 * 1024)),
        ] {
            assert!(validate_svg(&bad).is_err(), "accepted {bad:.80}");
        }
        let mut r = request();
        r.format = "svg".into();
        r.svg = Some(svg.into());
        assert!(validate(&r).is_err());
        r.flow_index = Some(2);
        assert!(validate(&r).is_ok());
        r.target = "attachment".into();
        assert!(validate(&r).is_err());
    }
    #[test]
    fn state_and_attachment_limits_are_validated() {
        let mut r = request();
        r.state = Some(serde_json::json!(["not an object"]));
        assert!(validate(&r).is_err());
        r.state = Some(serde_json::json!({"text": "x".repeat(16384)}));
        assert!(validate(&r).is_err());
        r.state = Some(serde_json::json!({"tab": "Details"}));
        assert!(validate(&r).is_ok());
        r.target = "attachment".into();
        r.format = "pdf".into();
        assert!(validate(&r).is_err());
        r.target = "file".into();
        r.copy = true;
        assert!(validate(&r).is_err());
        r.copy = false;
        assert!(validate(&r).is_ok());
        r.theme = vec![("color".into(), "red".into())];
        assert!(validate(&r).is_err());
    }
    #[test]
    fn labels_and_navigation_are_exact() {
        assert!(own_label("main", "canvas-capture-1").is_err());
        assert!(own_label("canvas-capture-2", "canvas-capture-1").is_err());
        assert!(own_label("canvas-capture-1", "canvas-capture-1").is_ok());
        let u = tauri::Url::parse("tauri://localhost/index.html?window=canvas-capture").unwrap();
        assert!(allowed_navigation(&u, &u, None));
        let doc = tauri::Url::parse("artifact://localhost/job-document").unwrap();
        assert!(allowed_navigation(&doc, &u, Some(&doc)));
        assert!(!allowed_navigation(&doc, &u, None));
        for bad in [
            "https://example.com/",
            "tauri://localhost/index.html?window=main",
            "tauri://localhost/index.html?window=canvas-capture#x",
            "artifact://localhost/other-document",
        ] {
            assert!(!allowed_navigation(
                &tauri::Url::parse(bad).unwrap(),
                &u,
                Some(&doc)
            ));
        }
    }
    #[test]
    fn sizes_and_magic_are_bounded() {
        assert_eq!(height(80.0, "png", "slide").unwrap(), 720);
        assert!(height(8001.0, "png", "doc").unwrap_err().contains("PDF"));
        assert!(height(14401.0, "pdf", "doc").is_err());
        assert!(height(f64::NAN, "png", "doc").is_err());
        assert!(capture_scale(b"not png", "png", 800).is_err());
        assert_eq!(capture_scale(b"%PDF-1.7", "pdf", 800).unwrap(), 1.0);
    }
    #[test]
    fn save_never_replaces_and_refuses_symlink_folder() {
        let t = tempfile::tempdir().unwrap();
        let first = save(t.path(), "chart-v2-doc", "png", b"first").unwrap();
        let second = save(t.path(), "chart-v2-doc", "png", b"second").unwrap();
        assert_eq!(fs::read(first).unwrap(), b"first");
        assert!(second.ends_with("chart-v2-doc-2.png"));
        fs::create_dir_all(t.path().join("workspaces/test")).unwrap();
        fs::create_dir_all(t.path().join("me")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                t.path().join("me"),
                t.path().join("workspaces/test/outputs"),
            )
            .unwrap();
            assert!(output_dir(t.path(), "test").is_err());
            assert!(!t.path().join("me/exports").exists());
        }
    }
    #[test]
    fn exhausted_names_leave_every_existing_file_intact() {
        let t = tempfile::tempdir().unwrap();
        for _ in 0..50 {
            save(t.path(), "chart-v1-doc", "png", b"kept").unwrap();
        }
        assert!(save(t.path(), "chart-v1-doc", "png", b"replacement").is_err());
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 50);
        for file in fs::read_dir(t.path()).unwrap() {
            assert_eq!(fs::read(file.unwrap().path()).unwrap(), b"kept");
        }
    }

    #[test]
    fn a_coo_export_is_always_a_file_and_follows_the_same_rules() {
        let canvas = "<hc-canvas id=\"chart\" title=\"Chart\">Hi</hc-canvas>";
        let ok = serde_json::json!({ "workspaceId": "w", "source": canvas, "format": "pdf", "look": "dark" });
        let request = agent_request(&ok).unwrap();
        assert_eq!(
            (
                request.target.as_str(),
                request.scheme.as_str(),
                request.format.as_str(),
                request.preset.as_str(),
                request.copy,
                request.revision
            ),
            ("file", "dark", "pdf", "doc", false, Some(1))
        );
        for bad in [
            serde_json::json!({ "source": canvas }),
            serde_json::json!({ "workspaceId": "w", "source": "not a canvas" }),
            serde_json::json!({ "workspaceId": "w", "source": canvas, "preset": "huge" }),
            serde_json::json!({ "workspaceId": "w", "source": canvas, "format": "svg" }),
            serde_json::json!({ "workspaceId": "w", "source": canvas, "svg": "<svg ", "flowIndex": 1 }),
        ] {
            assert!(agent_request(&bad).is_err(), "{bad}");
        }
    }
}
