//! The first-run setup sheet's progress (H6, with CX21 and F2, 2026-11-19;
//! design `sources-and-import-2026-11-13` section 5).
//!
//! State lives in config.json as `setup: { "version": 1, "steps": { "<step>":
//! "done" | "skipped" } }`. Steps, in order: [`STEPS`]. A step missing from
//! the map is open, so quitting mid-setup resumes at the first open step and
//! a later version adds only its new steps.
//!
//! **Who sees it.** The first-run folder setup
//! (`directory_connections::connect_locked`) writes [`fresh_block`] only when
//! the config had neither `vaultPath` nor `setup`, so the sheet opens after
//! that restart. A config with no `setup` block is complete: people who set
//! Hollow up before H6 see nothing new. Run setup again (Settings, Sources)
//! writes a fresh block through [`reopen`].
//!
//! The commands are UI plumbing (the sheet's own progress), never actions:
//! the person's clicks inside the sheet go through each step's own commands.

use crate::commands::AppState;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;

/// The config key.
pub(crate) const KEY: &str = "setup";
/// The block's version; a bump adds only a missing step.
pub(crate) const VERSION: u64 = 1;
/// The sheet's steps, in order: Connect your context, Find what's already
/// here, Where your work lives, Your agents, Start.
pub(crate) const STEPS: [&str; 5] = ["context", "find", "sources", "agents", "start"];

/// One step and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStep {
    pub id: String,
    /// `open`, `done` or `skipped`.
    pub state: String,
}

/// The sheet's progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub version: u64,
    pub steps: Vec<SetupStep>,
    /// The first open step; `None` once every step is done or skipped.
    pub current: Option<String>,
    pub complete: bool,
}

/// The block a fresh config gets: every step open.
pub(crate) fn fresh_block() -> Value {
    json!({ "version": VERSION, "steps": {} })
}

/// Whether the first-run folder setup should add [`fresh_block`]: only a
/// config that has never named a vault and has no block yet.
pub(crate) fn wants_block(map: &BTreeMap<String, Value>) -> bool {
    !map.contains_key("vaultPath") && !map.contains_key(KEY)
}

/// The progress `map` records. No block: complete.
pub(crate) fn status_of(map: &BTreeMap<String, Value>) -> SetupStatus {
    let block = map.get(KEY).and_then(Value::as_object);
    let recorded = block
        .and_then(|b| b.get("steps"))
        .and_then(Value::as_object);
    let steps: Vec<SetupStep> = STEPS
        .iter()
        .map(|id| {
            let state = match block {
                None => "done",
                Some(_) => match recorded.and_then(|s| s.get(*id)).and_then(Value::as_str) {
                    Some("done") => "done",
                    Some("skipped") => "skipped",
                    _ => "open",
                },
            };
            SetupStep {
                id: (*id).to_string(),
                state: state.to_string(),
            }
        })
        .collect();
    let current = steps
        .iter()
        .find(|s| s.state == "open")
        .map(|s| s.id.clone());
    SetupStatus {
        version: block
            .and_then(|b| b.get("version"))
            .and_then(Value::as_u64)
            .unwrap_or(VERSION),
        complete: current.is_none(),
        current,
        steps,
    }
}

/// Record `step` as `outcome` (`done` or `skipped`). Refused for an unknown
/// step or outcome, and when setup isn't open (no block).
pub(crate) fn mark(
    map: &mut BTreeMap<String, Value>,
    step: &str,
    outcome: &str,
) -> Result<(), String> {
    if !STEPS.contains(&step) {
        return Err(format!("Unknown setup step `{step}`"));
    }
    if !matches!(outcome, "done" | "skipped") {
        return Err(format!("A setup step is done or skipped, not `{outcome}`"));
    }
    let Some(block) = map.get_mut(KEY).and_then(Value::as_object_mut) else {
        return Err("Setup is already finished. Use Run setup again in Settings, Sources.".into());
    };
    let steps = block
        .entry("steps")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(steps) = steps.as_object_mut() {
        steps.insert(step.to_string(), Value::String(outcome.to_string()));
    } else {
        let mut fresh = Map::new();
        fresh.insert(step.to_string(), Value::String(outcome.to_string()));
        *steps = Value::Object(fresh);
    }
    Ok(())
}

/// Run setup again: every step open.
pub(crate) fn reopen(map: &mut BTreeMap<String, Value>) {
    map.insert(KEY.into(), fresh_block());
}

fn config_path(state: &AppState) -> Result<PathBuf, String> {
    if state.config_path.as_os_str().is_empty() {
        crate::setup::config_write_path()
    } else {
        Ok(state.config_path.clone())
    }
}

fn status_at(path: &Path) -> Result<SetupStatus, String> {
    Ok(status_of(&crate::setup::load_config_map(path)?))
}

fn edit_at(
    path: &Path,
    edit: impl FnOnce(&mut BTreeMap<String, Value>) -> Result<(), String>,
) -> Result<SetupStatus, String> {
    let mut map = crate::setup::load_config_map(path)?;
    let before = map.clone();
    edit(&mut map)?;
    if map != before {
        crate::setup::publish_config(path, &map)?;
    }
    Ok(status_of(&map))
}

/// The sheet's progress: every step's state and the first open one.
#[tauri::command(async)]
pub fn first_run_status(state: State<'_, AppState>) -> Result<SetupStatus, String> {
    status_at(&config_path(&state)?)
}

/// Mark one step done or skipped (the sheet's Continue or Skip).
#[tauri::command(async)]
pub fn first_run_set_step(
    state: State<'_, AppState>,
    step: String,
    outcome: String,
) -> Result<SetupStatus, String> {
    edit_at(&config_path(&state)?, |map| mark(map, &step, &outcome))
}

/// Run setup again (Settings, Sources): every step open.
#[tauri::command(async)]
pub fn first_run_reopen(state: State<'_, AppState>) -> Result<SetupStatus, String> {
    edit_at(&config_path(&state)?, |map| {
        reopen(map);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn map_of(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_fresh_block_opens_at_the_first_step() {
        let mut map = BTreeMap::new();
        assert!(wants_block(&map));
        map.insert(KEY.into(), fresh_block());
        let status = status_of(&map);
        assert_eq!(status.current.as_deref(), Some("context"));
        assert!(!status.complete);
        assert_eq!(status.version, VERSION);
        assert!(status.steps.iter().all(|s| s.state == "open"));
        assert_eq!(
            status
                .steps
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            STEPS.to_vec()
        );
    }

    #[test]
    fn an_existing_config_without_the_block_is_complete() {
        let map = map_of(json!({ "vaultPath": "/v", "projectsPath": "/p" }));
        assert!(!wants_block(&map));
        let status = status_of(&map);
        assert!(status.complete);
        assert_eq!(status.current, None);
        assert!(status.steps.iter().all(|s| s.state == "done"));
        // Marking a step is refused: setup isn't open.
        let mut map = map;
        assert!(mark(&mut map, "find", "done").is_err());
        assert!(!map.contains_key(KEY));
    }

    #[test]
    fn marking_steps_resumes_at_the_next_open_one() {
        let mut map = map_of(json!({ "vaultPath": "/v", "setup": fresh_block() }));
        mark(&mut map, "context", "skipped").unwrap();
        assert_eq!(status_of(&map).current.as_deref(), Some("find"));
        mark(&mut map, "find", "done").unwrap();
        mark(&mut map, "agents", "done").unwrap();
        let status = status_of(&map);
        // `sources` is still open, so it comes before `start`.
        assert_eq!(status.current.as_deref(), Some("sources"));
        assert_eq!(status.steps[0].state, "skipped");
        assert_eq!(status.steps[3].state, "done");
        mark(&mut map, "sources", "done").unwrap();
        mark(&mut map, "start", "done").unwrap();
        assert!(status_of(&map).complete);
        assert!(mark(&mut map, "start", "maybe").is_err());
        assert!(mark(&mut map, "import", "done").is_err());
    }

    #[test]
    fn an_unknown_recorded_value_counts_as_open() {
        let map =
            map_of(json!({ "setup": { "version": 1, "steps": { "context": "half", "find": 3 } } }));
        assert_eq!(status_of(&map).current.as_deref(), Some("context"));
    }

    #[test]
    fn reopen_resets_every_step_and_is_written_through_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"vaultPath":"/v","custom":{"keep":true}}"#).unwrap();
        assert!(status_at(&path).unwrap().complete);
        let opened = edit_at(&path, |map| {
            reopen(map);
            Ok(())
        })
        .unwrap();
        assert_eq!(opened.current.as_deref(), Some("context"));
        let after = edit_at(&path, |map| mark(map, "context", "done")).unwrap();
        assert_eq!(after.current.as_deref(), Some("find"));
        let saved = crate::setup::load_config_map(&path).unwrap();
        assert_eq!(saved["custom"]["keep"], true);
        assert_eq!(saved["setup"]["steps"]["context"], "done");
        let again = edit_at(&path, |map| {
            reopen(map);
            Ok(())
        })
        .unwrap();
        assert!(again.steps.iter().all(|s| s.state == "open"));
    }

    #[test]
    fn folder_setup_writes_the_block_only_for_a_fresh_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Projects");
        let v = dir.path().join("Vault");
        fs::create_dir(&p).unwrap();
        fs::create_dir(&v).unwrap();
        let (p, v) = (p.canonicalize().unwrap(), v.canonicalize().unwrap());
        let (ps, vs) = (p.to_str().unwrap(), v.to_str().unwrap());

        // A fresh config: the sheet opens after the restart.
        let fresh = dir.path().join("fresh.json");
        crate::directory_connections::connect(ps, vs, &fresh).unwrap();
        let map = crate::setup::load_config_map(&fresh).unwrap();
        assert_eq!(map[KEY], fresh_block());
        assert_eq!(status_of(&map).current.as_deref(), Some("context"));
        // Connecting again (Settings' Bring existing folders) keeps progress.
        let mut map = map;
        mark(&mut map, "context", "done").unwrap();
        crate::setup::publish_config(&fresh, &map).unwrap();
        crate::directory_connections::connect(ps, vs, &fresh).unwrap();
        let map = crate::setup::load_config_map(&fresh).unwrap();
        assert_eq!(map[KEY]["steps"]["context"], "done");

        // An existing config that already named a vault: nothing new.
        let existing = dir.path().join("existing.json");
        fs::write(&existing, r#"{"vaultPath":"/old"}"#).unwrap();
        crate::directory_connections::connect(ps, vs, &existing).unwrap();
        let map = crate::setup::load_config_map(&existing).unwrap();
        assert!(!map.contains_key(KEY));
        assert!(status_of(&map).complete);
    }
}
