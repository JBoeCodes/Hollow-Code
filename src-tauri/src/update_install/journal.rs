//! One journal per install or go back: `<state>/update-journal/<id>.json`,
//! written whole and durably (temp file, fsync, rename, folder fsync) before
//! and after each step, so a crash leaves the old file or the new one and
//! the next run reads what was done instead of guessing from the folders.
//!
//! States, in order: `prepared` (lock held, inputs recorded),
//! `staged-verified` (the copy beside the app verified), `snapshot-written`
//! (the Sessions snapshot, or `--no-snapshot`), `promoted` (every planned
//! move done: the new app is in place), `started` (once per reopen attempt;
//! a go back that restores data records `data-restored` between its restore
//! moves and the swap), then one settled state: `healthy`, `rolled-back`,
//! `failed` (nothing moved, or every move undone), `unchecked` (`--no-relaunch`) or `start-failed`
//! (a go back whose app did not start). `rolling-back` is in flight.
//! These folders are this installer's own; the `.mjs` keeps `journal/`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) const DIR: &str = "update-journal";
pub(crate) const SCHEMA: u32 = 1;

/// States after which nothing is in flight.
pub(crate) const SETTLED: &[&str] = &[
    "healthy",
    "rolled-back",
    "failed",
    "unchecked",
    "start-failed",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Install,
    GoBack,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Op {
    Rename,
    Swap,
}

/// One planned move. `ino` is the inode of the folder that moves from
/// `from` to `to` (for a swap, the one that ends at `to`): a folder keeps
/// its inode through a rename, so recovery can tell whether this move
/// happened even when both paths exist (a swap).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Move {
    pub(crate) op: Op,
    pub(crate) from: PathBuf,
    pub(crate) to: PathBuf,
    pub(crate) ino: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotFile {
    pub(crate) source: PathBuf,
    pub(crate) copy: PathBuf,
    /// The store's `PRAGMA user_version` (equal in the copy).
    pub(crate) user_version: i64,
    pub(crate) bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub(crate) skipped: bool,
    pub(crate) folder: Option<PathBuf>,
    pub(crate) files: Vec<SnapshotFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attempt {
    pub(crate) attempt: u32,
    pub(crate) launched_at: String,
    /// `healthy`, `launch-failed`, `never-appeared` or `exited`.
    pub(crate) outcome: String,
    pub(crate) pid: Option<i32>,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Step {
    pub(crate) step: String,
    pub(crate) at: String,
    #[serde(flatten)]
    pub(crate) detail: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    #[serde(default)]
    pub(crate) schema: u32,
    pub(crate) kind: Kind,
    pub(crate) id: String,
    pub(crate) pid: i32,
    pub(crate) started_at: String,
    pub(crate) state: String,
    pub(crate) app: PathBuf,
    #[serde(default)]
    pub(crate) staged: Option<PathBuf>,
    #[serde(default)]
    pub(crate) expected_version: Option<String>,
    #[serde(default)]
    pub(crate) expected_team: Option<String>,
    #[serde(default)]
    pub(crate) from_version: Option<String>,
    #[serde(default)]
    pub(crate) to_version: Option<String>,
    /// Whether the installed app verified and so becomes `.previous`.
    #[serde(default)]
    pub(crate) keep_current: Option<bool>,
    /// The inode of the verified copy beside the app (the new app).
    #[serde(default)]
    pub(crate) new_ino: Option<u64>,
    /// The planned moves, written before the first one.
    /// Always written (`null` until planned). Required: a journal without
    /// the field is unreadable, never read as "nothing planned" (serde would
    /// otherwise read a missing `Option` as `None`).
    #[serde(deserialize_with = "required_option")]
    pub(crate) plan: Option<Vec<Move>>,
    pub(crate) moves_done: usize,
    #[serde(default)]
    pub(crate) snapshot: Option<Snapshot>,
    #[serde(default)]
    pub(crate) attempts: Vec<Attempt>,
    #[serde(default)]
    pub(crate) steps: Vec<Step>,
    /// A go back's Sessions restore (`--restore-snapshot`, row U6); its
    /// moves are the first ones of `plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) restore: Option<super::restore::RestoreRecord>,
}

impl Record {
    pub(crate) fn new(kind: Kind, id: &str, pid: i32, app: &Path, now: &str) -> Self {
        Self {
            schema: SCHEMA,
            kind,
            id: id.to_string(),
            pid,
            started_at: now.to_string(),
            state: "new".into(),
            app: app.to_path_buf(),
            staged: None,
            expected_version: None,
            expected_team: None,
            from_version: None,
            to_version: None,
            keep_current: None,
            new_ino: None,
            plan: None,
            moves_done: 0,
            snapshot: None,
            attempts: Vec::new(),
            steps: Vec::new(),
            restore: None,
        }
    }

    pub(crate) fn settled(&self) -> bool {
        SETTLED.contains(&self.state.as_str())
    }
}

#[derive(Debug)]
pub(crate) struct Journal {
    pub(crate) path: PathBuf,
    pub(crate) record: Record,
}

impl Journal {
    /// A new journal for install `id` under `state_dir`; refuses an id that
    /// already has one.
    pub(crate) fn create(state_dir: &Path, record: Record) -> io::Result<Self> {
        let folder = state_dir.join(DIR);
        super::lock::create_private_dir(&folder)?;
        let path = folder.join(format!("{}.json", safe_id(&record.id)));
        if path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("install id {} already has a journal", record.id),
            ));
        }
        let journal = Self { path, record };
        journal.save()?;
        Ok(journal)
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&self.record).map_err(io::Error::other)?;
        write_durably(&self.path, &format!("{text}\n"))
    }

    /// Record `state` (with `detail`) and save it.
    pub(crate) fn step(&mut self, state: &str, at: &str, detail: Value) -> io::Result<()> {
        let detail = match detail {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => {
                let mut map = Map::new();
                map.insert("detail".into(), other);
                map
            }
        };
        self.record.steps.push(Step {
            step: state.to_string(),
            at: at.to_string(),
            detail,
        });
        self.record.state = state.to_string();
        self.save()
    }
}

/// Every readable journal under `state_dir`, oldest first, and the paths
/// that could not be read.
pub(crate) fn list(state_dir: &Path) -> (Vec<Journal>, Vec<(PathBuf, String)>) {
    let folder = state_dir.join(DIR);
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let Ok(entries) = fs::read_dir(&folder) else {
        return (found, unreadable);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        match fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Record>(&t).map_err(|e| e.to_string()))
        {
            Ok(record) if record.schema != SCHEMA => {
                unreadable.push((path, format!("schema {}, not {SCHEMA}", record.schema)))
            }
            Ok(record) => found.push(Journal { path, record }),
            Err(why) => unreadable.push((path, why)),
        }
    }
    found.sort_by(|a, b| a.record.started_at.cmp(&b.record.started_at));
    (found, unreadable)
}

/// States from which apps may have moved: a journal set aside in one of them
/// blocks until a person looks.
pub(crate) const MOVING: &[&str] = &[
    "snapshot-written",
    "data-restored",
    "promoted",
    "started",
    "rolling-back",
];

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The set-aside journals (`*.json.bad`) last written after the newest
/// settled journal, each with whether its text still names a [`MOVING`]
/// state. Older ones are history.
pub(crate) fn recent_set_aside(state_dir: &Path) -> Vec<(PathBuf, bool)> {
    let (all, _) = list(state_dir);
    let newest_settled = all
        .iter()
        .filter(|j| j.record.settled())
        .filter_map(|j| modified(&j.path))
        .max();
    let Ok(entries) = fs::read_dir(state_dir.join(DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".json.bad"))
        .filter(|p| match (modified(p), newest_settled) {
            (Some(bad), Some(settled)) => bad > settled,
            _ => true,
        })
        .map(|p| {
            let moving = fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.get("state").and_then(|s| s.as_str()).map(str::to_string))
                .is_some_and(|state| MOVING.contains(&state.as_str()));
            (p, moving)
        })
        .collect()
}

/// Keep the newest `keep` set-aside journals.
pub(crate) fn prune_set_aside(state_dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(state_dir.join(DIR)) else {
        return;
    };
    let mut bad: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".json.bad"))
        .map(|p| (modified(&p).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    bad.sort();
    bad.reverse();
    for (_, old) in bad.into_iter().skip(keep) {
        let _ = fs::remove_file(old);
    }
}

/// Keep the newest `keep` settled journals; never one still in flight.
pub(crate) fn prune(state_dir: &Path, keep: usize) {
    let (all, _) = list(state_dir);
    let settled: Vec<&Journal> = all.iter().filter(|j| j.record.settled()).collect();
    let extra = settled.len().saturating_sub(keep);
    for journal in settled.into_iter().take(extra) {
        let _ = fs::remove_file(&journal.path);
    }
}

pub(crate) fn safe_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect()
}

/// Write `text` so a crash leaves the old file or the new one.
pub(crate) fn write_durably(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        // The rename is already atomic; a folder fsync is extra durability.
        if let Ok(dir) = fs::File::open(dir) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// A plain `Option` deserializer; naming it with `deserialize_with` makes
/// serde require the field to be present (it may still be `null`).
fn required_option<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de)
}
