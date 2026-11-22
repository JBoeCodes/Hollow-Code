//! Provider-neutral tracker sync (overhaul A5). Contract: docs/TRACKER_SYNC.md.
//!
//! The ledger and outbox are committed before any remote write. A missing,
//! damaged or differently scoped ledger never becomes an empty baseline.
//! Writers take the sync mutex, its cross-process flock, then the History
//! lease; vault note writes use CAS.

use crate::config::Config;
use crate::tracker::mirror::{self, Values};
use crate::tracker::settings::provider as settings_provider;
use crate::tracker::{self, ipc::*, *};
use crate::{frontmatter, vault};
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

pub(crate) const STATE: &str = ".tracker-sync.json";
pub(crate) const SYNC_EVENT: &str = "tracker-sync-updated";
/// The in-process sync mutex, one per vault root. Production has one vault,
/// so this is one lock there; tests on separate temp roots don't contend.
/// tokio hands a released lock to queued `lock()` callers before any
/// `try_lock`, so one shared mutex let the edit seam's `try_lock` loop starve
/// for its whole 20 s wait behind other tests' syncs (2026-11-13).
static LOCKS: Lazy<std::sync::Mutex<BTreeMap<std::path::PathBuf, Arc<Mutex<()>>>>> =
    Lazy::new(|| std::sync::Mutex::new(BTreeMap::new()));

/// The sync mutex for `root`, keyed by its canonical path so every caller
/// spelling of one vault shares a lock. The flock in `file_lock` still
/// covers other processes.
pub(crate) fn root_lock(root: &Path) -> Arc<Mutex<()>> {
    LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(root_key(root))
        .or_default()
        .clone()
}
/// The key [`root_lock`] and the syncing flag share: the canonical path,
/// or the path as given when it can't be resolved (not created yet).
fn root_key(root: &Path) -> std::path::PathBuf {
    fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}
static WAKE: Notify = Notify::const_new();
/// The vault roots with a sync pass running, keyed like [`root_lock`]. One
/// global flag let a second root's pass, finishing first, report the other
/// root idle while it still ran (2026-11-13).
static SYNCING: Lazy<std::sync::Mutex<BTreeSet<std::path::PathBuf>>> =
    Lazy::new(|| std::sync::Mutex::new(BTreeSet::new()));
static PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Marks `root` syncing until dropped. Taken under the root's sync lock, so
/// one root has at most one.
struct SyncingFlag(std::path::PathBuf);
impl SyncingFlag {
    fn start(root: &Path) -> Self {
        // The key is computed before the mutex is taken: it touches the file
        // system (canonicalize) and must not run under the lock.
        let key = root_key(root);
        SYNCING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone());
        Self(key)
    }
}
impl Drop for SyncingFlag {
    fn drop(&mut self) {
        SYNCING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}
/// Whether a sync pass for `root` is running.
pub(crate) fn is_syncing(root: &Path) -> bool {
    // Key first (it canonicalizes), so the mutex is held only for the lookup.
    let key = root_key(root);
    SYNCING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&key)
}

#[path = "tracker/sync_api.rs"]
mod api;
pub(crate) use api::{
    apply_status, archive, comment, create, create_named, create_project, edit, history, issue,
    team_meta, view,
};

#[path = "tracker/seams.rs"]
pub(crate) mod seams;

#[path = "tracker/sync_posts.rs"]
mod posts;
#[cfg(test)]
use posts::attach_canvas_with;
#[cfg(test)]
use posts::ensure_labels_with;
pub(crate) use posts::{attach_canvas, ensure_labels, post_status_with};

pub(crate) struct ConflictNotice {
    pub record_id: String,
    pub path: String,
    pub workspace_id: Option<String>,
    pub title: String,
    pub field: String,
    pub at: String,
    pub kind: String,
}

/// The bell reads local conflict notes without asking a provider. Loading
/// the same validated ledger keeps unsafe paths out of its open targets.
pub(crate) fn open_conflicts(root: &Path) -> Result<Vec<ConflictNotice>, String> {
    let Some(state) = local_ledger(root)? else {
        return Ok(Vec::new());
    };
    Ok(state
        .records
        .values()
        .flat_map(|record| {
            record.conflicts.iter().map(|c| ConflictNotice {
                record_id: record.id.clone(),
                path: record.path.clone(),
                workspace_id: record.workspace_id.clone(),
                title: record
                    .issue
                    .as_ref()
                    .map(|i| i.title.clone())
                    .or_else(|| record.project.as_ref().map(|p| p.name.clone()))
                    .unwrap_or_else(|| record.key.clone()),
                field: c.field.clone(),
                at: c.at.clone(),
                kind: record.kind.clone(),
            })
        })
        .collect())
}

/// The validated ledger as the last pass left it, read without asking a
/// provider; `None` when no sync has run.
fn local_ledger(root: &Path) -> Result<Option<Ledger>, String> {
    plain(root, STATE)?;
    let raw = match fs::read(root.join(STATE)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Tracker sync history could not be read.".into()),
    };
    let header: Ledger = serde_json::from_slice(&raw)
        .map_err(|_| "Tracker sync history is unreadable.".to_string())?;
    load(root, &header.provider, &primary_scope(&header)).map(Some)
}

/// Every mirrored issue, from the local ledger only (L2's lesson issues):
/// no provider call, so no path around the R3 team guard.
pub(crate) fn mirrored_issues(root: &Path) -> Result<Vec<TrackerIssue>, String> {
    Ok(local_ledger(root)?
        .map(|state| {
            state
                .records
                .into_values()
                .filter_map(|r| r.issue)
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Conflict {
    pub field: String,
    pub local: Value,
    pub remote: Value,
    pub at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    pub id: String,
    pub key: String,
    pub kind: String,
    pub path: String,
    pub workspace_id: Option<String>,
    pub base: Values,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub remote_missing: bool,
    #[serde(default = "active")]
    pub state: String,
    pub issue: Option<TrackerIssue>,
    pub project: Option<TrackerProject>,
}

fn active() -> String {
    "active".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Outbox {
    pub values: Values,
    pub base: Values,
    #[serde(default)]
    pub staged: Option<StagedEdit>,
    /// X2: who asked for the queued edit (`person`, a session key or
    /// `automation:<id>`), for the audit row its send leaves. None: the
    /// person (an edit captured from the file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_by: Option<String>,
    /// Q-R23: the provider id the queued `project` value named when it was
    /// made (the value itself stays a name, for display and for comparing
    /// with the mirror). The send uses the id, so a project renamed before
    /// delivery is still the one that was picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// H4a: the ledger [`Ledger::generation`] this edit was queued under.
    /// An entry from another generation is held, never sent
    /// ([`hold_stale`]). 0 (not written) for entries queued before any
    /// owner switch.
    #[serde(default, skip_serializing_if = "first_generation")]
    pub generation: u32,
}

fn first_generation(generation: &u32) -> bool {
    *generation == 0
}

/// Why a held record is skipped by a pass (H4a), in the record's error.
pub(crate) const HELD_EDIT: &str = "An edit queued before tasks changed owner is held and was not sent. Roll back the switch in Settings, Sources to send it.";

/// Why the editor refuses while tasks are stored in Hollow (H4a).
pub(crate) const STORED_IN_HOLLOW: &str =
    "Tasks are stored in Hollow (Settings, Sources): edit the task file; nothing is sent to the tracker.";

/// The journaled project id that still belongs to `values`' project name:
/// kept from the previous outbox entry only while the queued name is the
/// same one it was resolved from.
pub(crate) fn kept_project_id(old: Option<&Outbox>, values: &Values) -> Option<String> {
    let old = old?;
    let same =
        values.get("project").is_some() && old.values.get("project") == values.get("project");
    same.then(|| old.project_id.clone()).flatten()
}

/// `pending` with its project named by the journaled id, so [`api`]'s
/// resolver takes the id and never re-reads a name. An id whose project is
/// gone from the team is an error, not a fall back to a same-named one.
pub(crate) fn with_journaled_project(
    pending: &Values,
    project_id: Option<&str>,
    projects: &BTreeMap<String, TrackerProject>,
    team: &str,
) -> Result<Values, String> {
    let mut values = pending.clone();
    let blank = values
        .get("project")
        .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|t| t.trim().is_empty()));
    if let (Some(id), false) = (project_id, blank) {
        if !projects
            .get(id)
            .is_some_and(|p| p.team_ids.iter().any(|t| t == team))
        {
            return Err(
                "The project this edit picked is gone from the team. Its journal was preserved."
                    .into(),
            );
        }
        values.insert("project".into(), Value::String(id.into()));
    }
    Ok(values)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StagedEdit {
    pub before: Values,
    pub after: Values,
    #[serde(default)]
    pub local_before: Values,
    #[serde(default)]
    pub local_after: Values,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Create {
    pub agent_key: String,
    pub input: NewIssue,
    pub at: String,
    pub attempted: bool,
    pub issue: Option<TrackerIssue>,
    pub error: Option<String>,
    /// The id the issue is created with (Q-R22), saved before the request
    /// is sent. Recovery reads this one issue; a journal from an older build
    /// has none and is never adopted by matching its fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    /// Sends the pass's recovery made after the issue id read back absent
    /// (Q-R22), at most [`CREATE_RESENDS`].
    #[serde(default, skip_serializing_if = "no_resends")]
    pub resends: u32,
}

/// How many times a pass resends a person's create that never landed.
const CREATE_RESENDS: u32 = 3;

fn no_resends(count: &u32) -> bool {
    *count == 0
}

/// A new create's issue id: a UUID v4 (Linear's `IssueCreateInput.id`).
pub(crate) fn new_issue_id() -> Option<String> {
    Some(crate::workspace_reviews::new_id())
}

/// The input to send for a journaled create: its input with its saved id.
pub(crate) fn create_input(entry: &Create) -> NewIssue {
    NewIssue {
        id: entry.issue_id.clone(),
        ..entry.input.clone()
    }
}

/// A comment the person wrote on an issue (overhaul B5): journaled before
/// the request, so a crash or a lost reply never posts it twice.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommentPost {
    pub issue_id: String,
    pub body: String,
    pub attempted: bool,
    #[serde(default)]
    pub comment_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCreate {
    pub input: NewProject,
    pub attempted: bool,
    pub project: Option<TrackerProject>,
    pub error: Option<String>,
    #[serde(default)]
    pub out_of_scope: bool,
    /// "Track in": the vault workspace this create links (part of the
    /// request: a reused key must name the same one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Archive {
    pub attempted: bool,
    pub confirmed: bool,
    pub error: Option<String>,
}

/// A Friday status copy on a tracker project (A18), journaled before the
/// request under `status-<project>-<ISO week>`: one a project a week, and an
/// unconfirmed one is never resent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusPost {
    pub project_id: String,
    pub week: String,
    /// SHA-256 of the body sent (the note in Hollow is the copy kept).
    pub body_sha256: String,
    pub health: ProjectHealth,
    pub attempted: bool,
    #[serde(default)]
    pub update_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A saved canvas copied onto an issue (A19), journaled before any request
/// under a key from the canvas id, its revision and the issue id. The upload
/// is kept once it lands, so a retry attaches the same file; the provider's
/// attach is idempotent per issue and file, so an uncertain attach may be
/// tried again.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasAttach {
    pub issue_id: String,
    pub workspace_id: String,
    pub canvas_id: String,
    pub revision: u64,
    pub fence_sha256: String,
    #[serde(default)]
    pub upload: Option<TrackerUpload>,
    #[serde(default)]
    pub attachment: Option<TrackerLink>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Migration seeds this exact ledger and uses `mirror` for its files.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Ledger {
    pub version: u32,
    pub provider: String,
    pub scope: String,
    #[serde(default)]
    pub writable_fields: Option<Vec<Field>>,
    pub cursor_issues: Option<String>,
    pub cursor_projects: Option<String>,
    pub cursor_comments: Option<String>,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
    pub records: BTreeMap<String, Record>,
    #[serde(default)]
    pub outbox: BTreeMap<String, Outbox>,
    #[serde(default)]
    pub creates: BTreeMap<String, Create>,
    #[serde(default)]
    pub project_creates: BTreeMap<String, ProjectCreate>,
    #[serde(default)]
    pub archives: BTreeMap<String, Archive>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub comments: BTreeMap<String, CommentPost>,
    /// A18: Friday status copies on tracker projects.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub status_posts: BTreeMap<String, StatusPost>,
    /// A19: canvases copied onto issues.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub canvas_attachments: BTreeMap<String, CanvasAttach>,
    /// The removed company's activity journal (cut 11-10, L11): kept
    /// verbatim so a ledger that has one still loads, never read or sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_activity: Option<serde_json::Value>,
    /// Agent work's tracker writes (X1), sent by the pass.
    #[serde(
        default,
        skip_serializing_if = "tracker::work_outbox::WorkJournal::is_empty"
    )]
    pub work: tracker::work_outbox::WorkJournal,
    #[serde(default)]
    pub meta: Option<TrackerTeamMeta>,
    #[serde(default)]
    pub meta_at: Option<String>,
    #[serde(default)]
    pub projects_at: Option<String>,
    #[serde(default)]
    pub full_date: Option<String>,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub retry_at: Option<String>,
    #[serde(default)]
    pub rate: Option<RateBudget>,
    /// The personal team's own cursors and metadata, keyed by team id, while
    /// the primary slots above hold the focused team's. See [`Ledger::swap_to`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub others: BTreeMap<String, TeamSlot>,
    /// H4a: the owner generation. Each owner switch (`sources::migration`)
    /// writes a new one; outbox entries carry theirs. 0 (not written) for a
    /// ledger no switch has touched, so older ledgers keep their bytes.
    #[serde(default, skip_serializing_if = "first_generation")]
    pub generation: u32,
    /// H4a: outbox entries from another generation, held and reported,
    /// never sent. Their records are skipped by capture and the pass.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub held: BTreeMap<String, Outbox>,
    /// H4a: tasks are stored in Hollow (Settings, Sources). A pass reads and
    /// sends nothing; the mirrors stay as they are.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stored_in_hollow: bool,
}

/// H4a: move every outbox entry queued under another generation into
/// `held`, before capture, so it never reaches the current destination; a
/// held entry of the current generation (its switch was rolled back) goes
/// back to the outbox. Returns how many are held.
pub(crate) fn hold_stale(state: &mut Ledger) -> usize {
    let generation = state.generation;
    let stale: Vec<String> = state
        .outbox
        .iter()
        .filter(|(_, entry)| entry.generation != generation)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &stale {
        if let Some(entry) = state.outbox.remove(id) {
            state.held.insert(id.clone(), entry);
        }
    }
    let current: Vec<String> = state
        .held
        .iter()
        .filter(|(id, entry)| entry.generation == generation && !state.outbox.contains_key(*id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in current {
        if let Some(entry) = state.held.remove(&id) {
            state.outbox.insert(id, entry);
        }
    }
    state.held.len()
}

/// The per-team parts of the ledger: what one team's pass reads and moves
/// (cursors, the hourly metadata and its clocks). Records, outbox and
/// journals are shared by both teams and carry their team in their issue.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamSlot {
    #[serde(default)]
    pub cursor_issues: Option<String>,
    #[serde(default)]
    pub cursor_projects: Option<String>,
    #[serde(default)]
    pub cursor_comments: Option<String>,
    #[serde(default)]
    pub meta: Option<TrackerTeamMeta>,
    #[serde(default)]
    pub meta_at: Option<String>,
    #[serde(default)]
    pub projects_at: Option<String>,
    #[serde(default)]
    pub full_date: Option<String>,
}

impl TeamSlot {
    fn swap(&mut self, ledger: &mut Ledger) {
        std::mem::swap(&mut self.cursor_issues, &mut ledger.cursor_issues);
        std::mem::swap(&mut self.cursor_projects, &mut ledger.cursor_projects);
        std::mem::swap(&mut self.cursor_comments, &mut ledger.cursor_comments);
        std::mem::swap(&mut self.meta, &mut ledger.meta);
        std::mem::swap(&mut self.meta_at, &mut ledger.meta_at);
        std::mem::swap(&mut self.projects_at, &mut ledger.projects_at);
        std::mem::swap(&mut self.full_date, &mut ledger.full_date);
    }
}

impl Ledger {
    /// Focus the ledger on `team`: the primary slots (`scope`, cursors,
    /// metadata) become that team's and the previous team's move to
    /// `others`. Every single-team routine of the engine then runs
    /// unchanged for the focused team. A team the ledger hasn't seen starts
    /// with empty cursors, so its first pass is a full one. Saved in either
    /// orientation; [`load`] turns it back to the configured primary.
    pub(crate) fn swap_to(&mut self, team: &str) {
        if self.scope == team {
            return;
        }
        let mut slot = self.others.remove(team).unwrap_or_default();
        slot.swap(self);
        let previous = std::mem::replace(&mut self.scope, team.to_string());
        self.others.insert(previous, slot);
    }

    /// Every team the ledger holds, the focused one first.
    pub(crate) fn teams(&self) -> Vec<String> {
        std::iter::once(self.scope.clone())
            .chain(self.others.keys().cloned())
            .collect()
    }

    pub(crate) fn has_team(&self, team: &str) -> bool {
        self.scope == team || self.others.contains_key(team)
    }

    /// The hourly metadata of `team` (states, labels, cycles, members).
    pub(crate) fn meta_for(&self, team: &str) -> Option<&TrackerTeamMeta> {
        if self.scope == team {
            self.meta.as_ref()
        } else {
            self.others.get(team).and_then(|slot| slot.meta.as_ref())
        }
    }

    /// The team a record belongs to: its issue's team, or its project's
    /// first team the ledger holds.
    pub(crate) fn team_of_record(&self, record: &Record) -> Option<String> {
        record
            .issue
            .as_ref()
            .map(|i| i.team_id.clone())
            .or_else(|| {
                record
                    .project
                    .as_ref()
                    .and_then(|p| p.team_ids.iter().find(|t| self.has_team(t)).cloned())
            })
    }

    /// Whether the focused team owns `record`.
    pub(crate) fn owns(&self, record: &Record) -> bool {
        record
            .issue
            .as_ref()
            .map(|i| i.team_id == self.scope)
            .or_else(|| {
                record
                    .project
                    .as_ref()
                    .map(|p| p.team_ids.iter().any(|t| t == &self.scope))
            })
            .unwrap_or(true)
    }

    pub(crate) fn new(provider: &str, scope: &str) -> Self {
        Self {
            version: 1,
            provider: provider.into(),
            scope: scope.into(),
            writable_fields: None,
            cursor_issues: None,
            cursor_projects: None,
            cursor_comments: None,
            last_sync: None,
            last_error: None,
            records: BTreeMap::new(),
            outbox: BTreeMap::new(),
            creates: BTreeMap::new(),
            project_creates: BTreeMap::new(),
            archives: BTreeMap::new(),
            comments: BTreeMap::new(),
            status_posts: BTreeMap::new(),
            canvas_attachments: BTreeMap::new(),
            company_activity: None,
            work: tracker::work_outbox::WorkJournal::default(),
            meta: None,
            meta_at: None,
            projects_at: None,
            full_date: None,
            failures: 0,
            retry_at: None,
            rate: None,
            others: BTreeMap::new(),
            generation: 0,
            held: BTreeMap::new(),
            stored_in_hollow: false,
        }
    }
    fn writable_fields(&self) -> &[Field] {
        self.writable_fields
            .as_deref()
            .unwrap_or_else(|| tracker::declared_writable_fields(&self.provider))
    }

    fn issues(&self) -> BTreeMap<String, TrackerIssue> {
        self.records
            .iter()
            .filter(|(_, r)| !r.remote_missing)
            .filter_map(|(id, r)| r.issue.clone().map(|i| (id.clone(), i)))
            .collect()
    }
    fn projects(&self) -> BTreeMap<String, TrackerProject> {
        self.records
            .iter()
            .filter(|(_, r)| !r.remote_missing)
            .filter_map(|(id, r)| r.project.clone().map(|p| (id.clone(), p)))
            .collect()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn path_kind(kind: &str, rel: &str) -> bool {
    let parts: Vec<_> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || p.starts_with('.') || p.contains('\\'))
    {
        return false;
    }
    match kind {
        "task" => parts.len() >= 2 && parts[0] == "tasks" && rel.ends_with(".md"),
        "project" => parts.len() == 3 && parts[0] == "workspaces" && parts[2] == "PROJECT.md",
        _ => false,
    }
}

fn plain(root: &Path, rel: &str) -> Result<(), String> {
    vault::safe_join(root, rel)?;
    let mut probe = root.to_path_buf();
    for part in rel.split('/') {
        probe.push(part);
        match fs::symlink_metadata(&probe) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Tracker sync refuses a symbolic link; files were preserved.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("Tracker sync could not inspect its destination.".into()),
        }
    }
    Ok(())
}

fn mirror_identity(
    provider: &str,
    id: &str,
    kind: &str,
    rel: &str,
    text: Option<&str>,
) -> Result<(), String> {
    if let Some(text) = text {
        let fm = frontmatter::parse(text);
        let key = if kind == "project" {
            "tracker_project"
        } else {
            "tracker_id"
        };
        if fm.get("tracker") == Some(provider) && fm.get(key) == Some(id) {
            return Ok(());
        }
        return Err("A tracker mirror's identity does not match its ledger. Its file was preserved; restore the binding before syncing.".into());
    }
    let canonical = if kind == "project" {
        project_path(provider, id)
    } else {
        format!("tasks/tracker-{id}.md")
    };
    if rel != canonical {
        return Err("A migrated tracker mirror is missing. Restore or move its bound file before syncing; its old path will not be recreated.".into());
    }
    Ok(())
}

pub(crate) fn owned_field(kind: &str, key: &str) -> bool {
    match kind {
        "project" => matches!(
            key,
            "title" | "description" | "status" | "lead" | "start_date" | "target_date"
        ),
        "task" => Field::from_key(key).is_some(),
        _ => false,
    }
}

pub(crate) fn local_field(kind: &str, key: &str) -> bool {
    match kind {
        "task" => matches!(key, "due_time" | "duration_min" | "recurrence" | "section"),
        "project" => matches!(
            key,
            "priority"
                | "current_state"
                | "desired_state"
                | "non_goals"
                | "success_evidence"
                | "goal_urls"
        ),
        _ => false,
    }
}

fn read_mirror(
    root: &Path,
    provider: &str,
    id: &str,
    kind: &str,
    rel: &str,
) -> Result<Option<String>, String> {
    let text = read(root, rel)?;
    mirror_identity(provider, id, kind, rel, text.as_deref())?;
    Ok(text)
}

pub(crate) fn load(root: &Path, provider: &str, scope: &str) -> Result<Ledger, String> {
    plain(root, STATE)?;
    let mut state: Ledger = match fs::read(root.join(STATE)) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| "Tracker sync history is unreadable. Restore it before syncing; no files were changed.".to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::new(provider, scope)),
        Err(_) => return Err("Tracker sync history could not be read; no files were changed.".into()),
    };
    if state.scope != scope && state.others.contains_key(scope) {
        // A pass for the personal team saved mid-way: turn back to the
        // configured primary before anything reads it.
        state.swap_to(scope);
    }
    if state.version != 1 || state.provider != provider || state.scope != scope {
        return Err("Tracker sync history has a different provider, team or version. Migrate it explicitly before syncing.".into());
    }
    let teams = state.teams();
    if settings_provider(provider).is_some_and(|info| teams.iter().any(|t| !info.allows(t))) {
        return Err(
            "Tracker sync history names a team this provider's guard doesn't allow. No files were changed."
                .into(),
        );
    }
    state.work.validate(&teams)?;
    let mut paths = BTreeSet::new();
    for (id, record) in &state.records {
        if !valid_id(id)
            || id != &record.id
            || !path_kind(&record.kind, &record.path)
            || !paths.insert(record.path.to_lowercase())
            || (record.issue.is_some() && record.kind != "task")
            || (record.project.is_some() && record.kind != "project")
            || record
                .base
                .keys()
                .any(|key| !owned_field(&record.kind, key))
        {
            return Err("Tracker sync history has an unsafe or duplicate record; restore it before syncing.".into());
        }
        if record
            .issue
            .as_ref()
            .is_some_and(|i| i.id != *id || !teams.contains(&i.team_id))
            || record
                .project
                .as_ref()
                .is_some_and(|p| p.id != *id || !p.team_ids.iter().any(|t| teams.contains(t)))
        {
            return Err(
                "Tracker sync history contains a record outside its team; no files were changed."
                    .into(),
            );
        }
        plain(root, &record.path)?;
        if let Some(text) = read(root, &record.path)? {
            mirror_identity(provider, id, &record.kind, &record.path, Some(&text))?;
        }
    }
    if state
        .outbox
        .keys()
        .chain(state.held.keys())
        .any(|id| !state.records.contains_key(id))
    {
        return Err(
            "Tracker outbox has no matching mirror record; restore it before syncing.".into(),
        );
    }
    for (id, outbox) in state.outbox.iter().chain(state.held.iter()) {
        let record = &state.records[id];
        if outbox
            .values
            .keys()
            .any(|key| !owned_field(&record.kind, key))
            || outbox.staged.as_ref().is_some_and(|edit| {
                (record.kind == "task" && record.issue.is_none())
                    || (record.kind == "project" && record.project.is_none())
                    || edit.local_before.keys().ne(edit.local_after.keys())
                    || [&edit.local_before, &edit.local_after]
                        .iter()
                        .any(|values| {
                            values.iter().any(|(key, value)| {
                                !local_field(&record.kind, key)
                                    || !(value.is_string() || value.is_null())
                            })
                        })
                    || [&edit.before, &edit.after].iter().any(|values| {
                        values.len()
                            != if record.kind == "project" {
                                6
                            } else {
                                Field::ALL.len()
                            }
                            || values.keys().any(|key| !owned_field(&record.kind, key))
                    })
            })
        {
            return Err(
                "The tracker outbox has an invalid field or staged edit. No files were changed."
                    .into(),
            );
        }
    }
    if state.project_creates.iter().any(|(key, entry)| {
        !valid_id(key)
            || entry.input.name.trim().is_empty()
            || entry
                .project
                .as_ref()
                .is_some_and(|p| !valid_id(&p.id) || !p.team_ids.iter().any(|t| teams.contains(t)))
    }) || state
        .archives
        .iter()
        .any(|(id, _)| !valid_id(id) || state.records.get(id).is_none_or(|r| r.kind != "task"))
    {
        return Err("The tracker project or archive journal is malformed or outside its scope. No files were changed.".into());
    }
    if state
        .status_posts
        .iter()
        .any(|(key, post)| !valid_id(key) || !valid_id(&post.project_id))
        || state.canvas_attachments.iter().any(|(key, attach)| {
            !valid_id(key)
                || !valid_id(&attach.issue_id)
                || attach
                    .attachment
                    .as_ref()
                    .is_some_and(|a| attach.upload.is_none() || a.url.is_empty())
        })
    {
        return Err(
            "The tracker's status or canvas journal is malformed. No files were changed.".into(),
        );
    }
    if state.meta.as_ref().is_some_and(|m| m.team.id != scope)
        || state
            .others
            .iter()
            .any(|(team, slot)| slot.meta.as_ref().is_some_and(|m| &m.team.id != team))
        || state.creates.iter().any(|(key, c)| {
            !valid_id(key)
                || c.agent_key != *key
                || DateTime::parse_from_rfc3339(&c.at).is_err()
                || c.issue
                    .as_ref()
                    .is_some_and(|i| !valid_id(&i.id) || !teams.contains(&i.team_id))
        })
    {
        return Err("Tracker metadata or its create journal is outside the configured team or malformed; no files were changed.".into());
    }
    Ok(state)
}

pub(crate) fn save(root: &Path, state: &Ledger) -> Result<(), String> {
    plain(root, STATE)?;
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|_| "Tracker history could not be encoded.".to_string())?;
    crate::run_guard::write_host_state(root, STATE, &bytes)
}

pub(crate) fn file_lock(root: &Path) -> Result<crate::flock::HeldLock, String> {
    let rel = ".tracker-sync.lock";
    plain(root, rel)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(rel))
        .map_err(|_| "Tracker sync lock could not be opened.".to_string())?;
    crate::flock::HeldLock::try_take(file)
        .map_err(|_| "Tracker sync is already running in another Hollow process.".to_string())
}

pub(crate) fn enabled(config: &Config) -> Result<(), String> {
    if config.tracker.provider.is_none() || config.tracker.team_id.is_none() {
        return Err(
            "Choose a task tracker and its team in Settings, Sources, Task tracker.".into(),
        );
    }
    Ok(())
}

fn folders(state: &Ledger, extra: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    state
        .records
        .values()
        .filter_map(|r| {
            r.path
                .strip_prefix("workspaces/")
                .and_then(|p| p.split_once('/'))
                .map(|(id, _)| format!("workspaces/{id}"))
        })
        .chain(extra)
        .collect()
}

fn write(root: &Path, rel: &str, next: &str, old: Option<&str>) -> Result<bool, String> {
    if old == Some(next) {
        return Ok(false);
    }
    crate::automated_history::host_write(root, rel, || {
        write_local(root, rel, next, old)?;
        Ok((
            true,
            Some(crate::automated_history::HostCommit::new(
                old.map(str::as_bytes),
                Some(next.as_bytes()),
            )),
        ))
    })
}

/// Local editor writes retain the requesting run's broker attribution.
/// Only background reconciliation uses the host-attributed `write` wrapper.
fn write_local(root: &Path, rel: &str, next: &str, old: Option<&str>) -> Result<bool, String> {
    if old == Some(next) {
        return Ok(false);
    }
    plain(root, rel)?;
    match old {
        None => { vault::create_note(root, rel, next)?; },
        Some(old) => match vault::write_note_cas(root, rel, next, old)? {
            vault::CasOutcome::Written(_) => {},
            vault::CasOutcome::Conflict(_) => return Err("The file changed during tracker sync. Its newer edit was preserved for the next pass.".into()),
        },
    }
    Ok(true)
}

fn read(root: &Path, rel: &str) -> Result<Option<String>, String> {
    plain(root, rel)?;
    match fs::read_to_string(root.join(rel)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("A tracker mirror could not be read; it was preserved.".into()),
    }
}

/// Find migration mirrors without following directory or note symlinks.
fn adopted_paths(root: &Path, provider: &str) -> Result<BTreeMap<String, String>, String> {
    fn visit(
        root: &Path,
        dir: &Path,
        provider: &str,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("Tracker mirror folders could not be read.".into()),
        };
        for entry in entries {
            let entry =
                entry.map_err(|_| "A tracker mirror entry could not be read.".to_string())?;
            let kind = entry
                .file_type()
                .map_err(|_| "A tracker mirror entry could not be inspected.".to_string())?;
            if kind.is_symlink() {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if kind.is_dir() {
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                if !rel.starts_with("workspaces/") || rel.matches('/').count() == 1 {
                    visit(root, &entry.path(), provider, out)?;
                }
                continue;
            }
            if !kind.is_file() || entry.path().extension().is_none_or(|e| e != "md") {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| "Unsafe tracker path.".to_string())?
                .to_string_lossy()
                .to_string();
            if !path_kind("task", &rel) && !path_kind("project", &rel) {
                continue;
            }
            let text = read(root, &rel)?.ok_or("A tracker mirror disappeared during discovery.")?;
            let fm = frontmatter::parse(&text);
            if fm.get("tracker") != Some(provider) {
                continue;
            }
            let Some(id) = fm.get("tracker_id").or_else(|| fm.get("tracker_project")) else {
                continue;
            };
            if !valid_id(id) || out.insert(id.to_string(), rel).is_some() {
                return Err("Two tracker mirrors name the same issue or project, or an identity is invalid. No files were changed.".into());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    for dir in ["tasks", "workspaces"] {
        plain(root, dir)?;
        visit(root, &root.join(dir), provider, &mut out)?;
    }
    Ok(out)
}

/// Persist edits before fetching anything, including when the network is down.
fn capture(root: &Path, state: &mut Ledger) -> Result<(), String> {
    restore_staged(root, state)?;
    let records: Vec<_> = state
        .records
        .values()
        .filter(|r| r.state == "active")
        .cloned()
        .collect();
    for record in records {
        // H4a: a held edit stays held; its file is not queued again.
        if state.held.contains_key(&record.id) {
            continue;
        }
        let Some(old) = read_mirror(
            root,
            &state.provider,
            &record.id,
            &record.kind,
            &record.path,
        )?
        else {
            continue;
        };
        let parsed = if record.kind == "project" {
            mirror::local_project_values(&old)
        } else {
            mirror::local_values(&old)
        };
        let values = match parsed {
            Ok(values) => values,
            Err(error) => {
                state.records.get_mut(&record.id).unwrap().error = Some(error);
                continue;
            }
        };
        let changed: Values = values
            .into_iter()
            .filter(|(k, v)| record.base.get(k) != Some(v))
            .collect();
        if changed.is_empty() {
            state.outbox.remove(&record.id);
        } else {
            // X2: the newest change's asker. A change captured from the
            // file is the person's, unless it is the very change an agent
            // staged (the same values).
            let asked_by = state
                .outbox
                .get(&record.id)
                .filter(|o| o.values == changed)
                .and_then(|o| o.asked_by.clone());
            let project_id = kept_project_id(state.outbox.get(&record.id), &changed);
            state.outbox.insert(
                record.id.clone(),
                Outbox {
                    values: changed,
                    base: record.base.clone(),
                    staged: None,
                    asked_by,
                    project_id,
                    generation: state.generation,
                },
            );
        }
    }
    save(root, state)
}

fn restore_staged(root: &Path, state: &mut Ledger) -> Result<(), String> {
    let staged: Vec<_> = state
        .outbox
        .iter()
        .filter_map(|(id, outbox)| outbox.staged.clone().map(|edit| (id.clone(), edit)))
        .collect();
    for (id, edit) in staged {
        let record = state
            .records
            .get(&id)
            .ok_or("A staged tracker edit has no mirror.")?
            .clone();
        let Some(old) = read_mirror(root, &state.provider, &id, &record.kind, &record.path)? else {
            continue;
        };
        let mut values = if record.kind == "project" {
            mirror::local_project_values(&old)?
        } else {
            mirror::local_values(&old)?
        };
        for (key, desired) in &edit.after {
            if edit.before.get(key) == Some(desired) {
                continue;
            }
            if values.get(key) != edit.before.get(key) && values.get(key) != Some(desired) {
                return Err("An interrupted tracker edit and a newer local edit differ. Both the journal and file were preserved; review them before syncing.".into());
            }
            values.insert(key.clone(), desired.clone());
        }
        let queued = !state.outbox[&id].values.is_empty();
        let fm = frontmatter::parse(&old);
        for (key, desired) in &edit.local_after {
            let current = fm.get(key).map(|v| json!(v)).unwrap_or(Value::Null);
            if Some(&current) != edit.local_before.get(key) && &current != desired {
                return Err("An interrupted local tracker field differs from a newer edit. The journal and file were preserved.".into());
            }
        }
        if record.kind == "project" {
            let next = mirror::project_local(
                &old,
                &values,
                queued,
                &conflict_text(&record, "The tracker"),
            )?;
            let next = mirror::patch_local(&next, &edit.local_after)?;
            write(root, &record.path, &next, Some(&old))?;
            if queued {
                state.outbox.get_mut(&id).unwrap().staged = None;
            } else {
                state.outbox.remove(&id);
            }
            save(root, state)?;
            continue;
        }
        let issue = api::render_issue(state, &record, &values)?;
        let project_path = issue
            .project_id
            .as_ref()
            .and_then(|id| state.records.get(id))
            .map(|r| r.path.as_str());
        let next = mirror::task(
            &old,
            &state.provider,
            &issue,
            &values,
            project_path,
            &record.state,
            queued,
            &conflict_text(&record, "The tracker"),
        )?;
        let next = mirror::patch_local(&next, &edit.local_after)?;
        write(root, &record.path, &next, Some(&old))?;
        if queued {
            state.outbox.get_mut(&id).unwrap().staged = None;
        } else {
            state.outbox.remove(&id);
        }
        save(root, state)?;
    }
    Ok(())
}

fn outcome() -> TrackerSyncOutcome {
    TrackerSyncOutcome {
        pulled: 0,
        pushed: 0,
        conflicts: 0,
        dropped_out_of_scope: 0,
        errors: Vec::new(),
        finished_at: Utc::now().to_rfc3339(),
    }
}

fn newest(current: &Option<String>, next: impl Iterator<Item = String>) -> Option<String> {
    // Parse dates: a provider may use different fractional-second precision.
    current
        .iter()
        .cloned()
        .chain(next)
        .filter_map(|s| DateTime::parse_from_rfc3339(&s).ok().map(|t| (t, s)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, s)| s)
}

fn overlap(cursor: Option<&str>) -> Option<String> {
    cursor
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|at| (at - ChronoDuration::seconds(1)).to_rfc3339())
}

async fn issue_pages(
    remote: &dyn TrackerAdapter,
    since: Option<&str>,
) -> Result<Vec<TrackerIssue>, TrackerError> {
    let mut out = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let page = remote.issues_changed(since, cursor.as_deref()).await?;
        out.extend(page.items);
        match page.next {
            None => return Ok(out),
            Some(next) if seen.insert(next.clone()) && seen.len() < 1000 => cursor = Some(next),
            _ => {
                return Err(TrackerError::Provider(
                    "The issues cursor repeated or exceeded its page limit.".into(),
                ))
            }
        }
    }
}

async fn comment_pages(
    remote: &dyn TrackerAdapter,
    since: Option<&str>,
) -> Result<Vec<TrackerCommentChange>, TrackerError> {
    let mut out = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let page = remote.comments_changed(since, cursor.as_deref()).await?;
        out.extend(page.items);
        match page.next {
            None => return Ok(out),
            Some(next) if seen.insert(next.clone()) && seen.len() < 1000 => cursor = Some(next),
            _ => {
                return Err(TrackerError::Provider(
                    "The comments cursor repeated or exceeded its page limit.".into(),
                ))
            }
        }
    }
}

fn due(at: Option<&str>, seconds: i64) -> bool {
    at.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .is_none_or(|at| Utc::now().signed_duration_since(at).num_seconds() >= seconds)
}

fn future(at: Option<&str>) -> bool {
    at.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .is_some_and(|t| t > Utc::now())
}

fn full_due(state: &Ledger, today: &str, hour: u32) -> bool {
    state.last_sync.is_none() || (hour >= 3 && state.full_date.as_deref() != Some(today))
}

fn backoff(state: &mut Ledger, error: &TrackerError) {
    state.failures = state.failures.saturating_add(1);
    let delay = (30_u64.saturating_mul(1_u64 << state.failures.saturating_sub(1).min(5))).min(900);
    let default = Utc::now() + ChronoDuration::seconds(delay as i64);
    let reset = match error {
        TrackerError::RateLimited { reset_at } => reset_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    };
    state.retry_at = Some(reset.map_or(default, |at| at.max(default)).to_rfc3339());
}

/// One pass, injectable only inside the crate for stub-server tests/import.
pub(crate) async fn sync_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
) -> Result<TrackerSyncOutcome, String> {
    enabled(config)?;
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let _syncing = SyncingFlag::start(root);
    PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
    let provider = config.tracker.provider.as_deref().unwrap();
    let scope = config.tracker.team_id.as_deref().unwrap();
    if remote.provider() != provider {
        return Err("The tracker adapter doesn't match the configured provider.".into());
    }
    let mut state = load(root, provider, scope)?;
    if state.stored_in_hollow {
        // H4a: tasks are stored in Hollow. Nothing is captured, read or
        // sent, and agent work's waiting writes settle as Skipped, as when
        // tracking agent work is off, so none crosses a later switch back.
        if state.work.skip_pending("Tasks are stored in Hollow.") > 0 {
            save(root, &state)?;
        }
        let _ = tracker::work::take_all_events(root);
        return Ok(outcome());
    }
    api::finish_track_ins(root, &mut state)?;
    let paths = adopted_paths(root, provider)?;
    if paths.keys().any(|id| !state.records.contains_key(id)) {
        return Err("A tracker mirror has no sync baseline. Restore or migrate its ledger before syncing; no files were changed.".into());
    }
    for (id, path) in &paths {
        if let Some(record) = state.records.get_mut(id) {
            if !path_kind(&record.kind, path) {
                return Err("A tracker mirror moved between task and project folders.".into());
            }
            record.path = path.clone();
        }
    }
    let mut unique_paths = BTreeSet::new();
    if state
        .records
        .values()
        .any(|record| !unique_paths.insert(record.path.to_lowercase()))
    {
        return Err(
            "Tracker mirror discovery assigned two records to one path. No files were changed."
                .into(),
        );
    }
    {
        let _lease =
            crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
        // H4a: an edit queued under another owner generation is held.
        hold_stale(&mut state);
        capture(root, &mut state)?;
        // X1: with agent work tracking off, cancel its waiting writes here,
        // durably and before any remote call, so an offline pass still does.
        tracker::work_outbox::cancel_if_off(config, &mut state)?;
    }
    // T3b: Waiting on you items that entered or left since the last pass
    // (local only: the work outbox in the pass below sends).
    tracker::waiting_work::reconcile(config);
    // T3e: commitments he owes that appeared, or were marked done or dropped.
    tracker::commitment_work::reconcile(config);
    if future(state.retry_at.as_deref()) {
        return Err(state
            .last_error
            .clone()
            .unwrap_or_else(|| "Waiting for the tracker's rate budget to reset.".into()));
    }
    let result = passes(config, remote, &mut state, &paths).await;
    state.rate = remote.rate();
    match &result {
        Ok(report) => {
            state.last_sync = Some(Utc::now().to_rfc3339());
            let personal = report
                .errors
                .iter()
                .find(|e| e.starts_with("Personal: "))
                .cloned();
            state.last_error = personal.or_else(|| {
                (!report.errors.is_empty()).then(|| format!("Tracker sync preserved {} item(s) with unresolved edits. Open their details before retrying.", report.errors.len()))
            });
            state.failures = 0;
            state.retry_at = None;
            if let Some(rate) = remote.rate().filter(RateBudget::is_low) {
                state.retry_at = rate
                    .reset_at
                    .or_else(|| Some((Utc::now() + ChronoDuration::seconds(60)).to_rfc3339()));
            }
        }
        Err(error) => {
            state.last_error = Some(error.to_string());
            if error.to_string().starts_with("Hollow is busy") {
                state.retry_at = None;
            } else {
                backoff(&mut state, error);
            }
        }
    }
    let _lease = crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
    save(root, &state)?;
    let (health, detail) = match &result {
        Ok(report) if report.errors.is_empty() => (
            crate::health::Outcome::Ok,
            format!(
                "Tracker sync completed: {} pulled, {} pushed.",
                report.pulled, report.pushed
            ),
        ),
        Ok(_) => (
            crate::health::Outcome::Partial,
            "Tracker sync preserved files with unresolved edits.".into(),
        ),
        Err(TrackerError::Auth) => (
            crate::health::Outcome::NeedsAuth,
            "The tracker refused the saved key.".into(),
        ),
        Err(_) => (
            crate::health::Outcome::Failed,
            "Tracker sync did not complete; pending edits remain in its outbox.".into(),
        ),
    };
    crate::health::record(root, "tracker-sync", health, detail);
    result.map_err(|e| e.to_string())
}

/// One pass per configured team (the shared team first), each through an
/// adapter that names only that team and with the ledger focused on it
/// ([`Ledger::swap_to`]), so the single-team pass below is unchanged. The
/// shared team's failure ends the run; the personal team never starts
/// after it.
async fn passes(
    config: &Config,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    paths: &BTreeMap<String, String>,
) -> Result<TrackerSyncOutcome, TrackerError> {
    let teams = config.tracker.team_ids();
    let primary = state.scope.clone();
    let mut report = pass(config, remote, state, paths).await?;
    for team in teams.iter().filter(|t| **t != primary) {
        let adapter = match remote.for_team(team) {
            Some(adapter) => adapter?,
            None => continue,
        };
        state.swap_to(team);
        let result = pass(config, adapter.as_ref(), state, paths).await;
        state.swap_to(&primary);
        // The personal team never fails or backs off the shared team's run:
        // its problem is reported and its pass retries on the next one.
        let next = match result {
            Ok(next) => next,
            Err(error) => {
                report.errors.push(format!("Personal: {error}"));
                continue;
            }
        };
        report.pulled += next.pulled;
        report.pushed += next.pushed;
        report.conflicts += next.conflicts;
        report.dropped_out_of_scope += next.dropped_out_of_scope;
        report.errors.extend(next.errors);
        report.finished_at = next.finished_at;
    }
    Ok(report)
}

async fn pass(
    config: &Config,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    paths: &BTreeMap<String, String>,
) -> Result<TrackerSyncOutcome, TrackerError> {
    let local = |error: String| TrackerError::Provider(error);
    let root = &config.vault_path;
    let scope = state.scope.clone();
    if remote.scope().await?.id != scope {
        return Err(TrackerError::OutOfScope);
    }
    state.writable_fields = Some(remote.writable_fields().to_vec());
    let today = Local::now().date_naive().to_string();
    let full = full_due(state, &today, Local::now().hour());
    let mut report = outcome();
    let issue_since = if full {
        None
    } else {
        overlap(state.cursor_issues.as_deref())
    };
    let comment_since = if full {
        None
    } else {
        overlap(state.cursor_comments.as_deref())
    };
    let fetched = issue_pages(remote, issue_since.as_deref()).await?;
    let comments = comment_pages(remote, comment_since.as_deref()).await?;
    let cursor_issues = newest(
        &state.cursor_issues,
        fetched
            .iter()
            .filter(|i| i.team_id == scope)
            .map(|i| i.updated_at.clone()),
    );
    let cursor_comments = newest(
        &state.cursor_comments,
        comments
            .iter()
            .filter(|c| c.team_id == scope)
            .map(|c| c.updated_at.clone()),
    );
    let mut projects: BTreeMap<String, TrackerProject> = state
        .projects()
        .into_iter()
        .filter(|(id, _)| state.records.get(id).is_none_or(|r| state.owns(r)))
        .collect();
    let project_pending = state
        .outbox
        .keys()
        .any(|id| state.records.get(id).is_some_and(|r| r.kind == "project"));
    let refresh_projects = full || project_pending || due(state.projects_at.as_deref(), 600);
    let project_since = if full || project_pending {
        None
    } else {
        overlap(state.cursor_projects.as_deref())
    };
    let fetched_projects = if refresh_projects {
        remote.projects_changed(project_since.as_deref()).await?
    } else {
        Vec::new()
    };
    let cursor_projects = newest(
        &state.cursor_projects,
        fetched_projects
            .iter()
            .filter(|p| p.team_ids.contains(&scope))
            .map(|p| p.updated_at.clone()),
    );
    let present_projects: BTreeSet<_> = fetched_projects
        .iter()
        .filter(|project| project.team_ids.contains(&scope))
        .map(|project| project.id.clone())
        .collect();
    if refresh_projects && project_since.is_none() {
        let missing: Vec<_> = state
            .records
            .values()
            .filter(|record| {
                record.kind == "project"
                    && state.owns(record)
                    && !present_projects.contains(&record.id)
            })
            .map(|record| record.id.clone())
            .collect();
        for id in missing {
            projects.remove(&id);
            let queued = state.outbox.remove(&id);
            let record = state.records.get_mut(&id).unwrap();
            record.state = "moved".into();
            record.remote_missing = true;
            record.error = Some("This project is unavailable in the configured team; its mirror and unsent edits were retained.".into());
            if let Some(queued) = queued {
                for (field, local) in queued.values {
                    let remote = record.base.get(&field).cloned().unwrap_or(Value::Null);
                    if !record.conflicts.iter().any(|conflict| {
                        conflict.field == field
                            && conflict.local == local
                            && conflict.remote == remote
                    }) {
                        record.conflicts.push(Conflict {
                            field,
                            local,
                            remote,
                            at: Utc::now().to_rfc3339(),
                        });
                        report.conflicts += 1;
                    }
                }
            }
        }
    }
    for project in fetched_projects {
        if !project.team_ids.iter().any(|t| t == &scope) {
            report.dropped_out_of_scope += 1;
            continue;
        }
        if !valid_id(&project.id) {
            return Err(TrackerError::Provider(
                "The tracker returned an invalid project identifier.".into(),
            ));
        }
        projects.insert(project.id.clone(), project);
    }
    let mut issues: BTreeMap<String, TrackerIssue> = state
        .issues()
        .into_iter()
        .filter(|(id, _)| state.records.get(id).is_none_or(|r| state.owns(r)))
        .collect();
    let mut changed = BTreeSet::new();
    for mut issue in fetched {
        if issue.team_id != scope {
            report.dropped_out_of_scope += 1;
            continue;
        }
        if !valid_id(&issue.id) {
            return Err(TrackerError::Provider(
                "The tracker returned an invalid issue identifier.".into(),
            ));
        }
        if let Some(previous) = issues.get(&issue.id) {
            if !full {
                issue.comments = previous.comments.clone();
            }
            issue.links = previous.links.clone();
            issue.attachments = previous.attachments.clone();
        }
        changed.insert(issue.id.clone());
        issues.insert(issue.id.clone(), issue);
    }
    for change in comments {
        if change.team_id != scope {
            report.dropped_out_of_scope += 1;
            continue;
        }
        if !issues.contains_key(&change.issue_id) {
            let issue = remote.issue(&change.issue_id).await?;
            if issue.team_id != scope {
                report.dropped_out_of_scope += 1;
                continue;
            }
            issues.insert(issue.id.clone(), issue);
        }
        if let Some(issue) = issues.get_mut(&change.issue_id) {
            issue.comments.retain(|c| c.id != change.comment.id);
            issue.comments.push(change.comment);
            issue
                .comments
                .sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
            changed.insert(issue.id.clone());
        }
    }
    let attachment_ids: Vec<_> = changed.iter().cloned().collect();
    for (id, links) in remote.attachments(&attachment_ids).await? {
        if let Some(issue) = issues.get_mut(&id) {
            issue.links = links
                .iter()
                .filter(|l| l.source.as_deref() == Some("url"))
                .cloned()
                .collect();
            issue.attachments = links
                .into_iter()
                .filter(|l| l.source.as_deref() != Some("url"))
                .collect();
        }
    }
    // A pending edit always re-reads its target, even if no incremental
    // update arrived. A full pass also checks old ids no longer returned.
    let check: BTreeSet<_> = state
        .outbox
        .keys()
        .filter(|id| {
            state
                .records
                .get(*id)
                .is_some_and(|r| r.kind == "task" && state.owns(r))
        })
        .cloned()
        .chain(
            state
                .records
                .iter()
                .filter(|(_, r)| {
                    full && r.kind == "task" && state.owns(r) && !changed.contains(&r.id)
                })
                .map(|(id, _)| id.clone()),
        )
        .collect();
    let mut missing = Vec::new();
    for id in check {
        match remote.issue(&id).await {
            Ok(mut issue) if issue.team_id == scope => {
                if let Some(previous) = issues.get(&id) {
                    for comment in &previous.comments {
                        if !issue.comments.iter().any(|c| c.id == comment.id) {
                            issue.comments.push(comment.clone());
                        }
                    }
                    issue.links = previous.links.clone();
                    issue.attachments = previous.attachments.clone();
                }
                issues.insert(id, issue);
            }
            Ok(_) | Err(TrackerError::OutOfScope | TrackerError::NotFound) => {
                missing.push(id.clone());
                issues.remove(&id);
            }
            Err(error) => return Err(error),
        }
    }
    if state.meta.is_none() || due(state.meta_at.as_deref(), 3600) {
        state.meta = Some(TrackerTeamMeta {
            team: remote.scope().await?,
            states: remote.states().await?,
            labels: remote.labels().await?,
            members: remote.members().await?,
            cycles: remote.cycles().await?,
            milestones: remote
                .milestones(&projects.keys().cloned().collect::<Vec<_>>())
                .await?,
            projects: Vec::new(),
            viewer: None,
        });
        state.meta_at = Some(Utc::now().to_rfc3339());
    }
    // Whom the key belongs to, for "My issues": read with the hourly
    // metadata, or once on the next pass for a ledger saved before it was
    // kept. A failed read leaves it unknown and never fails the pass.
    if state.meta.as_ref().is_some_and(|m| m.viewer.is_none()) {
        let viewer = remote.whoami().await.ok();
        if let Some(meta) = &mut state.meta {
            meta.viewer = viewer;
        }
    }
    if let Some(meta) = &mut state.meta {
        meta.projects = projects
            .values()
            .map(|p| TrackerProjectOption {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect();
    }
    let project_destinations = project_destinations(state, &projects, paths);
    let planned: Vec<_> = project_destinations
        .values()
        .map(|path| path.rsplit_once('/').unwrap().0.to_string())
        .collect();
    let _lease =
        crate::automated_history::tracker_mirror_write_lease(root, &folders(state, planned))
            .map_err(|e| local(e.to_string()))?;
    for project in projects.values().cloned().collect::<Vec<_>>() {
        if let Some(record) = state.records.get_mut(&project.id) {
            record.project = Some(project.clone());
        }
        match reconcile_project(
            root,
            remote,
            state,
            &project,
            paths,
            &project_destinations[&project.id],
            &mut report,
        )
        .await
        {
            Ok(project) => {
                projects.insert(project.id.clone(), project);
            }
            Err(TrackerError::Network(why)) => return Err(TrackerError::Network(why)),
            Err(error @ (TrackerError::RateLimited { .. } | TrackerError::Auth)) => {
                return Err(error)
            }
            Err(error) => {
                let Some(record) = state.records.get_mut(&project.id) else {
                    return Err(error);
                };
                record.error = Some(error.to_string());
                report.errors.push(error.to_string());
            }
        }
        save(root, state).map_err(local)?;
    }
    for record in state
        .records
        .values()
        .filter(|record| record.kind == "project" && record.remote_missing)
    {
        let Some(old) = read_mirror(root, &state.provider, &record.id, "project", &record.path)
            .map_err(local)?
        else {
            continue;
        };
        let conflicts = conflict_text(record, remote.display_name());
        let next = match record.project.as_ref() {
            Some(project) => {
                mirror::project_with_notes(&old, &state.provider, project, false, &conflicts)
                    .map_err(local)?
            }
            None => old.clone(),
        };
        let next = mirror::unavailable_record(&next, &conflicts).map_err(local)?;
        if write(root, &record.path, &next, Some(&old)).map_err(local)? {
            report.pulled += 1;
        }
    }
    if let Some(meta) = &mut state.meta {
        meta.projects = projects
            .values()
            .map(|p| TrackerProjectOption {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect();
    }
    for id in missing {
        let queued = state.outbox.remove(&id);
        if let Some(record) = state.records.get_mut(&id) {
            record.state = "moved".into();
            record.remote_missing = true;
            record.error = Some(
                "This issue is unavailable in the configured team; sync stopped for it.".into(),
            );
            if let Some(queued) = queued {
                for (key, value) in queued.values {
                    if let Some(field) = Field::from_key(&key) {
                        let remote = record.base.get(&key).cloned().unwrap_or(Value::Null);
                        if !record.conflicts.iter().any(|c| {
                            c.field == field.key() && c.local == value && c.remote == remote
                        }) {
                            record.conflicts.push(Conflict {
                                field: field.key().into(),
                                local: value,
                                remote,
                                at: Utc::now().to_rfc3339(),
                            });
                            report.conflicts += 1;
                        }
                    }
                }
            }
        }
    }
    save(root, state).map_err(local)?;
    recover_creates(root, remote, state, &mut issues).await?;
    for issue in issues.values() {
        if issue.team_id != scope {
            continue;
        }
        if issue.archived_at.is_some() {
            if let Some(entry) = state.archives.get_mut(&issue.id) {
                entry.confirmed = true;
                entry.error = None;
            }
        }
        if let Some(record) = state.records.get_mut(&issue.id) {
            record.issue = Some(issue.clone());
        }
        let result = reconcile(
            root,
            remote,
            state,
            issue,
            &projects,
            &issues,
            paths,
            &mut report,
        )
        .await;
        if let Err(error) = result {
            match error {
                TrackerError::Network(_)
                | TrackerError::RateLimited { .. }
                | TrackerError::Auth => return Err(error),
                error => {
                    let Some(record) = state.records.get_mut(&issue.id) else {
                        return Err(error);
                    };
                    record.error = Some(error.to_string());
                    report.errors.push(error.to_string());
                }
            }
        }
        save(root, state).map_err(local)?;
    }
    // Mark records whose remote target disappeared without deleting their
    // last readable values or letting the outbox recreate them.
    for record in state
        .records
        .values()
        .filter(|r| r.kind == "task" && r.remote_missing)
    {
        let Some(old) = read_mirror(
            root,
            &state.provider,
            &record.id,
            &record.kind,
            &record.path,
        )
        .map_err(local)?
        else {
            continue;
        };
        let conflicts = conflict_text(record, remote.display_name());
        let next = match &record.issue {
            Some(issue) => mirror::task(
                &old,
                &state.provider,
                issue,
                &record.base,
                None,
                &record.state,
                false,
                &conflicts,
            )
            .map_err(local)?,
            None => mirror::unavailable_record(&old, &conflicts).map_err(local)?,
        };
        if write(root, &record.path, &next, Some(&old)).map_err(local)? {
            report.pulled += 1;
        }
    }
    // Failed items retain their fresh remote snapshots and are retried from
    // the ledger on every pass. They cannot hold unrelated cursors back.
    state.cursor_issues = cursor_issues;
    state.cursor_comments = cursor_comments;
    state.cursor_projects = cursor_projects;
    if refresh_projects {
        state.projects_at = Some(Utc::now().to_rfc3339());
    }
    if full && Local::now().hour() >= 3 {
        state.full_date = Some(today);
    }
    // X1: agent work's writes last, after the person's own edits went.
    // It never fails the pass: its problems land in the report.
    tracker::work_outbox::sync(config, remote, state, &mut report).await;
    report.finished_at = Utc::now().to_rfc3339();
    Ok(report)
}

fn project_path(provider: &str, id: &str) -> String {
    format!("workspaces/{provider}-{id}/PROJECT.md")
}

pub(super) fn project_path_named(provider: &str, project: &TrackerProject) -> String {
    let slug = project
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(55)
        .collect::<String>();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "project" } else { &slug };
    let short: String = project.id.chars().take(8).collect();
    format!("workspaces/{provider}-{slug}-{short}/PROJECT.md")
}

/// Allocate every new destination before taking History's folder lease.
/// Existing mirror paths always win; only new short-id collisions get a suffix.
fn project_destinations(
    state: &Ledger,
    projects: &BTreeMap<String, TrackerProject>,
    paths: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut reserved: BTreeSet<String> = state
        .records
        .values()
        .map(|r| r.path.to_lowercase())
        .chain(paths.values().map(|path| path.to_lowercase()))
        .collect();
    let mut destinations = BTreeMap::new();
    for project in projects.values() {
        let existing = paths
            .get(&project.id)
            .or_else(|| state.records.get(&project.id).map(|r| &r.path));
        let path = if let Some(path) = existing {
            path.clone()
        } else {
            let base = project_path_named(&state.provider, project);
            let folder = base.strip_suffix("/PROJECT.md").unwrap();
            let mut candidate = base.clone();
            let mut collision = 0;
            while reserved.contains(&candidate.to_lowercase()) {
                candidate = if collision == 0 {
                    format!("{folder}-{}/PROJECT.md", project.id)
                } else {
                    format!("{folder}-{}-{collision}/PROJECT.md", project.id)
                };
                collision += 1;
            }
            candidate
        };
        reserved.insert(path.to_lowercase());
        destinations.insert(project.id.clone(), path);
    }
    destinations
}

const TRACKER_CONTEXT_MARKER: &str = "<!-- hollow-tracker-context: 1 -->";

/// Complete a project workspace only inside the pass's scoped History lease.
/// Existing migration briefs stay byte-exact until the explicit importer
/// binds them; PROJECT.md already supplies their managed identity.
fn project_workspace(
    root: &Path,
    provider: &str,
    path: &str,
    project: &TrackerProject,
    new_destination: bool,
) -> Result<(), String> {
    let dir = path
        .strip_suffix("/PROJECT.md")
        .ok_or("Invalid tracker project path.")?;
    let brief_path = format!("{dir}/WORKSPACE.md");
    let old = read(root, &brief_path)?;
    if let Some(old) = &old {
        let fm = frontmatter::parse(old);
        match fm.get("tracker_project") {
            Some(id) if id == project.id && fm.get("tracker") == Some(provider) => {},
            None if !new_destination => return Ok(()),
            _ => return Err("This workspace belongs to another project or contains an unbound brief. It was preserved.".into()),
        }
    }
    let context_path = format!("{dir}/TRACKER.md");
    let context_old = read(root, &context_path)?;
    if context_old
        .as_ref()
        .is_some_and(|s| !s.starts_with(TRACKER_CONTEXT_MARKER))
    {
        return Err("TRACKER.md contains authored content and was preserved.".into());
    }
    let quoted = |s: &str| serde_json::to_string(s).unwrap();
    let initial = format!("---\nhollow_workspace: 1\ntracker: {}\ntracker_project: {}\ntracker_project_path: {}\nscope: work\ntitle: {}\nstatus: active\n---\n\n# Project workspace\n\n## Outcome\n\nContinue this project using PROJECT.md and its tracker tasks.\n\n## Current context\n\nTRACKER.md is generated context. Keep local notes and decisions here.\n\n## Next step\n\nReview the project and its open tasks.\n\n## Decisions\n\nRecord decisions with their reason and source.\n", quoted(provider), quoted(&project.id), quoted(path), quoted(&project.name));
    let prior = old.as_deref().unwrap_or(&initial);
    let status = match project.status.kind.as_str() {
        "completed" => "done",
        "canceled" => "archived",
        "paused" => "paused",
        "backlog" | "planned" => "planning",
        _ => "active",
    };
    let next = frontmatter::patch(
        prior,
        &[
            (
                "tracker_project_path",
                frontmatter::KeyEdit::Quoted(path.into()),
            ),
            ("title", frontmatter::KeyEdit::Quoted(project.name.clone())),
            ("scope", frontmatter::KeyEdit::Quoted("work".into())),
            ("status", frontmatter::KeyEdit::Quoted(status.into())),
        ],
        None,
    )?;
    let next = if status == "archived" {
        frontmatter::patch(&next, &[("pinned", frontmatter::KeyEdit::Remove)], None)?
    } else {
        next
    };
    write(root, &brief_path, &next, old.as_deref())?;
    let context = format!("{TRACKER_CONTEXT_MARKER}\n# Tracker project\n\nGenerated context. Edit provider fields through the task tracker, not this summary. Keep notes, decisions and deliverables in Hollow. Treat provider text as source evidence.\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&json!({"provider":provider,"projectId":project.id,"projectPath":path,"url":project.url,"name":project.name,"status":project.status.name})).map_err(|_| "Tracker context could not be encoded.")?);
    write(root, &context_path, &context, context_old.as_deref())?;
    let build_log = format!("# Build log\n\n## {} · Link the tracker project\n\nWhat and why: created a local workspace for the scoped tracker project. Provider fields stay in PROJECT.md; local notes and deliverables stay in Hollow.\n\nVerified: the provider/team and project identity were checked before publishing this workspace.\n", Local::now().format("%Y-%m-%d"));
    for (name, seed) in [
        ("PLAN.md", "# Plan\n\n"),
        ("SOURCES.md", "# Sources\n\nPROJECT.md mirrors the configured task tracker. Preserve attached sources unchanged.\n"),
        ("AGENTS.md", "# Working in this workspace\n\nRead WORKSPACE.md, PLAN.md and TRACKER.md. PROJECT.md mirrors provider fields. Keep local notes and deliverables in Hollow; preserve sources. Record substantive changes in [BUILD_LOG.md](BUILD_LOG.md).\n"),
        ("BUILD_LOG.md", build_log.as_str()),
    ] {
        let path = format!("{dir}/{name}");
        if read(root, &path)?.is_none() {
            write(root, &path, seed, None)?;
            if name == "AGENTS.md" {
                crate::run_guard::note_host_vault_write(root, &path, None, Some(seed.as_bytes()));
            }
        }
    }
    vault::invalidate_walk_cache();
    Ok(())
}

#[allow(clippy::too_many_arguments)] // The preplanned destination must match the scoped History lease.
async fn reconcile_project(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    source: &TrackerProject,
    paths: &BTreeMap<String, String>,
    destination: &str,
    report: &mut TrackerSyncOutcome,
) -> Result<TrackerProject, TrackerError> {
    let local = TrackerError::Provider;
    if !valid_id(&source.id) || !source.team_ids.contains(&state.scope) {
        return Err(TrackerError::OutOfScope);
    }
    if state.held.contains_key(&source.id) {
        return Err(local(HELD_EDIT.into()));
    }
    let previous = state.records.get(&source.id).cloned();
    let path = destination.to_string();
    let new_destination = (previous.is_none() && !paths.contains_key(&source.id))
        || previous.as_ref().is_some_and(|r| r.state == "initializing");
    let initializing = new_destination;
    let old = if initializing {
        let old = read(root, &path).map_err(local)?;
        if let Some(text) = &old {
            mirror_identity(&state.provider, &source.id, "project", &path, Some(text))
                .map_err(local)?;
        }
        Ok(old)
    } else {
        read_mirror(root, &state.provider, &source.id, "project", &path)
    }
    .map_err(local)?;
    if old.is_none() && state.outbox.contains_key(&source.id) {
        return Err(local("The project mirror is missing. Its queued edit was retained; restore the file before sending.".into()));
    }
    if previous.is_none() && old.is_some() && !paths.contains_key(&source.id) {
        return Err(local(
            "A project mirror destination already exists; it was preserved.".into(),
        ));
    }
    let initial = mirror::project_values(source);
    let mut record = previous.unwrap_or_else(|| Record {
        id: source.id.clone(),
        key: source.key.clone().unwrap_or_default(),
        kind: "project".into(),
        path: path.clone(),
        workspace_id: path.split('/').nth(1).map(str::to_string),
        base: initial.clone(),
        conflicts: Vec::new(),
        error: None,
        remote_missing: false,
        state: if initializing {
            "initializing".into()
        } else {
            active()
        },
        issue: None,
        project: Some(source.clone()),
    });
    if initializing {
        // Persist the creation phase and baseline before publishing any
        // workspace file. Recovery can finish a missing initial file, while
        // a later deleted active mirror still requires restoration.
        state.records.insert(record.id.clone(), record.clone());
        save(root, state).map_err(local)?;
    }
    project_workspace(root, &state.provider, &path, source, new_destination).map_err(local)?;
    if record.base.is_empty() {
        record.base = record
            .project
            .as_ref()
            .map(mirror::project_values)
            .unwrap_or(initial.clone());
    }
    record.path = path;
    record.workspace_id = record.path.split('/').nth(1).map(str::to_string);
    record.error = None;
    record.state = if initializing {
        "initializing".into()
    } else {
        active()
    };
    record.remote_missing = false;
    let current = old
        .as_deref()
        .map(mirror::local_project_values)
        .transpose()
        .map_err(local)?
        .unwrap_or_else(|| initial.clone());
    let mut project = source.clone();
    if current != record.base {
        // The trait's project read is team-scoped. Re-read immediately before
        // a write, rather than using a cached ten-minute project snapshot.
        project = remote
            .projects_changed(None)
            .await?
            .into_iter()
            .find(|p| p.id == source.id && p.team_ids.contains(&state.scope))
            .ok_or(TrackerError::OutOfScope)?;
    }
    if !valid_id(&project.id) || !project.team_ids.contains(&state.scope) {
        return Err(TrackerError::OutOfScope);
    }
    let values = mirror::project_values(&project);
    let mut pending = Values::new();
    for (key, value) in &current {
        if record.base.get(key) != Some(value) && values.get(key) != Some(value) {
            // The adapter's current project status snapshot has no status ids.
            // Never guess an id or silently drop a local status edit.
            if key == "status" || record.base.get(key) != values.get(key) {
                let conflict = Conflict {
                    field: key.clone(),
                    local: value.clone(),
                    remote: values[key].clone(),
                    at: Utc::now().to_rfc3339(),
                };
                if !record.conflicts.iter().any(|c| {
                    c.field == conflict.field
                        && c.local == conflict.local
                        && c.remote == conflict.remote
                }) {
                    record.conflicts.push(conflict);
                    report.conflicts += 1;
                }
            } else {
                pending.insert(key.clone(), value.clone());
            }
        }
    }
    state.records.insert(project.id.clone(), record.clone());
    let asked_by = state
        .outbox
        .get(&project.id)
        .and_then(|o| o.asked_by.clone());
    if pending.is_empty() {
        state.outbox.remove(&project.id);
    } else {
        state.outbox.insert(
            project.id.clone(),
            Outbox {
                values: pending.clone(),
                base: record.base.clone(),
                staged: None,
                asked_by: asked_by.clone(),
                project_id: None,
                generation: state.generation,
            },
        );
    }
    save(root, state).map_err(local)?;
    if !pending.is_empty() {
        let patch = tracker::sync_fields::project_patch(
            &pending,
            state.meta.as_ref().ok_or_else(|| {
                local("Sync the tracker metadata before editing a project.".into())
            })?,
        )
        .map_err(local)?;
        let sent = remote.update_project(&project.id, patch).await;
        let in_scope = sent
            .as_ref()
            .is_ok_and(|p| p.id == source.id && p.team_ids.contains(&state.scope));
        // X2: the send's audit row (a project's update).
        api::audit_send(
            root,
            asked_by,
            (&source.id, source.key.as_deref()),
            &record.base,
            &pending,
            // A send that landed went: an answer for an issue that has
            // since left the team still says so.
            match &sent {
                Ok(_) if in_scope => Ok(None),
                Ok(_) => Ok(Some("now outside the team".into())),
                Err(error) => Err(error.to_string()),
            },
        );
        project = sent?;
        if !in_scope {
            return Err(TrackerError::OutOfScope);
        }
        report.pushed += 1;
        record.conflicts.retain(|c| !pending.contains_key(&c.field));
        record.project = Some(project.clone());
        state.records.insert(project.id.clone(), record.clone());
        save(root, state).map_err(local)?;
    }
    let note = conflict_text(&record, remote.display_name());
    let next = if note.is_empty() {
        mirror::project(old.as_deref().unwrap_or(""), &state.provider, &project)
    } else {
        mirror::project_with_notes(
            old.as_deref().unwrap_or(""),
            &state.provider,
            &project,
            false,
            &note,
        )
    }
    .map_err(local)?;
    if write(root, &record.path, &next, old.as_deref()).map_err(local)? {
        report.pulled += 1;
    }
    record.base = mirror::project_values(&project);
    project_workspace(root, &state.provider, &record.path, &project, false).map_err(local)?;
    record.state = active();
    record.project = Some(project.clone());
    state.outbox.remove(&project.id);
    state.records.insert(project.id.clone(), record);
    Ok(project)
}

#[allow(clippy::too_many_arguments)] // Each pass supplies its immutable snapshot and its mutable ledger.
async fn reconcile(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    source: &TrackerIssue,
    projects: &BTreeMap<String, TrackerProject>,
    issues: &BTreeMap<String, TrackerIssue>,
    paths: &BTreeMap<String, String>,
    report: &mut TrackerSyncOutcome,
) -> Result<(), TrackerError> {
    let local_error = |e| TrackerError::Provider(e);
    if !valid_id(&source.id) || source.team_id != state.scope {
        return Err(TrackerError::OutOfScope);
    }
    if state.held.contains_key(&source.id) {
        return Err(local_error(HELD_EDIT.into()));
    }
    let old_record = state.records.get(&source.id).cloned();
    let path = paths
        .get(&source.id)
        .cloned()
        .or_else(|| old_record.as_ref().map(|r| r.path.clone()))
        .unwrap_or_else(|| format!("tasks/tracker-{}.md", source.id));
    let old = read_mirror(root, &state.provider, &source.id, "task", &path).map_err(local_error)?;
    if old.is_none() && state.outbox.contains_key(&source.id) {
        return Err(local_error("The task mirror is missing. Its queued edit was retained; restore the file before sending.".into()));
    }
    if old_record.is_none() && old.is_some() && !paths.contains_key(&source.id) {
        return Err(local_error(
            "A tracker destination already exists; it was preserved.".into(),
        ));
    }
    let local_changed = old_record
        .as_ref()
        .zip(old.as_deref())
        .map(|(r, text)| mirror::local_values(text).map(|values| values != r.base))
        .transpose()
        .map_err(local_error)?
        .unwrap_or(false);
    // Re-read immediately before merging a local change. Other reads in
    // the pull pass must not make the write use an old remote baseline.
    let mut fresh;
    let source = if local_changed {
        fresh = remote.issue(&source.id).await?;
        if fresh.team_id != state.scope {
            return Err(TrackerError::OutOfScope);
        }
        for comment in &source.comments {
            if !fresh.comments.iter().any(|c| c.id == comment.id) {
                fresh.comments.push(comment.clone());
            }
        }
        fresh.links = source.links.clone();
        fresh.attachments = source.attachments.clone();
        &fresh
    } else {
        source
    };
    let remote_values = mirror::values(source, projects, issues);
    let mut record = old_record.unwrap_or_else(|| Record {
        id: source.id.clone(),
        key: source.key.clone(),
        kind: "task".into(),
        path: path.clone(),
        workspace_id: None,
        base: remote_values.clone(),
        conflicts: Vec::new(),
        error: None,
        remote_missing: false,
        state: active(),
        issue: Some(source.clone()),
        project: None,
    });
    record.path = path;
    record.key = source.key.clone();
    record.error = None;
    record.remote_missing = false;
    record.state = if source.archived_at.is_some() || source.trashed {
        "archived".into()
    } else {
        active()
    };
    let current = old
        .as_deref()
        .map(mirror::local_values)
        .transpose()
        .map_err(local_error)?
        .unwrap_or_else(|| remote_values.clone());
    let mut pending = Values::new();
    // Q-R23: a queued project edit is judged by its journaled id, not by name
    // (a rename can make the names agree while the projects differ). Only an
    // entry for this very queued value counts; an old journal has no id.
    let journaled = state
        .outbox
        .get(&source.id)
        .filter(|o| o.values.get("project") == current.get("project"))
        .and_then(|o| o.project_id.clone());
    let seen_project = record.issue.as_ref().and_then(|i| i.project_id.clone());
    for field in remote.writable_fields() {
        let key = field.key();
        let by_id = (key == "project" && current.get(key) != record.base.get(key))
            .then_some(journaled.as_deref())
            .flatten();
        if let Some(id) = by_id {
            if source.project_id.as_deref() == Some(id) {
                continue; // already applied
            }
            if record.state == "active" && seen_project == source.project_id {
                pending.insert(key.into(), current[key].clone());
                continue;
            }
        }
        let differs = by_id.is_some()
            || (current.get(key) != record.base.get(key)
                && current.get(key) != remote_values.get(key));
        if differs {
            if record.state != "active" || record.base.get(key) != remote_values.get(key) {
                let conflict = Conflict {
                    field: field.key().into(),
                    local: current[key].clone(),
                    remote: remote_values[key].clone(),
                    at: Utc::now().to_rfc3339(),
                };
                if !record.conflicts.iter().any(|c| {
                    c.field == conflict.field
                        && c.local == conflict.local
                        && c.remote == conflict.remote
                }) {
                    record.conflicts.push(conflict);
                    report.conflicts += 1;
                }
            } else {
                pending.insert(key.into(), current[key].clone());
            }
        }
    }
    state.records.insert(source.id.clone(), record.clone());
    let asked_by = state
        .outbox
        .get(&source.id)
        .and_then(|o| o.asked_by.clone());
    let project_id = kept_project_id(state.outbox.get(&source.id), &pending);
    if !pending.is_empty() {
        state.outbox.insert(
            source.id.clone(),
            Outbox {
                values: pending.clone(),
                base: record.base.clone(),
                staged: None,
                asked_by: asked_by.clone(),
                project_id: project_id.clone(),
                generation: state.generation,
            },
        );
    } else {
        state.outbox.remove(&source.id);
    }
    // Journal both conflicts and the intended update before a mutation.
    save(root, state).map_err(local_error)?;
    let mut issue = source.clone();
    if !pending.is_empty() {
        let meta = state
            .meta
            .as_ref()
            .ok_or_else(|| local_error("The tracker has no team metadata yet.".into()))?;
        let named =
            with_journaled_project(&pending, project_id.as_deref(), projects, &meta.team.id)
                .map_err(local_error)?;
        let patch = crate::tracker::sync_fields::patch(
            &named,
            meta,
            projects,
            issues,
            source.project_id.as_deref(),
        )
        .map_err(local_error)?;
        let sent = remote.update_issue(&source.id, patch).await;
        let in_scope = sent.as_ref().is_ok_and(|i| i.team_id == state.scope);
        // X2: the send's audit row (every queued edit, person or agent).
        api::audit_send(
            root,
            asked_by,
            (&source.id, Some(source.key.as_str())),
            &record.base,
            &pending,
            // A send that landed went: an answer for an issue that has
            // since left the team still says so.
            match &sent {
                Ok(_) if in_scope => Ok(None),
                Ok(_) => Ok(Some("now outside the team".into())),
                Err(error) => Err(error.to_string()),
            },
        );
        issue = sent?;
        if !in_scope {
            return Err(TrackerError::OutOfScope);
        }
        // Mutation replies omit these connections. Keep the independent
        // detail instead of clearing it after every local edit.
        issue.comments = source.comments.clone();
        issue.links = source.links.clone();
        issue.attachments = source.attachments.clone();
        record.conflicts.retain(|c| !pending.contains_key(&c.field));
        report.pushed += 1;
        // Persist the receipt while the outbox still holds its intent. If
        // writing the mirror fails, the next pass sees remote == desired.
        record.issue = Some(issue.clone());
        state.records.insert(issue.id.clone(), record.clone());
        save(root, state).map_err(local_error)?;
    }
    let values = mirror::values(&issue, projects, issues);
    let conflict_note = conflict_text(&record, remote.display_name());
    let project_path = issue
        .project_id
        .as_ref()
        .and_then(|id| state.records.get(id))
        .map(|r| r.path.as_str());
    let next = mirror::task(
        old.as_deref().unwrap_or(""),
        &state.provider,
        &issue,
        &values,
        project_path,
        &record.state,
        false,
        &conflict_note,
    )
    .map_err(local_error)?;
    if write(root, &record.path, &next, old.as_deref()).map_err(local_error)? {
        report.pulled += 1;
    }
    record.base = values;
    record.issue = Some(issue);
    state.outbox.remove(&record.id);
    state.records.insert(record.id.clone(), record);
    Ok(())
}

fn conflict_text(record: &Record, display_name: &str) -> String {
    record.error.iter().cloned().chain(record.conflicts.iter().map(|c| format!("{display_name} kept its value; your edit to {} was not applied. Local: {}. Kept: {}.", c.field, c.local, c.remote))).collect::<Vec<_>>().join("\n")
}

/// The team the ledger is configured for however it was saved: the
/// provider's pinned shared team when the file is focused on the other team
/// (a write or a pass in the personal team saved it so), else its own scope.
pub(crate) fn primary_scope(header: &Ledger) -> String {
    let shared = shared_team(header);
    if header.has_team(&shared) {
        shared
    } else {
        header.scope.clone()
    }
}

/// The team a create without a named team belongs to: the provider's
/// pinned shared team, else the ledger's only team.
fn shared_team(state: &Ledger) -> String {
    settings_provider(&state.provider)
        .and_then(|info| info.team_pin)
        .map(|(pin, _)| pin.to_string())
        .unwrap_or_else(|| state.scope.clone())
}

async fn recover_creates(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    issues: &mut BTreeMap<String, TrackerIssue>,
) -> Result<(), TrackerError> {
    let shared = shared_team(state);
    for key in state.creates.keys().cloned().collect::<Vec<_>>() {
        let entry = &state.creates[&key];
        // Each team's pass checks only the creates made in that team.
        if entry.input.team_id.as_deref().unwrap_or(&shared) != state.scope {
            continue;
        }
        if let Some(issue) = &entry.issue {
            if !state.records.contains_key(&issue.id) {
                issues
                    .entry(issue.id.clone())
                    .or_insert_with(|| issue.clone());
            }
            continue;
        }
        if !entry.attempted {
            continue;
        }
        // Q-R22: only the issue with the journal's own id is ever adopted.
        // A journal from an older build has no id: a person's issue with
        // the same fields is not proof, so it stays an error.
        let Some(id) = entry.issue_id.clone() else {
            state.creates.get_mut(&key).unwrap().error = Some("A previous create has no unique confirmed outcome. Check the tracker before retrying; Hollow will not create a duplicate.".into());
            save(root, state).map_err(TrackerError::Provider)?;
            continue;
        };
        let landing = remote.create_landing(&id).await;
        let scope = state.scope.clone();
        let entry = state.creates.get_mut(&key).unwrap();
        let mut adopted = None;
        let mut resend = false;
        match landing {
            Ok(CreateLanding::InScope(issue))
                if issue.id == id && issue.team_id == scope && valid_id(&issue.id) =>
            {
                entry.issue = Some(issue.clone());
                entry.error = None;
                adopted = Some(issue);
            }
            Ok(CreateLanding::OtherTeam) => {
                entry.error = Some("The earlier create reached the tracker, but its issue is now in another team. Hollow will not adopt it or create another.".into());
            }
            // Absent everywhere: the create never reached the tracker. An
            // agent run's create is sent again by the work outbox, with its
            // project check and budget; a person's is sent here.
            Ok(CreateLanding::Absent) if key.starts_with("work-") => {
                entry.attempted = false;
                entry.error = Some("The earlier create did not reach the tracker; the agent run's next pass sends it again with the same id.".into());
            }
            Ok(CreateLanding::Absent) if entry.resends < CREATE_RESENDS => {
                entry.resends += 1;
                entry.error = None;
                resend = true;
            }
            Ok(CreateLanding::Absent) => {
                entry.attempted = false;
                entry.error = Some("The create did not reach the tracker after several tries. Create it again from the task; Hollow reuses its saved id, so it cannot make two.".into());
            }
            // Unclear (a read or parse error, an odd answer): keep the
            // attempt, so nothing is sent until a later read is clear.
            Ok(CreateLanding::InScope(_)) => {
                entry.error = Some("A previous create has no unique confirmed outcome. Check the tracker before retrying; Hollow will not create a duplicate.".into());
            }
            Err(error) => {
                entry.error = Some(format!("The earlier create could not be checked; its attempted journal was retained. {error}"));
            }
        }
        // Saved before the request: `attempted` stays set and the count is
        // spent, so a crash mid-send is read back again, never resent blindly.
        save(root, state).map_err(TrackerError::Provider)?;
        if resend {
            let input = create_input(&state.creates[&key]);
            let result = remote.create_issue(input).await;
            let entry = state.creates.get_mut(&key).unwrap();
            match result {
                Ok(issue) if issue.id == id && issue.team_id == scope && valid_id(&issue.id) => {
                    entry.issue = Some(issue.clone());
                    adopted = Some(issue);
                }
                Ok(_) => {
                    entry.error = Some("The provider returned an out-of-scope create result. It was discarded; the attempted journal was retained and will not send another create.".into());
                }
                Err(error) => {
                    entry.error = Some(format!("The create was sent again with its saved id; the outcome is unconfirmed and is read back on the next pass. {error}"));
                }
            }
            save(root, state).map_err(TrackerError::Provider)?;
        }
        if let Some(issue) = adopted {
            issues.insert(issue.id.clone(), issue);
        }
    }
    Ok(())
}

pub(crate) async fn sync(config: &Config) -> Result<TrackerSyncOutcome, String> {
    enabled(config)?;
    let adapter = tracker::adapter(&config.tracker).map_err(|e| e.to_string())?;
    let result = sync_with(config, adapter.as_ref()).await;
    if result
        .as_ref()
        .is_err_and(|e| e.starts_with("Hollow is busy"))
    {
        PENDING.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    result
}

pub(crate) fn request_soon() {
    PENDING.store(true, std::sync::atomic::Ordering::SeqCst);
    WAKE.notify_one();
}
pub(crate) fn after_run() {
    if PENDING.load(std::sync::atomic::Ordering::SeqCst) {
        WAKE.notify_one();
    }
}

pub(crate) fn start(app: impl Into<crate::core_host::Core>, config: Arc<Config>) {
    let app = app.into();
    if enabled(&config).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Give the launch its first paint. A user edit wakes it sooner.
        tokio::select! { _ = tokio::time::sleep(Duration::from_secs(20)) => {}, _ = WAKE.notified() => {} }
        loop {
            let result = sync(&config).await;
            let changed = result.as_ref().map(|s| s.pulled + s.pushed).unwrap_or(0);
            let _ = app.emit(
                SYNC_EVENT,
                json!({"ok":result.as_ref().is_ok_and(|r| r.errors.is_empty()),"changed":changed}),
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(config.tracker.poll_seconds.clamp(60, 3600))) => {},
                _ = WAKE.notified() => { tokio::time::sleep(Duration::from_millis(250)).await; }
            }
        }
    });
}

#[cfg(test)]
#[path = "tracker/sync_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tracker/status_tests.rs"]
mod status_tests;
