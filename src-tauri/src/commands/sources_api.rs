//! Settings, Sources' Tauri commands (overhaul H4a; rules in `sources.rs`,
//! switches in `sources/migration.rs`, contract
//! `docs/TRACKER_SYNC.md#sources-of-truth-h4a`). Typed wrappers are in
//! `src/sourcesApi.ts`; each command has an app action of the same name
//! under `sources.`.
//!
//! Reads use config.json as saved (what Settings shows), the ledger as last
//! saved and the key's state (never the key). `sources_set` refuses an owner
//! change and points to the switch; the switch commands never call the
//! tracker.

use crate::commands::AppState;
use crate::sources::migration::{self, Env, Target};
use crate::sources::{self, Access, Context, ItemFacts, LedgerFacts, SourcesStatus};
use crate::tracker::ipc::TrackerViewRow;
use crate::tracker::settings::{self, KeyState};
use crate::types::Task;
use std::path::{Path, PathBuf};
use tauri::State;

fn config_path(state: &AppState) -> Result<PathBuf, String> {
    if state.config_path.as_os_str().is_empty() {
        crate::setup::config_write_path()
    } else {
        Ok(state.config_path.clone())
    }
}

/// The switch's paths for this app (also the `sources.` actions').
pub(crate) fn env_for(state: &AppState) -> Result<Env, String> {
    if state.config.vault_path.as_os_str().is_empty() {
        return Err("Choose a Hollow folder first.".into());
    }
    let config_path = config_path(state)?;
    Ok(Env {
        vault: state.config.vault_path.clone(),
        migrations: Env::migrations_for(&config_path),
        config_path,
    })
}

/// The settings a label or a status row reads: config.json as saved, the
/// ledger as last saved and the key's state (never the key).
pub(crate) fn context_at(
    config_path: &Path,
    vault: &Path,
) -> Result<(Context, Option<LedgerFacts>), String> {
    let map = crate::setup::load_config_map(config_path)?;
    let (sources_config, tracker) = sources::SourcesConfig::from_map(&map);
    let ledger = LedgerFacts::read(vault);
    // Touch the Keychain only once a provider is chosen.
    let key = match tracker.provider.as_deref() {
        Some(provider) => settings::key_state(provider),
        None => KeyState::Missing,
    };
    let ctx = Context {
        sources: sources_config,
        tracker,
        access: Access {
            key,
            last_read: ledger.as_ref().and_then(|l| l.last_read.clone()),
            failures: ledger.as_ref().map_or(0, |l| l.failures),
        },
    };
    Ok((ctx, ledger))
}

/// [`sources_status`]'s body (also the `sources.status` action).
pub(crate) fn status_for(state: &AppState) -> Result<SourcesStatus, String> {
    let env = env_for(state)?;
    let (ctx, ledger) = context_at(&env.config_path, &env.vault)?;
    let library = crate::connectors::library_names().ok();
    let mut status = sources::status(&ctx, &env.vault, ledger.as_ref(), library.as_deref());
    status.unfinished_migration = migration::unfinished(&env).map(|r| r.id);
    Ok(status)
}

/// One task's facts for [`sources::label`], from its tracker mirror.
fn facts_of(task: &Task) -> ItemFacts {
    match &task.tracker {
        Some(meta) => ItemFacts {
            tracker: Some(meta.provider.clone()),
            tracker_id: Some(meta.id.clone()).filter(|id| !id.is_empty()),
            tracker_project: None,
            tracker_url: meta.url.clone(),
        },
        None => ItemFacts::default(),
    }
}

/// Fill each task's `source_label` (H4b): the tasks role's binding and the
/// task's own tracker facts, so the Tasks list and the phone's
/// `/v1/tasks` say the same words.
///
/// A task the tracker doesn't hold (the Tasks page's Local tasks) lives in
/// the vault whatever the role says, so it reads "Stored in Hollow".
pub(crate) fn label_tasks(ctx: &Context, items: &mut [Task]) {
    let binding = sources::global_tasks(ctx);
    let mut local = binding.clone();
    local.owner = sources::OwnerKind::Vault;
    local.chosen = true;
    local.was_tracked = None;
    for task in items {
        let facts = facts_of(task);
        // Any tracker link, even one missing its id, is the tracker's (the
        // phone's `trackerLinked` says so too).
        let which = if task.tracker.is_some() {
            &binding
        } else {
            &local
        };
        task.source_label = Some(sources::label(which, &facts));
    }
}

/// [`label_tasks`] from the saved settings; a list still reads when the
/// settings can't (its tasks then carry no label).
pub(crate) fn label_tasks_at(config_path: &Path, vault: &Path, items: &mut [Task]) {
    if items.is_empty() || vault.as_os_str().is_empty() {
        return;
    }
    if let Ok((ctx, _)) = context_at(config_path, vault) {
        label_tasks(&ctx, items);
    }
}

/// Fill each tracker issue row's `source_label` (H4b): the tasks role's
/// binding and the row's own issue, so the issue list and detail carry the
/// same words as a task.
pub(crate) fn label_rows(ctx: &Context, provider: Option<&str>, rows: &mut [TrackerViewRow]) {
    let binding = sources::global_tasks(ctx);
    for row in rows {
        let facts = ItemFacts {
            tracker: provider.map(str::to_string),
            tracker_id: Some(row.id.clone()),
            tracker_project: None,
            tracker_url: Some(row.url.clone()).filter(|u| !u.is_empty()),
        };
        row.source_label = Some(sources::label(&binding, &facts));
    }
}

/// [`label_rows`] from this app's saved settings; rows stay unlabeled when
/// the settings can't be read.
pub(crate) fn label_rows_for(
    state: &AppState,
    provider: Option<&str>,
    rows: &mut [TrackerViewRow],
) {
    if rows.is_empty() {
        return;
    }
    let Ok(path) = config_path(state) else { return };
    if let Ok((ctx, _)) = context_at(&path, &state.config.vault_path) {
        label_rows(&ctx, provider, rows);
    }
}

/// [`label_tasks_at`] for this app's own config.
pub(crate) fn label_tasks_for(state: &AppState, items: &mut [Task]) {
    if let Ok(path) = config_path(state) {
        label_tasks_at(&path, &state.config.vault_path, items);
    }
}

/// [`sources_set`]'s body: Save. Refuses an owner change (a switch).
pub(crate) fn save(state: &AppState, update: &sources::Update) -> Result<SourcesStatus, String> {
    let path = config_path(state)?;
    let mut map = crate::setup::load_config_map(&path)?;
    let before = map.clone();
    sources::apply_update(&mut map, update)?;
    if map != before {
        crate::setup::publish_config(&path, &map)?;
    }
    status_for(state)
}

/// Settings, Sources: the three role rows (owner, inherited, last read,
/// queued and held edits, conflicts, problems), workspaces with an override
/// or a binding, and knowledge references.
#[tauri::command(async)]
pub fn sources_status(state: State<'_, AppState>) -> Result<SourcesStatus, String> {
    status_for(&state)
}

/// Settings, Sources' Save: knowledge references; an owner change is
/// refused with the way to switch.
#[tauri::command(async)]
pub fn sources_set(
    state: State<'_, AppState>,
    update: sources::Update,
) -> Result<SourcesStatus, String> {
    save(&state, &update)
}

/// An owner switch's dry run: counts and files, nothing written.
#[tauri::command(async)]
pub fn sources_migration_plan(
    state: State<'_, AppState>,
    target: Target,
) -> Result<migration::Plan, String> {
    migration::plan(&env_for(&state)?, &target)
}

/// Apply an owner switch (or finish one that stopped part way).
#[tauri::command(async)]
pub async fn sources_migration_apply(
    state: State<'_, AppState>,
    target: Target,
) -> Result<migration::Outcome, String> {
    let env = env_for(&state)?;
    migration::apply(&env, &target).await
}

/// Roll back one owner switch by its id.
#[tauri::command(async)]
pub async fn sources_migration_rollback(
    state: State<'_, AppState>,
    id: String,
) -> Result<migration::Record, String> {
    let env = env_for(&state)?;
    migration::rollback(&env, &id).await
}

/// Every owner switch, newest first, and the one that stopped part way.
#[tauri::command(async)]
pub fn sources_migration_status(state: State<'_, AppState>) -> Result<migration::Status, String> {
    Ok(migration::status(&env_for(&state)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::LabelKind;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn ctx(config: serde_json::Value, key: KeyState) -> Context {
        let map: BTreeMap<String, serde_json::Value> = serde_json::from_value(config).unwrap();
        Context::from_map(
            &map,
            Access {
                key,
                last_read: None,
                failures: 0,
            },
        )
    }

    fn mirrored(url: &str) -> Task {
        let text = format!("---\ntitle: A\nstatus: Todo\ntracker: linear\ntracker_id: abc\ntracker_url: {url}\n---\n");
        Task {
            tracker: crate::tracker_sync::seams::task_meta(&text),
            ..Default::default()
        }
    }

    #[test]
    fn a_local_task_is_stored_in_hollow_and_a_mirror_is_tracked() {
        let tracked = ctx(
            json!({"tracker": {"provider": "linear", "linear": {"teamId": crate::tracker::linear::TEAM_ID, "teamKey": "ENG"}}}),
            KeyState::Saved,
        );
        let mut items = vec![
            Task::default(),
            mirrored("https://linear.app/x/issue/ENG-1"),
        ];
        label_tasks(&tracked, &mut items);
        let local = items[0].source_label.as_ref().unwrap();
        let issue = items[1].source_label.as_ref().unwrap();
        assert_eq!(issue.kind, LabelKind::Tracked);
        assert_eq!(
            issue.url.as_deref(),
            Some("https://linear.app/x/issue/ENG-1")
        );
        assert!(issue.open_label.is_some());
        // A task the tracker doesn't hold lives in the vault.
        assert_eq!(local.kind, LabelKind::Stored);
        assert_eq!(local.text, "Stored in Hollow");

        let vault = ctx(json!({}), KeyState::Missing);
        let mut items = vec![Task::default()];
        label_tasks(&vault, &mut items);
        let label = items[0].source_label.as_ref().unwrap();
        assert_eq!(label.kind, LabelKind::Stored);
        assert_eq!(label.text, "Stored in Hollow");
    }

    #[test]
    fn every_issue_row_carries_its_own_label() {
        let tracked = ctx(
            json!({"tracker": {"provider": "linear", "linear": {"teamId": crate::tracker::linear::TEAM_ID, "teamKey": "ENG"}}}),
            KeyState::Saved,
        );
        let mut rows = vec![sample_row("https://linear.app/x/issue/ENG-3")];
        label_rows(&tracked, Some("linear"), &mut rows);
        let label = rows[0].source_label.as_ref().unwrap();
        assert_eq!(label.kind, LabelKind::Tracked);
        assert_eq!(label.text, "Tracked in Linear");
        assert_eq!(
            label.url.as_deref(),
            Some("https://linear.app/x/issue/ENG-3")
        );
    }

    fn sample_row(url: &str) -> TrackerViewRow {
        TrackerViewRow {
            id: "i3".into(),
            key: "ENG-3".into(),
            title: "A".into(),
            status_name: "Todo".into(),
            status_kind: crate::tracker::StateKind::Unstarted,
            priority: 0,
            labels: Vec::new(),
            project: None,
            project_id: None,
            due: None,
            assignee: None,
            assignee_id: None,
            estimate: None,
            cycle: None,
            milestone: None,
            team_key: None,
            updated_at: String::new(),
            url: url.into(),
            path: None,
            lock: None,
            pending: false,
            conflict: false,
            archived: false,
            source_label: None,
        }
    }

    #[test]
    fn a_tracker_without_its_key_reads_unreachable_never_stored() {
        let no_key = ctx(
            json!({"tracker": {"provider": "linear", "linear": {"teamId": crate::tracker::linear::TEAM_ID, "teamKey": "ENG"}}}),
            KeyState::Missing,
        );
        let mut items = vec![mirrored("https://linear.app/x/issue/ENG-2")];
        label_tasks(&no_key, &mut items);
        let label = items[0].source_label.as_ref().unwrap();
        assert_eq!(label.kind, LabelKind::Unreachable);
        assert_ne!(label.text, "Stored in Hollow");
    }

    #[test]
    fn unreadable_settings_leave_tasks_unlabeled() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{ not json").unwrap();
        let mut items = vec![Task::default()];
        label_tasks_at(&config, dir.path(), &mut items);
        assert!(items[0].source_label.is_none());
    }
}
