//! The Appearance editor's commands, and the COO's own route to the same
//! store (`settings.read_custom_icon`, `save_custom_icon`, `reset_custom_icon`,
//! `select_app_icon_preset`): the same compare-and-swap `change`, but a COO
//! action never sees or returns the original image's bytes.
use crate::custom_icons::{self as icons, IconRecord, Original, Prepared};
use tauri::Emitter;

#[tauri::command]
pub async fn read_custom_icon(
    target: String,
    include_original: Option<bool>,
) -> Result<Option<IconRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut record = icons::read_at(&icons::path(&target)?)?;
        if include_original != Some(true) {
            if let Some(r) = &mut record {
                r.original.bytes.clear();
                r.editor = serde_json::Value::Null;
                r.pngs.retain(|size, _| *size == 128);
            }
        }
        Ok(record)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn prepare_custom_icon(original: Original) -> Result<Prepared, String> {
    tauri::async_runtime::spawn_blocking(move || icons::prepare(&original))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) async fn change(
    app: tauri::AppHandle,
    target: String,
    expected: Option<String>,
    next: Option<IconRecord>,
) -> Result<Option<IconRecord>, String> {
    let _guard = icons::WRITE_LOCK.lock().await;
    let path = icons::path(&target)?;
    let write_path = path.clone();
    let saved = next.clone();
    let old = tauri::async_runtime::spawn_blocking(move || {
        let old = match icons::read_at(&write_path) {
            Ok(old) => old,
            Err(e) if e.starts_with("ICON_DAMAGED:") && saved.is_none() && expected.is_none() => {
                // Only the person's explicit Reset can recover a damaged store.
                // Keep its exact bytes beside it before the atomic replacement.
                let backup = write_path
                    .with_extension(format!("damaged-{:016x}.json", rand::random::<u64>()));
                std::fs::hard_link(&write_path, &backup)
                    .map_err(|e| format!("Could not preserve the damaged icon: {e}"))?;
                None
            }
            Err(e) => return Err(e),
        };
        if old.as_ref().map(|r| &r.revision) != expected.as_ref() {
            return Err("ICON_CONFLICT: The saved icon changed. Nothing was replaced.".to_string());
        }
        icons::write_at(&write_path, saved.as_ref())?;
        Ok(old)
    })
    .await
    .map_err(|e| e.to_string())??;
    if target == "app" {
        let dock = next.clone();
        let handle = app.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let dispatched = app.run_on_main_thread(move || {
            let _ = send.send(icons::apply_runtime(&handle, dock.as_ref()));
        });
        let applied = match dispatched {
            Ok(()) => receive.await.map_err(|e| e.to_string()).and_then(|r| r),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = applied {
            icons::write_at(&path, old.as_ref()).map_err(|rollback| {
                format!(
                    "Dock update failed: {e}; restoring the saved choice also failed: {rollback}"
                )
            })?;
            // A tray/window failure may follow a successful Dock update.
            // Restore the runtime artwork as well as the persisted record.
            let handle = app.clone();
            let (send, receive) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let _ = send.send(icons::apply_runtime(&handle, old.as_ref()));
            })
            .map_err(|rollback| {
                format!("App icon update failed: {e}; runtime restore failed: {rollback}")
            })?;
            receive
                .await
                .map_err(|rollback| {
                    format!("App icon update failed: {e}; runtime restore failed: {rollback}")
                })?
                .map_err(|rollback| {
                    format!("App icon update failed: {e}; runtime restore failed: {rollback}")
                })?;
            return Err(e);
        }
    }
    let _ = app.emit("custom-icon-changed", &target);
    Ok(next)
}

#[tauri::command]
pub async fn save_custom_icon(
    app: tauri::AppHandle,
    target: String,
    original: Original,
    rendered: String,
    editor: serde_json::Value,
    expected: Option<String>,
) -> Result<Option<IconRecord>, String> {
    icons::path(&target)?;
    let record = tauri::async_runtime::spawn_blocking(move || {
        icons::make_record(original, &rendered, editor)
    })
    .await
    .map_err(|e| e.to_string())??;
    change(app, target, expected, Some(record)).await
}

#[tauri::command]
pub async fn reset_custom_icon(
    app: tauri::AppHandle,
    target: String,
    expected: Option<String>,
) -> Result<Option<IconRecord>, String> {
    change(app, target, expected, None).await
}

#[tauri::command]
pub async fn select_app_icon_preset(
    app: tauri::AppHandle,
    preset: String,
    expected: Option<String>,
) -> Result<Option<IconRecord>, String> {
    let next =
        tauri::async_runtime::spawn_blocking(move || crate::app_icon_presets::record(&preset))
            .await
            .map_err(|e| e.to_string())??;
    // Exactly the same compare-and-swap, runtime rollback and event as Upload.
    // No target argument: presets can never change the Main agent's artwork.
    change(app, "app".into(), expected, next).await
}

/// `settings.read_custom_icon`: what is saved for `target`, as a summary
/// (no original image, editor state or rendered bytes).
pub(crate) async fn agent_read(target: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let record = icons::read_at(&icons::path(&target)?)?;
        Ok(icons::summary(&target, record.as_ref()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `settings.prepare_custom_icon`: whether the image at `path` can be an
/// icon, with its format and size.
pub(crate) async fn agent_prepare(path: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || icons::inspect_file(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

/// The saved revision of `target`'s icon (what `change` compares against).
async fn saved_revision(target: &str) -> Result<Option<String>, String> {
    let target = target.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(icons::read_at(&icons::path(&target)?)?.map(|r| r.revision))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `settings.save_custom_icon`: render the image at `path` here and save it
/// as `target`'s icon. `expected` is the revision being replaced; left out,
/// it is whatever is saved now.
pub(crate) async fn agent_save(
    app: &tauri::AppHandle,
    target: String,
    path: String,
    expected: Option<String>,
) -> Result<serde_json::Value, String> {
    icons::path(&target)?;
    let file = std::path::PathBuf::from(path);
    let record = tauri::async_runtime::spawn_blocking(move || icons::record_from_file(&file))
        .await
        .map_err(|e| e.to_string())??;
    let expected = match expected {
        Some(revision) => Some(revision),
        None => saved_revision(&target).await?,
    };
    let saved = change(app.clone(), target.clone(), expected, Some(record)).await?;
    Ok(icons::summary(&target, saved.as_ref()))
}

/// `settings.reset_custom_icon`: back to the stock artwork. A damaged store
/// is recovered the way the person's Reset does (its bytes are kept beside it).
pub(crate) async fn agent_reset(
    app: &tauri::AppHandle,
    target: String,
    expected: Option<String>,
) -> Result<serde_json::Value, String> {
    icons::path(&target)?;
    let expected = match expected {
        Some(revision) => Some(revision),
        None => match saved_revision(&target).await {
            Ok(revision) => revision,
            Err(e) if e.starts_with("ICON_DAMAGED:") => None,
            Err(e) => return Err(e),
        },
    };
    let saved = change(app.clone(), target.clone(), expected, None).await?;
    Ok(icons::summary(&target, saved.as_ref()))
}

/// `settings.select_app_icon_preset`: bundled artwork for the app icon.
pub(crate) async fn agent_preset(
    app: &tauri::AppHandle,
    preset: String,
    expected: Option<String>,
) -> Result<serde_json::Value, String> {
    let next =
        tauri::async_runtime::spawn_blocking(move || crate::app_icon_presets::record(&preset))
            .await
            .map_err(|e| e.to_string())??;
    let expected = match expected {
        Some(revision) => Some(revision),
        None => saved_revision("app").await?,
    };
    let saved = change(app.clone(), "app".into(), expected, next).await?;
    Ok(icons::summary("app", saved.as_ref()))
}

/// The card a COO icon change shows when it would replace a picture the
/// person uploaded (its saved original is discarded); none when nothing is
/// saved or the saved icon is bundled artwork. `target` defaults to `app`.
pub(crate) fn replace_card(
    _cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let target =
        a.0.get("target")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("app");
    let saved = icons::path(target).and_then(|p| icons::read_at(&p));
    Ok(match saved {
        Ok(Some(r)) if r.preset.is_none() => Some(
            crate::app_actions::Card::new("Hollow app tools: replace an uploaded icon")
                .line(format!(
                    "Replaces the {} icon the person uploaded. Its saved original picture is discarded.",
                    if target == "mainAgent" { "COO" } else { "app" }
                )),
        ),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(dir: &std::path::Path, name: &str, w: u32, h: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
            .save(&path)
            .unwrap();
        path
    }

    #[test]
    fn a_picture_file_becomes_a_record_and_its_summary_carries_no_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let record = icons::record_from_file(&picture(dir.path(), "icon.png", 40, 20)).unwrap();
        let looked = icons::inspect_file(&picture(dir.path(), "again.png", 40, 20)).unwrap();
        assert_eq!(looked["width"], 40);
        assert_eq!(looked["format"], "png");
        assert_eq!(record.original.format, "png");
        assert_eq!(record.pngs.len(), icons::SIZES.len());
        assert!(!record.original.bytes.is_empty());
        let shown = icons::summary("app", Some(&record)).to_string();
        assert!(shown.contains(&record.revision));
        assert!(!shown.contains(&record.original.bytes), "{shown}");
        for png in record.pngs.values() {
            assert!(!shown.contains(png), "{shown}");
        }
        assert!(!shown.contains("editor"), "{shown}");
        assert_eq!(
            icons::summary("mainAgent", None),
            serde_json::json!({ "target": "mainAgent", "custom": false })
        );
    }

    #[test]
    fn a_picture_that_cannot_be_an_icon_is_refused_with_nothing_saved() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("a.svg");
        std::fs::write(&svg, "<svg/>").unwrap();
        assert!(icons::record_from_file(&svg).unwrap_err().contains("SVG"));
        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(icons::record_from_file(&txt).is_err());
        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, "not a png").unwrap();
        assert!(icons::record_from_file(&fake).is_err());
        assert!(icons::record_from_file(std::path::Path::new("relative.png")).is_err());
        assert!(icons::record_from_file(&dir.path().join("missing.png")).is_err());
        assert!(icons::inspect_file(&fake).is_err());
    }

    #[test]
    fn an_unknown_target_is_refused_before_anything_is_read() {
        let read = tauri::async_runtime::block_on(agent_read("dock".into()));
        assert_eq!(read.unwrap_err(), "Unknown icon target");
    }
}
