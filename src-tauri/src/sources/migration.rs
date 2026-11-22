//! Owner switches (overhaul H4a, design phase 1b): moving tasks between the
//! vault and the tracker, for every workspace or for one, is a migration,
//! never a Save. It reuses the `tracker-import` pattern: [`plan`] is the dry
//! run (counts first, nothing written), [`apply`] journals every vault file
//! through `tracker_import::files` (backup, `journal.jsonl`, `manifest.json`)
//! and [`rollback`] undoes it byte for byte.
//!
//! - Refused while any queued edit, held edit, unconfirmed create or
//!   conflict is open. A rollback ignores held edits: it is what releases
//!   them (the pass returns an entry of the generation it goes back to).
//! - Every switch writes a new ledger generation ([`Ledger::generation`]):
//!   an outbox entry from an older one is held, never sent to the new
//!   destination (`tracker_sync::hold_stale`).
//! - No network, ever: the tracker side is never archived, deleted or
//!   edited. A switch to Hollow keeps every mirror file and adds a "Was
//!   tracked in Linear" line to each tracked workspace's PROJECT.md; the
//!   ledger then says `storedInHollow` and the tracker pass reads and sends
//!   nothing until a switch back.
//! - An interrupted apply resumes: its record says `applying`, and the next
//!   apply of the same switch finishes the remaining steps (files already
//!   written are no-ops, the ledger and config steps check their hashes).
//!
//! Per workspace, this build switches a workspace that mirrors no tracker
//! project (a personal one) either way. Moving one tracked project to
//! Hollow while the rest stay tracked needs the pass to skip that project,
//! which is a follow-up; it is refused with a message.
//!
//! Records live in `<migrations>/<id>/` (the app's own folder beside
//! config.json, never the vault): `record.json`, `vault/<stamp>-sources/`
//! (the files journal and manifest), `ledger.before.json` and
//! `config.before.json` (0600: config.json can hold secrets).

use super::{read_override, resolve, Access, Context, LedgerFacts, OwnerKind, WAS_TRACKED_MARK};
use crate::frontmatter::{self, KeyEdit};
use crate::tracker::settings::KeyState;
use crate::tracker_import::files::{self, Overlay};
use crate::tracker_import::{sha256, Report};
use crate::tracker_sync::{self, Ledger};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Where a switch reads and writes. Every path is the caller's, so tests
/// use temp folders and nothing here falls back to the home folder.
#[derive(Debug, Clone)]
pub struct Env {
    pub vault: PathBuf,
    pub config_path: PathBuf,
    /// The switch records' folder (`sources-migrations` beside config.json).
    pub migrations: PathBuf,
}

impl Env {
    /// The records folder for a config file.
    pub fn migrations_for(config_path: &Path) -> PathBuf {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sources-migrations")
    }
}

/// What to switch: every workspace (`workspace` none) or one, to `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationTarget"))]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(default)]
    pub workspace: Option<String>,
    pub to: OwnerKind,
}

/// One vault file the switch writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationFile"))]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub path: String,
    /// `create` or `change`.
    pub action: String,
    pub why: String,
}

/// The dry run: counts first, nothing written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationPlan"))]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub target: Target,
    pub from: OwnerKind,
    /// Already there: apply changes nothing.
    pub nothing_to_do: bool,
    /// Why apply would refuse; nothing was changed.
    pub refused: Option<String>,
    pub files: Vec<PlannedFile>,
    /// "sources.tasks.owner: tracker to vault", when config.json changes.
    pub config_change: Option<String>,
    pub generation_from: u32,
    pub generation_to: u32,
    pub stored_in_hollow_after: bool,
    pub queued_edits: u32,
    pub held_edits: u32,
    pub conflicts: u32,
    /// Mirror files kept exactly as they are.
    pub mirrored_tasks: u32,
    pub mirrored_projects: u32,
    /// Always 0: a switch never writes to the tracker.
    pub tracker_writes: u32,
    pub notes: Vec<String>,
}

/// A switch's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationPhase"))]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Started and not finished (interrupted): apply it again to resume, or
    /// roll it back.
    Applying,
    Applied,
    RolledBack,
}

/// `record.json`: what a switch did, enough to resume and roll it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationRecord"))]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub phase: Phase,
    pub target: Target,
    pub from: OwnerKind,
    pub created_at: String,
    pub vault: String,
    pub generation_from: u32,
    pub generation_to: u32,
    pub stored_before: bool,
    pub stored_after: bool,
    /// The switch changes the ledger (one exists, or a switch to Hollow
    /// creates one so the pass stays off).
    pub ledger_step: bool,
    pub ledger_existed: bool,
    pub ledger_before_sha: Option<String>,
    pub ledger_after_sha: Option<String>,
    pub config_step: bool,
    pub config_before_sha: Option<String>,
    pub config_after_sha: Option<String>,
    #[serde(default)]
    pub rolled_back_at: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Where a test stops an apply, to prove it resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Stop {
    AfterFiles,
    AfterLedger,
}

const RECORD: &str = "record.json";
const LEDGER_BEFORE: &str = "ledger.before.json";
const CONFIG_BEFORE: &str = "config.before.json";

/// The marker line a switch to Hollow adds to a tracked PROJECT.md.
fn marker_line(provider_name: &str, key: Option<&str>, url: Option<&str>) -> String {
    let key = key.unwrap_or("its project");
    match url {
        Some(url) => format!("Was tracked in {provider_name}: [{key}]({url}) {WAS_TRACKED_MARK}"),
        None => format!("Was tracked in {provider_name} ({key}). {WAS_TRACKED_MARK}"),
    }
}

fn add_marker(text: &str, line: &str) -> String {
    let sep = if text.ends_with('\n') || text.is_empty() {
        ""
    } else {
        "\n"
    };
    format!("{text}{sep}\n{line}\n")
}

fn remove_marker(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !line.contains(WAS_TRACKED_MARK) {
            out.push_str(line);
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// The ledger as saved: validated through the engine's own load when the
/// tracker is set up, else parsed; None when there is none.
fn current_ledger(env: &Env, ctx: &Context) -> Result<Option<Ledger>, String> {
    let path = env.vault.join(tracker_sync::STATE);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err("Tracker sync history could not be read; nothing was changed.".into())
        }
        Ok(meta) if !meta.is_file() => {
            return Err("Tracker sync history isn't a plain file; nothing was changed.".into())
        }
        Ok(_) => {}
    }
    match (
        ctx.tracker.provider.as_deref(),
        ctx.tracker.team_id.as_deref(),
    ) {
        (Some(provider), Some(team)) => tracker_sync::load(&env.vault, provider, team).map(Some),
        _ => super::read_ledger(&env.vault)
            .map(Some)
            .ok_or_else(|| "Tracker sync history is unreadable; nothing was changed.".into()),
    }
}

fn workspace_ids(vault: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(vault.join("workspaces"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| crate::workspaces::valid_id(id))
        .collect();
    ids.sort();
    ids
}

struct Planned {
    plan: Plan,
    overlay: Overlay,
    ledger: Option<Ledger>,
    ledger_existed: bool,
    config: Option<BTreeMap<String, serde_json::Value>>,
}

fn ctx_for(env: &Env) -> Result<(Context, BTreeMap<String, serde_json::Value>), String> {
    let map = crate::setup::load_config_map(&env.config_path)?;
    // A switch reads no Keychain: reachability doesn't change what it writes.
    Ok((Context::from_map(&map, Access::default()), map))
}

/// The dry run for `target`.
pub fn plan(env: &Env, target: &Target) -> Result<Plan, String> {
    Ok(plan_inner(env, target)?.plan)
}

fn plan_inner(env: &Env, target: &Target) -> Result<Planned, String> {
    let (ctx, map) = ctx_for(env)?;
    let ledger = current_ledger(env, &ctx)?;
    let ledger_existed = ledger.is_some();
    let facts = ledger.as_ref().map(LedgerFacts::of).unwrap_or_default();
    let mut overlay = Overlay::new(&env.vault);
    let mut why: BTreeMap<String, String> = BTreeMap::new();
    let mut notes = Vec::new();
    let provider_name = ctx
        .tracker
        .provider_info()
        .map(|p| p.display_name.to_string())
        .unwrap_or_else(|| "the tracker".into());
    let mut refused: Option<String> = None;
    let from;
    let mut config = None;
    let mut stored_after = facts.stored_in_hollow;
    match &target.workspace {
        None => {
            from = ctx.sources.tasks;
            if target.to == OwnerKind::Tracker && ctx.tracker.missing(KeyState::Saved).is_some() {
                refused = Some("Choose a task tracker and its team in Settings, Sources before tracking tasks there; a switch never widens what Settings allows. Nothing was changed.".into());
            }
            for id in workspace_ids(&env.vault) {
                let Ok(ov) = read_override(&env.vault, &id) else {
                    continue;
                };
                let project_rel = format!("workspaces/{id}/PROJECT.md");
                match target.to {
                    OwnerKind::Vault => {
                        if ov.choice == Some(OwnerKind::Tracker) {
                            for rel in override_files(&env.vault, &id) {
                                let Some(text) = overlay.text(&rel) else {
                                    continue;
                                };
                                let next = frontmatter::patch(
                                    &text,
                                    &[(super::OVERRIDE_KEY, KeyEdit::Remove)],
                                    None,
                                )
                                .map_err(|e| format!("{rel}: {e}"))?;
                                overlay.set(&rel, next.into_bytes())?;
                                why.insert(rel, "Its tasks_source: tracker would have no tracker pass while tasks are stored in Hollow; removed.".into());
                            }
                        }
                        if ov.tracker_project.is_some() && ov.was_tracked.is_none() {
                            let Some(text) = overlay.text(&project_rel) else {
                                continue;
                            };
                            let key = ledger
                                .as_ref()
                                .and_then(|l| {
                                    l.records.get(ov.tracker_project.as_deref().unwrap_or(""))
                                })
                                .map(|r| r.key.clone())
                                .filter(|k| !k.is_empty());
                            let line = marker_line(
                                &provider_name,
                                key.as_deref(),
                                ov.tracker_url.as_deref(),
                            );
                            overlay.set(&project_rel, add_marker(&text, &line).into_bytes())?;
                            why.insert(project_rel, format!("Adds \"Was tracked in {provider_name}\" with its link; the mirror's fields stay as they are."));
                        }
                    }
                    OwnerKind::Tracker => {
                        if ov.was_tracked.is_some()
                            || overlay
                                .text(&project_rel)
                                .is_some_and(|t| t.contains(WAS_TRACKED_MARK))
                        {
                            let Some(text) = overlay.text(&project_rel) else {
                                continue;
                            };
                            overlay.set(&project_rel, remove_marker(&text).into_bytes())?;
                            why.insert(
                                project_rel,
                                "Removes the \"Was tracked\" line: it is tracked again.".into(),
                            );
                        }
                    }
                }
            }
            if from != target.to {
                let mut next = map.clone();
                super::write_block(&mut next, Some(target.to), None);
                config = Some(next);
            }
            stored_after = target.to == OwnerKind::Vault && ctx.tracker.provider.is_some();
        }
        Some(id) => {
            let current = resolve(&ctx, &env.vault, Some(id));
            from = current.owner;
            let ov = read_override(&env.vault, id)?;
            let title = ov.title.clone().unwrap_or_else(|| id.clone());
            match target.to {
                OwnerKind::Vault if from == OwnerKind::Tracker => {
                    if ov.tracker_project.is_some() {
                        refused = Some(format!("{title} mirrors a {provider_name} project. Moving one tracked project to Hollow while the rest stay tracked isn't built yet: switch every workspace in Settings, Sources, or keep it tracked. Nothing was changed."));
                    } else {
                        let rel = ov.file.clone();
                        let text = overlay.text(&rel).ok_or_else(|| {
                            format!(
                                "{title} has no brief to hold its setting; nothing was changed."
                            )
                        })?;
                        let next = frontmatter::patch(
                            &text,
                            &[(super::OVERRIDE_KEY, KeyEdit::Set("vault".into()))],
                            None,
                        )
                        .map_err(|e| format!("{rel}: {e}; nothing was changed."))?;
                        overlay.set(&rel, next.into_bytes())?;
                        why.insert(
                            rel,
                            "tasks_source: vault (this workspace's tasks stay in Hollow).".into(),
                        );
                    }
                }
                OwnerKind::Tracker if from == OwnerKind::Vault => {
                    if ctx.sources.tasks != OwnerKind::Tracker || facts.stored_in_hollow {
                        refused = Some(format!("Tasks are stored in Hollow in Settings, Sources, so the tracker pass is off; one workspace can't be tracked on its own yet. Switch every workspace to {provider_name} first. Nothing was changed."));
                    } else if ctx.tracker.missing(KeyState::Saved).is_some() {
                        refused = Some("Choose a task tracker and its team in Settings, Sources first. Nothing was changed.".into());
                    } else {
                        for rel in override_files(&env.vault, id) {
                            let Some(text) = overlay.text(&rel) else {
                                continue;
                            };
                            let next = frontmatter::patch(
                                &text,
                                &[(super::OVERRIDE_KEY, KeyEdit::Remove)],
                                None,
                            )
                            .map_err(|e| format!("{rel}: {e}; nothing was changed."))?;
                            overlay.set(&rel, next.into_bytes())?;
                            why.insert(rel, format!("Removes tasks_source: vault; this workspace inherits {provider_name} from Settings."));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let files: Vec<PlannedFile> = overlay
        .planned()
        .into_iter()
        .map(|(path, exists)| PlannedFile {
            why: why.get(&path).cloned().unwrap_or_default(),
            action: if exists { "change" } else { "create" }.into(),
            path,
        })
        .collect();
    let open = facts.queued + facts.held + facts.conflicts;
    if refused.is_none() && open > 0 {
        refused = Some(format!(
            "{} queued edit(s), {} held edit(s) and {} conflict(s) are open. Send them (Sync now) or resolve them first, so nothing reaches the wrong place. Nothing was changed.",
            facts.queued, facts.held, facts.conflicts
        ));
    }
    // A ledger whose stored flag disagrees with Settings is fixed by the
    // same switch (a config edited by hand).
    let ledger_mismatch =
        target.workspace.is_none() && ledger_existed && facts.stored_in_hollow != stored_after;
    let nothing_to_do =
        files.is_empty() && config.is_none() && from == target.to && !ledger_mismatch;
    let ledger_step =
        !nothing_to_do && (ledger_existed || (stored_after && ctx.tracker.team_id.is_some()));
    let generation_from = facts.generation;
    let generation_to = if ledger_step {
        generation_from + 1
    } else {
        generation_from
    };
    let ledger = if ledger_step {
        let mut next = match ledger {
            Some(ledger) => ledger,
            None => Ledger::new(
                ctx.tracker.provider.as_deref().unwrap_or_default(),
                ctx.tracker.team_id.as_deref().unwrap_or_default(),
            ),
        };
        next.generation = generation_to;
        next.stored_in_hollow = stored_after;
        Some(next)
    } else {
        None
    };
    if stored_after && !facts.stored_in_hollow {
        notes.push(format!("The {provider_name} pass stops: nothing is read from or sent to {provider_name} until tasks are switched back. Mirror files stay as they are."));
    }
    if ledger_existed && !stored_after && facts.stored_in_hollow {
        notes.push(format!("The {provider_name} pass starts again after the switch; its first pass changes no mirror bytes."));
    }
    notes.push(format!(
        "Nothing in {provider_name} is archived, deleted or edited."
    ));
    let plan = Plan {
        target: target.clone(),
        from,
        nothing_to_do,
        refused,
        files,
        config_change: config.as_ref().map(|_| {
            format!(
                "sources.tasks.owner: {} to {}",
                from.as_str(),
                target.to.as_str()
            )
        }),
        generation_from,
        generation_to,
        stored_in_hollow_after: if ledger_step {
            stored_after
        } else {
            facts.stored_in_hollow
        },
        queued_edits: facts.queued,
        held_edits: facts.held,
        conflicts: facts.conflicts,
        mirrored_tasks: facts.mirrored_tasks,
        mirrored_projects: facts.mirrored_projects,
        tracker_writes: 0,
        notes,
    };
    Ok(Planned {
        plan,
        overlay,
        ledger,
        ledger_existed,
        config,
    })
}

/// The workspace files that carry `tasks_source`.
fn override_files(vault: &Path, id: &str) -> Vec<String> {
    ["PROJECT.md", "WORKSPACE.md"]
        .into_iter()
        .map(|name| format!("workspaces/{id}/{name}"))
        .filter(|rel| {
            fs::read_to_string(vault.join(rel))
                .is_ok_and(|text| frontmatter::parse(&text).get(super::OVERRIDE_KEY).is_some())
        })
        .collect()
}

fn valid_record_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn new_id() -> String {
    format!(
        "{}-{:06x}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        rand::random::<u32>() & 0xff_ffff
    )
}

fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|_| "The switch records' folder couldn't be created.".to_string())?;
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    Ok(())
}

/// A backup written once, owner-only, never over an existing one (a resumed
/// apply keeps the first attempt's bytes).
fn write_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => {
            return Err("A switch backup couldn't be written; nothing more was changed.".into())
        }
    };
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "A switch backup couldn't be written; nothing more was changed.".into())
}

fn save_record(dir: &Path, record: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?;
    files::write_atomic(&dir.join(RECORD), &bytes)
}

fn load_record(dir: &Path) -> Result<Record, String> {
    let raw = fs::read(dir.join(RECORD)).map_err(|_| "That switch has no record.".to_string())?;
    serde_json::from_slice(&raw).map_err(|_| "That switch's record is damaged.".to_string())
}

/// Every switch, newest first.
pub fn list(env: &Env) -> Vec<Record> {
    let mut out: Vec<Record> = fs::read_dir(&env.migrations)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| load_record(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| {
        (b.created_at.as_str(), b.id.as_str()).cmp(&(a.created_at.as_str(), a.id.as_str()))
    });
    out
}

/// Whether `a` was made after `b`. Ids carry a random suffix, so two
/// switches in one second order by their timestamps, not their ids.
fn newer(a: &Record, b: &Record) -> bool {
    (a.created_at.as_str(), a.id.as_str()) > (b.created_at.as_str(), b.id.as_str())
}

/// The switch that stopped part way, if any.
pub fn unfinished(env: &Env) -> Option<Record> {
    list(env).into_iter().find(|r| r.phase == Phase::Applying)
}

fn sha_of(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|b| sha256(&b))
}

/// `sources_migration_apply`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationOutcome"))]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub plan: Plan,
    /// None when there was nothing to do.
    pub record: Option<Record>,
    pub resumed: bool,
    pub note: String,
}

/// Apply `target` (or resume the same switch after an interruption).
pub async fn apply(env: &Env, target: &Target) -> Result<Outcome, String> {
    apply_with(env, target, None).await
}

pub(crate) async fn apply_with(
    env: &Env,
    target: &Target,
    stop: Option<Stop>,
) -> Result<Outcome, String> {
    let _guard = tracker_sync::root_lock(&env.vault).lock_owned().await;
    let _file = tracker_sync::file_lock(&env.vault)?;
    let pending = unfinished(env);
    if let Some(record) = &pending {
        if &record.target != target {
            return Err(format!(
                "Switch {} stopped part way. Apply that same switch again to finish it, or roll it back, before another. Nothing was changed.",
                record.id
            ));
        }
    }
    let planned = plan_inner(env, target)?;
    if let Some(why) = &planned.plan.refused {
        return Err(why.clone());
    }
    let resumed = pending.is_some();
    if planned.plan.nothing_to_do && !resumed {
        return Ok(Outcome {
            plan: planned.plan,
            record: None,
            resumed,
            note: "Already there; nothing was changed.".into(),
        });
    }
    private_dir(&env.migrations)?;
    let mut record = match pending {
        Some(record) => record,
        None => {
            let id = new_id();
            let vault = env
                .vault
                .canonicalize()
                .map_err(|_| "The vault folder doesn't exist.".to_string())?
                .to_string_lossy()
                .to_string();
            Record {
                id,
                phase: Phase::Applying,
                target: target.clone(),
                from: planned.plan.from,
                created_at: chrono::Local::now().to_rfc3339(),
                vault,
                generation_from: planned.plan.generation_from,
                generation_to: planned.plan.generation_to,
                stored_before: super::read_ledger(&env.vault).is_some_and(|l| l.stored_in_hollow),
                stored_after: planned.plan.stored_in_hollow_after,
                ledger_step: planned.ledger.is_some(),
                ledger_existed: planned.ledger_existed,
                ledger_before_sha: None,
                ledger_after_sha: None,
                config_step: planned.config.is_some(),
                config_before_sha: None,
                config_after_sha: None,
                rolled_back_at: None,
                notes: Vec::new(),
            }
        }
    };
    let dir = env.migrations.join(&record.id);
    private_dir(&dir)?;
    if !resumed {
        save_record(&dir, &record)?;
    }
    // 1. Vault files, journaled with a manifest (tracker-import's apply).
    if !planned.overlay.planned().is_empty() {
        let folders: BTreeSet<String> = planned
            .overlay
            .planned()
            .iter()
            .filter_map(|(rel, _)| {
                rel.strip_prefix("workspaces/")
                    .and_then(|p| p.split_once('/'))
                    .map(|(id, _)| format!("workspaces/{id}"))
            })
            .collect();
        let _lease = crate::automated_history::tracker_mirror_write_lease(&env.vault, &folders)
            .map_err(|e| e.to_string())?;
        let backups = dir.join("vault");
        private_dir(&backups)?;
        let mut report = Report::default();
        files::apply(
            &env.vault,
            &backups,
            "sources",
            &planned.overlay,
            &mut report,
        )?;
    }
    if stop == Some(Stop::AfterFiles) {
        return Err("stopped after the vault files (test)".into());
    }
    // 2. The ledger: the new generation and the stored flag.
    if record.ledger_step {
        let path = env.vault.join(tracker_sync::STATE);
        let current = sha_of(&path);
        let done = record.ledger_after_sha.is_some() && current == record.ledger_after_sha;
        if !done {
            if record.ledger_before_sha.is_none() {
                if let Ok(bytes) = fs::read(&path) {
                    write_backup(&dir.join(LEDGER_BEFORE), &bytes)?;
                }
                record.ledger_before_sha = current.clone();
                save_record(&dir, &record)?;
            }
            let mut next = match super::read_ledger(&env.vault) {
                Some(ledger) => ledger,
                None => planned
                    .ledger
                    .clone()
                    .ok_or("The tracker's history went missing during the switch.")?,
            };
            next.generation = record.generation_to;
            next.stored_in_hollow = record.stored_after;
            tracker_sync::save(&env.vault, &next)?;
            record.ledger_after_sha = sha_of(&path);
            save_record(&dir, &record)?;
        }
    }
    if stop == Some(Stop::AfterLedger) {
        return Err("stopped after the ledger (test)".into());
    }
    // 3. config.json's sources block.
    if record.config_step {
        let current = sha_of(&env.config_path);
        let done = record.config_after_sha.is_some() && current == record.config_after_sha;
        if !done {
            if record.config_before_sha.is_none() {
                let bytes = fs::read(&env.config_path).unwrap_or_default();
                write_backup(&dir.join(CONFIG_BEFORE), &bytes)?;
                record.config_before_sha = current.clone();
                save_record(&dir, &record)?;
            }
            let mut map = crate::setup::load_config_map(&env.config_path)?;
            super::write_block(&mut map, Some(record.target.to), None);
            crate::setup::publish_config(&env.config_path, &map)?;
            record.config_after_sha = sha_of(&env.config_path);
        }
    }
    record.phase = Phase::Applied;
    save_record(&dir, &record)?;
    Ok(Outcome {
        plan: planned.plan,
        note: if resumed {
            format!("Switch {} finished (it had stopped part way).", record.id)
        } else {
            format!("Switched. Roll back with switch {}.", record.id)
        },
        record: Some(record),
        resumed,
    })
}

/// Undo switch `id`: vault files byte for byte from their manifests (newest
/// run first), the ledger and config.json from their backups. Refused while
/// any queued edit or conflict is open, like a switch.
pub async fn rollback(env: &Env, id: &str) -> Result<Record, String> {
    if !valid_record_id(id) {
        return Err("That isn't a switch id.".into());
    }
    let dir = env.migrations.join(id);
    let mut record = load_record(&dir)?;
    if record.phase == Phase::RolledBack {
        return Err("This switch was already rolled back.".into());
    }
    let _guard = tracker_sync::root_lock(&env.vault).lock_owned().await;
    let _file = tracker_sync::file_lock(&env.vault)?;
    let here = env
        .vault
        .canonicalize()
        .map_err(|_| "The vault folder doesn't exist.".to_string())?;
    if here.to_string_lossy() != record.vault {
        return Err("That switch was made on another vault; nothing was changed.".into());
    }
    let (ctx, _) = ctx_for(env)?;
    if let Some(facts) = current_ledger(env, &ctx)?.as_ref().map(LedgerFacts::of) {
        // Held edits don't count: rolling back is how they are released
        // (the pass sends an entry of the generation it returns to).
        if facts.queued + facts.conflicts > 0 {
            return Err(format!(
                "{} queued edit(s) and {} conflict(s) are open. Send or resolve them before rolling back. Nothing was changed.",
                facts.queued, facts.conflicts
            ));
        }
    }
    // Only the newest live switch can be undone: an older one would put
    // back a generation and config block a later switch already replaced.
    if list(env)
        .iter()
        .any(|r| newer(r, &record) && r.phase != Phase::RolledBack)
    {
        return Err(
            "A newer switch is still in place. Roll that one back first. Nothing was changed."
                .into(),
        );
    }
    let mut notes = Vec::new();
    // 3. config.json.
    if record.config_step && record.config_before_sha.is_some() {
        let current = sha_of(&env.config_path);
        // Equal to the before hash: never written (an interrupted apply),
        // or already restored.
        if current != record.config_before_sha {
            let before = fs::read(dir.join(CONFIG_BEFORE)).map_err(|_| {
                "config.json's backup is missing; nothing more was changed.".to_string()
            })?;
            if current == record.config_after_sha {
                crate::setup::publish_config_bytes(&env.config_path, &before)?;
            } else {
                // Changed since (another setting saved): put back only the
                // sources block it had.
                let old: BTreeMap<String, serde_json::Value> =
                    serde_json::from_slice(&before).unwrap_or_default();
                let mut map = crate::setup::load_config_map(&env.config_path)?;
                match old.get("sources") {
                    Some(value) => map.insert("sources".into(), value.clone()),
                    None => map.remove("sources"),
                };
                crate::setup::publish_config(&env.config_path, &map)?;
                notes.push(
                    "config.json changed after the switch; only its sources block was put back."
                        .to_string(),
                );
            }
        }
    }
    // 2. The ledger.
    // No after hash is needed: an apply that stopped between writing the
    // ledger and recording its hash still has to be put back.
    if record.ledger_step {
        let path = env.vault.join(tracker_sync::STATE);
        let current = sha_of(&path);
        if current != record.ledger_before_sha {
            if current == record.ledger_after_sha {
                match fs::read(dir.join(LEDGER_BEFORE)) {
                    Ok(before) if record.ledger_existed => crate::run_guard::write_host_state(
                        &env.vault,
                        tracker_sync::STATE,
                        &before,
                    )?,
                    _ if !record.ledger_existed => fs::remove_file(&path)
                        .map_err(|_| "The tracker's history couldn't be removed.".to_string())?,
                    _ => {
                        return Err(
                            "The tracker history's backup is missing; nothing more was changed."
                                .into(),
                        )
                    }
                }
            } else if let Some(mut ledger) = super::read_ledger(&env.vault) {
                // A pass ran since: keep its work, put back the owner.
                ledger.generation = record.generation_from;
                ledger.stored_in_hollow = record.stored_before;
                tracker_sync::save(&env.vault, &ledger)?;
                notes.push("The tracker's history changed after the switch; its generation and stored flag were put back.".to_string());
            }
        }
    }
    // 1. Vault files, newest run first.
    let mut errors = Vec::new();
    let backups = dir.join("vault");
    let mut runs: Vec<PathBuf> = fs::read_dir(&backups)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    runs.sort();
    if !runs.is_empty() {
        let folders: BTreeSet<String> = workspace_ids(&env.vault)
            .into_iter()
            .map(|id| format!("workspaces/{id}"))
            .collect();
        let _lease = crate::automated_history::tracker_mirror_write_lease(&env.vault, &folders)
            .map_err(|e| e.to_string())?;
        for run in runs.iter().rev() {
            if run.join("rolled-back.json").exists() {
                continue;
            }
            let manifest = if run.join("manifest.json").exists() {
                run.join("manifest.json")
            } else {
                run.join("journal.jsonl")
            };
            if let Err(e) = files::rollback(&env.vault, &manifest) {
                errors.push(e);
            }
        }
    }
    record.notes.extend(notes);
    if !errors.is_empty() {
        save_record(&dir, &record)?;
        return Err(format!(
            "The rollback left {} problem(s); run it again once they are fixed: {}",
            errors.len(),
            errors.join(" ")
        ));
    }
    record.phase = Phase::RolledBack;
    record.rolled_back_at = Some(chrono::Local::now().to_rfc3339());
    save_record(&dir, &record)?;
    Ok(record)
}

/// `sources_migration_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesMigrationStatus"))]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Every switch, newest first.
    pub switches: Vec<Record>,
    /// The id of a switch that stopped part way.
    pub unfinished: Option<String>,
}

pub fn status(env: &Env) -> Status {
    let switches = list(env);
    Status {
        unfinished: switches
            .iter()
            .find(|r| r.phase == Phase::Applying)
            .map(|r| r.id.clone()),
        switches,
    }
}

#[cfg(test)]
pub(crate) fn tree(vault: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let first = rel.split('/').next().unwrap_or("");
            if first.starts_with('.') && rel != tracker_sync::STATE {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                walk(base, &path, out);
            } else if let Ok(bytes) = fs::read(&path) {
                out.insert(rel, sha256(&bytes));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(vault, vault, &mut out);
    out
}
