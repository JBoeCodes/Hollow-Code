//! Shared WebKit image encoding and the canvas export capture primitives.
//! The private occlusion setter is feature-detected; unavailable means a clear
//! failure, never a blank image reported as a successful export.

#[cfg(target_os = "macos")]
mod native {
    use block2::RcBlock;
    use objc2::{msg_send, sel, MainThreadMarker};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSPasteboard};
    use objc2_foundation::{NSData, NSDictionary, NSError, NSString};
    use objc2_web_kit::{WKPDFConfiguration, WKSnapshotConfiguration, WKWebView};
    use std::sync::Mutex;
    use tokio::sync::oneshot::Sender;

    /// Decode HEIC on this Mac before it is saved as an attachment. AppKit
    /// reads the real container; a filename or MIME claim cannot opt in.
    pub(crate) fn heic_png(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if !super::is_heic(bytes) {
            return Ok(None);
        }
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(bytes))
            .ok_or("This HEIC image could not be decoded.")?;
        let width = rep.pixelsWide();
        let height = rep.pixelsHigh();
        if width <= 0
            || height <= 0
            || width > 8192
            || height > 8192
            || width.saturating_mul(height) > 40_000_000
        {
            return Err(
                "This HEIC image is too large to convert (40 million pixels maximum).".into(),
            );
        }
        // SAFETY: an empty dictionary is valid for PNG encoding.
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("This HEIC image could not be converted to PNG.")?
        .to_vec();
        if png.len() as u64 > crate::chat::MAX_IMAGE_BYTES {
            return Err("The converted image is over the 10 MB image preview limit.".into());
        }
        Ok(Some(png))
    }

    pub fn png(image: &NSImage) -> Result<Vec<u8>, String> {
        let tiff = image
            .TIFFRepresentation()
            .ok_or("The capture had no pixels.")?;
        let rep =
            NSBitmapImageRep::imageRepWithData(&tiff).ok_or("The capture could not be read.")?;
        // SAFETY: an empty properties dictionary is valid for PNG encoding.
        unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .map(|data| data.to_vec())
        .ok_or_else(|| "The capture could not be saved as PNG.".into())
    }

    pub fn enable_frames(
        view: &tauri::Webview,
        tx: Sender<Result<(), String>>,
    ) -> Result<(), String> {
        view.with_webview(move |platform| {
            // SAFETY: with_webview runs on the main thread, with its WKWebView alive.
            let result = unsafe {
                let wk = &*(platform.inner() as *const WKWebView);
                let available: bool =
                    msg_send![wk, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)];
                if available {
                    let _: () = msg_send![wk, _setWindowOcclusionDetectionEnabled: false];
                    Ok(())
                } else {
                    Err(
                        "This WebKit version cannot draw an offscreen canvas. Nothing was saved."
                            .into(),
                    )
                }
            };
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())
    }

    pub fn take(
        view: &tauri::Webview,
        pdf: bool,
        tx: Sender<Result<Vec<u8>, String>>,
    ) -> Result<(), String> {
        view.with_webview(move |platform| {
            let reply = Mutex::new(Some(tx));
            // SAFETY: the platform view and the main-thread marker are valid in this callback.
            unsafe {
                let wk = &*(platform.inner() as *const WKWebView);
                let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
                if pdf {
                    let config = WKPDFConfiguration::new(mtm);
                    config.setRect(wk.bounds());
                    let done = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
                        if let Some(tx) = reply.lock().ok().and_then(|mut v| v.take()) {
                            let result = data
                                .as_ref()
                                .map(|d| d.to_vec())
                                .ok_or_else(|| failure(error));
                            let _ = tx.send(result);
                        }
                    });
                    wk.createPDFWithConfiguration_completionHandler(Some(&config), &done);
                } else {
                    let config = WKSnapshotConfiguration::new(mtm);
                    config.setRect(wk.bounds());
                    config.setAfterScreenUpdates(true);
                    let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                        if let Some(tx) = reply.lock().ok().and_then(|mut v| v.take()) {
                            let result = image.as_ref().ok_or_else(|| failure(error)).and_then(png);
                            let _ = tx.send(result);
                        }
                    });
                    wk.takeSnapshotWithConfiguration_completionHandler(Some(&config), &done);
                }
            }
        })
        .map_err(|e| e.to_string())
    }

    unsafe fn failure(error: *mut NSError) -> String {
        error
            .as_ref()
            .map(|e| e.localizedDescription().to_string())
            .unwrap_or_else(|| "WebKit returned no capture.".into())
    }

    pub fn copy(
        app: &tauri::AppHandle,
        bytes: Vec<u8>,
        tx: Sender<Result<(), String>>,
    ) -> Result<(), String> {
        app.run_on_main_thread(move || {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            let ok = pasteboard.setData_forType(
                Some(&NSData::with_bytes(&bytes)),
                &NSString::from_str("public.png"),
            );
            let _ = tx.send(if ok {
                Ok(())
            } else {
                Err("The image was saved, but could not be copied to the clipboard.".into())
            });
        })
        .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
pub use native::*;

fn is_heic(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(
            &bytes[8..12],
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1"
        )
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn heic_png(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if is_heic(bytes) {
        Err("HEIC conversion currently needs macOS.".into())
    } else {
        Ok(None)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn enable_frames(
    _: &tauri::Webview,
    _: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<(), String> {
    Err("Canvas export currently needs macOS.".into())
}
#[cfg(not(target_os = "macos"))]
pub fn take(
    _: &tauri::Webview,
    _: bool,
    _: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
) -> Result<(), String> {
    Err("Canvas export currently needs macOS.".into())
}
#[cfg(not(target_os = "macos"))]
pub fn copy(
    _: &tauri::AppHandle,
    _: Vec<u8>,
    _: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<(), String> {
    Err("Canvas export currently needs macOS.".into())
}
