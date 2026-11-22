//! Go back to the previous version: the plan Settings, Updates shows
//! (updates design section 9; lane U row U6). The flow that acts on it is
//! `goback_run.rs`.
//!
//! [`plan`] reads only: whether `.Hollow.app.previous` exists beside the
//! installed app and what it is (its Info.plist), when each version was
//! installed (the installer's journals), and the **schema verdict**: every
//! store's current `PRAGMA user_version` (`hollow_databases::list`) against
//! what the previous build knew. A build records what it knows at launch in
//! `<installer>/known-schemas.json` ([`record_known_schema`]); for a
//! previous build that predates that record, the Sessions snapshot's
//! recorded `user_version` stands in for the Sessions stores.
//!
//! What an older build does with a newer store decides the verdict (row
//! U6's first test, `goback_store_tests.rs`): Sessions and the companion
//! ledger never read `user_version`, so they open a newer database blindly;
//! a newer one makes the verdict `needsRestore` (Go back must put the data
//! from before the update back). Both stamp their own `SCHEMA_VERSION`
//! (raised with a breaking migration), which [`known_schema`] reports. The
//! restore covers only the Sessions databases the snapshot copied, so a
//! newer store outside it (the companion ledger) refuses Go back instead.
//! The three indexes refuse a newer index and
//! leave it, so search falls back to scanning: they only add a sentence to
//! `indexesOff`.

use crate::hollow_databases::DbKind;
use crate::update_install::journal::{self, Kind, Record, Snapshot};
use crate::update_install::{restore, verify, Layout, BUNDLE_ID};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `<installer>/known-schemas.json`: version -> store -> schema version.
pub(crate) const KNOWN_FILE: &str = "known-schemas.json";
/// Versions kept in it (newest by insertion).
const KNOWN_KEEP: usize = 30;

/// The schema versions this build knows, keyed as the release manifest's
/// `schema` object is (`SCHEMA_SOURCES` in `scripts/release/nightly.mjs`).
pub(crate) fn known_schema() -> BTreeMap<String, i64> {
    BTreeMap::from([
        ("sessions".to_string(), crate::session_store::SCHEMA_VERSION),
        (
            "companion".to_string(),
            crate::companion_inbox::LEDGER_SCHEMA_VERSION,
        ),
        (
            "searchIndex".to_string(),
            crate::vault_search_index::SCHEMA_VERSION,
        ),
        (
            "embedIndex".to_string(),
            crate::vault_embed_index::SCHEMA_VERSION,
        ),
        (
            "projectIndex".to_string(),
            crate::project_text_index::SCHEMA_VERSION,
        ),
    ])
}

fn store_key(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Sessions => "sessions",
        DbKind::Companion => "companion",
        DbKind::SearchIndex => "searchIndex",
        DbKind::EmbedIndex => "embedIndex",
        DbKind::ProjectIndex => "projectIndex",
    }
}

/// How an older build treats a store stamped newer than it knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreRule {
    /// Opens and writes it as if it knew it (Sessions, companion ledger).
    Blind,
    /// Refuses it and leaves it; the feature is off until a newer build
    /// (the three indexes, all rebuildable).
    Refuses,
}

pub(crate) fn store_rule(kind: DbKind) -> StoreRule {
    match kind {
        DbKind::Sessions | DbKind::Companion => StoreRule::Blind,
        DbKind::SearchIndex | DbKind::EmbedIndex | DbKind::ProjectIndex => StoreRule::Refuses,
    }
}

fn index_words(kind: DbKind) -> &'static str {
    match kind {
        DbKind::SearchIndex => "keyword search index",
        DbKind::EmbedIndex => "meaning search index",
        DbKind::ProjectIndex => "Projects content index",
        DbKind::Sessions | DbKind::Companion => "store",
    }
}

type Known = BTreeMap<String, BTreeMap<String, i64>>;

fn read_known(state_dir: &Path) -> Known {
    std::fs::read(state_dir.join(KNOWN_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Record what this build knows under its version, so a newer build can
/// tell, after an update, whether going back here is safe. Best effort.
pub(crate) fn record_known_schema(state_dir: &Path, version: &str) {
    if !super::state::valid_version(version) {
        return;
    }
    let mut known = read_known(state_dir);
    let mine = known_schema();
    if known.get(version) == Some(&mine) {
        return;
    }
    known.insert(version.to_string(), mine);
    while known.len() > KNOWN_KEEP {
        // The lowest version goes first.
        let lowest = known
            .keys()
            .min_by(|a, b| verify::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();
        match lowest {
            Some(v) => known.remove(&v),
            None => break,
        };
    }
    if crate::update_install::lock::create_private_dir(state_dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&known) {
        let _ = journal::write_durably(&state_dir.join(KNOWN_FILE), &format!("{text}\n"));
    }
}

// ── The plan the window gets ────────────────────────────────────────────

/// `update_go_back_plan`: what Settings, Updates shows for Go back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateGoBackPlan {
    /// Some Go back button is enabled (plain, or with the data restore when
    /// the verdict is `needsRestore`).
    pub available: bool,
    /// Why not, in plain words, when not available.
    pub disabled_reason: Option<String>,
    pub running_version: String,
    /// When the running version was installed (its install journal), the
    /// moment the snapshot was taken.
    pub running_installed_at: Option<String>,
    pub previous: Option<UpdatePreviousBuild>,
    /// `None` without a usable previous copy.
    pub schema: Option<UpdateSchemaVerdict>,
    /// The Sessions snapshot taken before the running version's install,
    /// when it is the one from the previous version.
    pub snapshot: Option<UpdateSnapshotInfo>,
    /// `update_go_back {restoreData: true}` would be accepted: the
    /// snapshot exists and passes `quick_check`.
    pub restore_available: bool,
    /// Indexes the previous version would leave off (search then scans the
    /// vault), one sentence each. Never blocks Go back.
    pub indexes_off: Vec<String>,
    /// The version Go back skips (this one) until a newer one is published.
    pub will_skip: String,
    /// The version skipped now ("Skipping 0.6.n" with Undo).
    pub skipped_version: Option<String>,
    /// "Go back when chats finish" is waiting.
    pub waiting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreviousBuild {
    pub version: String,
    /// When it was installed (its install journal), when known.
    pub installed_at: Option<String>,
}

/// Can the previous version use today's data as it is?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum UpdateSchemaVerdict {
    /// Every store it would open blindly is at a version it knows: a plain
    /// swap (`update_go_back {restoreData: false}`).
    Compatible,
    /// A store it would open blindly is newer than it knows: Go back only
    /// with the data from before the update (`restoreData: true`).
    NeedsRestore {
        newer: Vec<UpdateStoreVersion>,
        /// What the restore sets aside, counted from the live Sessions
        /// databases; `None` when it couldn't be counted.
        lost: Option<UpdateLostData>,
        /// One sentence for the dialog.
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateStoreVersion {
    /// `sessions`, `companion`, `searchIndex`, `embedIndex`, `projectIndex`.
    pub store: String,
    /// The file's name.
    pub file: String,
    pub current: i64,
    /// What the previous build knows; `None` when it didn't say.
    pub known: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshotInfo {
    /// When it was taken (RFC 3339).
    pub at: String,
    pub files: u32,
    /// Every copy passes `quick_check` at its recorded version.
    pub intact: bool,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLostData {
    /// The snapshot's time: what came after is set aside.
    pub since: String,
    /// Chats started since.
    pub chats: u64,
    /// Messages (turns) sent since, in any chat.
    pub messages: u64,
    /// "3 chats and 12 messages since …", plain words.
    pub summary: String,
}

/// What the plan reads; the client fills it, tests build it.
pub(crate) struct PlanInputs<'a> {
    pub(crate) running_version: &'a str,
    /// The running bundle; `None` in a dev run.
    pub(crate) installed_app: Option<&'a Path>,
    pub(crate) state_dir: Option<&'a Path>,
    /// The app support folder (`Sessions`, `SearchIndex`, ... inside).
    pub(crate) support: Option<&'a Path>,
    /// A review copy, or no app data.
    pub(crate) review_copy: bool,
    /// An install or go back is starting in this app.
    pub(crate) busy: bool,
    /// Another installer holds the lock.
    pub(crate) lock_live: bool,
    pub(crate) skipped_version: Option<String>,
    pub(crate) waiting: bool,
}

pub(crate) const NO_PREVIOUS: &str =
    "There's no previous version on this Mac yet. Hollow keeps one after its first update.";
pub(crate) const NOT_FROM_APPLICATIONS: &str =
    "This Hollow isn't running from the Applications folder, so it can't go back.";
pub(crate) const REVIEW_COPY: &str = "This copy of Hollow can't change the installed app.";
pub(crate) const BUSY: &str = "An update is installing; try again after Hollow reopens.";
pub(crate) const LOCKED: &str = "Another install is running; try again when it finishes.";

/// Is `app` in the Applications folder? (`/Applications`; under test, any
/// folder named `Applications`.)
pub(crate) fn in_applications(app: &Path) -> bool {
    let Some(parent) = app.parent() else {
        return false;
    };
    if cfg!(test) {
        parent.file_name().is_some_and(|n| n == "Applications")
    } else {
        parent == Path::new("/Applications")
    }
}

/// The newest settled install that put `version` in place.
fn install_of<'a>(journals: &'a [journal::Journal], version: &str) -> Option<&'a Record> {
    journals.iter().rev().map(|j| &j.record).find(|r| {
        r.kind == Kind::Install
            && matches!(r.state.as_str(), "healthy" | "unchecked")
            && r.to_version.as_deref() == Some(version)
    })
}

fn step_at(record: &Record, step: &str) -> Option<String> {
    record
        .steps
        .iter()
        .find(|s| s.step == step)
        .map(|s| s.at.clone())
}

/// The snapshot of the running version's install, and when it was taken,
/// when it came from `previous`.
pub(crate) fn snapshot_for(
    journals: &[journal::Journal],
    running: &str,
    previous: &str,
) -> Option<(Snapshot, String)> {
    let r = install_of(journals, running)?;
    if r.from_version.as_deref() != Some(previous) {
        return None;
    }
    let snap = r
        .snapshot
        .clone()
        .filter(|s| !s.skipped && s.folder.is_some())?;
    let at = step_at(r, "snapshot-written").unwrap_or_else(|| r.started_at.clone());
    Some((snap, at))
}

/// A copy's `quick_check` result, kept while the file is unchanged (its
/// size, modification time and inode): the plan is read on every Settings
/// visit and before each Go back, and a full check of a large Sessions copy
/// takes seconds. The helper checks every copy in full again before it moves
/// anything (`restore::prepare`).
type CheckKey = (PathBuf, i64, u64, std::time::SystemTime, u64);
static CHECKED: std::sync::Mutex<BTreeMap<CheckKey, Result<(), String>>> =
    std::sync::Mutex::new(BTreeMap::new());
/// Entries kept (a few snapshots of a few databases).
const CHECKED_KEEP: usize = 32;

pub(crate) fn cached_check(path: &Path, version: i64) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return restore::check(path, version);
    };
    let key: CheckKey = (
        path.to_path_buf(),
        version,
        meta.len(),
        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
        meta.ino(),
    );
    let lock = || CHECKED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(found) = lock().get(&key) {
        return found.clone();
    }
    let result = restore::check(path, version);
    let mut cache = lock();
    cache.retain(|k, _| k.0 != key.0);
    if cache.len() >= CHECKED_KEEP {
        cache.clear();
    }
    cache.insert(key, result.clone());
    result
}

fn snapshot_info(snap: &Snapshot, at: &str) -> UpdateSnapshotInfo {
    let problem = if snap.files.is_empty() {
        Some("the snapshot holds no databases".to_string())
    } else {
        snap.files
            .iter()
            .find_map(|f| cached_check(&f.copy, f.user_version).err())
    };
    UpdateSnapshotInfo {
        at: at.to_string(),
        files: snap.files.len() as u32,
        intact: problem.is_none(),
        problem,
    }
}

/// Chats and messages in the live Sessions databases since `since`
/// (read-only; `julianday` compares the stored RFC 3339 times whatever
/// their precision or offset).
pub(crate) fn count_since(files: &[PathBuf], since: &str) -> Result<(u64, u64), String> {
    use rusqlite::{Connection, OpenFlags};
    let (mut chats, mut messages) = (0u64, 0u64);
    for live in files.iter().filter(|p| p.exists()) {
        let conn = Connection::open_with_flags(
            live,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("{} can't be read: {e}", live.display()))?;
        let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
        let count = |sql: &str| -> Result<u64, String> {
            conn.query_row(sql, [since], |r| r.get::<_, i64>(0))
                .map(|n| n.max(0) as u64)
                .map_err(|e| format!("{} can't be counted: {e}", live.display()))
        };
        chats +=
            count("SELECT count(*) FROM sessions WHERE julianday(created_at) > julianday(?1)")?;
        messages +=
            count("SELECT count(*) FROM turns WHERE julianday(started_at) > julianday(?1)")?;
    }
    Ok((chats, messages))
}

/// The same file (both resolved when they exist, else as written).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "Nov 15 at 09:30" in local time, or the stamp as given.
pub(crate) fn when(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d at %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| at.to_string())
}

fn lost_data(snap: &Snapshot, since: &str, previous: &str) -> Option<UpdateLostData> {
    let live: Vec<PathBuf> = snap.files.iter().map(|f| f.source.clone()).collect();
    let (chats, messages) = count_since(&live, since).ok()?;
    let summary = if chats == 0 && messages == 0 {
        format!(
            "Nothing was added to your chats since {}, so nothing is set aside.",
            when(since)
        )
    } else {
        format!(
            "{} and {} since {} won't be in Hollow {previous}. They're kept aside in a copy beside the Sessions database (.before-go-back), not deleted.",
            plural(chats, "chat started", "chats started"),
            plural(messages, "message", "messages"),
            when(since)
        )
    };
    Some(UpdateLostData {
        since: since.to_string(),
        chats,
        messages,
        summary,
    })
}

/// The plan. Reads only. An install starting here, or another installer
/// holding the lock, disables an otherwise available Go back.
pub(crate) fn plan(i: &PlanInputs) -> UpdateGoBackPlan {
    let mut out = plan_inner(i);
    if out.available && (i.busy || i.lock_live) {
        out.available = false;
        out.disabled_reason = Some(if i.busy { BUSY } else { LOCKED }.to_string());
    }
    out
}

fn plan_inner(i: &PlanInputs) -> UpdateGoBackPlan {
    let mut out = UpdateGoBackPlan {
        available: false,
        disabled_reason: None,
        running_version: i.running_version.to_string(),
        running_installed_at: None,
        previous: None,
        schema: None,
        snapshot: None,
        restore_available: false,
        indexes_off: Vec::new(),
        will_skip: i.running_version.to_string(),
        skipped_version: i.skipped_version.clone(),
        waiting: i.waiting,
    };
    let refuse = |mut out: UpdateGoBackPlan, why: &str| {
        out.disabled_reason = Some(why.to_string());
        out
    };
    if i.review_copy {
        return refuse(out, REVIEW_COPY);
    }
    let Some(app) = i.installed_app.filter(|a| in_applications(a) && a.exists()) else {
        return refuse(out, NOT_FROM_APPLICATIONS);
    };
    let Some(state_dir) = i.state_dir else {
        return refuse(
            out,
            "Hollow can't find its installer folder, so it can't go back.",
        );
    };
    let layout = Layout::for_app(app);
    let (journals, _) = journal::list(state_dir);
    out.running_installed_at = install_of(&journals, i.running_version)
        .map(|r| step_at(r, "promoted").unwrap_or_else(|| r.started_at.clone()));
    if !layout.previous.exists() {
        return refuse(out, NO_PREVIOUS);
    }
    let info = match verify::read_info(&layout.previous) {
        Ok(info) => info,
        Err(e) => {
            let why = format!("The previous copy of Hollow can't be read ({e}).");
            return refuse(out, &why);
        }
    };
    if info.bundle_id != BUNDLE_ID {
        let why = format!(
            "The kept copy beside Hollow isn't Hollow ({}).",
            info.bundle_id
        );
        return refuse(out, &why);
    }
    out.previous = Some(UpdatePreviousBuild {
        version: info.version.clone(),
        installed_at: install_of(&journals, &info.version)
            .map(|r| step_at(r, "promoted").unwrap_or_else(|| r.started_at.clone())),
    });
    if verify::compare_versions(&info.version, i.running_version) != Some(std::cmp::Ordering::Less)
    {
        let why = format!(
            "The kept copy is Hollow {}, which isn't older than this one.",
            info.version
        );
        return refuse(out, &why);
    }

    // The snapshot and the schema verdict.
    let snap = snapshot_for(&journals, i.running_version, &info.version);
    if let Some((s, at)) = &snap {
        let si = snapshot_info(s, at);
        out.restore_available = si.intact;
        out.snapshot = Some(si);
    }
    let known_all = read_known(state_dir);
    let known = known_all.get(&info.version);
    let snap_sessions = snap
        .as_ref()
        .and_then(|(s, _)| s.files.iter().map(|f| f.user_version).max());
    let known_for = |kind: DbKind| -> Option<i64> {
        known
            .and_then(|k| k.get(store_key(kind)).copied())
            .or(match kind {
                DbKind::Sessions => snap_sessions,
                _ => None,
            })
    };
    let mut newer = Vec::new();
    // The live paths of `newer`, to tell which ones the restore covers.
    let mut newer_paths: Vec<PathBuf> = Vec::new();
    let mut off: Vec<DbKind> = Vec::new();
    if let Some(support) = i.support {
        for f in crate::hollow_databases::list(support, Path::new("/"), false) {
            let Some(current) = f.schema_version else {
                continue;
            };
            let k = known_for(f.kind);
            match store_rule(f.kind) {
                // Every build knows 0 (an unstamped store); above what it
                // knows, or above 0 with nothing known, it would misread.
                StoreRule::Blind if current > k.unwrap_or(0) => {
                    newer_paths.push(PathBuf::from(&f.path));
                    newer.push(UpdateStoreVersion {
                        store: store_key(f.kind).into(),
                        file: Path::new(&f.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        current,
                        known: k,
                    })
                }
                StoreRule::Refuses if k.is_some_and(|k| current > k) && !off.contains(&f.kind) => {
                    off.push(f.kind)
                }
                _ => {}
            }
        }
    }
    out.indexes_off = off
        .into_iter()
        .map(|kind| {
            format!(
                "Hollow {} leaves the {} off (it was rebuilt for this version), so search reads your files directly until you update again.",
                info.version,
                index_words(kind)
            )
        })
        .collect();
    if newer.is_empty() {
        out.schema = Some(UpdateSchemaVerdict::Compatible);
        out.available = true;
        return out;
    }
    // The restore puts back only what the snapshot copied (the Sessions
    // databases): a newer store outside it (the companion ledger) would
    // still be opened blindly, so the restore can't make Go back safe.
    let uncovered: Vec<String> = newer
        .iter()
        .zip(&newer_paths)
        .filter(|(_, path)| {
            !snap
                .as_ref()
                .is_some_and(|(s, _)| s.files.iter().any(|f| same_file(&f.source, path)))
        })
        .map(|(n, _)| n.file.clone())
        .collect();
    let lost = snap
        .as_ref()
        .and_then(|(s, at)| lost_data(s, at, &info.version));
    let summary = format!(
        "Hollow {} can't safely read the chats this version saved (it would open {} without knowing its newer format). Going back puts your chats back as they were before the update.",
        info.version,
        newer
            .iter()
            .map(|n| n.file.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.schema = Some(UpdateSchemaVerdict::NeedsRestore {
        newer,
        lost,
        summary,
    });
    if !uncovered.is_empty() {
        out.restore_available = false;
        let why = format!(
            "Hollow {} can't safely read {} as this version left {}, and the data from before the update doesn't include {}, so it can't go back.",
            info.version,
            uncovered.join(", "),
            if uncovered.len() == 1 { "it" } else { "them" },
            if uncovered.len() == 1 { "it" } else { "them" },
        );
        return refuse(out, &why);
    }
    if !out.restore_available {
        let why = format!(
            "Hollow {} can't safely read the chats this version saved, and there's no intact copy of them from before the update to put back{}.",
            info.version,
            out.snapshot
                .as_ref()
                .and_then(|s| s.problem.as_ref())
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        );
        return refuse(out, &why);
    }
    out.available = true;
    out
}
