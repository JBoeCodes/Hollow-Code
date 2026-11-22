//! Brief helpers shared by the workspace readers: [`read_prefix`] (a bounded
//! read of a brief), [`project_of`] (the project a brief names) and
//! [`BRIEF_PREFIX`] (how much of a brief is read).
//!
//! Called `workspace_map.rs` until the 11-10 sweep. The workspace map
//! (master plan #18, overnight B4: the galaxy view, its clusters and topics,
//! the `workspace_map` command and the `workspaces.workspace_map` agent
//! action) was removed 11-10 (S6) with its screen; these helpers stayed:
//! `canvas_items`, `triage`, `meeting_prep`, `workspace_connections` and
//! `tracker/sync_tests` call them.

use std::collections::BTreeMap;
use std::path::Path;

/// How much of a brief is read.
pub(crate) const BRIEF_PREFIX: u64 = 64 * 1024;

/// The first `cap` bytes of a file as text (`None` when it can't be read).
pub(crate) fn read_prefix(path: &Path, cap: u64) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The project a brief names, as (key, label): the project note's vault
/// path and its name. Only project notes count (`projects/**.md` or a
/// Notion mirror's `PROJECT.md`), found again after a status move.
pub(crate) fn project_of(
    vault: &Path,
    brief: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    if let Some(path) = brief
        .get("tracker_project_path")
        .filter(|path| !path.trim().is_empty())
    {
        let project = crate::goal_context::resolve_tracker_project_url(vault, brief, path)?;
        return Some((format!("project:{}", project.vault_path?), project.name));
    }
    let keys: &[&str] = if brief
        .get("tracker_project_path")
        .is_some_and(|p| !p.trim().is_empty())
    {
        &["tracker_project_path"]
    } else {
        &["notion_project_path", "project_url"]
    };
    keys.iter().find_map(|k| {
        let raw = brief
            .get(*k)
            .map(|v| v.trim().trim_matches('"'))
            .filter(|v| !v.is_empty())?;
        let (project, _moved) = crate::goal_context::resolve_project_url(vault, raw)?;
        let path = project.vault_path.clone()?;
        Some((format!("project:{path}"), project.name))
    })
}
