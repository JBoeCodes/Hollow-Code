//! A project's portfolio fields in its workspace's `PROJECT.md` (overhaul B3,
//! board S4; the field map is the tracker design's section 5). These live in
//! the vault only: the tracker owns a tracked project's name, status, lead,
//! dates and description, and its sync pass patches only those keys, so the
//! keys here are never sent anywhere and never overwritten by a pass.
//!
//! Reads say whether the project is tracked (`tracker` and `tracker_project`
//! in the frontmatter). Writes are a compare-and-swap on the file's revision,
//! touch only the portfolio keys, and create `PROJECT.md` (title from the
//! workspace) when a vault-only workspace has none yet.

use crate::frontmatter::{self, KeyEdit};
use crate::vault::{self, CasOutcome, Expected};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Single-line text fields, by vault key.
pub(crate) const TEXT_KEYS: [&str; 7] = [
    "department",
    "type",
    "hours_saved_per_year",
    "value_per_year",
    "owners_note",
    "github_url",
    "live_url",
];
/// List fields (vault paths or names), by vault key.
pub(crate) const LIST_KEYS: [&str; 7] = [
    "requested_by",
    "stakeholders",
    "domains",
    "meetings",
    "captures",
    "war_room",
    "docs",
];
const MAX_TEXT: usize = 2000;
const MAX_ITEMS: usize = 100;
const MAX_ITEM: usize = 400;
const LIMIT: u64 = 1_000_000;

/// Whom a tracked project belongs to in the tracker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PortfolioTracked {
    pub provider: String,
    pub project_id: String,
    pub url: Option<String>,
}

/// `project_portfolio`: one workspace's project record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectPortfolio {
    pub workspace_id: String,
    /// `workspaces/<id>/PROJECT.md`, whether or not it exists yet.
    pub path: String,
    pub exists: bool,
    pub title: String,
    pub status: Option<String>,
    pub target_date: Option<String>,
    /// The tracker's description for a tracked project, else the vault's.
    pub description: String,
    pub tracked: Option<PortfolioTracked>,
    #[cfg_attr(test, ts(type = "Record<string, string>"))]
    pub text: BTreeMap<String, String>,
    #[cfg_attr(test, ts(type = "Record<string, string[]>"))]
    pub lists: BTreeMap<String, Vec<String>>,
    /// The file's revision, or empty when it doesn't exist (the write's base).
    pub revision: String,
}

/// `set_project_portfolio`: new values by vault key (a string for a text
/// key, a list of strings for a list key; empty clears), and the revision
/// the editor read.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PortfolioUpdate {
    pub workspace_id: String,
    #[cfg_attr(test, ts(type = "Record<string, string | string[]>"))]
    pub values: BTreeMap<String, serde_json::Value>,
    pub expected_revision: String,
}

pub(crate) const CHANGED: &str = "PORTFOLIO_CHANGED";

fn rel(workspace_id: &str) -> String {
    format!("workspaces/{workspace_id}/PROJECT.md")
}

fn read_bounded(root: &Path, rel: &str) -> Result<Option<String>, String> {
    use std::io::Read;
    // The literal path, before `safe_join` resolves links: a PROJECT.md that
    // links to another workspace's must never be read or written as this one.
    match std::fs::symlink_metadata(root.join(rel)) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("PROJECT.md is a symbolic link; Hollow edits only the file itself".into())
        }
        _ => {}
    }
    let full = vault::safe_join(root, rel)?;
    match std::fs::symlink_metadata(&full) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("PROJECT.md is a symbolic link; Hollow edits only the file itself".into())
        }
        Ok(meta) if !meta.is_file() => return Err("PROJECT.md is not a file".into()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("PROJECT.md: {e}")),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&full)
        .map_err(|e| format!("read PROJECT.md: {e}"))?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read PROJECT.md: {e}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("PROJECT.md is larger than the 1 MB reader limit".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "PROJECT.md is not UTF-8 text".into())
}

fn workspace_title(root: &Path, workspace_id: &str) -> String {
    let brief =
        std::fs::read_to_string(root.join(format!("workspaces/{workspace_id}/WORKSPACE.md")))
            .unwrap_or_default();
    frontmatter::parse(&brief)
        .get("title")
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| workspace_id.to_string())
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The record as `text` reads (None: no PROJECT.md yet).
pub(crate) fn portfolio_of(
    root: &Path,
    workspace_id: &str,
    text: Option<&str>,
) -> ProjectPortfolio {
    let fm = text.map(frontmatter::parse);
    let get = |key: &str| fm.as_ref().and_then(|f| nonempty(f.get(key)));
    let tracked = match (get("tracker"), get("tracker_project")) {
        (Some(provider), Some(project_id)) => Some(PortfolioTracked {
            provider,
            project_id,
            url: get("tracker_url"),
        }),
        _ => None,
    };
    let description = match (text, &tracked) {
        (Some(t), Some(_)) => crate::tracker::mirror::block(t, "description")
            .ok()
            .flatten()
            .or_else(|| get("description"))
            .unwrap_or_default(),
        _ => get("description").unwrap_or_default(),
    };
    ProjectPortfolio {
        workspace_id: workspace_id.to_string(),
        path: rel(workspace_id),
        exists: text.is_some(),
        title: get("title").unwrap_or_else(|| workspace_title(root, workspace_id)),
        status: get("status"),
        target_date: get("target_date"),
        description,
        tracked,
        text: TEXT_KEYS
            .iter()
            .map(|k| (k.to_string(), get(k).unwrap_or_default()))
            .collect(),
        lists: LIST_KEYS
            .iter()
            .map(|k| {
                let items = text
                    .and_then(|t| frontmatter::list_items(t, k))
                    .unwrap_or_default();
                (k.to_string(), items)
            })
            .collect(),
        revision: text
            .map(|t| vault::revision(t.as_bytes()))
            .unwrap_or_default(),
    }
}

pub fn read(root: &Path, workspace_id: &str) -> Result<ProjectPortfolio, String> {
    crate::workspaces::folder(root, workspace_id)?;
    let rel = rel(workspace_id);
    if !root.join(format!("workspaces/{workspace_id}")).is_dir() {
        return Err("That workspace doesn't exist.".into());
    }
    let text = read_bounded(root, &rel)?;
    Ok(portfolio_of(root, workspace_id, text.as_deref()))
}

/// One key's validated edit.
fn edit_for(key: &str, value: &serde_json::Value) -> Result<KeyEdit, String> {
    if TEXT_KEYS.contains(&key) {
        let text = value
            .as_str()
            .ok_or_else(|| format!("{key} takes text."))?
            .trim();
        if text.contains('\n') || text.len() > MAX_TEXT {
            return Err(format!(
                "{key} is one line of at most {MAX_TEXT} characters."
            ));
        }
        if text.is_empty() {
            return Ok(KeyEdit::Remove);
        }
        if matches!(key, "hours_saved_per_year" | "value_per_year")
            && text
                .replace([',', '$', ' '], "")
                .parse::<f64>()
                .map_or(true, |n| !n.is_finite())
        {
            return Err(format!("{key} must be a number."));
        }
        if matches!(key, "github_url" | "live_url")
            && !(text.starts_with("https://") || text.starts_with("http://"))
        {
            return Err(format!("{key} must be a web address (https://...)."));
        }
        return Ok(KeyEdit::Quoted(text.to_string()));
    }
    if LIST_KEYS.contains(&key) {
        let items = value
            .as_array()
            .ok_or_else(|| format!("{key} takes a list."))?
            .iter()
            .map(|v| v.as_str().map(|s| s.trim().to_string()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("{key} takes a list of text."))?;
        let items: Vec<String> = items.into_iter().filter(|s| !s.is_empty()).collect();
        if items.len() > MAX_ITEMS || items.iter().any(|s| s.contains('\n') || s.len() > MAX_ITEM) {
            return Err(format!(
                "{key} holds at most {MAX_ITEMS} one-line items of at most {MAX_ITEM} characters."
            ));
        }
        if items.is_empty() {
            return Ok(KeyEdit::Remove);
        }
        let mut lines = vec![format!("{key}:")];
        lines.extend(
            items
                .iter()
                .map(|s| format!("  - {}", serde_json::to_string(s).unwrap())),
        );
        return Ok(KeyEdit::Block(lines));
    }
    Err(format!(
        "{key} isn't a portfolio field. The tracker owns a tracked project's name, status, lead, dates and description."
    ))
}

pub fn write(root: &Path, update: &PortfolioUpdate) -> Result<ProjectPortfolio, String> {
    let id = update.workspace_id.as_str();
    crate::workspaces::folder(root, id)?;
    if !root.join(format!("workspaces/{id}")).is_dir() {
        return Err("That workspace doesn't exist.".into());
    }
    if update.values.is_empty() {
        return Err("Nothing to save.".into());
    }
    let edits = update
        .values
        .iter()
        .map(|(k, v)| edit_for(k, v).map(|e| (k.as_str(), e)))
        .collect::<Result<Vec<_>, _>>()?;
    let rel = rel(id);
    if crate::run_guard::is_protected_target(root, &rel) {
        return Err(format!("{rel} is a protected path"));
    }
    let guard = vault::lock_mutate();
    let old = read_bounded(root, &rel)?;
    let base = old
        .as_deref()
        .map(|t| vault::revision(t.as_bytes()))
        .unwrap_or_default();
    if base != update.expected_revision {
        return Err(format!(
            "{CHANGED}: PROJECT.md changed since it was read; nothing was saved. Read it again."
        ));
    }
    let start = match &old {
        Some(text) => text.clone(),
        None => format!(
            "---\ntitle: {}\n---\n\n# {}\n",
            serde_json::to_string(&workspace_title(root, id)).unwrap(),
            workspace_title(root, id)
        ),
    };
    let next = frontmatter::patch(&start, &edits, None)?;
    let expected = match &old {
        Some(_) => Expected::Revision(&base),
        None => Expected::Absent,
    };
    match vault::write_note_expecting_locked(&guard, root, &rel, &next, expected)? {
        CasOutcome::Written(_) => {}
        CasOutcome::Conflict(_) => {
            return Err(format!(
                "{CHANGED}: PROJECT.md changed while it was being saved; nothing was saved. Read it again."
            ))
        }
    }
    drop(guard);
    Ok(portfolio_of(root, id, Some(&next)))
}

/// Mark a vault-only project as tracked once the tracker made its project
/// ("Track in Linear"): its PROJECT.md (created with the workspace's title
/// when missing) gets the tracker's fields and identity exactly as a pass
/// writes them (`tracker::mirror::project`), so the next pass adopts this
/// file instead of making a new workspace, and finds nothing to send back.
/// Portfolio keys and the rest of the file stay. Idempotent; refuses a file
/// that already names another tracker project.
pub(crate) fn link_tracked(
    root: &Path,
    workspace_id: &str,
    provider: &str,
    project: &crate::tracker::TrackerProject,
) -> Result<(), String> {
    crate::workspaces::folder(root, workspace_id)?;
    if !root.join(format!("workspaces/{workspace_id}")).is_dir() {
        return Err("That workspace doesn't exist any more; Hollow won't recreate it.".into());
    }
    let rel = rel(workspace_id);
    let guard = vault::lock_mutate();
    let old = read_bounded(root, &rel)?;
    if let Some(text) = &old {
        let fm = frontmatter::parse(text);
        match fm
            .get("tracker_project")
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(id) if id == project.id && fm.get("tracker") == Some(provider) => return Ok(()),
            Some(_) => return Err(
                "This project's PROJECT.md already names another tracker project; it was left alone."
                    .into(),
            ),
            None => {}
        }
    }
    let title = workspace_title(root, workspace_id);
    let start = old.clone().unwrap_or_else(|| {
        format!(
            "---\ntitle: {}\n---\n\n# {title}\n",
            serde_json::to_string(&title).unwrap()
        )
    });
    let next = crate::tracker::mirror::project(&start, provider, project)?;
    let base = old.as_deref().map(|t| vault::revision(t.as_bytes()));
    let expected = match &base {
        Some(b) => Expected::Revision(b),
        None => Expected::Absent,
    };
    match vault::write_note_expecting_locked(&guard, root, &rel, &next, expected)? {
        CasOutcome::Written(_) => Ok(()),
        CasOutcome::Conflict(_) => Err("PROJECT.md changed while Hollow was linking it; run Track in the tracker again to finish (it won't make a second project).".into()),
    }
}

/// Whether a workspace's PROJECT.md is already tracked (for refusing a
/// second "Track in" before any request goes out).
pub(crate) fn tracked_id(root: &Path, workspace_id: &str) -> Result<Option<String>, String> {
    crate::workspaces::folder(root, workspace_id)?;
    if !root.join(format!("workspaces/{workspace_id}")).is_dir() {
        return Err("That workspace doesn't exist.".into());
    }
    Ok(read_bounded(root, &rel(workspace_id))?.and_then(|t| {
        frontmatter::parse(&t)
            .get("tracker_project")
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("workspaces/ws-1")).unwrap();
        std::fs::write(
            dir.path().join("workspaces/ws-1/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: \"Benefits Assistant\"\n---\n\n# Benefits Assistant\n",
        )
        .unwrap();
        dir
    }

    fn update(values: serde_json::Value, rev: &str) -> PortfolioUpdate {
        PortfolioUpdate {
            workspace_id: "ws-1".into(),
            values: serde_json::from_value(values).unwrap(),
            expected_revision: rev.into(),
        }
    }

    #[test]
    fn a_vault_only_workspace_reads_empty_then_saves_a_new_project_md() {
        let v = vault();
        let p = read(v.path(), "ws-1").unwrap();
        assert!(!p.exists && p.tracked.is_none() && p.revision.is_empty());
        assert_eq!(p.title, "Benefits Assistant");
        assert_eq!(p.text["department"], "");
        let saved = write(
            v.path(),
            &update(json!({"department": "AI", "hours_saved_per_year": "1,200", "domains": ["library/records/domains/hr.md", " "]}), ""),
        )
        .unwrap();
        assert!(saved.exists);
        assert_eq!(saved.text["department"], "AI");
        assert_eq!(saved.text["hours_saved_per_year"], "1,200");
        assert_eq!(
            saved.lists["domains"],
            vec!["library/records/domains/hr.md"]
        );
        let file = std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap();
        assert!(file.contains("title: \"Benefits Assistant\""));
        assert_eq!(read(v.path(), "ws-1").unwrap(), saved);
    }

    #[test]
    fn a_tracked_project_keeps_the_trackers_keys_and_refuses_them_as_portfolio() {
        let v = vault();
        let text = "---\ntitle: \"Receipts\"\nstatus: \"In Progress\"\ntracker: \"linear\"\ntracker_project: \"p-1\"\ntracker_url: \"https://linear.app/x/project/p-1\"\nlead: \"Alex\"\n---\n\n<!-- tracker:description -->\nFrom the tracker.\n<!-- /tracker:description -->\n";
        std::fs::write(v.path().join("workspaces/ws-1/PROJECT.md"), text).unwrap();
        let p = read(v.path(), "ws-1").unwrap();
        assert_eq!(p.tracked.as_ref().unwrap().project_id, "p-1");
        assert_eq!(p.status.as_deref(), Some("In Progress"));
        assert_eq!(p.description.trim(), "From the tracker.");
        let err = write(v.path(), &update(json!({"status": "Done"}), &p.revision)).unwrap_err();
        assert!(err.contains("isn't a portfolio field"), "{err}");
        let saved = write(
            v.path(),
            &update(json!({"type": "Automation"}), &p.revision),
        )
        .unwrap();
        let file = std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap();
        for kept in [
            "tracker_project: \"p-1\"",
            "lead: \"Alex\"",
            "status: \"In Progress\"",
            "From the tracker.",
        ] {
            assert!(file.contains(kept), "{kept} kept");
        }
        assert_eq!(saved.text["type"], "Automation");
    }

    #[test]
    fn a_stale_revision_bad_numbers_and_bad_urls_are_refused_and_nothing_changes() {
        let v = vault();
        let first = write(v.path(), &update(json!({"department": "AI"}), "")).unwrap();
        let before = std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap();
        assert!(
            write(v.path(), &update(json!({"department": "Ops"}), "stale"))
                .unwrap_err()
                .starts_with(CHANGED)
        );
        assert!(write(
            v.path(),
            &update(json!({"value_per_year": "lots"}), &first.revision)
        )
        .is_err());
        assert!(write(
            v.path(),
            &update(json!({"live_url": "example.com"}), &first.revision)
        )
        .is_err());
        assert!(write(
            v.path(),
            &update(json!({"meetings": "one"}), &first.revision)
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap(),
            before
        );
        // Empty clears.
        let cleared = write(
            v.path(),
            &update(json!({"department": ""}), &first.revision),
        )
        .unwrap();
        assert_eq!(cleared.text["department"], "");
    }

    fn made(id: &str) -> crate::tracker::TrackerProject {
        crate::tracker::TrackerProject {
            id: id.into(),
            key: Some("P-ENG-30".into()),
            url: format!("https://linear.app/x/project/{id}"),
            name: "Benefits Assistant".into(),
            description: "Tracked now.".into(),
            status: crate::tracker::TrackerProjectStatus {
                name: "Backlog".into(),
                kind: "backlog".into(),
            },
            lead: None,
            start_date: None,
            target_date: None,
            team_ids: vec!["team".into()],
            updated_at: "2026-11-10T00:00:00Z".into(),
            progress: None,
            links: Vec::new(),
        }
    }

    #[test]
    fn linking_a_tracked_project_is_idempotent_and_never_relinks() {
        let v = vault();
        std::fs::write(
            v.path().join("workspaces/ws-1/PROJECT.md"),
            "---\ntitle: \"Benefits Assistant\"\nstatus: \"Dropped\"\nnotion_id: \"34db\"\n---\n\nNotes stay.\n",
        )
        .unwrap();
        let p = read(v.path(), "ws-1").unwrap();
        write(v.path(), &update(json!({"department": "AI"}), &p.revision)).unwrap();
        link_tracked(v.path(), "ws-1", "linear", &made("p-9")).unwrap();
        let file = std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap();
        link_tracked(v.path(), "ws-1", "linear", &made("p-9")).unwrap();
        assert_eq!(
            std::fs::read_to_string(v.path().join("workspaces/ws-1/PROJECT.md")).unwrap(),
            file
        );
        let p = read(v.path(), "ws-1").unwrap();
        assert_eq!(p.tracked.unwrap().project_id, "p-9");
        assert_eq!(
            p.status.as_deref(),
            Some("Backlog"),
            "the tracker's status, so a pass finds nothing to send"
        );
        assert_eq!(p.description.trim(), "Tracked now.");
        assert_eq!(p.text["department"], "AI");
        assert!(file.contains("notion_id: \"34db\"") && file.contains("Notes stay."));
        assert_eq!(
            tracked_id(v.path(), "ws-1").unwrap().as_deref(),
            Some("p-9")
        );
        assert!(link_tracked(v.path(), "ws-1", "linear", &made("p-10")).is_err());
        assert!(read(v.path(), "../x").is_err());
        assert!(read(v.path(), "missing").is_err());
        // A PROJECT.md that links to another workspace's is refused.
        std::fs::create_dir_all(v.path().join("workspaces/ws-2")).unwrap();
        std::os::unix::fs::symlink(
            v.path().join("workspaces/ws-1/PROJECT.md"),
            v.path().join("workspaces/ws-2/PROJECT.md"),
        )
        .unwrap();
        assert!(read(v.path(), "ws-2")
            .unwrap_err()
            .contains("symbolic link"));
        assert!(link_tracked(v.path(), "ws-2", "linear", &made("p-11")).is_err());
    }
}
