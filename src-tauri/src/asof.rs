//! As-of history (master plan 2026-10, F5): what a workspace's brief, plan,
//! tasks and decisions said on a given day. See `docs/AS_OF.md`.
//!
//! **Snapshots.** Nightly (the `nightly` window job), Hollow hashes each
//! workspace's `WORKSPACE.md` and `PLAN.md`, every `tasks/*.md` and every
//! `decisions/*.md`, stores new contents in History's content-addressed
//! object store (`.hollow-history/objects`) and writes one manifest,
//! `.hollow-history/snapshots/<YYYY-MM-DD>.json` (`{day, takenAt, files:
//! {path: hash}}`). A day whose files all match the latest snapshot stores
//! nothing. Files over 2 MiB are left out (as History does).
//!
//! **Retention.** History's own: 90 days. The newest snapshot is always
//! kept. History's object GC counts snapshot references
//! (`automated_history::enforce_retention` → [`referenced`]), so a snapshot
//! never loses its objects; History's 512 MB ceiling applies to runs only.
//!
//! **Reading.** [`read`] returns a path's content as of the end of a local
//! day: the newer of the newest snapshot that day or before and History's
//! newest agent-run version finished by then. Neither: "not captured"
//! (uncaptured is not known, never shown as empty). A path a snapshot
//! didn't hold that day, when others were, reads "absent".
//!
//! **Metrics.** `.hollow-history/metrics/<YYYY-MM-DD>.json`: per workspace,
//! turns and known cost (the usage ledger), files changed since the
//! previous snapshot and open plan rows, for #18 and #19.

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const SNAPSHOTS: &str = "snapshots";
const METRICS: &str = "metrics";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// History's age limit, shared.
pub const MAX_AGE_DAYS: i64 = 90;

/// One snapshot manifest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Local day, `YYYY-MM-DD`.
    pub day: String,
    pub taken_at: String,
    /// Vault-relative path → SHA-256 of its content.
    pub files: BTreeMap<String, String>,
}

fn root(vault: &Path) -> PathBuf {
    crate::automated_history::history_dir(vault)
}

fn snapshots_dir(vault: &Path) -> PathBuf {
    root(vault).join(SNAPSHOTS)
}

/// Every readable snapshot, oldest first.
pub fn snapshots(vault: &Path) -> Vec<Snapshot> {
    snapshots_checked(vault).unwrap_or_default()
}

/// Every snapshot, oldest first; an error when the folder or any manifest
/// can't be read (so GC never runs blind).
pub fn snapshots_checked(vault: &Path) -> Result<Vec<Snapshot>, String> {
    let entries = match std::fs::read_dir(snapshots_dir(vault)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read snapshots: {e}")),
    };
    let mut out = Vec::new();
    let mut unreadable = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") {
            continue;
        }
        match std::fs::read(e.path())
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Snapshot>(&b).map_err(|e| e.to_string()))
        {
            Ok(s) => out.push(s),
            Err(_) => unreadable.push(name),
        }
    }
    if !unreadable.is_empty() {
        return Err(format!("unreadable snapshot(s): {}", unreadable.join(", ")));
    }
    out.sort_by(|a, b| a.day.cmp(&b.day).then(a.taken_at.cmp(&b.taken_at)));
    Ok(out)
}

/// The object hashes snapshots hold (History's GC keeps them). An error
/// when a manifest can't be read: History then skips GC (review fix), as it
/// does for an unreadable run record.
pub fn referenced(vault: &Path) -> Result<HashSet<String>, String> {
    Ok(snapshots_checked(vault)?
        .into_iter()
        .flat_map(|s| s.files.into_values())
        .collect())
}

/// The moment a snapshot stands for: one taken before 06:00 (the nightly
/// window) is the state at the end of the previous day.
fn covers(taken: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::Timelike;
    let local = taken.with_timezone(&Local);
    if local.hour() < 6 {
        end_of(local.date_naive() - Duration::days(1)) - Duration::seconds(1)
    } else {
        taken
    }
}

/// Whether `path` is a kind of file snapshots track (else "absent" from a
/// snapshot means nothing: it was never captured).
fn trackable(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["workspaces", _, file] => matches!(*file, "WORKSPACE.md" | "PLAN.md"),
        ["tasks", f] | ["decisions", f] => f.ends_with(".md"),
        _ => false,
    }
}

/// The files a snapshot covers, vault-relative.
pub fn tracked(vault: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let md_in = |dir: &str| -> Vec<String> {
        std::fs::read_dir(vault.join(dir))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
                    .map(|n| format!("{dir}/{n}"))
                    .collect()
            })
            .unwrap_or_default()
    };
    for ws in crate::workspaces::list(vault).unwrap_or_default() {
        for file in ["WORKSPACE.md", "PLAN.md"] {
            let rel = format!("{}/{file}", ws.path);
            if vault.join(&rel).is_file() {
                out.push(rel);
            }
        }
    }
    out.extend(md_in("tasks"));
    out.extend(md_in(crate::records::DECISIONS_DIR));
    out.sort();
    out.dedup();
    out
}

/// Take today's snapshot: `Ok(Some(day))` when one was written, `Ok(None)`
/// when nothing changed since the latest.
pub fn take(vault: &Path, now: DateTime<Utc>) -> Result<Option<String>, String> {
    // Named for the day it stands for (a 03:00 snapshot is the day before's).
    let day = covers(now)
        .with_timezone(&Local)
        .format("%Y-%m-%d")
        .to_string();
    let mut files = BTreeMap::new();
    let mut contents = Vec::new();
    for rel in tracked(vault) {
        let full = vault.join(&rel);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&full) else {
            continue;
        };
        let hash = crate::automated_history::content_hash(&bytes);
        files.insert(rel, hash.clone());
        contents.push((hash, bytes));
    }
    if snapshots(vault).last().is_some_and(|s| s.files == files) {
        return Ok(None);
    }
    let snapshot = Snapshot {
        day: day.clone(),
        taken_at: now.to_rfc3339(),
        files,
    };
    crate::automated_history::with_history_lock(|| {
        // The objects are named once the snapshot file is written (Q-R9).
        let _objects = crate::automated_history::objects_in_flight(vault)?;
        for (hash, bytes) in &contents {
            crate::automated_history::store_object(vault, hash, bytes)?;
        }
        let dir = snapshots_dir(vault);
        std::fs::create_dir_all(&dir).map_err(|e| format!("snapshots: {e}"))?;
        let bytes = serde_json::to_vec_pretty(&snapshot).map_err(|e| e.to_string())?;
        crate::vault::atomic_write(&dir.join(format!("{day}.json")), &bytes)
    })?;
    Ok(Some(day))
}

/// Drop snapshots older than [`MAX_AGE_DAYS`], always keeping the newest;
/// how many were removed. Objects go at History's next GC.
pub fn prune(vault: &Path, now: DateTime<Utc>) -> usize {
    let cutoff = (now.with_timezone(&Local) - Duration::days(MAX_AGE_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    let all = snapshots(vault);
    let Some(newest) = all.last().map(|s| s.day.clone()) else {
        return 0;
    };
    let mut removed = 0;
    for s in all.iter().filter(|s| s.day < cutoff && s.day != newest) {
        if std::fs::remove_file(snapshots_dir(vault).join(format!("{}.json", s.day))).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// The end of local day `day` (the next local midnight).
fn end_of(day: NaiveDate) -> DateTime<Utc> {
    let next = day + Duration::days(1);
    Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&next.and_hms_opt(0, 0, 0).unwrap_or_default()))
}

/// One as-of answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsOf {
    pub path: String,
    pub as_of: String,
    /// `snapshot`, `history`, `absent` (a snapshot that day didn't hold it)
    /// or `not-captured`.
    pub source: String,
    /// When that version was captured (RFC 3339), if any.
    pub captured_at: Option<String>,
    /// The History run, for a `history` answer.
    pub run_id: Option<String>,
    pub text: Option<String>,
    pub note: String,
}

/// `path` as of the end of local day `day` (`YYYY-MM-DD`).
pub fn read(vault: &Path, path: &str, day: &str) -> Result<AsOf, String> {
    let path = path.trim().trim_start_matches("vault:").to_string();
    crate::vault::safe_join(vault, &path)?;
    let date = NaiveDate::parse_from_str(day.trim().get(..10).unwrap_or(day), "%Y-%m-%d")
        .map_err(|_| format!("'{day}' is not a day (YYYY-MM-DD)."))?;
    let cutoff = end_of(date);
    let at = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    };
    let snap = snapshots(vault)
        .into_iter()
        .rfind(|s| at(&s.taken_at).is_some_and(|t| covers(t) < cutoff));
    let run = crate::automated_history::list_note_history(vault, &path, 200)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            r.finished_at
                .as_deref()
                .and_then(at)
                .is_some_and(|t| t < cutoff)
        })
        .max_by(|a, b| a.finished_at.cmp(&b.finished_at));
    let snap_at = snap.as_ref().and_then(|s| at(&s.taken_at)).map(covers);
    let run_at = run
        .as_ref()
        .and_then(|r| r.finished_at.as_deref())
        .and_then(at);
    let base = |source: &str,
                captured: Option<String>,
                run_id: Option<String>,
                text: Option<String>,
                note: String| AsOf {
        path: path.clone(),
        as_of: date.to_string(),
        source: source.into(),
        captured_at: captured,
        run_id,
        text,
        note,
    };
    let history_newer = match (run_at, snap_at) {
        (Some(r), Some(s)) => r > s,
        (Some(_), None) => true,
        _ => false,
    };
    if history_newer {
        let run = run.expect("checked");
        return match crate::automated_history::read_version(vault, &run.id, &path, "after") {
            Ok(text) => Ok(base(
                if text.is_some() { "history" } else { "absent" },
                run.finished_at.clone(),
                Some(run.id.clone()),
                text,
                format!(
                    "As an agent run left it ({}).",
                    run.finished_at.unwrap_or_default()
                ),
            )),
            Err(e) => Ok(base(
                "not-captured",
                None,
                Some(run.id),
                None,
                format!("History can't show that version: {e}"),
            )),
        };
    }
    if let Some(s) = snap {
        return Ok(match s.files.get(&path) {
            Some(hash) => {
                let bytes = crate::automated_history::load_object(vault, hash)?;
                let text = String::from_utf8(bytes).map_err(|_| format!("{path} was not text"))?;
                base(
                    "snapshot",
                    Some(s.taken_at.clone()),
                    None,
                    Some(text),
                    format!("From the snapshot of {}.", s.day),
                )
            }
            None if !trackable(&path) => base(
                "not-captured",
                None,
                None,
                None,
                "Snapshots don't track this file, and no agent run changed it by then: not known."
                    .into(),
            ),
            None => base(
                "absent",
                Some(s.taken_at.clone()),
                None,
                None,
                format!(
                    "The snapshot of {} didn't hold it: it didn't exist then, or wasn't tracked.",
                    s.day
                ),
            ),
        });
    }
    Ok(base(
        "not-captured",
        None,
        None,
        None,
        "Hollow holds no version from then: snapshots start when this build was installed, and History only keeps agent runs. Not known, not empty.".into(),
    ))
}

// ── Metrics ────────────────────────────────────────────────────────────────

/// One workspace's day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayMetrics {
    pub turns: u64,
    pub cost_usd: f64,
    pub unknown_cost_runs: u64,
    pub files_changed: u64,
    pub open_plan_rows: u64,
}

/// Open `- [ ]` rows of a PLAN.md.
fn open_rows(plan: &str) -> u64 {
    plan.lines()
        .filter(|l| l.trim_start().starts_with("- [ ]"))
        .count() as u64
}

/// The day's metrics per workspace, given the snapshot just taken (or the
/// latest) and the one before it.
pub fn metrics_for(
    vault: &Path,
    day: NaiveDate,
    now: DateTime<Utc>,
    fresh_snapshot: bool,
) -> BTreeMap<String, DayMetrics> {
    let start = end_of(day - Duration::days(1));
    let end = end_of(day).min(now.max(start));
    let rows = crate::run_ledger::rows_between(
        vault,
        &crate::session_store::db_time(start),
        &crate::session_store::db_time(end),
    )
    .unwrap_or_default();
    let all = snapshots(vault);
    let (prev, last) = match all.as_slice() {
        [.., a, b] => (Some(a.clone()), Some(b.clone())),
        [b] => (None, Some(b.clone())),
        [] => (None, None),
    };
    let mut out = BTreeMap::new();
    for ws in crate::workspaces::list(vault).unwrap_or_default() {
        let mut m = DayMetrics::default();
        for r in rows.iter().filter(|r| r.workspace == ws.id) {
            if r.surface == "chat" {
                m.turns += 1;
            }
            match r.cost_usd {
                Some(c) => m.cost_usd += c,
                None if r.spawned => m.unknown_cost_runs += 1,
                None => {}
            }
        }
        // Files changed: only when tonight's pass took a new snapshot (an
        // old diff would be counted again otherwise).
        if let (true, Some(prev), Some(last)) = (fresh_snapshot, &prev, &last) {
            let prefix = format!("{}/", ws.path);
            let changed =
                |p: &String, h: &String| p.starts_with(&prefix) && prev.files.get(p) != Some(h);
            m.files_changed = last.files.iter().filter(|(p, h)| changed(p, h)).count() as u64
                + prev
                    .files
                    .keys()
                    .filter(|p| p.starts_with(&prefix) && !last.files.contains_key(*p))
                    .count() as u64;
        }
        m.open_plan_rows = open_rows(
            &std::fs::read_to_string(vault.join(&ws.path).join("PLAN.md")).unwrap_or_default(),
        );
        m.cost_usd = (m.cost_usd * 100.0).round() / 100.0;
        out.insert(ws.id, m);
    }
    out
}

fn write_metrics(vault: &Path, day: &str, m: &BTreeMap<String, DayMetrics>) -> Result<(), String> {
    let dir = root(vault).join(METRICS);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&json!({ "day": day, "workspaces": m }))
        .map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&dir.join(format!("{day}.json")), &bytes)
}

/// One workspace's stored daily metrics in `[from, to]`, oldest first.
pub fn workspace_metrics(
    vault: &Path,
    workspace: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(root(vault).join(METRICS)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Value)> = entries
        .flatten()
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter_map(|v| {
            let day = v.get("day")?.as_str()?.to_string();
            let m = v.get("workspaces")?.get(workspace)?.clone();
            Some((day, m))
        })
        .filter(|(day, _)| {
            from.is_none_or(|f| day.as_str() >= f) && to.is_none_or(|t| day.as_str() <= t)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter()
        .map(|(day, mut m)| {
            if let Some(obj) = m.as_object_mut() {
                obj.insert("day".into(), json!(day));
            }
            m
        })
        .collect()
}

/// The nightly pass: snapshot, metrics, prune.
pub fn nightly(vault: &Path, now: DateTime<Utc>) -> Result<Value, String> {
    let taken = take(vault, now)?;
    // The pass runs in the night: its metrics are the day that just ended.
    let day = covers(now).with_timezone(&Local).date_naive();
    let metrics = metrics_for(vault, day, now, taken.is_some());
    write_metrics(vault, &day.to_string(), &metrics)?;
    let pruned = prune(vault, now);
    Ok(json!({ "snapshot": taken, "workspaces": metrics.len(), "pruned": pruned }))
}

/// [`nightly`] as a window job (the `nightly` window, run once on wake).
pub(crate) fn nightly_job(
    _app: tauri::AppHandle,
    config: crate::config::Config,
    _key: String,
    _late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        let vault = config.vault_path.clone();
        tauri::async_runtime::spawn_blocking(move || nightly(&vault, Utc::now()).map(|_| ()))
            .await
            .map_err(|e| e.to_string())?
    })
}

// ── "What did I know on 9/15?" (#17) ───────────────────────────────────────

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
/// The as-of block's text budget (characters).
const BLOCK_MAX: usize = 12_000;
const FILE_MAX: usize = 5_000;

fn month(word: &str) -> Option<u32> {
    let w = word.trim_end_matches('.');
    if w.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| w.starts_with(m) && (w.len() == 3 || "january february march april may june july august september october november december sept".split(' ').any(|full| full == w)))
        .map(|i| i as u32 + 1)
}

/// The past day a request asks about ("as of 2026-09-15", "on 9/15", "on
/// September 15"), when it asks what was known or true then. A date with no
/// year is this year's; one still ahead this year isn't read as past.
pub fn asked_day(request: &str, today: NaiveDate) -> Option<NaiveDate> {
    let lower = request.to_lowercase();
    let cued = [
        "as of",
        "what did i know",
        "what did we know",
        "what did hollow know",
        "knew",
        "back on",
        "at the time we",
    ]
    .iter()
    .any(|c| lower.contains(c));
    if !cued {
        return None;
    }
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '?' | '!' | '(' | ')' | ';'))
        .map(|w| w.trim_matches(|c: char| c == '.' || c == '"' || c == '\''))
        .filter(|w| !w.is_empty())
        .collect();
    let in_year = |y: Option<i32>, m: u32, d: u32| -> Option<NaiveDate> {
        match y {
            Some(y) => NaiveDate::from_ymd_opt(y, m, d),
            None => {
                use chrono::Datelike;
                // A yearless date still ahead this year is ambiguous (last
                // year's, or a due date): not read as a past day.
                NaiveDate::from_ymd_opt(today.year(), m, d).filter(|this| *this <= today)
            }
        }
    };
    let year = |w: &str| w.parse::<i32>().ok().filter(|y| (2000..=2100).contains(y));
    let day_num = |w: &str| {
        w.trim_end_matches(|c: char| c.is_alphabetic())
            .parse::<u32>()
            .ok()
            .filter(|d| (1..=31).contains(d))
    };
    for (i, w) in words.iter().enumerate() {
        let found = if let Ok(d) = NaiveDate::parse_from_str(w, "%Y-%m-%d") {
            Some(d)
        } else if w.contains('/') {
            // A token like "and/or" is simply not a date (never an early
            // return for the whole request).
            (|| {
                let parts: Vec<&str> = w.split('/').collect();
                match parts.as_slice() {
                    [m, d] => in_year(None, m.parse().ok()?, d.parse().ok()?),
                    [m, d, y] => {
                        let y: i32 = y.parse().ok()?;
                        in_year(
                            Some(if y < 100 { 2000 + y } else { y }),
                            m.parse().ok()?,
                            d.parse().ok()?,
                        )
                    }
                    _ => None,
                }
            })()
        } else if let Some(m) = month(w) {
            let after = words.get(i + 1).and_then(|n| day_num(n));
            let before = i.checked_sub(1).and_then(|j| day_num(words[j]));
            let y = words.iter().skip(i + 1).take(2).find_map(|n| year(n));
            after.or(before).and_then(|d| in_year(y, m, d))
        } else {
            None
        };
        if let Some(d) = found.filter(|d| *d < today) {
            return Some(d);
        }
    }
    None
}

fn capped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n… (cut; read the rest with history.read_as_of)");
    out
}

/// Whether `text` contains `name` as whole words ("Ops" is not in "stops").
fn names_word(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The workspace a question is about: the chat's own, else the one
/// workspace whose title the request names.
fn workspace_for(vault: &Path, cwd: &Path, request: &str) -> Option<crate::workspaces::Workspace> {
    if let Ok(Some(ws)) = crate::workspaces::workspace_for_cwd(vault, cwd) {
        if !crate::workspaces::is_main_agent(&ws.id) {
            return Some(ws);
        }
    }
    let lower = request.to_lowercase();
    let named: Vec<_> = crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.title.chars().count() >= 3 && names_word(&lower, &w.title.to_lowercase()))
        .collect();
    (named.len() == 1)
        .then(|| named.into_iter().next())
        .flatten()
}

/// The `<as-of-context>` block for a request asking what was known on a past
/// day: the workspace's brief and plan as of then (with where each version
/// came from) and the decisions settled by then. `None` when the request
/// asks no such thing or names no workspace.
pub fn question_block(vault: &Path, cwd: &Path, request: &str, today: NaiveDate) -> Option<String> {
    let day = asked_day(request, today)?;
    let ws = workspace_for(vault, cwd, request)?;
    let mut block = format!(
        "<as-of-context date=\"{day}\" workspace=\"{}\">\nWhat Hollow held for this workspace at the end of {day} (local), for a question about then. In your answer, name each source and its capture date; for anything not captured, say it isn't known rather than guessing or treating it as empty.\n",
        ws.id
    );
    let mut body = String::new();
    for file in ["WORKSPACE.md", "PLAN.md"] {
        let rel = format!("{}/{file}", ws.path);
        match read(vault, &rel, &day.to_string()) {
            Ok(a) if a.text.is_some() => body.push_str(&format!(
                "\n## {file}: {} ({})\n{}\n",
                a.note,
                a.captured_at.as_deref().unwrap_or(""),
                capped(a.text.as_deref().unwrap_or(""), FILE_MAX)
            )),
            Ok(a) => body.push_str(&format!("\n## {file}: {} [{}]\n", a.note, a.source)),
            Err(e) => body.push_str(&format!("\n## {file}: unreadable ({e})\n")),
        }
    }
    let decisions = crate::records::find_decisions(
        vault,
        Some(&ws.id),
        Some("settled"),
        None,
        Some(&day.to_string()),
    );
    if !decisions.is_empty() {
        body.push_str("\n## Decisions settled by then\n");
        for d in decisions.iter().take(10) {
            body.push_str(&format!("- {} — {} ({})\n", d.decided_at, d.text, d.path));
        }
    }
    // The sources are the person's own old notes: cited data, not instructions.
    // Hollow's own cut pointer stays outside the wrapped region.
    let cut = body.chars().count() > BLOCK_MAX;
    let wrapped = crate::untrusted::wrap(
        crate::untrusted::Origin::Vault,
        &format!("as of {day}, workspace {}", ws.id),
        &body.chars().take(BLOCK_MAX).collect::<String>(),
    );
    block.push_str(&wrapped);
    if cut {
        block.push_str("\n… (cut; read the rest with history.read_as_of)");
    }
    block.push_str("\n</as-of-context>");
    Some(block)
}

// ── Commands ───────────────────────────────────────────────────────────────

/// A vault file as it stood at the end of a day (see [`read`]).
#[tauri::command]
pub async fn history_read_as_of(
    state: tauri::State<'_, crate::commands::AppState>,
    path: String,
    date: String,
) -> Result<AsOf, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || read(&vault, &path, &date))
        .await
        .map_err(|e| e.to_string())?
}

/// One workspace's daily metrics.
#[tauri::command]
pub fn workspace_metrics_daily(
    state: tauri::State<'_, crate::commands::AppState>,
    workspace: String,
    from: Option<String>,
    to: Option<String>,
) -> Vec<Value> {
    workspace_metrics(
        &state.config.vault_path,
        &workspace,
        from.as_deref(),
        to.as_deref(),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn vault() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("hollow-asof-")
            .tempdir()
            .unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("workspaces/ops")).unwrap();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n\nVersion one.\n",
        )
        .unwrap();
        std::fs::write(
            v.join("workspaces/ops/PLAN.md"),
            "# Plan\n\n- [ ] a\n- [x] b\n- [ ] c\n",
        )
        .unwrap();
        dir
    }

    pub(crate) fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(2026, 10, day, hour, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn an_unchanged_day_stores_nothing() {
        let dir = vault();
        let v = dir.path();
        // Taken at 03:00: it stands for the end of the day before.
        assert_eq!(take(v, at(1, 3)).unwrap().as_deref(), Some("2026-09-30"));
        assert_eq!(take(v, at(2, 3)).unwrap(), None, "nothing changed");
        assert_eq!(snapshots(v).len(), 1);
        std::fs::write(v.join("workspaces/ops/PLAN.md"), "# Plan\n\n- [x] a\n").unwrap();
        assert_eq!(take(v, at(3, 3)).unwrap().as_deref(), Some("2026-10-02"));
        assert_eq!(snapshots(v).len(), 2);
        // Content-addressed: the unchanged brief is one object.
        let objects = std::fs::read_dir(v.join(".hollow-history/objects"))
            .unwrap()
            .count();
        assert_eq!(objects, 3, "brief + two plans");
    }

    #[test]
    fn as_of_reads_across_a_persons_edit_an_agent_edit_and_a_gap() {
        let dir = vault();
        let v = dir.path();
        // Before any snapshot: not known, never empty.
        let early = read(v, "workspaces/ops/WORKSPACE.md", "2026-09-15").unwrap();
        assert_eq!(early.source, "not-captured");
        assert!(early.text.is_none());
        take(v, at(1, 3)).unwrap();
        // The person edits the brief on the 2nd; the snapshot of the 3rd
        // holds it.
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n\nVersion two (mine).\n",
        )
        .unwrap();
        take(v, at(3, 3)).unwrap();
        // The 03:00 snapshot of the 3rd is the end of the 2nd (review fix:
        // the person's edit on the 2nd shows as of the 2nd).
        let first = read(v, "workspaces/ops/WORKSPACE.md", "2026-10-01").unwrap();
        assert_eq!(first.source, "snapshot");
        assert!(first.text.unwrap().contains("Version one."));
        let second = read(v, "workspaces/ops/WORKSPACE.md", "2026-10-02").unwrap();
        assert!(second.text.unwrap().contains("Version two (mine)."));
        // An agent run edits it on the 4th: History's version wins after it.
        let handle =
            crate::automated_history::begin_run(v, "chat", Some("test-agent-edit")).unwrap();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n\nVersion three (agent).\n",
        )
        .unwrap();
        crate::automated_history::finish_run(v, handle, "completed").unwrap();
        let today = Local::now().format("%Y-%m-%d").to_string();
        let agent = read(v, "workspaces/ops/WORKSPACE.md", &today).unwrap();
        assert_eq!(agent.source, "history", "{agent:?}");
        assert!(agent.text.unwrap().contains("Version three (agent)."));
        // A path no snapshot held that day.
        let absent = read(v, "tasks/new.md", "2026-10-03").unwrap();
        // Review fix: a file snapshots never track is not known, not absent.
        let people = read(v, "people/Ana.md", "2026-10-03").unwrap();
        assert_eq!(people.source, "not-captured");
        assert_eq!(absent.source, "absent");
        assert!(read(v, "../x.md", "2026-10-03").is_err());
        assert!(read(v, "workspaces/ops/WORKSPACE.md", "October").is_err());
    }

    #[test]
    fn retention_keeps_the_newest_snapshot_and_history_gc_keeps_its_objects() {
        let dir = vault();
        let v = dir.path();
        let now = Utc::now();
        let old = now - Duration::days(200);
        take(v, old).unwrap();
        // Only snapshot, and old: kept (the newest).
        assert_eq!(prune(v, now), 0);
        std::fs::write(v.join("workspaces/ops/PLAN.md"), "# Plan\n\n- [ ] new\n").unwrap();
        take(v, old + Duration::days(1)).unwrap();
        assert_eq!(prune(v, now), 1, "the older one goes, the newest stays");
        let left = snapshots(v);
        assert_eq!(left.len(), 1);
        // History's GC runs with no run referencing these objects: they stay.
        crate::automated_history::run_retention_for_test(v);
        let hash = left[0].files["workspaces/ops/PLAN.md"].clone();
        assert!(crate::automated_history::load_object(v, &hash).is_ok());
        // Review fix: an unreadable snapshot manifest stops object GC, as an
        // unreadable run record does.
        std::fs::write(v.join(".hollow-history/snapshots/broken.json"), "{not json").unwrap();
        let stray = crate::automated_history::content_hash(b"unreferenced");
        crate::automated_history::with_history_lock(|| {
            crate::automated_history::store_object(v, &stray, b"unreferenced")
        })
        .unwrap();
        crate::automated_history::run_retention_for_test(v);
        assert!(
            crate::automated_history::load_object(v, &stray).is_ok(),
            "nothing collected blind"
        );
    }

    #[test]
    fn the_asked_day_is_read_from_the_request() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let day = |r: &str| asked_day(r, today).map(|d| d.to_string());
        assert_eq!(
            day("what did I know on 9/15 about Atlas?").as_deref(),
            Some("2026-09-15")
        );
        assert_eq!(
            day("and/or what did we know on 9/15").as_deref(),
            Some("2026-09-15")
        );
        assert_eq!(
            day("as of 2026-09-01, what was the plan").as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            day("What did I know on September 15?").as_deref(),
            Some("2026-09-15")
        );
        assert_eq!(
            day("what did I know on 15 Sept 2025").as_deref(),
            Some("2025-09-15")
        );
        assert_eq!(
            day("as of 12/20 what did we know"),
            None,
            "yearless and ahead: ambiguous"
        );
        assert_eq!(
            day("as of 12/20/2025 what did we know").as_deref(),
            Some("2025-12-20")
        );
        assert_eq!(day("draft the RFP answer for 9/15"), None, "no as-of cue");
        assert_eq!(day("what is due on 10/20?"), None, "a future day");
        assert_eq!(day("we decided on the plan"), None);
        assert_eq!(
            day("reply to the email sent on 10/3"),
            None,
            "a date alone isn't an as-of question"
        );
    }

    #[test]
    fn a_what_did_i_know_question_gets_the_versions_with_their_dates() {
        let dir = vault();
        let v = dir.path();
        take(v, at(1, 3)).unwrap();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n\nVersion two.\n",
        )
        .unwrap();
        take(v, at(5, 3)).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        // From the vault root, naming the workspace.
        let block = question_block(v, v, "what did I know on 10/2 about Ops?", today).unwrap();
        assert!(
            block.starts_with("<as-of-context date=\"2026-10-02\" workspace=\"ops\">"),
            "{block}"
        );
        assert!(
            block.contains("Version one.") && !block.contains("Version two."),
            "{block}"
        );
        assert!(block.contains("From the snapshot of 2026-09-30"), "{block}");
        assert!(block.ends_with("</as-of-context>"));
        // Before any snapshot: said, not guessed.
        let early = question_block(v, v, "what did I know on 9/15 about Ops?", today).unwrap();
        assert!(early.contains("[not-captured]"), "{early}");
        assert!(question_block(v, v, "what's the plan for Ops?", today).is_none());
        assert!(
            question_block(v, v, "what did I know on 10/2?", today).is_none(),
            "no workspace named"
        );
    }

    #[test]
    fn metrics_count_turns_cost_changes_and_open_rows() {
        let dir = vault();
        let v = dir.path();
        take(v, at(1, 3)).unwrap();
        std::fs::write(v.join("workspaces/ops/PLAN.md"), "# Plan\n\n- [ ] a\n").unwrap();
        take(v, at(2, 3)).unwrap();
        let mut row = crate::session_store::tests::usage_row_fixture(
            "k1",
            &crate::session_store::db_time(at(2, 2)),
            "success",
        );
        row.cost_usd = Some(1.25);
        row.spawned = true;
        row.surface = "chat".into();
        crate::session_store::upsert_usage(v, &row).unwrap();
        crate::session_store::set_usage_attribution(
            v,
            "k1",
            &crate::run_attribution::Attribution {
                workspace_id: Some("ops".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let m = metrics_for(
            v,
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            at(2, 23),
            true,
        );
        let ops = &m["ops"];
        assert_eq!(
            (
                ops.turns,
                ops.cost_usd,
                ops.files_changed,
                ops.open_plan_rows
            ),
            (1, 1.25, 1, 1)
        );
        write_metrics(v, "2026-10-02", &m).unwrap();
        let read_back = workspace_metrics(v, "ops", Some("2026-10-01"), None);
        assert_eq!(read_back[0]["day"], "2026-10-02");
        assert_eq!(read_back[0]["turns"], 1);
    }
}
