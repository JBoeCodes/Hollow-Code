//! A short-lived, read-only marker for a successful shared-tab inspection.
//! Coordinates never grant input access and are never accepted from the page.
use super::{isolated, navigated, SelectorRead};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Webview};

const LIFETIME: Duration = Duration::from_secs(3);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static MARKERS: LazyLock<Mutex<BTreeMap<String, Marker>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Clone)]
struct Marker {
    share: crate::tab_grants::Grant,
    gen: u64,
    seq: u64,
    made: Instant,
    url: String,
    point: Point,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Point {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scroll_x: f64,
    scroll_y: f64,
}

/// Presentation only. Bounds and permission identity stay native-owned.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    duration_ms: u32,
    color: String,
    background: String,
    ink: String,
}

pub(super) fn original_share(
    shares: Vec<crate::tab_grants::Grant>,
    scope: &str,
    run: &str,
    tab_id: &str,
    generation: u64,
) -> Option<crate::tab_grants::Grant> {
    shares.into_iter().find(|g| {
        g.tab_id == tab_id
            && g.session_key == scope
            && g.run.as_deref() == Some(run)
            && g.generation == generation
    })
}

fn point(read: &SelectorRead) -> Option<Point> {
    let number = |v: &serde_json::Value, key| v.get(key)?.as_f64().filter(|n| n.is_finite());
    let width = number(&read.viewport, "w")?;
    let height = number(&read.viewport, "h")?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    read.matches.iter().find_map(|m| {
        if m.get("visible")?.as_bool()? && !m.get("covered")?.as_bool()? {
            let b = m.get("box")?;
            let w = number(b, "w")?;
            let h = number(b, "h")?;
            let x = number(b, "x")? + w / 2.0;
            let y = number(b, "y")? + h / 2.0;
            if w > 0.0 && h > 0.0 && x >= 0.0 && y >= 0.0 && x < width && y < height {
                return Some(Point {
                    x,
                    y,
                    width,
                    height,
                    scroll_x: number(&read.viewport, "scrollX")?,
                    scroll_y: number(&read.viewport, "scrollY")?,
                });
            }
        }
        None
    })
}

pub(super) fn remember(
    app: &AppHandle,
    share: crate::tab_grants::Grant,
    gen: u64,
    read: &SelectorRead,
) {
    let Some(run) = share.run.as_deref() else {
        return;
    };
    if navigated(&share.tab_id) != gen
        || crate::tab_grants::still_shared(&share.session_key, run, &share.tab_id, share.generation)
            .is_err()
        || matches!(
            crate::host_scope::Scope::effective().decide_url(&read.url),
            crate::host_scope::Decision::Denied(_)
        )
    {
        return;
    }
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let tab_id = share.tab_id.clone();
    let point = point(read);
    let visible = point.is_some();
    if let Ok(mut markers) = MARKERS.lock() {
        markers.retain(|_, m| m.made.elapsed() < LIFETIME);
        markers.remove(&tab_id);
        if let Some(point) = point {
            markers.insert(
                tab_id.clone(),
                Marker {
                    share,
                    gen,
                    seq,
                    made: Instant::now(),
                    url: read.url.clone(),
                    point,
                },
            );
        }
    }
    let _ = app.emit_to(
        "main",
        "browser-inspection",
        serde_json::json!({ "tabId": tab_id, "gen": gen, "seq": seq, "visible": visible }),
    );
}

pub(super) fn forget(tab_id: &str) {
    if let Ok(mut markers) = MARKERS.lock() {
        markers.remove(tab_id);
    }
}

/// Evaluated in WebKit's isolated client world, above its native page. The
/// app's DOM is behind that webview and cannot draw a cursor over it.
pub(super) fn render(webview: &Webview, tab_id: &str, style: Option<&Style>) {
    let marker = MARKERS.lock().ok().and_then(|m| m.get(tab_id).cloned());
    let payload = marker
        .as_ref()
        .zip(style)
        .filter(|(m, _)| {
            m.made.elapsed() < LIFETIME
                && navigated(tab_id) == m.gen
                && m.share.run.as_deref().is_some_and(|run| {
                    crate::tab_grants::still_shared(
                        &m.share.session_key,
                        run,
                        tab_id,
                        m.share.generation,
                    )
                    .is_ok()
                })
        })
        .map(|(m, s)| {
            serde_json::json!({
                "seq": m.seq, "url": m.url, "point": m.point,
                "remaining": LIFETIME.saturating_sub(m.made.elapsed()).as_millis() as u64,
                "duration": s.duration_ms.min(400),
                "color": s.color.chars().take(120).collect::<String>(),
                "background": s.background.chars().take(120).collect::<String>(),
                "ink": s.ink.chars().take(120).collect::<String>(),
            })
        });
    let script = include_str!("agent_cursor.js").replace(
        "__PAYLOAD__",
        &payload.unwrap_or(serde_json::Value::Null).to_string(),
    );
    let (tx, _rx) = tokio::sync::oneshot::channel();
    // A decoration failing must never hide or break the underlying page.
    let _ = isolated::eval(webview, script, 32, tx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_replacement_share_cannot_publish_an_old_readers_cursor() {
        let share = crate::tab_grants::Grant {
            tab_id: "tab".into(),
            session_key: "chat".into(),
            run: Some("run".into()),
            generation: 8,
            title: "Chat".into(),
            granted_at: String::new(),
        };
        assert!(original_share(vec![share.clone()], "chat", "run", "tab", 8).is_some());
        assert!(original_share(vec![share.clone()], "chat", "run", "tab", 7).is_none());
        assert!(original_share(vec![share.clone()], "other", "run", "tab", 8).is_none());
        assert!(original_share(vec![share.clone()], "chat", "earlier", "tab", 8).is_none());
        assert!(original_share(vec![share.clone()], "chat", "run", "other", 8).is_none());
        assert!(original_share(Vec::new(), "chat", "run", "tab", 8).is_none());
    }

    fn read(matches: serde_json::Value) -> SelectorRead {
        SelectorRead {
            viewport: json!({"w": 800, "h": 600, "scrollX": 0, "scrollY": 12}),
            matches: matches.as_array().unwrap().clone(),
            ..Default::default()
        }
    }

    #[test]
    fn only_visible_uncovered_in_viewport_matches_get_a_marker() {
        let hidden = json!({"visible":false,"covered":false,"box":{"x":0,"y":0,"w":10,"h":10}});
        let covered = json!({"visible":true,"covered":true,"box":{"x":0,"y":0,"w":10,"h":10}});
        let outside = json!({"visible":true,"covered":false,"box":{"x":900,"y":0,"w":10,"h":10}});
        let valid = json!({"visible":true,"covered":false,"box":{"x":10,"y":20,"w":40,"h":60}});
        assert!(point(&read(json!([hidden, covered, outside]))).is_none());
        let p = point(&read(json!([valid]))).unwrap();
        assert_eq!((p.x, p.y, p.scroll_y), (30.0, 50.0, 12.0));
    }

    #[test]
    fn missing_or_degenerate_geometry_never_invents_a_position() {
        assert!(point(&read(json!([{"visible":true,"covered":false}]))).is_none());
        assert!(point(&read(
            json!([{"visible":true,"covered":false,"box":{"x":0,"y":0,"w":0,"h":10}}])
        ))
        .is_none());
        assert!(point(&SelectorRead::default()).is_none());
    }
}
