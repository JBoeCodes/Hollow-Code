//! Agenda (next-level P10, 2026-10-01): due tasks, read-only calendar events
//! and local focus blocks, three separate things that never merge.
//!
//! - Calendar events come from the selected Mac or Microsoft source, read
//!   through the same read-only `calendarView` route as the built-in connector
//!   (`m365_mcp::agenda_events`). Nothing here creates or changes a remote
//!   event. Not signed in is a state, not an error.
//! - Focus blocks are local planning: `{vault}/.hollow/focus-blocks.json`
//!   (`schemaVersion` 1). They live in the vault, so they travel with it and
//!   work offline, and they are never written to a calendar. Writes are
//!   compare-and-swap on `updatedAt`.
//! - Due tasks stay in the task files; the frontend lays them out.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::state_file::OnDamage;

static LOCK: Mutex<()> = Mutex::new(());

/// How long a complete calendar read is reused (overnight S7: Today,
/// Prepare, the weekly review and the Main agent's meeting check each asked
/// Microsoft 365 for the same days within seconds).
const EVENTS_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// Windows kept at once; the oldest goes first.
const EVENTS_KEPT: usize = 16;

/// `(from, to)` → when it was read, and the view.
type KeptEvents = Vec<((String, String), std::time::Instant, Value)>;
static EVENTS: Mutex<KeptEvents> = Mutex::new(Vec::new());
/// Bumped by [`forget_events`] (see `recordings::RECENT_GENERATION`).
static EVENTS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The kept view of `[from, to)`, while younger than [`EVENTS_TTL`] at `now`.
fn kept_events(kept: &KeptEvents, from: &str, to: &str, now: std::time::Instant) -> Option<Value> {
    kept.iter()
        .find(|((f, t), at, _)| {
            f == from && t == to && now.saturating_duration_since(*at) < EVENTS_TTL
        })
        .map(|(_, _, v)| v.clone())
}

/// Keep `view` of `[from, to)` if it is a complete, error-free read.
fn keep_events(kept: &mut KeptEvents, from: &str, to: &str, at: std::time::Instant, view: &Value) {
    if view["connected"] != true || view["complete"] != true || !view["error"].is_null() {
        return;
    }
    kept.retain(|((f, t), _, _)| !(f == from && t == to));
    kept.push(((from.to_string(), to.to_string()), at, view.clone()));
    if kept.len() > EVENTS_KEPT {
        kept.remove(0);
    }
}

/// Forget every kept Microsoft read after account or Agenda source changes.
pub(crate) fn forget_events() {
    EVENTS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// How [`events_with`] uses the kept reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reuse {
    /// Reuse a kept read; keep a new complete one.
    Kept,
    /// Ask Microsoft again (the Agenda's Refresh); keep what comes back.
    Fresh,
    /// Ask Microsoft and keep nothing: a window no one asks for twice (the
    /// meeting check's from-now window).
    Once,
}
const SCHEMA_VERSION: u32 = 1;
const MAX_BLOCKS: usize = 5000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FocusBlock {
    #[serde(default)]
    pub id: String,
    pub title: String,
    /// RFC 3339 with offset: the instant, independent of later zone changes.
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub task_url: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    schema_version: u32,
    #[serde(default)]
    blocks: Vec<FocusBlock>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

fn store_path(vault: &Path) -> PathBuf {
    vault.join(".hollow").join("focus-blocks.json")
}

/// The focus blocks. Only a missing file is empty: an unreadable store is an
/// error and stays put, and a damaged one is moved aside
/// (`focus-blocks.json.corrupt-<time>`) before the error (B·F08).
fn read_store(vault: &Path) -> Result<Store, String> {
    read_store_or(vault, OnDamage::SetAside)
}

fn read_store_or(vault: &Path, on_damage: OnDamage) -> Result<Store, String> {
    let store = crate::state_file::read_json_or::<Store>(
        &store_path(vault),
        "Focus block store",
        on_damage,
    )?;
    Ok(store.unwrap_or_else(|| Store {
        schema_version: SCHEMA_VERSION,
        ..Default::default()
    }))
}

fn write_store(vault: &Path, store: &Store) -> Result<(), String> {
    let mut s = store.clone();
    s.schema_version = s.schema_version.max(SCHEMA_VERSION);
    crate::vault::atomic_write(
        &store_path(vault),
        serde_json::to_string_pretty(&s)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
}

fn instant(s: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|_| format!("{s} is not an RFC 3339 time with an offset"))
}

/// Blocks overlapping `[from, to)`, by start.
pub fn list(vault: &Path, from: &str, to: &str) -> Result<Vec<FocusBlock>, String> {
    list_with(vault, from, to, OnDamage::SetAside)
}

/// [`list`] for an agent action: a damaged store is an error but stays where
/// it is, so the user's Agenda shows the error instead of an empty day
/// (B·F08).
pub fn list_unattended(vault: &Path, from: &str, to: &str) -> Result<Vec<FocusBlock>, String> {
    list_with(vault, from, to, OnDamage::Leave)
}

fn list_with(
    vault: &Path,
    from: &str,
    to: &str,
    on_damage: OnDamage,
) -> Result<Vec<FocusBlock>, String> {
    let (a, b) = (instant(from)?, instant(to)?);
    let _g = LOCK.lock().map_err(|_| "agenda lock poisoned")?;
    let mut out: Vec<_> = read_store_or(vault, on_damage)?
        .blocks
        .into_iter()
        .filter(|x| match (instant(&x.start), instant(&x.end)) {
            (Ok(s), Ok(e)) => s < b && e > a,
            _ => false,
        })
        .collect();
    out.sort_by(|x, y| x.start.cmp(&y.start));
    Ok(out)
}

/// Create (empty `id`) or update a block. An update must carry the
/// `updatedAt` it was read with; a block changed since is refused
/// (`BLOCK_CHANGED:`), never overwritten.
pub fn save(vault: &Path, mut block: FocusBlock) -> Result<FocusBlock, String> {
    let title = block.title.trim().to_string();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("a focus block needs a title (1-200 characters)".into());
    }
    let (s, e) = (instant(&block.start)?, instant(&block.end)?);
    if e <= s || e - s > chrono::Duration::hours(24) {
        return Err("a focus block ends after it starts and lasts at most 24 hours".into());
    }
    block.title = title;
    let _g = LOCK.lock().map_err(|_| "agenda lock poisoned")?;
    let mut store = read_store(vault)?;
    let now = chrono::Utc::now().to_rfc3339();
    if block.id.is_empty() {
        if store.blocks.len() >= MAX_BLOCKS {
            return Err(format!(
                "at most {MAX_BLOCKS} focus blocks; delete old ones first"
            ));
        }
        block.id = format!(
            "fb-{:016x}{:08x}",
            rand::random::<u64>(),
            rand::random::<u32>()
        );
        block.created_at = now.clone();
        block.updated_at = now;
        store.blocks.push(block.clone());
    } else {
        let Some(slot) = store.blocks.iter_mut().find(|x| x.id == block.id) else {
            return Err(format!("no focus block {}", block.id));
        };
        if slot.updated_at != block.updated_at {
            return Err(format!(
                "BLOCK_CHANGED: “{}” changed since it was loaded; reload the agenda",
                slot.title
            ));
        }
        block.created_at = slot.created_at.clone();
        block.updated_at = now;
        *slot = block.clone();
    }
    write_store(vault, &store)?;
    Ok(block)
}

pub fn delete(vault: &Path, id: &str) -> Result<(), String> {
    let _g = LOCK.lock().map_err(|_| "agenda lock poisoned")?;
    let mut store = read_store(vault)?;
    let before = store.blocks.len();
    store.blocks.retain(|b| b.id != id);
    if store.blocks.len() == before {
        return Err(format!("no focus block {id}"));
    }
    write_store(vault, &store)
}

/// Calendar events for `[from, to)` with their freshness. `connected: false`
/// when Hollow has no Microsoft sign-in; an error from Graph is reported, not
/// thrown, so local planning keeps working offline. `complete: false` with the
/// reason in `partial` when the read stopped before Graph's last page (a cap,
/// a failing later page, the deadline), so the Agenda never calls a partial
/// calendar fresh.
/// `reuse` says how kept reads are used.
pub(crate) async fn events_with(from: &str, to: &str, reuse: Reuse) -> Value {
    let generation = crate::calendar_source::generation();
    let account_generation = EVENTS_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    let settings = match crate::calendar_source::settings() {
        Ok(s) => s,
        Err(e) => return crate::calendar_source::unavailable(&e),
    };
    let mut view = match settings.source {
        crate::calendar_source::Source::Microsoft => microsoft_events_with(from, to, reuse).await,
        _ => {
            // Native reads are intentionally not cached: permission and account
            // changes outside Hollow must be visible on the next refresh.
            let (a, b, chosen) = (from.to_string(), to.to_string(), settings.clone());
            crate::vault::spawn_blocking_logged(move || {
                crate::calendar_source::native_view(&chosen, &a, &b)
            })
            .await
            .unwrap_or_else(|_| {
                crate::calendar_source::unavailable("Calendar read was interrupted")
            })
        }
    };
    if generation != crate::calendar_source::generation()
        || crate::calendar_source::settings().as_ref() != Ok(&settings)
    {
        return crate::calendar_source::unavailable(
            "Calendar source changed during this read. Refresh to read the selected source.",
        );
    }
    view["sourceGeneration"] = serde_json::json!(generation);
    if settings.source == crate::calendar_source::Source::Microsoft {
        view["accountGeneration"] = serde_json::json!(account_generation);
    }
    if !source_is_current(&view) {
        return crate::calendar_source::unavailable(
            "Calendar access changed during this read. Refresh to retry.",
        );
    }
    view
}

pub(crate) fn source_is_current(view: &Value) -> bool {
    generations_match(
        view,
        crate::calendar_source::generation(),
        EVENTS_GENERATION.load(std::sync::atomic::Ordering::SeqCst),
    )
}

fn generations_match(view: &Value, source: u64, account: u64) -> bool {
    view["sourceGeneration"]
        .as_u64()
        .is_none_or(|g| g == source)
        && view["accountGeneration"]
            .as_u64()
            .is_none_or(|g| g == account)
}

async fn microsoft_events_with(from: &str, to: &str, reuse: Reuse) -> Value {
    let fetched_at = chrono::Utc::now().to_rfc3339();
    // A sign-in Microsoft refused says why (five asks B3, review #6).
    if let Some(problem) = crate::m365_auth::stored_problem() {
        return serde_json::json!({ "connected": true, "fetchedAt": fetched_at, "events": [], "complete": false, "partial": null, "error": problem, "source": "Microsoft 365" });
    }
    if !crate::m365_auth::signed_in() {
        return serde_json::json!({ "connected": false, "fetchedAt": fetched_at, "events": [], "complete": true, "partial": null, "error": null, "source": "Microsoft 365 (not signed in)" });
    }
    // A complete read of the same window in the last minute is reused (its
    // `fetchedAt` says when it was read).
    let now = std::time::Instant::now();
    if reuse == Reuse::Kept {
        if let Some(view) = kept_events(
            &EVENTS.lock().unwrap_or_else(|e| e.into_inner()),
            from,
            to,
            now,
        ) {
            return view;
        }
    }
    let generation = EVENTS_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    let view = events_view(
        crate::m365_mcp::agenda_events(from, to, 200).await,
        fetched_at,
    );
    // Not across a sign-in or sign-out that came while it ran.
    if reuse != Reuse::Once
        && generation == EVENTS_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
    {
        keep_events(
            &mut EVENTS.lock().unwrap_or_else(|e| e.into_inner()),
            from,
            to,
            now,
            &view,
        );
    }
    view
}

fn events_view(read: Result<Value, String>, fetched_at: String) -> Value {
    match read {
        Ok(v) => {
            serde_json::json!({ "connected": true, "fetchedAt": fetched_at, "timeZone": v["timeZone"], "events": v["events"], "complete": v["complete"] != false, "partial": v["partial"], "error": null, "source": "Microsoft 365 · read only" })
        }
        Err(e) => {
            serde_json::json!({ "connected": true, "fetchedAt": fetched_at, "events": [], "complete": false, "partial": null, "error": e, "source": "Microsoft 365 · read only" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_account_changes_do_not_invalidate_native_reads() {
        let native = serde_json::json!({"sourceGeneration": 3});
        let microsoft = serde_json::json!({"sourceGeneration": 3, "accountGeneration": 4});
        assert!(generations_match(&native, 3, 5));
        assert!(!generations_match(&native, 4, 5));
        assert!(generations_match(&microsoft, 3, 4));
        assert!(!generations_match(&microsoft, 3, 5));
        assert!(!generations_match(&microsoft, 4, 4));
    }

    #[test]
    fn a_complete_calendar_read_is_reused_for_a_minute_and_a_failed_one_never() {
        let t0 = std::time::Instant::now();
        let ok = events_view(
            Ok(serde_json::json!({ "events": [{"subject": "Board"}], "complete": true })),
            "at".into(),
        );
        let partial = events_view(
            Ok(serde_json::json!({ "events": [], "complete": false, "partial": "cap" })),
            "at".into(),
        );
        let failed = events_view(Err("Graph 503".into()), "at".into());
        let mut kept = KeptEvents::new();
        keep_events(&mut kept, "a", "b", t0, &ok);
        keep_events(&mut kept, "a", "c", t0, &partial);
        keep_events(&mut kept, "a", "d", t0, &failed);
        assert_eq!(kept.len(), 1, "only the complete read is kept");
        let later = t0 + std::time::Duration::from_secs(30);
        assert_eq!(
            kept_events(&kept, "a", "b", later).unwrap()["events"][0]["subject"],
            "Board"
        );
        assert!(kept_events(&kept, "a", "c", later).is_none());
        assert!(
            kept_events(&kept, "a", "b", t0 + EVENTS_TTL).is_none(),
            "stale after a minute"
        );
        // Bounded: the oldest window goes.
        for n in 0..EVENTS_KEPT {
            keep_events(&mut kept, &format!("w{n}"), "z", t0, &ok);
        }
        assert_eq!(kept.len(), EVENTS_KEPT);
        assert!(kept_events(&kept, "a", "b", later).is_none());
    }

    fn block(title: &str, start: &str, end: &str) -> FocusBlock {
        FocusBlock {
            title: title.into(),
            start: start.into(),
            end: end.into(),
            ..Default::default()
        }
    }

    #[test]
    fn blocks_are_local_validated_and_compare_and_swap() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        assert!(save(
            v,
            block("", "2026-10-01T09:00:00-04:00", "2026-10-01T10:00:00-04:00")
        )
        .is_err());
        assert!(save(
            v,
            block(
                "Backwards",
                "2026-10-01T10:00:00-04:00",
                "2026-10-01T09:00:00-04:00"
            )
        )
        .is_err());
        let a = save(
            v,
            block(
                "Write section 3",
                "2026-10-01T09:00:00-04:00",
                "2026-10-01T10:30:00-04:00",
            ),
        )
        .unwrap();
        assert!(a.id.starts_with("fb-") && !a.updated_at.is_empty());
        // Overlap query is by instant, across offsets.
        assert_eq!(
            list(v, "2026-10-01T14:00:00Z", "2026-10-01T15:00:00Z")
                .unwrap()
                .len(),
            1
        );
        assert!(list(v, "2026-10-01T15:00:00Z", "2026-10-01T16:00:00Z")
            .unwrap()
            .is_empty());
        // A stale update is refused.
        let mut stale = a.clone();
        stale.updated_at = "old".into();
        stale.title = "Renamed".into();
        assert!(save(v, stale).unwrap_err().starts_with("BLOCK_CHANGED:"));
        let mut fresh = a.clone();
        fresh.title = "Renamed".into();
        assert_eq!(save(v, fresh).unwrap().title, "Renamed");
        delete(v, &a.id).unwrap();
        assert!(delete(v, &a.id).is_err());
    }

    #[test]
    fn a_partial_calendar_read_reaches_the_agenda_as_partial() {
        let partial = events_view(
            Ok(
                serde_json::json!({"events": [{"id": "e0"}], "complete": false, "partial": "page 2 failed: Microsoft Graph 503"}),
            ),
            "t".into(),
        );
        assert_eq!(
            (
                partial["complete"].clone(),
                partial["partial"].clone(),
                partial["error"].clone()
            ),
            (
                serde_json::json!(false),
                serde_json::json!("page 2 failed: Microsoft Graph 503"),
                Value::Null
            )
        );
        assert_eq!(
            partial["events"].as_array().unwrap().len(),
            1,
            "what was read is still shown"
        );
        let whole = events_view(
            Ok(serde_json::json!({"events": [], "complete": true, "partial": null})),
            "t".into(),
        );
        assert_eq!(whole["complete"], true);
    }

    #[test]
    fn unknown_fields_survive_and_corrupt_stores_are_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join(".hollow")).unwrap();
        std::fs::write(
            store_path(v),
            r#"{"schemaVersion":1,"blocks":[],"future":1}"#,
        )
        .unwrap();
        save(
            v,
            block(
                "A",
                "2026-11-01T01:30:00-04:00",
                "2026-11-01T01:30:00-05:00",
            ),
        )
        .unwrap();
        assert!(std::fs::read_to_string(store_path(v))
            .unwrap()
            .contains("\"future\""));
        std::fs::write(store_path(v), "nope").unwrap();
        let err = list(v, "2026-11-01T00:00:00Z", "2026-11-02T00:00:00Z").unwrap_err();
        assert!(err.contains("Focus block store was damaged"), "{err}");
        let kept = crate::state_file::corrupt_copies(&store_path(v));
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"nope");
        // A save over a damaged store fails rather than replacing it. Not
        // UTF-8: before B·F08 this read as empty and was overwritten.
        std::fs::write(store_path(v), b"\xff\xfe").unwrap();
        let b = block("B", "2026-11-01T09:00:00Z", "2026-11-01T10:00:00Z");
        assert!(save(v, b).is_err());
        let kept = crate::state_file::corrupt_copies(&store_path(v));
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert_eq!(
            std::fs::read(&kept[1]).unwrap(),
            b"\xff\xfe",
            "the newest copy"
        );
    }

    #[test]
    fn an_agent_read_leaves_a_damaged_store_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join(".hollow")).unwrap();
        std::fs::write(store_path(v), "nope").unwrap();
        let err = list_unattended(v, "2026-11-01T00:00:00Z", "2026-11-02T00:00:00Z").unwrap_err();
        assert!(err.contains("Focus block store is damaged"), "{err}");
        assert_eq!(std::fs::read(store_path(v)).unwrap(), b"nope");
        assert!(crate::state_file::corrupt_copies(&store_path(v)).is_empty());
    }
}
