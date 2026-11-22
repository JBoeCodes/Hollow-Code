use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

/// Hollow's config.json. Unknown keys are ignored, so an older file still
/// loads: its `notion` block, `notionToken`, `notionSyncIntervalMin`,
/// `workspaceSource` and `sourceOfTruth` are no longer read here (Notion
/// two-way sync was removed on 2026-11-10). The one-time importer
/// (`tracker_import`) reads the Notion keys straight from the file, and
/// config writes keep them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub vault_path: PathBuf,
    /// Optional legacy Claude transcript import source; never a runtime requirement.
    #[serde(default = "crate::setup::default_claude_projects_path")]
    pub claude_projects_path: PathBuf,
    /// Optional root to scan for git repos powering the Today contribution
    /// heatmap. Defaults to the vault's *parent* directory when absent (the
    /// vault itself isn't a repo, but the user's projects live beside it).
    /// Scanned one level deep — every immediate subdir with a `.git`.
    #[serde(default)]
    pub git_repos_path: Option<PathBuf>,
    /// The task tracker (`tracker`: provider, interval, the one team). A
    /// malformed block loads as "off" with a note, never as a config error
    /// (`tracker::settings`).
    #[serde(default, deserialize_with = "crate::tracker::settings::tolerant")]
    pub tracker: crate::tracker::settings::TrackerConfig,
}

impl Config {
    /// Empty-paths placeholder for setup mode: when no config.json could be
    /// loaded, the app boots with this instead of exiting so the Onboarding
    /// wizard can run. The empty `vault_path` never exists, so everything
    /// gated on `vault_path.exists()` (watcher, schedulers) stays off.
    pub fn placeholder() -> Self {
        Self {
            vault_path: PathBuf::new(),
            claude_projects_path: PathBuf::new(),
            git_repos_path: None,
            tracker: crate::tracker::settings::TrackerConfig::default(),
        }
    }

    #[cfg(test)]
    pub fn workspace_source(&self) -> WorkspaceSource {
        WorkspaceSource::Obsidian
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSource {
    Obsidian,
}

impl WorkspaceSource {
    pub fn as_str(self) -> &'static str {
        "obsidian"
    }
}

/// Locate config.json by checking, in order:
/// 1) $HOLLOW_CONFIG env var
/// 2) ~/.config/Hollow/config.json
/// 3) ~/Library/Application Support/Hollow/config.json
/// 4) ./config.json (project dir for dev)
///
/// A candidate build (`instance.rs`) looks only at `$HOLLOW_CONFIG` and its
/// own `~/Library/Application Support/Hollow-<label>/config.json`, never at
/// the everyday app's, and refuses a vault that overlaps the everyday one.
pub fn locate_and_load() -> Result<(Config, PathBuf), String> {
    let path = locate_config()?;
    let txt = fs::read_to_string(&path).map_err(|e| format!("read config: {e}"))?;
    let cfg: Config = serde_json::from_str(&txt).map_err(|e| format!("parse config: {e}"))?;
    if let Some(why) = crate::instance::refuse_everyday_vault(&[cfg.vault_path.as_path()]) {
        return Err(why);
    }
    Ok((cfg, path))
}

fn locate_config() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "home dir not found".to_string())?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let env = std::env::var("HOLLOW_CONFIG").ok();
    let candidates = config_candidates(crate::instance::current(), env.as_deref(), &home, &cwd);
    if let Some(found) = candidates.iter().find(|c| c.exists()) {
        return Ok(found.clone());
    }
    Err(format!("config.json not found. Looked in: {candidates:?}"))
}

/// Where config.json may live, first hit wins.
pub(crate) fn config_candidates(
    instance: &crate::instance::Instance,
    env: Option<&str>,
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = env
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if instance.is_candidate() {
        candidates.push(
            home.join("Library/Application Support")
                .join(instance.support_name())
                .join("config.json"),
        );
        return candidates;
    }
    candidates.extend([
        home.join(".config").join("Hollow").join("config.json"),
        home.join("Library")
            .join("Application Support")
            .join("Hollow")
            .join("config.json"),
        // Project-local fallback for dev (when running `tauri dev`).
        // Tauri's working dir is src-tauri/, so step up one.
        cwd.join("config.json"),
        cwd.join("..").join("config.json"),
    ]);
    candidates
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_candidate_never_falls_back_to_the_everyday_config() {
        let home = std::path::Path::new("/Users/someone");
        let cwd = std::path::Path::new("/work/hollow/src-tauri");
        let candidate = crate::instance::Instance::from_identifier("com.hollow.app.candidate");
        let everyday = crate::instance::Instance::everyday();

        let found = config_candidates(&candidate, Some("/tmp/bed/config.json"), home, cwd);
        assert_eq!(
            found,
            vec![
                PathBuf::from("/tmp/bed/config.json"),
                PathBuf::from(
                    "/Users/someone/Library/Application Support/Hollow-candidate/config.json"
                ),
            ]
        );
        assert_eq!(config_candidates(&candidate, None, home, cwd).len(), 1);

        // The everyday lookup order is unchanged.
        let found = config_candidates(&everyday, None, home, cwd);
        assert_eq!(
            found[0],
            PathBuf::from("/Users/someone/.config/Hollow/config.json")
        );
        assert_eq!(
            found[1],
            PathBuf::from("/Users/someone/Library/Application Support/Hollow/config.json")
        );
        assert_eq!(found.len(), 4);
        assert_eq!(
            config_candidates(&everyday, Some(""), home, cwd).len(),
            4,
            "empty HOLLOW_CONFIG is ignored"
        );
    }

    use super::*;

    /// Notion two-way sync was removed on 2026-11-10: an older config with
    /// any of its keys still loads, and nothing here reads them.
    #[test]
    fn an_older_config_with_notion_keys_still_loads() {
        for source in ["notion", "both", "obsidian"] {
            let config: Config = serde_json::from_value(serde_json::json!({
                "vaultPath": "/vault", "claudeProjectsPath": "/agents",
                "workspaceSource": source,
                "sourceOfTruth": {"tasks": "notion", "projects": "notion"},
                "notionToken": "legacy", "notionSyncIntervalMin": 5,
                "notion": {"twoWaySync": true, "token": "t", "taskDataSourceId": "tasks",
                    "projectDataSourceId": "projects", "taskProperties": {"title": "Name"},
                    "contentDataSourceId": "c", "contentProperties": {"title": "Name"}}
            }))
            .unwrap();
            assert_eq!(config.workspace_source(), WorkspaceSource::Obsidian);
            assert_eq!(config.vault_path, PathBuf::from("/vault"));
        }
    }
}
