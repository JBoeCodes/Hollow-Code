//! Local icon records. Never writes to the vault or the signed app bundle.
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_RECORD: u64 = 16 * 1024 * 1024;
pub const SIZES: [u32; 5] = [16, 32, 64, 128, 512];
pub static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Original {
    pub format: String,
    pub bytes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IconRecord {
    pub version: u8,
    pub revision: String,
    /// Absent in existing uploads/maker records. Only the preset command sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub original: Original,
    pub editor: serde_json::Value,
    pub pngs: BTreeMap<u32, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

pub fn path(target: &str) -> Result<PathBuf, String> {
    if !["app", "mainAgent"].contains(&target) {
        return Err("Unknown icon target".into());
    }
    Ok(crate::agent_settings::app_data_dir()?
        .join("custom-icons")
        .join(format!("{target}.json")))
}

fn bytes(encoded: &str, cap: usize) -> Result<Vec<u8>, String> {
    if encoded.len() > cap.div_ceil(3) * 4 {
        return Err("Icon exceeds the size limit".into());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid icon encoding")?;
    if bytes.is_empty() || bytes.len() > cap {
        return Err("Icon is empty or exceeds 8 MiB".into());
    }
    Ok(bytes)
}

fn dimensions(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 || u64::from(w) * u64::from(h) > 16_000_000 {
        return Err("Use an icon up to 4096 px per side and 16 megapixels".into());
    }
    Ok(())
}

fn raster(data: &[u8], format: ImageFormat) -> Result<DynamicImage, String> {
    let reader = ImageReader::with_format(Cursor::new(data), format);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("Invalid image: {e}"))?;
    dimensions(w, h)?;
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("Cannot decode icon: {e}"))
}

fn png(image: &DynamicImage) -> Result<String, String> {
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(STANDARD.encode(out.into_inner()))
}

/// Modern ICNS stores PNG representations. Inspect every chunk length before
/// choosing the largest supported representation; legacy encodings fail clearly.
fn icns(data: &[u8]) -> Result<DynamicImage, String> {
    let invalid = || "Invalid ICNS; export a PNG or an ICNS with PNG representations".to_string();
    if data.len() < 8
        || &data[..4] != b"icns"
        || u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize != data.len()
    {
        return Err(invalid());
    }
    let mut at = 8;
    let mut best: Option<DynamicImage> = None;
    let mut chunks = 0;
    while at < data.len() {
        chunks += 1;
        if chunks > 64 {
            return Err(invalid());
        }
        if data.len() - at < 8 {
            return Err(invalid());
        }
        let len = u32::from_be_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        if len < 8 || len > data.len() - at {
            return Err(invalid());
        }
        let chunk = &data[at + 8..at + len];
        if chunk.starts_with(b"\x89PNG\r\n\x1a\n") {
            let image = raster(chunk, ImageFormat::Png)?;
            if image.width() != image.height() || image.width() > 1024 {
                return Err(invalid());
            }
            if best.as_ref().is_none_or(|b| image.width() > b.width()) {
                best = Some(image);
            }
        }
        at += len;
    }
    best.ok_or_else(invalid)
}

/// Strict static SVG subset: reject active content rather than silently changing
/// its meaning. No CSS, URL-bearing attributes, entities or external resources.
fn svg_dimensions(svg: &str) -> Result<(u32, u32), String> {
    use quick_xml::{events::Event, Reader};
    let bad =
        || "Use a static SVG without scripts, styles, links or external references".to_string();
    let mut reader = Reader::from_str(svg);
    let mut stack = Vec::<String>::new();
    let mut seen = false;
    let mut namespace = false;
    let mut size = None;
    let mut elements = 0;
    loop {
        match reader.read_event().map_err(|_| bad())? {
            Event::Start(e) | Event::Empty(e) => {
                elements += 1;
                if elements > 10_000 || stack.len() > 32 {
                    return Err(bad());
                }
                let name = std::str::from_utf8(e.name().into_inner())
                    .map_err(|_| bad())?
                    .to_string();
                if ![
                    "svg",
                    "g",
                    "defs",
                    "linearGradient",
                    "radialGradient",
                    "stop",
                    "path",
                    "rect",
                    "circle",
                    "ellipse",
                    "line",
                    "polyline",
                    "polygon",
                    "text",
                    "tspan",
                    "title",
                    "desc",
                ]
                .contains(&name.as_str())
                {
                    return Err(format!("SVG element <{name}> is not supported. {}", bad()));
                }
                if stack.is_empty() && (seen || name != "svg") {
                    return Err(bad());
                }
                if name == "svg" && seen {
                    return Err(bad());
                }
                let mut width = None;
                let mut height = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| bad())?;
                    let key = std::str::from_utf8(attr.key.into_inner()).map_err(|_| bad())?;
                    let value = attr.unescape_value().map_err(|_| bad())?;
                    if ![
                        "xmlns",
                        "viewBox",
                        "id",
                        "offset",
                        "stop-color",
                        "stop-opacity",
                        "gradientUnits",
                        "gradientTransform",
                        "fx",
                        "fy",
                        "width",
                        "height",
                        "x",
                        "y",
                        "x1",
                        "y1",
                        "x2",
                        "y2",
                        "cx",
                        "cy",
                        "r",
                        "rx",
                        "ry",
                        "d",
                        "points",
                        "fill",
                        "fill-rule",
                        "fill-opacity",
                        "stroke",
                        "stroke-width",
                        "stroke-linecap",
                        "stroke-linejoin",
                        "stroke-opacity",
                        "stroke-dasharray",
                        "opacity",
                        "transform",
                        "font-size",
                        "font-family",
                        "font-weight",
                        "text-anchor",
                        "dominant-baseline",
                    ]
                    .contains(&key)
                    {
                        return Err(bad());
                    }
                    if key == "xmlns" {
                        if value != "http://www.w3.org/2000/svg" {
                            return Err(bad());
                        }
                        if name == "svg" {
                            namespace = true;
                        }
                    } else {
                        let internal = ["fill", "stroke"].contains(&key)
                            && value
                                .strip_prefix("url(#")
                                .and_then(|v| v.strip_suffix(')'))
                                .is_some_and(|id| {
                                    !id.is_empty()
                                        && id.chars().all(|c| {
                                            c.is_ascii_alphanumeric() || c == '-' || c == '_'
                                        })
                                });
                        if (!internal && value.to_ascii_lowercase().contains("url"))
                            || value.contains(['\\', '&', '<', '>', '@', ':'])
                        {
                            return Err(bad());
                        }
                    }
                    if name == "svg" {
                        if key == "width" || key == "height" {
                            let n = value
                                .trim_end_matches("px")
                                .parse::<u32>()
                                .map_err(|_| bad())?;
                            dimensions(n, 1)?;
                            if key == "width" {
                                width = Some(n);
                            } else {
                                height = Some(n);
                            }
                        }
                        if key == "viewBox" {
                            let v: Vec<f64> = value
                                .split(|c: char| c.is_whitespace() || c == ',')
                                .filter(|s| !s.is_empty())
                                .map(str::parse)
                                .collect::<Result<_, _>>()
                                .map_err(|_| bad())?;
                            if v.len() != 4
                                || v.iter().any(|n| !n.is_finite())
                                || v[2] < 1.0
                                || v[3] < 1.0
                                || v[2] > 4096.0
                                || v[3] > 4096.0
                            {
                                return Err(bad());
                            }
                            size = Some((v[2].ceil() as u32, v[3].ceil() as u32));
                        }
                    }
                }
                if name == "svg" {
                    seen = true;
                    if let (Some(w), Some(h)) = (width, height) {
                        dimensions(w, h)?;
                        size = Some((w, h));
                    }
                }
                if !svg[..reader.buffer_position() as usize].ends_with("/>") {
                    stack.push(name);
                }
            }
            Event::End(e) => {
                if stack.pop().as_deref().map(str::as_bytes) != Some(e.name().as_ref()) {
                    return Err(bad());
                }
            }
            Event::Text(e) => {
                if stack.is_empty() && !e.iter().all(u8::is_ascii_whitespace) {
                    return Err(bad());
                }
            }
            Event::Decl(_) if !seen => {}
            Event::Comment(_) => {}
            Event::Eof => break,
            _ => return Err(bad()),
        }
    }
    let (w, h) = size.ok_or_else(bad)?;
    if !seen || !namespace || !stack.is_empty() {
        return Err(bad());
    }
    dimensions(w, h)?;
    Ok((w, h))
}

pub fn prepare(original: &Original) -> Result<Prepared, String> {
    let data = bytes(&original.bytes, MAX_INPUT)?;
    if original.format == "svg" {
        let svg = std::str::from_utf8(&data).map_err(|_| "SVG must be UTF-8")?;
        let (width, height) = svg_dimensions(svg)?;
        return Ok(Prepared {
            url: format!("data:image/svg+xml;base64,{}", STANDARD.encode(&data)),
            width,
            height,
        });
    }
    let image = match original.format.as_str() {
        "png" => raster(&data, ImageFormat::Png)?,
        "jpeg" => raster(&data, ImageFormat::Jpeg)?,
        "icns" => icns(&data)?,
        _ => return Err("Choose PNG, JPEG, SVG or ICNS".into()),
    };
    let preview = if image.width() > 1024 || image.height() > 1024 {
        image.thumbnail(1024, 1024)
    } else {
        image.clone()
    };
    Ok(Prepared {
        width: image.width(),
        height: image.height(),
        url: format!("data:image/png;base64,{}", png(&preview)?),
    })
}

pub fn make_record(
    original: Original,
    rendered: &str,
    editor: serde_json::Value,
) -> Result<IconRecord, String> {
    prepare(&original)?;
    if serde_json::to_vec(&editor)
        .map_err(|e| e.to_string())?
        .len()
        > 8192
    {
        return Err("Icon settings are too large".into());
    }
    let image = raster(&bytes(rendered, 2 * 1024 * 1024)?, ImageFormat::Png)?;
    if image.width() != 512 || image.height() != 512 {
        return Err("Rendered icon must be 512 × 512".into());
    }
    let mut pngs = BTreeMap::new();
    for size in SIZES {
        pngs.insert(
            size,
            png(&image.resize_exact(size, size, image::imageops::FilterType::Lanczos3))?,
        );
    }
    use sha2::{Digest, Sha256};
    let revision = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&original, &editor, &pngs)).map_err(|e| e.to_string())?
        )
    );
    Ok(IconRecord {
        version: 1,
        revision,
        preset: None,
        original,
        editor,
        pngs,
    })
}

/// A local image file read for use as an icon: its format name, bytes and
/// decoded picture (PNG, JPEG or ICNS only; an SVG needs the Appearance
/// editor's own rendering).
fn load_file(path: &Path) -> Result<(&'static str, Vec<u8>, DynamicImage), String> {
    if !path.is_absolute() {
        return Err("Give the image's full path. Nothing changed.".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't open the image: {e}"))?;
    if !meta.is_file() {
        return Err("That isn't a file. Nothing changed.".into());
    }
    if meta.len() == 0 || meta.len() > MAX_INPUT as u64 {
        return Err("Icon is empty or exceeds 8 MiB".into());
    }
    let format = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "png",
        Some("jpg") | Some("jpeg") => "jpeg",
        Some("icns") => "icns",
        Some("svg") => {
            return Err(
                "An SVG icon needs the Appearance editor's own rendering: ask the person to upload it there, or give a PNG, JPEG or ICNS. Nothing changed."
                    .into(),
            )
        }
        _ => return Err("Choose a PNG, JPEG or ICNS file. Nothing changed.".into()),
    };
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read the image: {e}"))?;
    if data.len() > MAX_INPUT {
        return Err("Icon is empty or exceeds 8 MiB".into());
    }
    let image = match format {
        "png" => raster(&data, ImageFormat::Png)?,
        "jpeg" => raster(&data, ImageFormat::Jpeg)?,
        _ => icns(&data)?,
    };
    Ok((format, data, image))
}

/// What a COO `prepare_custom_icon` says of an image file: its format and
/// size, nothing of its content.
pub fn inspect_file(path: &Path) -> Result<serde_json::Value, String> {
    let (format, data, image) = load_file(path)?;
    Ok(serde_json::json!({
        "ok": true,
        "format": format,
        "width": image.width(),
        "height": image.height(),
        "bytes": data.len(),
    }))
}

/// An icon record built from a local image file, rendered here instead of in
/// the Appearance editor (a COO action's route): the picture is fitted whole
/// inside 512 x 512 on a transparent square, never cropped.
pub fn record_from_file(path: &Path) -> Result<IconRecord, String> {
    let (format, data, image) = load_file(path)?;
    let fitted = image.resize(512, 512, image::imageops::FilterType::Lanczos3);
    let mut canvas = image::RgbaImage::new(512, 512);
    let x = i64::from((512 - fitted.width()) / 2);
    let y = i64::from((512 - fitted.height()) / 2);
    image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), x, y);
    let rendered = png(&DynamicImage::ImageRgba8(canvas))?;
    make_record(
        Original {
            format: format.into(),
            bytes: STANDARD.encode(&data),
        },
        &rendered,
        serde_json::json!({ "source": "coo", "fit": "contain" }),
    )
}

/// What a COO action may see of a saved icon: that one is set, which, its
/// revision and sizes. Never the original image, its editor state or any
/// rendered bytes (the original is the person's private picture).
pub fn summary(target: &str, record: Option<&IconRecord>) -> serde_json::Value {
    match record {
        None => serde_json::json!({ "target": target, "custom": false }),
        Some(r) => serde_json::json!({
            "target": target,
            "custom": true,
            "revision": r.revision,
            "preset": r.preset,
            "originalFormat": r.original.format,
            "originalBytes": r.original.bytes.len() / 4 * 3,
            "sizes": r.pngs.keys().collect::<Vec<_>>(),
        }),
    }
}

pub fn read_at(path: &Path) -> Result<Option<IconRecord>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut data = Vec::new();
    file.take(MAX_RECORD + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_RECORD {
        return Err("ICON_DAMAGED: Saved icon is too large; left unchanged".into());
    }
    // Only stored-data validation is damage. Opening/reading a file may fail
    // for ordinary I/O reasons and must keep that recovery category.
    let damaged = |e| format!("ICON_DAMAGED: Saved icon is damaged; left unchanged: {e}");
    let record: Option<IconRecord> =
        serde_json::from_slice(&data).map_err(|e| damaged(e.to_string()))?;
    if let Some(r) = &record {
        if r.version != 1 || r.pngs.len() != SIZES.len() {
            return Err(damaged("Unsupported saved icon".into()));
        }
        for size in SIZES {
            let data = bytes(
                r.pngs
                    .get(&size)
                    .ok_or_else(|| damaged("Incomplete saved icon".into()))?,
                2 * 1024 * 1024,
            )
            .map_err(&damaged)?;
            let image = raster(&data, ImageFormat::Png).map_err(&damaged)?;
            if image.width() != size || image.height() != size {
                return Err(damaged("Invalid saved icon dimensions".into()));
            }
        }
    }
    Ok(record)
}

pub fn write_at(path: &Path, record: Option<&IconRecord>) -> Result<(), String> {
    let data = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_RECORD {
        return Err("Icon record exceeds the size limit".into());
    }
    crate::vault::atomic_write_private(path, &data)
}

/// Keep the authored tiny representations when AppKit chooses a Dock size.
#[cfg(any(target_os = "macos", test))]
fn dock_data(record: &IconRecord) -> Result<Vec<u8>, String> {
    let mut chunks = Vec::new();
    for (size, tag) in [
        (16, b"icp4"),
        (32, b"icp5"),
        (64, b"icp6"),
        (128, b"ic07"),
        (512, b"ic09"),
    ] {
        let png = bytes(
            record.pngs.get(&size).ok_or("Missing Dock icon size")?,
            2 * 1024 * 1024,
        )?;
        chunks.extend_from_slice(tag);
        chunks.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
        chunks.extend_from_slice(&png);
    }
    let mut data = b"icns".to_vec();
    data.extend_from_slice(&((chunks.len() + 8) as u32).to_be_bytes());
    data.extend(chunks);
    Ok(data)
}

/// Must run on the UI thread. None restores the bundle's system-owned icon.
pub fn apply_dock(record: Option<&IconRecord>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AllocAnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;
        let mtm = MainThreadMarker::new().ok_or("Dock icon requires the main thread")?;
        let application = NSApplication::sharedApplication(mtm);
        let Some(record) = record else {
            // AppKit's documented restoration path; do not mask a bundled
            // appearance family with an explicit flattened fallback image.
            unsafe { application.setApplicationIconImage(None) };
            return Ok(());
        };
        let data = dock_data(record)?;
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&data))
            .ok_or("Cannot load Dock icon")?;
        // SAFETY: a live, non-null NSImage on AppKit's main thread.
        unsafe {
            application.setApplicationIconImage(Some(&image));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = record;
    Ok(())
}

/// Runtime identity only. The OS-owned bundle/notification icon stays untouched.
pub fn apply_runtime(app: &tauri::AppHandle, record: Option<&IconRecord>) -> Result<(), String> {
    use tauri::Manager;
    let custom = record.map(runtime_image).transpose()?;
    let window_icon = custom
        .clone()
        .or_else(|| app.default_window_icon().cloned());
    #[cfg(target_os = "macos")]
    let tray_icon = match &custom {
        Some(icon) => icon.clone(),
        None => tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))
            .map_err(|e| e.to_string())?,
    };
    #[cfg(not(target_os = "macos"))]
    let tray_icon = window_icon.clone().ok_or("Missing stock window icon")?;
    apply_dock(record)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_icon_with_as_template(
            Some(tray_icon),
            cfg!(target_os = "macos") && record.is_none(),
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(icon) = window_icon {
        for window in app.webview_windows().values() {
            window.set_icon(icon.clone()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn runtime_image(record: &IconRecord) -> Result<tauri::image::Image<'static>, String> {
    let png = bytes(
        record.pngs.get(&64).ok_or("Missing runtime icon")?,
        64 * 1024,
    )?;
    tauri::image::Image::from_bytes(&png).map_err(|e| e.to_string())
}

pub fn restore(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = WRITE_LOCK.lock().await;
        let result = tauri::async_runtime::spawn_blocking(|| read_at(&path("app")?)).await;
        match result {
            Ok(Ok(Some(record))) => {
                let handle = app.clone();
                let (send, receive) = tokio::sync::oneshot::channel();
                let _ = app.run_on_main_thread(move || {
                    let _ = send.send(apply_runtime(&handle, Some(&record)));
                });
                if let Ok(Err(e)) = receive.await {
                    eprintln!("Custom app icon: {e}");
                }
            }
            Ok(Ok(None)) => {}
            other => eprintln!("Custom Dock icon restore failed: {other:?}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_dock_retains_exact_small_preset_representations() {
        let record = crate::app_icon_presets::record("threshold")
            .unwrap()
            .unwrap();
        let data = dock_data(&record).unwrap();
        assert_eq!(&data[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize,
            data.len()
        );
        let mut at = 8;
        for (size, tag) in [
            (16, b"icp4"),
            (32, b"icp5"),
            (64, b"icp6"),
            (128, b"ic07"),
            (512, b"ic09"),
        ] {
            assert_eq!(&data[at..at + 4], tag);
            let len = u32::from_be_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
            assert_eq!(
                &data[at + 8..at + len],
                STANDARD.decode(&record.pngs[&size]).unwrap()
            );
            at += len;
        }
        assert_eq!(at, data.len());
        assert_eq!(icns(&data).unwrap().width(), 512);
    }

    fn original(format: &str, data: &[u8]) -> Original {
        Original {
            format: format.into(),
            bytes: STANDARD.encode(data),
        }
    }

    #[test]
    fn svg_rejects_active_external_and_excessive_content() {
        for body in [
            "<script>alert(1)</script>",
            "<image href='https://example.com/a' />",
            "<foreignObject/>",
            "<animate/>",
            "<rect onload='x'/>",
            "<style>rect{fill:red}</style>",
            "<rect fill='url(https://example.com)'/>",
            "<rect fill='url(&#104;ttps://example.com)'/>",
        ] {
            assert!(
                svg_dimensions(&format!("<svg width='32' height='32'>{body}</svg>")).is_err(),
                "{body}"
            );
        }
        for svg in ["<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><svg width='32' height='32'>&x;</svg>", "<svg width='99999999' height='32' viewBox='0 0 32 32'/>", "<svg viewBox='0 0 NaN 32'/>", "<svg width='32' height='32'/><svg width='32' height='32'/>", "<svg width='32' height='32'><g></svg>"] {
            assert!(svg_dimensions(svg).is_err(), "{svg}");
        }
        let safe = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><defs><linearGradient id='a'><stop offset='0' stop-color='#fff'/></linearGradient></defs><circle cx='16' cy='16' r='14' fill='url(#a)'/></svg>";
        assert_eq!(svg_dimensions(safe).unwrap(), (32, 32));
    }

    #[test]
    fn refused_svg_names_the_element() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'><defs><clipPath id='crop'><rect width='16' height='16'/></clipPath></defs></svg>";
        let error = svg_dimensions(svg).unwrap_err();
        assert!(error.contains("clipPath"), "{error}");
    }

    #[test]
    fn malformed_saved_record_has_a_stable_damage_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        std::fs::write(&path, b"{").unwrap();
        let error = read_at(&path).unwrap_err();
        assert!(error.starts_with("ICON_DAMAGED:"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"{");
    }

    #[test]
    fn malformed_and_wrong_type_inputs_are_refused() {
        assert!(svg_dimensions("<svg width='32' height='32'/>").is_err());
        assert!(prepare(&original("png", b"not a png")).is_err());
        assert!(prepare(&original("jpeg", b"GIF89a")).is_err());
        assert!(prepare(&original("html", b"<h1>x</h1>")).is_err());
        assert!(bytes(&"A".repeat(MAX_INPUT.div_ceil(3) * 4 + 4), MAX_INPUT).is_err());
        assert!(dimensions(4096, 4096).is_err());
        assert!(dimensions(0, 32).is_err());
        assert!(dimensions(4097, 1).is_err());
        assert!(icns(b"icns\0\0\0\x10ic07\0\0\0\0").is_err());
    }

    #[test]
    fn png_jpeg_and_icns_decode_with_real_dimensions() {
        let image = DynamicImage::new_rgb8(32, 16);
        for (format, name) in [(ImageFormat::Png, "png"), (ImageFormat::Jpeg, "jpeg")] {
            let mut data = Cursor::new(Vec::new());
            image.write_to(&mut data, format).unwrap();
            let prepared = prepare(&original(name, data.get_ref())).unwrap();
            assert_eq!((prepared.width, prepared.height), (32, 16));
        }
        let png = STANDARD
            .decode(png(&DynamicImage::new_rgba8(32, 32)).unwrap())
            .unwrap();
        let mut data = b"icns".to_vec();
        data.extend_from_slice(&((png.len() + 16) as u32).to_be_bytes());
        data.extend_from_slice(b"icp5");
        data.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
        data.extend_from_slice(&png);
        assert_eq!(prepare(&original("icns", &data)).unwrap().width, 32);
        data[7] = 0;
        assert!(icns(&data).is_err());
    }

    #[test]
    fn prepared_raster_is_bounded_without_changing_the_original() {
        let encoded = png(&DynamicImage::new_rgba8(2048, 1024)).unwrap();
        let original = Original {
            format: "png".into(),
            bytes: encoded.clone(),
        };
        let prepared = prepare(&original).unwrap();
        assert_eq!((prepared.width, prepared.height), (2048, 1024));
        let image = raster(
            &STANDARD
                .decode(prepared.url.split_once(',').unwrap().1)
                .unwrap(),
            ImageFormat::Png,
        )
        .unwrap();
        assert_eq!((image.width(), image.height()), (1024, 512));
        assert_eq!(original.bytes, encoded);
    }

    #[test]
    fn records_preserve_original_and_reset_atomically() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("icons").join("app.json");
        let encoded = png(&DynamicImage::new_rgba8(512, 512)).unwrap();
        let original = Original {
            format: "png".into(),
            bytes: encoded.clone(),
        };
        let record = make_record(original, &encoded, serde_json::json!({"fit":"crop"})).unwrap();
        write_at(&path, Some(&record)).unwrap();
        let saved = read_at(&path).unwrap().unwrap();
        assert_eq!(saved.original.bytes, encoded);
        assert_eq!(saved.pngs.keys().copied().collect::<Vec<_>>(), SIZES);
        assert_eq!(saved.revision, record.revision);
        let runtime = runtime_image(&saved).unwrap();
        assert_eq!((runtime.width(), runtime.height()), (64, 64));
        assert_eq!(runtime.rgba().len(), 64 * 64 * 4);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, b"broken").unwrap();
        assert!(read_at(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        write_at(&path, None).unwrap();
        assert!(read_at(&path).unwrap().is_none());
    }
}
