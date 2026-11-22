//! Project notes. Two locations, one format:
//! - a Notion project's two-way mirror lives inside its own workspace, at
//!   `workspaces/notion-<id>/PROJECT.md` (since 2026-09-26; `notion_sync`
//!   relocates older `projects/notion-<id>.md` mirrors there);
//! - a local project (legacy) lives under `projects/{status}/{name}.md`,
//!   its status in the folder name.
//!
//! [`is_project_rel`] and [`project_files`] are the one definition of where
//! project notes may live; every reader goes through them. YAML frontmatter
//! carries structured fields; the body is free-form markdown.

use crate::types::Project;
use crate::vault;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PROJECTS_DIR: &str = "projects";
const WORKSPACES_DIR: &str = "workspaces";
/// A workspace's own project note (the Notion mirror for `notion-*` ids).
pub const WORKSPACE_PROJECT_FILE: &str = "PROJECT.md";

/// Whether a vault-relative path names a place a project note may live:
/// `projects/**/*.md` or `workspaces/<id>/PROJECT.md`. Rejects traversal,
/// hidden or empty components and backslashes.
pub fn is_project_rel(rel: &str) -> bool {
    if rel.contains(['\\', '\0', '\n', '\r'])
        || rel
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.'))
    {
        return false;
    }
    if rel.starts_with("projects/") && rel.ends_with(".md") {
        return true;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() == 3 && parts[0] == WORKSPACES_DIR && parts[2] == WORKSPACE_PROJECT_FILE
}

/// A project ref (`vault:projects/….md` or `vault:workspaces/<id>/PROJECT.md`)
/// as a vault-relative path to an existing project note (agents' tools).
pub(crate) fn resolve_ref(vault: &Path, reference: &str) -> Result<String, String> {
    let raw = reference.trim();
    let rel = raw.strip_prefix("vault:").unwrap_or(raw);
    if !is_project_rel(rel) {
        return Err(format!(
            "Not a project reference: {raw:?}. Use a project's ref as list_projects returns it. Nothing was written."
        ));
    }
    let full = vault::safe_join(vault, rel)?;
    if !fs::symlink_metadata(&full).is_ok_and(|m| m.is_file()) {
        return Err(format!("No project at {raw}. Nothing was written."));
    }
    Ok(rel.to_string())
}

/// Every project note: `(vault-relative path, absolute path)`, sorted by path.
/// The `projects/` tree (dotfiles skipped) plus each non-hidden workspace's
/// regular-file `PROJECT.md` (a symlink is never followed).
pub(crate) fn project_files(vault: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let root = vault.join(PROJECTS_DIR);
    if root.exists() {
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if !name.ends_with(".md") || name.starts_with('.') {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(vault) else {
                continue;
            };
            out.push((
                rel.to_string_lossy().replace('\\', "/"),
                entry.path().to_path_buf(),
            ));
        }
    }
    if let Ok(entries) = fs::read_dir(vault.join(WORKSPACES_DIR)) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path().join(WORKSPACE_PROJECT_FILE);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
                out.push((
                    format!("{WORKSPACES_DIR}/{id}/{WORKSPACE_PROJECT_FILE}"),
                    path,
                ));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Parsed frontmatter for a project file. Mirrors `Project` plus the
/// fields the UI doesn't surface directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectFrontmatter {
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    /// Real ship date (RFC3339). Stamped when the project enters the
    /// `deployed` status, cleared when it leaves it. See [`stamp_deployed_at`].
    #[serde(default)]
    pub deployed_at: Option<String>,
    /// Optional, local-first outcome contract. Scalar strings keep the vault
    /// format hand-editable while still allowing multi-line text via the
    /// shared frontmatter quoting rules.
    #[serde(default)]
    pub current_state: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub non_goals: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
    /// Comma-separated stable `vault:goals/*.md` URLs in frontmatter.
    #[serde(default)]
    pub goal_urls: Vec<String>,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub notion_id: Option<String>,
    #[serde(default)]
    pub notion_url: Option<String>,
}

/// Keep `deployed_at` honest across status moves: entering `deployed`
/// stamps now (unless a hand-set value already exists); leaving `deployed`
/// clears it so an accidental ship doesn't inflate the weekly velocity.
fn stamp_deployed_at(fm: &mut ProjectFrontmatter) {
    if status_slug(&fm.status) == "deployed" {
        if fm.deployed_at.is_none() {
            fm.deployed_at = Some(Utc::now().to_rfc3339());
        }
    } else {
        fm.deployed_at = None;
    }
}

/// Stable identifier for a project — a `vault:` URI built from the
/// vault-relative path. The frontend keys drag-drop and selection by
/// this field, so it has to be unique per file.
fn stable_url(_fm: &ProjectFrontmatter, rel: &str) -> String {
    format!("vault:{}", rel)
}

/// Status folder convention: lowercase, no spaces. Empty → "discovery".
pub fn status_slug(status: &str) -> String {
    let s = status.trim().to_lowercase();
    if s.is_empty() {
        "discovery".into()
    } else {
        s.replace(' ', "-")
    }
}

/// Title-case the status slug back into a display label. Returns the
/// original frontmatter `status` value untouched if it already has
/// uppercase chars — that way users who hand-edit a project file to set
/// `status: Discovery` keep their casing.
pub fn status_display(status: &str) -> String {
    if status.chars().any(|c| c.is_uppercase()) {
        return status.to_string();
    }
    status
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn safe_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    s.chars().take(200).collect()
}

/// Tolerant reader for the hand-editable comma-separated goal link scalar.
/// Invalid hand-authored links remain visible so alignment can diagnose them;
/// in-app writes go through [`normalize_goal_urls`] and reject malformed URLs.
pub fn parse_goal_urls(value: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_string()))
        .map(str::to_string)
        .collect()
}

pub fn normalize_goal_urls(values: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in values {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let rel = value
            .strip_prefix("vault:")
            .ok_or_else(|| format!("goal link must be a vault URL: {value}"))?;
        let rest = rel
            .strip_prefix("goals/")
            .ok_or_else(|| format!("goal link must stay under goals/: {value}"))?;
        if rest.is_empty()
            || !rest.ends_with(".md")
            || rest.starts_with('.')
            || rest.contains('/')
            || rest.contains('\\')
            || rest.contains("..")
            || rest.chars().any(|c| matches!(c, '\n' | '\r' | '\0'))
        {
            return Err(format!("invalid goal link: {value}"));
        }
        if seen.insert(value.to_string()) {
            out.push(value.to_string());
        }
    }
    Ok(out)
}

/// Every project note ([`project_files`]), parsed, sorted by name.
pub fn list(vault: &Path) -> Vec<Project> {
    let mut out: Vec<Project> = project_files(vault)
        .into_iter()
        .filter_map(|(rel, path)| {
            let content = fs::read_to_string(&path).ok()?;
            parse(&rel, &content, vault)
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Resolve an explicitly selected project without guessing by its display name.
pub fn task_project_link(vault: &Path, url: &str) -> Result<(String, String), String> {
    let rel = url.strip_prefix("vault:").ok_or("Choose a local project")?;
    if !is_project_rel(rel) {
        return Err(
            "Choose a project note (a workspace's PROJECT.md or a note inside projects/)".into(),
        );
    }
    let content = crate::workspaces::read_file(vault, rel)?.content;
    let project = parse(rel, &content, vault).ok_or("Selected project has no title")?;
    Ok((project.name, url.into()))
}

/// Parse a single project markdown file into a `Project`. Returns
/// `None` if the file has no parseable frontmatter title.
pub fn parse(rel: &str, content: &str, _vault: &Path) -> Option<Project> {
    let (fm_map, body) = split_frontmatter(content);
    let title = fm_map.get("title")?.clone();
    if title.is_empty() {
        return None;
    }

    // Linked Notion notes use mirrored frontmatter status without moving files.
    // Local-only projects retain the legacy status-folder convention.
    let folder_status = rel
        .strip_prefix(&format!("{PROJECTS_DIR}/"))
        .and_then(|s| s.split('/').next())
        .filter(|s| !s.is_empty() && !s.ends_with(".md"))
        .map(|s| s.to_string());
    let status_raw = if fm_map.contains_key("notion_id") {
        fm_map.get("status").cloned().or(folder_status)
    } else {
        folder_status.or_else(|| fm_map.get("status").cloned())
    }
    .unwrap_or_else(|| "discovery".into());
    let status = status_display(&status_raw);

    let description = fm_map
        .get("description")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| body_excerpt(&body));

    let fm = ProjectFrontmatter {
        title: title.clone(),
        status: status_raw,
        description: description.clone(),
        created: fm_map.get("created").cloned().filter(|s| !s.is_empty()),
        priority: fm_map.get("priority").cloned().filter(|s| !s.is_empty()),
        target_date: fm_map.get("target_date").cloned().filter(|s| !s.is_empty()),
        deployed_at: fm_map.get("deployed_at").cloned().filter(|s| !s.is_empty()),
        current_state: fm_map
            .get("current_state")
            .cloned()
            .filter(|s| !s.is_empty()),
        desired_state: fm_map
            .get("desired_state")
            .cloned()
            .filter(|s| !s.is_empty()),
        non_goals: fm_map.get("non_goals").cloned().filter(|s| !s.is_empty()),
        success_evidence: fm_map
            .get("success_evidence")
            .cloned()
            .filter(|s| !s.is_empty()),
        goal_urls: parse_goal_urls(fm_map.get("goal_urls").map(String::as_str).unwrap_or("")),
        last_synced_at: fm_map
            .get("last_synced_at")
            .cloned()
            .filter(|s| !s.is_empty()),
        source: fm_map.get("source").cloned().filter(|s| !s.is_empty()),
        notion_id: fm_map.get("notion_id").cloned().filter(|s| !s.is_empty()),
        notion_url: fm_map.get("notion_url").cloned().filter(|s| !s.is_empty()),
    };

    let url = stable_url(&fm, rel);

    Some(Project {
        url,
        name: title,
        status,
        description,
        created: fm.created,
        vault_path: Some(rel.to_string()),
        priority: fm
            .priority
            .as_deref()
            .and_then(crate::vault_tasks::normalize_priority),
        target_date: fm.target_date,
        deployed_at: fm.deployed_at,
        current_state: fm.current_state,
        desired_state: fm.desired_state,
        non_goals: fm.non_goals,
        success_evidence: fm.success_evidence,
        goal_urls: fm.goal_urls,
    })
}

#[cfg(test)]
pub fn notion_id(vault: &Path, rel: &str) -> Option<String> {
    let full = vault::safe_join(vault, rel).ok()?;
    let content = fs::read_to_string(&full).ok()?;
    let (fm_map, _) = split_frontmatter(&content);
    fm_map.get("notion_id").cloned().filter(|s| !s.is_empty())
}

/// First non-blank, non-link paragraph from the body — used as the
/// project blurb when frontmatter `description:` is empty. Capped at
/// ~280 chars so it fits comfortably in the project card.
fn body_excerpt(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let trim = line.trim();
        if trim.is_empty() {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if trim.starts_with('[') && trim.contains("](") {
            // skip standalone markdown link lines
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(trim);
        if out.len() > 280 {
            break;
        }
    }
    if out.len() > 280 {
        let cut = out
            .char_indices()
            .take_while(|(i, _)| *i < 280)
            .last()
            .map(|(i, _)| i + 1)
            .unwrap_or(280);
        let mut t = out[..cut].to_string();
        t.push('…');
        t
    } else {
        out
    }
}

/// Find the project file with the given stable URL. Every project is
/// keyed by its `vault:` URI now. Returns the vault-relative path.
pub fn find_by_url(vault: &Path, url: &str) -> Option<String> {
    let rel = url.strip_prefix("vault:")?;
    // Resolve through safe_join so a crafted `vault:../…` URL can't
    // escape the vault root — reject it rather than hand back a
    // traversal path that a later set_status/delete would act on.
    let full = vault::safe_join(vault, rel).ok()?;
    if full.exists() {
        Some(rel.to_string())
    } else {
        None
    }
}

/// Crash-safe relocation of a project file to `dest_rel` with new content.
///
/// `content` is rendered first by the caller. We then **write the new
/// content to the destination, and only afterwards remove the old file** —
/// the reverse of a rename-then-rewrite. A crash mid-operation therefore
/// leaves a recoverable duplicate (old + new copies) rather than a file
/// stranded at the new path carrying the *old* status frontmatter.
///
/// Both paths are routed through `safe_join` so a `vault:`-sourced or
/// otherwise attacker-influenced path can't escape the vault. Everything runs
/// under the vault mutate lock, and only while the source still holds
/// `original` (the bytes the edit was planned from), so an edit that landed
/// after planning is refused rather than deleted with the old file. The
/// destination is published exclusively ([`vault::Expected::Absent`]): a
/// file already there, even one another program created a moment ago, is
/// never replaced (merged plan slice 4, A·D8; this used to check `exists()`
/// and then overwrite).
fn move_project_file(
    vault: &Path,
    src_rel: &str,
    dest_rel: &str,
    content: &str,
    original: &str,
) -> Result<(), String> {
    let src_full = vault::safe_join(vault, src_rel)?;
    let guard = vault::lock_mutate();
    let current = match fs::read_to_string(&src_full) {
        Ok(text) => text,
        Err(e) => return Err(format!("read {src_rel}: {e}")),
    };
    if current != original {
        return Err(format!(
            "{src_rel} changed while it was being moved, so nothing was moved; try again"
        ));
    }
    let written = vault::write_note_expecting_locked(
        &guard,
        vault,
        dest_rel,
        content,
        vault::Expected::Absent,
    )?;
    if let vault::CasOutcome::Conflict(_) = written {
        return Err(format!("destination already exists: {dest_rel}"));
    }
    // Remove the OLD file LAST. A failure here leaves a recoverable
    // duplicate, never a corrupted-status file.
    fs::remove_file(&src_full).map_err(|e| format!("remove old: {e}"))?;
    Ok(())
}

/// Whether the legacy edit helpers must leave this note where it is and read
/// its status from frontmatter: a workspace's own `PROJECT.md` (including a
/// legacy `notion-*` workspace's) or any note carrying a legacy `notion_id`
/// (an older `projects/notion-<id>.md` mirror, left where it is). Matches
/// how [`parse`] reads status. Everything else is a local project kept in `projects/{status}/`.
fn pinned_in_place(rel: &str, fm_map: &BTreeMap<String, String>) -> bool {
    rel.starts_with(&format!("{WORKSPACES_DIR}/"))
        || fm_map.get("notion_id").is_some_and(|id| !id.is_empty())
}

/// The modelled scalar keys, as they would be written for `fm`. A pinned
/// note keeps its status verbatim (a Notion option name such as `In
/// progress`); a local project writes the folder slug, as [`render`] does.
fn scalar_fields(fm: &ProjectFrontmatter, pinned: bool) -> Vec<(&'static str, Option<String>)> {
    fn text(value: &str) -> Option<String> {
        (!value.is_empty()).then(|| value.to_string())
    }
    let status = if pinned {
        text(fm.status.trim())
    } else {
        Some(status_slug(&fm.status))
    };
    let goal_urls = (!fm.goal_urls.is_empty()).then(|| fm.goal_urls.join(", "));
    vec![
        ("title", text(&fm.title)),
        ("status", status),
        ("priority", fm.priority.clone()),
        ("created", fm.created.clone()),
        ("target_date", fm.target_date.clone()),
        ("deployed_at", fm.deployed_at.clone()),
        // A pinned mirror keeps an empty description as `""` (the Notion
        // summary key): removing it would read back as the sync baseline, so
        // a cleared description would never reach Notion.
        (
            "description",
            if pinned {
                Some(fm.description.clone())
            } else {
                text(&fm.description)
            },
        ),
        ("current_state", fm.current_state.clone()),
        ("desired_state", fm.desired_state.clone()),
        ("non_goals", fm.non_goals.clone()),
        ("success_evidence", fm.success_evidence.clone()),
        ("goal_urls", goal_urls),
        ("source", fm.source.clone()),
        ("notion_id", fm.notion_id.clone()),
        ("notion_url", fm.notion_url.clone()),
        ("last_synced_at", fm.last_synced_at.clone()),
    ]
}

/// `Set` / `Remove` edits for exactly the modelled keys whose value differs
/// between `old` (parsed from the file) and `new`. Every other line, unknown
/// keys included, is left to [`crate::frontmatter::patch`] to keep verbatim.
fn scalar_edits(
    old: &ProjectFrontmatter,
    new: &ProjectFrontmatter,
    pinned: bool,
) -> Vec<(&'static str, crate::frontmatter::KeyEdit)> {
    use crate::frontmatter::KeyEdit;
    scalar_fields(old, pinned)
        .into_iter()
        .zip(scalar_fields(new, pinned))
        .filter(|((_, before), (_, after))| before != after)
        .map(|(_, (key, after))| match after {
            Some(value) => (key, KeyEdit::Set(value)),
            None => (key, KeyEdit::Remove),
        })
        .collect()
}

/// The derived `tags:` block for a local project whose status or priority
/// moved: fresh `status/…` and `priority/…` items, then every other existing
/// item in order. `None` (leave the key alone) when `tags` is a scalar or
/// inline list this cannot rewrite without losing it.
fn tag_edit(
    content: &str,
    fm_map: &BTreeMap<String, String>,
    fm: &ProjectFrontmatter,
) -> Option<crate::frontmatter::KeyEdit> {
    let existing = match crate::frontmatter::list_items(content, "tags") {
        Some(items) => items,
        None if fm_map.contains_key("tags") => return None,
        None => vec!["projects".to_string()],
    };
    fn item(tag: &str) -> String {
        if crate::frontmatter::needs_quoting(tag) {
            format!("  - {}", crate::frontmatter::quote(tag))
        } else {
            format!("  - {tag}")
        }
    }
    let mut lines = vec![
        "tags:".to_string(),
        item(&format!("status/{}", status_slug(&fm.status))),
    ];
    if let Some(p) = fm.priority.as_deref() {
        let slug = p.to_lowercase().replace(' ', "-");
        lines.push(item(&format!("priority/{slug}")));
    }
    lines.extend(
        existing
            .iter()
            .filter(|tag| !tag.starts_with("status/") && !tag.starts_with("priority/"))
            .map(|tag| item(tag.as_str())),
    );
    Some(crate::frontmatter::KeyEdit::Block(lines))
}

/// Plan one legacy edit against `content`: apply `edit`, then return the
/// line-patched file and the vault-relative path it belongs at. A pinned
/// note ([`pinned_in_place`]) always stays at `rel`, keeps its status
/// verbatim and its sync metadata (`last_synced_at`, and `deployed_at`
/// unless the status moved). A local project gets the legacy treatment:
/// `deployed_at` stamping, a fresh `last_synced_at`, refreshed status and
/// priority tags, and the `projects/{status}/` folder, keeping its filename
/// unless `rename_to_title`. Only changed keys are rewritten.
fn plan_edit(
    rel: &str,
    content: &str,
    rename_to_title: bool,
    edit: &dyn Fn(&mut ProjectFrontmatter) -> Result<(), String>,
) -> Result<(String, String), String> {
    let (fm_map, _) = split_frontmatter(content);
    let pinned = pinned_in_place(rel, &fm_map);
    let old = frontmatter_from_map(&fm_map);
    let mut new = old.clone();
    edit(&mut new)?;
    if new.title.trim().is_empty() {
        return Err(format!("project file has no title: {rel}"));
    }
    if !pinned || old.status.trim() != new.status.trim() {
        stamp_deployed_at(&mut new);
    }
    let target_rel = if pinned {
        rel.to_string()
    } else {
        new.last_synced_at = Some(Utc::now().to_rfc3339());
        let filename = Path::new(rel)
            .file_name()
            .filter(|_| !rename_to_title)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.md", safe_filename(&new.title)));
        format!("{PROJECTS_DIR}/{}/{filename}", status_slug(&new.status))
    };
    let mut edits = scalar_edits(&old, &new, pinned);
    if !pinned
        && (status_slug(&old.status) != status_slug(&new.status) || old.priority != new.priority)
    {
        if let Some(tags) = tag_edit(content, &fm_map, &new) {
            edits.push(("tags", tags));
        }
    }
    let patched = crate::frontmatter::patch(content, &edits, None)?;
    Ok((patched, target_rel))
}

/// A project edit with a [`crate::vault_tasks::Finish`] step over the
/// patched text, run inside the same `mutate_note` (or before a status-folder
/// move): the task broker passes its compare-and-swap check and, for a Notion
/// mirror, `notion_sync::conform_linked_edit`.
fn edit_project_finishing(
    vault: &Path,
    rel: &str,
    rename_to_title: bool,
    edit: impl Fn(&mut ProjectFrontmatter) -> Result<(), String>,
    finish: crate::vault_tasks::Finish,
) -> Result<String, String> {
    if crate::tracker_sync::seams::tracked(vault, rel) {
        return Err("This project belongs to the task tracker. Use its pinned tracker editor; no legacy fields were rewritten.".into());
    }
    if !is_project_rel(rel) {
        return Err(format!("not a project note: {rel}"));
    }
    let mut relocate: Option<(String, String, String)> = None;
    let written = vault::mutate_note(vault, rel, |content| {
        let (patched, target_rel) = plan_edit(rel, content, rename_to_title, &edit)?;
        let patched = finish(content, patched)?;
        if target_rel == rel {
            return Ok(patched);
        }
        relocate = Some((patched, target_rel, content.to_string()));
        Err("relocating".into())
    });
    match (written, relocate) {
        (_, Some((patched, target_rel, original))) => {
            move_project_file(vault, rel, &target_rel, &patched, &original)?;
            Ok(target_rel)
        }
        (Ok(_), None) => Ok(rel.to_string()),
        (Err(error), None) => Err(error),
    }
}

/// Change the status of an existing project. A local project moves into the
/// new status folder, renamed to match its title; a Notion mirror (or any
/// [`pinned_in_place`] note) is patched where it is. Returns the new
/// vault-relative path.
pub fn set_status(vault: &Path, rel: &str, new_status: &str) -> Result<String, String> {
    set_status_finishing(vault, rel, new_status, &|_, text| Ok(text))
}

/// [`set_status`] with a [`crate::vault_tasks::Finish`] step.
pub(crate) fn set_status_finishing(
    vault: &Path,
    rel: &str,
    new_status: &str,
    finish: crate::vault_tasks::Finish,
) -> Result<String, String> {
    let new_status = new_status.trim();
    if new_status.is_empty() {
        return Err("Choose a project status".into());
    }
    edit_project_finishing(
        vault,
        rel,
        true,
        |fm| {
            fm.status = new_status.to_string();
            Ok(())
        },
        finish,
    )
}

/// Apply an in-app edit to an existing project. Each `Some` field on the
/// input overwrites that frontmatter value (`Some("")` clears the nullable
/// ones); `None` leaves it untouched. Only the keys that change are
/// rewritten; every other frontmatter line and the body are kept verbatim.
///
/// A local project is relocated to the status folder when `status` changes.
/// Unlike [`set_status`] (which renames the file to match the title), this
/// **preserves the on-disk filename** — editing a project's priority or
/// description never silently renames or churns the file's `vault:` URL. The
/// frontmatter `title` carries the display name regardless of the filename.
/// A Notion mirror (or any [`pinned_in_place`] note) never moves.
/// Returns the new vault-relative path (unchanged unless the status moved).
pub fn update(
    vault: &Path,
    rel: &str,
    input: &crate::types::UpdateProjectInput,
) -> Result<String, String> {
    update_finishing(vault, rel, input, &|_, text| Ok(text))
}

/// [`update`] with a [`crate::vault_tasks::Finish`] step.
pub(crate) fn update_finishing(
    vault: &Path,
    rel: &str,
    input: &crate::types::UpdateProjectInput,
    finish: crate::vault_tasks::Finish,
) -> Result<String, String> {
    let edit = |fm: &mut ProjectFrontmatter| {
        if let Some(t) = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            fm.title = t.to_string();
        }
        if let Some(s) = input
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            fm.status = s.to_string();
        }
        // Nullable fields: `Some("")` clears, `Some(v)` sets, `None` leaves as-is.
        if let Some(p) = input.priority.as_deref() {
            fm.priority = crate::vault_tasks::normalize_priority(p);
        }
        if let Some(d) = input.target_date.as_deref() {
            let d = d.trim();
            fm.target_date = (!d.is_empty()).then(|| d.to_string());
        }
        if let Some(desc) = input.description.as_deref() {
            fm.description = desc.trim().to_string();
        }
        apply_outcome_fields(fm, input)
    };
    edit_project_finishing(vault, rel, false, edit, finish)
}

/// The outcome contract and goal links from `input`, shared by [`update`]
/// and [`update_local_fields`].
fn apply_outcome_fields(
    fm: &mut ProjectFrontmatter,
    input: &crate::types::UpdateProjectInput,
) -> Result<(), String> {
    if let Some(v) = input.current_state.as_deref() {
        fm.current_state = optional_text(v);
    }
    if let Some(v) = input.desired_state.as_deref() {
        fm.desired_state = optional_text(v);
    }
    if let Some(v) = input.non_goals.as_deref() {
        fm.non_goals = optional_text(v);
    }
    if let Some(v) = input.success_evidence.as_deref() {
        fm.success_evidence = optional_text(v);
    }
    if let Some(values) = input.goal_urls.as_ref() {
        fm.goal_urls = normalize_goal_urls(values)?;
    }
    Ok(())
}

/// Update only a project's outcome and goal fields without changing its
/// location, status, deployment timestamp, or historical import timestamp.
/// Line-patched under the vault mutate lock, so every other key survives.
/// Retained for compatibility; the main editor uses the complete local update.
#[cfg(test)]
pub fn update_local_fields(
    vault: &Path,
    rel: &str,
    input: &crate::types::UpdateProjectInput,
) -> Result<(), String> {
    if crate::tracker_sync::seams::tracked(vault, rel) {
        return Err(
            "Use the pinned tracker project editor for local fields on a tracker mirror.".into(),
        );
    }
    if !is_project_rel(rel) {
        return Err(format!("not a project note: {rel}"));
    }
    vault::mutate_note(vault, rel, |content| {
        let (fm_map, _) = split_frontmatter(content);
        let old = frontmatter_from_map(&fm_map);
        let mut new = old.clone();
        apply_outcome_fields(&mut new, input)?;
        let edits = scalar_edits(&old, &new, pinned_in_place(rel, &fm_map));
        crate::frontmatter::patch(content, &edits, None)
    })?;
    Ok(())
}

/// Create a new project file at `projects/{status}/{title}.md`, choosing a
/// non-colliding filename (` 2`, ` 3`, … suffix on collision). Returns the
/// vault-relative path. The in-app create flow uses this; before this,
/// projects were authored directly in the vault.
pub fn create(vault: &Path, fm: ProjectFrontmatter) -> Result<String, String> {
    create_keyed(vault, fm, None)
}

/// A new project's frontmatter from the create form (the Tasks page) or an
/// agent's `create_project`: status defaults to `discovery`, priority is
/// normalized, empty outcome fields are left out, `created` and
/// `last_synced_at` are now.
pub fn frontmatter_for_create(
    input: &crate::types::CreateProjectInput,
) -> Result<ProjectFrontmatter, String> {
    let now = Utc::now().to_rfc3339();
    let kept = |value: &Option<String>| value.clone().filter(|s| !s.trim().is_empty());
    Ok(ProjectFrontmatter {
        title: input.title.clone(),
        status: input
            .status
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "discovery".into()),
        description: input.description.clone().unwrap_or_default(),
        created: Some(now.clone()),
        priority: input
            .priority
            .as_deref()
            .and_then(crate::vault_tasks::normalize_priority),
        target_date: input.target_date.clone().filter(|s| !s.is_empty()),
        deployed_at: None,
        current_state: kept(&input.current_state),
        desired_state: kept(&input.desired_state),
        non_goals: kept(&input.non_goals),
        success_evidence: kept(&input.success_evidence),
        goal_urls: normalize_goal_urls(&input.goal_urls)?,
        last_synced_at: Some(now),
        source: None,
        notion_id: None,
        notion_url: None,
    })
}

/// The frontmatter key an agent-created project carries its scoped
/// idempotency key in ([`find_by_agent_key`]).
pub(crate) const AGENT_KEY: &str = "agent_key";

/// [`create`], writing `agent_key` into the same exclusive create so a
/// retried agent create finds the project ([`find_by_agent_key`]).
pub(crate) fn create_keyed(
    vault: &Path,
    mut fm: ProjectFrontmatter,
    agent_key: Option<&str>,
) -> Result<String, String> {
    stamp_deployed_at(&mut fm);
    let folder = status_slug(&fm.status);
    let base = safe_filename(fm.title.trim());
    if base.trim().is_empty() {
        return Err("project title is empty".into());
    }
    let mut content = render(&fm, "");
    if let Some(key) = agent_key {
        content = crate::frontmatter::patch(
            &content,
            &[(
                AGENT_KEY,
                crate::frontmatter::KeyEdit::Quoted(key.to_string()),
            )],
            None,
        )?;
    }
    for suffix in 1..1000 {
        let rel = if suffix == 1 {
            format!("{PROJECTS_DIR}/{folder}/{base}.md")
        } else {
            format!("{PROJECTS_DIR}/{folder}/{base} {suffix}.md")
        };
        // Exclusive publication also protects against an Obsidian create
        // landing after filename selection; never replace that writer's note.
        match vault::create_note(vault, &rel, &content) {
            Ok(_) => return Ok(rel),
            Err(error) if error.starts_with("Note already exists:") => continue,
            Err(error) => return Err(error),
        }
    }
    Err("too many project filename collisions".into())
}

/// The project an agent created with scoped key `key`, if any.
/// The project an agent created with scoped key `key` (an `agent_key` of
/// `<key>` or `<key>#<work>`), with the `<work>` part it stored.
pub(crate) fn find_by_agent_key(vault: &Path, key: &str) -> Option<(String, String)> {
    project_files(vault).into_iter().find_map(|(rel, path)| {
        let content = fs::read_to_string(&path).ok()?;
        let (fm, _) = split_frontmatter(&content);
        let stored = fm.get(AGENT_KEY)?;
        let (stored_key, work) = stored.split_once('#').unwrap_or((stored.as_str(), ""));
        (stored_key == key).then(|| (rel, work.to_string()))
    })
}

fn frontmatter_from_map(m: &BTreeMap<String, String>) -> ProjectFrontmatter {
    ProjectFrontmatter {
        title: m.get("title").cloned().unwrap_or_default(),
        status: m.get("status").cloned().unwrap_or_default(),
        description: m.get("description").cloned().unwrap_or_default(),
        created: m.get("created").cloned().filter(|s| !s.is_empty()),
        priority: m.get("priority").cloned().filter(|s| !s.is_empty()),
        target_date: m.get("target_date").cloned().filter(|s| !s.is_empty()),
        deployed_at: m.get("deployed_at").cloned().filter(|s| !s.is_empty()),
        current_state: m.get("current_state").cloned().filter(|s| !s.is_empty()),
        desired_state: m.get("desired_state").cloned().filter(|s| !s.is_empty()),
        non_goals: m.get("non_goals").cloned().filter(|s| !s.is_empty()),
        success_evidence: m.get("success_evidence").cloned().filter(|s| !s.is_empty()),
        goal_urls: parse_goal_urls(m.get("goal_urls").map(String::as_str).unwrap_or("")),
        last_synced_at: m.get("last_synced_at").cloned().filter(|s| !s.is_empty()),
        source: m.get("source").cloned().filter(|s| !s.is_empty()),
        notion_id: m.get("notion_id").cloned().filter(|s| !s.is_empty()),
        notion_url: m.get("notion_url").cloned().filter(|s| !s.is_empty()),
    }
}

/// Render frontmatter + body to the canonical markdown shape we write.
pub fn render(fm: &ProjectFrontmatter, body: &str) -> String {
    let folder = status_slug(&fm.status);
    let mut lines = vec!["---".to_string()];
    push_kv(&mut lines, "title", &fm.title);
    push_kv(&mut lines, "status", &folder);
    if let Some(p) = fm.priority.as_deref() {
        push_kv(&mut lines, "priority", p);
    }
    if let Some(c) = fm.created.as_deref() {
        push_kv(&mut lines, "created", c);
    }
    if let Some(t) = fm.target_date.as_deref() {
        push_kv(&mut lines, "target_date", t);
    }
    if let Some(d) = fm.deployed_at.as_deref() {
        push_kv(&mut lines, "deployed_at", d);
    }
    if !fm.description.is_empty() {
        push_kv(&mut lines, "description", &fm.description);
    }
    if let Some(v) = fm.current_state.as_deref() {
        push_kv(&mut lines, "current_state", v);
    }
    if let Some(v) = fm.desired_state.as_deref() {
        push_kv(&mut lines, "desired_state", v);
    }
    if let Some(v) = fm.non_goals.as_deref() {
        push_kv(&mut lines, "non_goals", v);
    }
    if let Some(v) = fm.success_evidence.as_deref() {
        push_kv(&mut lines, "success_evidence", v);
    }
    if !fm.goal_urls.is_empty() {
        push_kv(&mut lines, "goal_urls", &fm.goal_urls.join(", "));
    }
    if let Some(source) = fm.source.as_deref() {
        push_kv(&mut lines, "source", source);
    }
    if let Some(id) = fm.notion_id.as_deref() {
        push_kv(&mut lines, "notion_id", id);
    }
    if let Some(url) = fm.notion_url.as_deref() {
        push_kv(&mut lines, "notion_url", url);
    }
    let stamp = fm
        .last_synced_at
        .clone()
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    push_kv(&mut lines, "last_synced_at", &stamp);
    lines.push("tags:".into());
    lines.push(format!("  - status/{}", folder));
    if let Some(p) = fm.priority.as_deref() {
        let slug = p.to_lowercase().replace(' ', "-");
        lines.push(format!("  - priority/{slug}"));
    }
    lines.push("  - projects".into());
    lines.push("---".into());
    lines.push(String::new());
    let trimmed = body.trim();
    if !trimmed.is_empty() {
        lines.push(trimmed.to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn push_kv(lines: &mut Vec<String>, key: &str, value: &str) {
    crate::frontmatter::push_kv(lines, key, value);
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Split a markdown file into (frontmatter map, body) — body excludes
/// the closing `---` line. Tolerates files without frontmatter (whole
/// thing is body). Thin wrapper over the shared [`crate::frontmatter`]
/// parser; kept as a `pub` helper because callers and tests use it.
pub fn split_frontmatter(content: &str) -> (BTreeMap<String, String>, String) {
    crate::frontmatter::parse_map(content)
}

/// Build PathBuf for a project's vault file given title + status. Used
/// by tests.
#[cfg(test)]
pub fn path_for(title: &str, status: &str) -> PathBuf {
    let folder = status_slug(status);
    let filename = format!("{}.md", safe_filename(title));
    PathBuf::from(format!("{PROJECTS_DIR}/{folder}/{filename}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> TempDir {
        TempDir::new().unwrap()
    }

    /// Lay down a fresh project file at `projects/{status}/{title}.md`.
    /// Production has no create-project flow — projects are authored in
    /// the vault directly — so this is a test-only fixture builder.
    fn write_project(vault: &Path, fm: ProjectFrontmatter, body: Option<&str>) -> String {
        let rel = path_for(&fm.title, &fm.status)
            .to_string_lossy()
            .to_string();
        let content = render(&fm, body.unwrap_or(""));
        vault::write_note(vault, &rel, &content).unwrap();
        rel
    }

    #[test]
    fn concurrent_project_creates_preserve_every_unique_description() {
        let dir = tempfile::TempDir::new().unwrap();
        let barrier = std::sync::Barrier::new(8);
        let paths = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let root = dir.path();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        create(
                            root,
                            ProjectFrontmatter {
                                title: "Shared title".into(),
                                status: "active".into(),
                                description: format!("Unique description {index}"),
                                ..Default::default()
                            },
                        )
                        .unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 8);
        let descriptions: std::collections::HashSet<_> = paths
            .iter()
            .map(|rel| {
                let content = std::fs::read_to_string(dir.path().join(rel)).unwrap();
                split_frontmatter(&content).0["description"].clone()
            })
            .collect();
        assert_eq!(
            descriptions,
            (0..8)
                .map(|index| format!("Unique description {index}"))
                .collect()
        );
    }

    #[test]
    fn parse_round_trip() {
        let fm = ProjectFrontmatter {
            title: "My Project".into(),
            status: "Active".into(),
            description: "A short blurb".into(),
            created: Some("2026-01-15T00:00:00Z".into()),
            priority: Some("High".into()),
            target_date: Some("2026-06-30".into()),
            deployed_at: None,
            current_state: Some("Manual handoffs create drift".into()),
            desired_state: Some("One verified source of truth".into()),
            non_goals: Some("Replace every upstream system".into()),
            success_evidence: Some("A contributor can verify the flow end to end".into()),
            goal_urls: vec!["vault:goals/reliable-operations.md".into()],
            last_synced_at: Some("2026-05-27T10:00:00Z".into()),
            source: None,
            notion_id: None,
            notion_url: None,
        };
        let rendered = render(&fm, "Long-form body here.");
        let (map, body) = split_frontmatter(&rendered);
        assert_eq!(map.get("title"), Some(&"My Project".to_string()));
        assert_eq!(map.get("status"), Some(&"active".to_string()));
        assert_eq!(map.get("priority"), Some(&"High".to_string()));
        assert_eq!(
            map.get("current_state"),
            Some(&"Manual handoffs create drift".to_string())
        );
        assert_eq!(
            map.get("desired_state"),
            Some(&"One verified source of truth".to_string())
        );
        assert_eq!(
            map.get("non_goals"),
            Some(&"Replace every upstream system".to_string())
        );
        assert_eq!(
            map.get("success_evidence"),
            Some(&"A contributor can verify the flow end to end".to_string())
        );
        assert_eq!(
            map.get("goal_urls"),
            Some(&"vault:goals/reliable-operations.md".to_string())
        );
        assert!(body.contains("Long-form body here."));
        // Notion mirror metadata is omitted unless a connector explicitly sets it.
        assert!(!rendered.contains("notion"));
    }

    #[test]
    fn legacy_project_render_is_byte_compatible_without_outcome_fields() {
        let rendered = render(
            &ProjectFrontmatter {
                title: "Legacy".into(),
                status: "Active".into(),
                description: "Existing shape".into(),
                last_synced_at: Some("2026-05-27T10:00:00Z".into()),
                ..Default::default()
            },
            "Body stays put.",
        );
        assert_eq!(
            rendered,
            "---\ntitle: Legacy\nstatus: active\ndescription: Existing shape\nlast_synced_at: \"2026-05-27T10:00:00Z\"\ntags:\n  - status/active\n  - projects\n---\n\nBody stays put.\n",
        );
        let project = parse("projects/active/Legacy.md", &rendered, Path::new("/tmp")).unwrap();
        assert_eq!(project.current_state, None);
        assert_eq!(project.desired_state, None);
        assert_eq!(project.non_goals, None);
        assert_eq!(project.success_evidence, None);
    }

    #[test]
    fn notion_metadata_round_trips_when_present() {
        let dir = vault();
        let rel = create(
            dir.path(),
            ProjectFrontmatter {
                title: "Synced Project".into(),
                status: "Active".into(),
                source: Some("notion".into()),
                notion_id: Some("proj123".into()),
                notion_url: Some("https://notion.so/proj123".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(notion_id(dir.path(), &rel).as_deref(), Some("proj123"));
        let p = parse(
            &rel,
            &fs::read_to_string(dir.path().join(&rel)).unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(p.name, "Synced Project");
    }

    #[test]
    fn write_creates_in_status_folder() {
        let dir = vault();
        let fm = ProjectFrontmatter {
            title: "Test".into(),
            status: "Active".into(),
            ..Default::default()
        };
        let rel = write_project(dir.path(), fm, Some("body"));
        assert_eq!(rel, "projects/active/Test.md");
        assert!(dir.path().join(&rel).exists());
    }

    #[test]
    fn list_returns_status_from_folder() {
        let dir = vault();
        write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Alpha".into(),
                status: "Active".into(),
                ..Default::default()
            },
            None,
        );
        write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Beta".into(),
                status: "Backlog".into(),
                ..Default::default()
            },
            None,
        );
        let items = list(dir.path());
        assert_eq!(items.len(), 2);
        let alpha = items.iter().find(|p| p.name == "Alpha").unwrap();
        let beta = items.iter().find(|p| p.name == "Beta").unwrap();
        assert_eq!(alpha.status, "Active");
        assert_eq!(beta.status, "Backlog");
    }

    #[test]
    fn project_gets_vault_url() {
        let dir = vault();
        write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Local Only".into(),
                status: "Discovery".into(),
                ..Default::default()
            },
            None,
        );
        let items = list(dir.path());
        assert_eq!(items.len(), 1);
        assert!(items[0].url.starts_with("vault:projects/"));
    }

    #[test]
    fn set_status_moves_file() {
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Mover".into(),
                status: "Discovery".into(),
                ..Default::default()
            },
            None,
        );
        let new_rel = set_status(dir.path(), &rel, "Active").unwrap();
        assert_eq!(new_rel, "projects/active/Mover.md");
        assert!(!dir.path().join(&rel).exists());
        assert!(dir.path().join(&new_rel).exists());
    }

    #[test]
    fn find_by_url_rejects_path_escape() {
        let dir = vault();
        // A crafted vault: URL must not resolve outside the vault, even
        // when the target exists on disk.
        assert_eq!(find_by_url(dir.path(), "vault:../outside.md"), None);
        assert_eq!(find_by_url(dir.path(), "vault:../../etc/hosts"), None);
        assert_eq!(find_by_url(dir.path(), "vault:/etc/hosts"), None);
    }

    #[test]
    fn set_status_rejects_path_escape() {
        let dir = vault();
        assert!(set_status(dir.path(), "../evil.md", "Active").is_err());
        assert!(set_status(dir.path(), "/etc/passwd", "Active").is_err());
    }

    #[test]
    fn set_status_destination_carries_new_status_not_old() {
        // Crash-safety regression guard (BE-12): the destination file must
        // always hold the NEW status. The old non-atomic path could leave a
        // file at the new location still tagged with the old status.
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Shifter".into(),
                status: "Discovery".into(),
                ..Default::default()
            },
            Some("keep this body"),
        );
        let new_rel = set_status(dir.path(), &rel, "Active").unwrap();
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, body) = split_frontmatter(&content);
        assert_eq!(map.get("status"), Some(&"active".to_string()));
        assert!(body.contains("keep this body"), "body preserved on move");
        // No stale duplicate left behind at the old path.
        assert!(!dir.path().join(&rel).exists());
    }

    #[test]
    fn set_status_same_status_overwrites_in_place() {
        // When the status doesn't change folders, the file stays put and is
        // rewritten in place (no spurious removal).
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "StayPut".into(),
                status: "Active".into(),
                ..Default::default()
            },
            Some("body"),
        );
        let same = set_status(dir.path(), &rel, "Active").unwrap();
        assert_eq!(same, rel);
        assert!(dir.path().join(&rel).exists());
    }

    fn upd(rel: &str) -> crate::types::UpdateProjectInput {
        crate::types::UpdateProjectInput {
            url: format!("vault:{rel}"),
            ..Default::default()
        }
    }

    #[test]
    fn update_in_place_preserves_filename_and_body() {
        // Editing non-status fields rewrites the file in place — no rename,
        // no folder move — and keeps the body intact.
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Edit Me".into(),
                status: "Active".into(),
                ..Default::default()
            },
            Some("Body kept on edit."),
        );
        let new_rel = update(
            dir.path(),
            &rel,
            &crate::types::UpdateProjectInput {
                priority: Some("P1".into()),
                description: Some("New blurb".into()),
                ..upd(&rel)
            },
        )
        .unwrap();
        assert_eq!(new_rel, rel, "in-place edit keeps the path");
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, body) = split_frontmatter(&content);
        assert_eq!(map.get("priority"), Some(&"P1".to_string()));
        assert_eq!(map.get("description"), Some(&"New blurb".to_string()));
        assert!(body.contains("Body kept on edit."));
    }

    #[test]
    fn update_status_relocates_but_keeps_filename() {
        // A status change moves the file to the new folder while preserving
        // the original filename stem (unlike set_status, which renames to
        // the title). The new frontmatter carries the new status.
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Shift".into(),
                status: "Discovery".into(),
                ..Default::default()
            },
            Some("keep me"),
        );
        // Rename the stem so "preserve filename" is observable (set_status would force Shift.md).
        let renamed = "projects/discovery/custom-stem.md";
        fs::rename(dir.path().join(&rel), dir.path().join(renamed)).unwrap();

        let new_rel = update(
            dir.path(),
            renamed,
            &crate::types::UpdateProjectInput {
                status: Some("Active".into()),
                ..upd(renamed)
            },
        )
        .unwrap();
        assert_eq!(new_rel, "projects/active/custom-stem.md");
        assert!(!dir.path().join(renamed).exists(), "old file removed");
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, body) = split_frontmatter(&content);
        assert_eq!(map.get("status"), Some(&"active".to_string()));
        assert!(body.contains("keep me"));
    }

    #[test]
    fn update_clears_nullable_field_with_empty_string() {
        // `Some("")` clears target_date; `None` would have left it set.
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Dated".into(),
                status: "Active".into(),
                target_date: Some("2026-06-30".into()),
                ..Default::default()
            },
            None,
        );
        let new_rel = update(
            dir.path(),
            &rel,
            &crate::types::UpdateProjectInput {
                target_date: Some(String::new()),
                ..upd(&rel)
            },
        )
        .unwrap();
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, _) = split_frontmatter(&content);
        assert_eq!(
            map.get("target_date"),
            None,
            "empty string clears the field"
        );
    }

    #[test]
    fn outcome_contract_parses_updates_and_clears_without_touching_body() {
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Outcome Brief".into(),
                status: "Planning".into(),
                current_state: Some("Today: handoffs are manual".into()),
                desired_state: Some("A single, verified flow".into()),
                non_goals: Some("No full platform rewrite".into()),
                success_evidence: Some("A dry run passes\nand the operator signs off".into()),
                ..Default::default()
            },
            Some("Long-form implementation notes."),
        );

        let original = fs::read_to_string(dir.path().join(&rel)).unwrap();
        let parsed = parse(&rel, &original, dir.path()).unwrap();
        assert_eq!(
            parsed.current_state.as_deref(),
            Some("Today: handoffs are manual")
        );
        assert_eq!(
            parsed.desired_state.as_deref(),
            Some("A single, verified flow")
        );
        assert_eq!(
            parsed.non_goals.as_deref(),
            Some("No full platform rewrite")
        );
        assert_eq!(
            parsed.success_evidence.as_deref(),
            Some("A dry run passes\nand the operator signs off")
        );

        update(
            dir.path(),
            &rel,
            &crate::types::UpdateProjectInput {
                current_state: Some("Observed baseline".into()),
                non_goals: Some(String::new()),
                ..upd(&rel)
            },
        )
        .unwrap();
        let updated = fs::read_to_string(dir.path().join(&rel)).unwrap();
        let (map, body) = split_frontmatter(&updated);
        assert_eq!(
            map.get("current_state").map(String::as_str),
            Some("Observed baseline")
        );
        assert_eq!(
            map.get("non_goals"),
            None,
            "empty string clears the outcome field"
        );
        assert_eq!(
            map.get("desired_state").map(String::as_str),
            Some("A single, verified flow")
        );
        assert!(body.contains("Long-form implementation notes."));
    }

    #[test]
    fn local_outcome_update_preserves_notion_deployment_and_sync_metadata() {
        let dir = vault();
        let deployed_at = "2026-06-20T14:00:00Z";
        let synced_at = "2026-07-10T09:30:00Z";
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Notion-owned deployment".into(),
                status: "Deployed".into(),
                deployed_at: Some(deployed_at.into()),
                last_synced_at: Some(synced_at.into()),
                source: Some("notion".into()),
                notion_id: Some("page-1".into()),
                ..Default::default()
            },
            Some("Upstream-backed body."),
        );

        update_local_fields(
            dir.path(),
            &rel,
            &crate::types::UpdateProjectInput {
                desired_state: Some("Observable success".into()),
                ..upd(&rel)
            },
        )
        .unwrap();

        let updated = fs::read_to_string(dir.path().join(&rel)).unwrap();
        let (map, body) = split_frontmatter(&updated);
        assert_eq!(
            map.get("deployed_at").map(String::as_str),
            Some(deployed_at)
        );
        assert_eq!(
            map.get("last_synced_at").map(String::as_str),
            Some(synced_at)
        );
        assert_eq!(
            map.get("desired_state").map(String::as_str),
            Some("Observable success")
        );
        assert!(body.contains("Upstream-backed body."));
    }

    #[test]
    fn deployed_at_round_trips() {
        let fm = ProjectFrontmatter {
            title: "Shipped".into(),
            status: "Deployed".into(),
            deployed_at: Some("2026-06-10T12:00:00+00:00".into()),
            ..Default::default()
        };
        let rendered = render(&fm, "");
        let (map, _) = split_frontmatter(&rendered);
        assert_eq!(
            map.get("deployed_at"),
            Some(&"2026-06-10T12:00:00+00:00".to_string())
        );
        // parse() surfaces it on the Project.
        let p = parse("projects/deployed/Shipped.md", &rendered, Path::new("/tmp")).unwrap();
        assert_eq!(p.deployed_at.as_deref(), Some("2026-06-10T12:00:00+00:00"));
    }

    #[test]
    fn set_status_into_deployed_stamps_deployed_at() {
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Ship It".into(),
                status: "Active".into(),
                ..Default::default()
            },
            None,
        );
        let new_rel = set_status(dir.path(), &rel, "Deployed").unwrap();
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, _) = split_frontmatter(&content);
        let stamped = map.get("deployed_at").expect("deployed_at stamped on ship");
        assert!(
            stamped.starts_with("20"),
            "looks like a timestamp: {stamped}"
        );
    }

    #[test]
    fn set_status_out_of_deployed_clears_deployed_at() {
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Unshipped".into(),
                status: "Deployed".into(),
                deployed_at: Some("2026-06-01T00:00:00Z".into()),
                ..Default::default()
            },
            None,
        );
        let new_rel = set_status(dir.path(), &rel, "Active").unwrap();
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, _) = split_frontmatter(&content);
        assert_eq!(
            map.get("deployed_at"),
            None,
            "leaving deployed clears the stamp"
        );
    }

    #[test]
    fn update_preserves_existing_deployed_at() {
        // A hand-set (or earlier-stamped) ship date survives non-status edits.
        let dir = vault();
        let rel = write_project(
            dir.path(),
            ProjectFrontmatter {
                title: "Stays Shipped".into(),
                status: "Deployed".into(),
                deployed_at: Some("2026-06-01T00:00:00Z".into()),
                ..Default::default()
            },
            None,
        );
        let new_rel = update(
            dir.path(),
            &rel,
            &crate::types::UpdateProjectInput {
                description: Some("edited".into()),
                ..upd(&rel)
            },
        )
        .unwrap();
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, _) = split_frontmatter(&content);
        assert_eq!(
            map.get("deployed_at"),
            Some(&"2026-06-01T00:00:00Z".to_string())
        );
    }

    /// A Notion mirror as `notion_sync` leaves it: a status that is a Notion
    /// option name, JSON-quoted values, and keys the project model does not
    /// know (a scalar, a list, a custom tag).
    const MIRROR: &str = "---\ntitle: \"Mirror\"\nstatus: \"In progress\"\nsource: notion\nnotion_id: \"abc\"\nnotion_url: \"https://notion.so/abc\"\nowner: \"Alex\"\ncustom_list:\n  - one\n  - two\nlast_synced_at: \"2026-09-01T00:00:00Z\"\ntags:\n  - custom\n---\n\nMirror body.\n";
    const MIRROR_REL: &str = "workspaces/notion-abc/PROJECT.md";

    fn assert_mirror_kept(dir: &Path, content: &str) {
        assert!(
            !dir.join(PROJECTS_DIR).exists(),
            "nothing moved into projects/"
        );
        let (map, body) = split_frontmatter(content);
        assert_eq!(map.get("owner").map(String::as_str), Some("Alex"));
        assert_eq!(map.get("notion_id").map(String::as_str), Some("abc"));
        assert_eq!(
            map.get("last_synced_at").map(String::as_str),
            Some("2026-09-01T00:00:00Z"),
            "Notion sync metadata untouched"
        );
        assert!(
            content.contains("custom_list:\n  - one\n  - two\n"),
            "{content}"
        );
        assert!(content.contains("tags:\n  - custom\n---"), "{content}");
        assert_eq!(body.trim(), "Mirror body.");
    }

    #[test]
    fn update_on_notion_mirror_keeps_its_path_and_unknown_keys() {
        let dir = vault();
        vault::write_note(dir.path(), MIRROR_REL, MIRROR).unwrap();
        let new_rel = update(
            dir.path(),
            MIRROR_REL,
            &crate::types::UpdateProjectInput {
                status: Some("Done".into()),
                description: Some("Edited locally".into()),
                ..upd(MIRROR_REL)
            },
        )
        .unwrap();
        assert_eq!(new_rel, MIRROR_REL, "a mirror never moves");
        let content = fs::read_to_string(dir.path().join(MIRROR_REL)).unwrap();
        assert_mirror_kept(dir.path(), &content);
        let (map, _) = split_frontmatter(&content);
        assert_eq!(
            map.get("status").map(String::as_str),
            Some("Done"),
            "the Notion option name is written verbatim, not slugged"
        );
        assert_eq!(
            map.get("description").map(String::as_str),
            Some("Edited locally")
        );
    }

    #[test]
    fn set_status_on_notion_mirror_keeps_its_path_and_unknown_keys() {
        let dir = vault();
        vault::write_note(dir.path(), MIRROR_REL, MIRROR).unwrap();
        let new_rel = set_status(dir.path(), MIRROR_REL, "Done").unwrap();
        assert_eq!(new_rel, MIRROR_REL, "a mirror never moves");
        let content = fs::read_to_string(dir.path().join(MIRROR_REL)).unwrap();
        assert_mirror_kept(dir.path(), &content);
        let (map, _) = split_frontmatter(&content);
        assert_eq!(map.get("status").map(String::as_str), Some("Done"));
        // Unchanged keys keep their exact bytes.
        let head = "---\ntitle: \"Mirror\"\nstatus: Done\nsource: notion\n";
        assert!(content.starts_with(head), "{content}");
    }

    #[test]
    fn update_local_fields_on_notion_mirror_keeps_unknown_keys() {
        let dir = vault();
        vault::write_note(dir.path(), MIRROR_REL, MIRROR).unwrap();
        update_local_fields(
            dir.path(),
            MIRROR_REL,
            &crate::types::UpdateProjectInput {
                desired_state: Some("Observable success".into()),
                ..upd(MIRROR_REL)
            },
        )
        .unwrap();
        let content = fs::read_to_string(dir.path().join(MIRROR_REL)).unwrap();
        assert_mirror_kept(dir.path(), &content);
        let (map, _) = split_frontmatter(&content);
        assert_eq!(map.get("status").map(String::as_str), Some("In progress"));
        assert_eq!(
            map.get("desired_state").map(String::as_str),
            Some("Observable success")
        );
    }

    #[test]
    fn update_on_local_project_keeps_unknown_keys_and_custom_tags() {
        let dir = vault();
        let rel = "projects/discovery/Local.md";
        let original = "---\ntitle: Local\nstatus: discovery\nowner: Alex\nlinks:\n  - https://example.com\ntags:\n  - status/discovery\n  - projects\n  - client/acme\n---\n\nLocal body.\n";
        vault::write_note(dir.path(), rel, original).unwrap();
        let new_rel = update(
            dir.path(),
            rel,
            &crate::types::UpdateProjectInput {
                status: Some("Active".into()),
                priority: Some("P2".into()),
                ..upd(rel)
            },
        )
        .unwrap();
        assert_eq!(new_rel, "projects/active/Local.md");
        assert!(!dir.path().join(rel).exists(), "old file removed");
        let content = fs::read_to_string(dir.path().join(&new_rel)).unwrap();
        let (map, body) = split_frontmatter(&content);
        assert_eq!(map.get("owner").map(String::as_str), Some("Alex"));
        assert_eq!(map.get("status").map(String::as_str), Some("active"));
        assert!(
            content.contains("links:\n  - https://example.com\n"),
            "{content}"
        );
        assert_eq!(
            crate::frontmatter::list_items(&content, "tags"),
            Some(vec![
                "status/active".to_string(),
                "priority/p2".to_string(),
                "projects".to_string(),
                "client/acme".to_string(),
            ])
        );
        assert_eq!(body.trim(), "Local body.");
    }

    #[test]
    fn legacy_edits_refuse_notes_outside_project_locations() {
        let dir = vault();
        let rel = "tasks/not-a-project.md";
        vault::write_note(dir.path(), rel, "---\ntitle: Task\nstatus: todo\n---\n").unwrap();
        assert!(set_status(dir.path(), rel, "Active").is_err());
        assert!(update(dir.path(), rel, &upd(rel)).is_err());
        assert!(update_local_fields(dir.path(), rel, &upd(rel)).is_err());
        assert!(dir.path().join(rel).exists());
        assert!(!dir.path().join(PROJECTS_DIR).exists());
    }

    #[test]
    fn body_excerpt_skips_link_only_lines() {
        let body = "[a standalone link](https://x.com)\n\nReal content here.";
        let snippet = body_excerpt(body);
        assert!(!snippet.contains("standalone link"));
        assert!(snippet.contains("Real content"));
    }

    #[test]
    fn split_frontmatter_handles_quoted_values() {
        let md = "---\ntitle: \"With: colon\"\nstatus: active\n---\n\nbody";
        let (map, body) = split_frontmatter(md);
        assert_eq!(map.get("title"), Some(&"With: colon".to_string()));
        assert_eq!(map.get("status"), Some(&"active".to_string()));
        assert_eq!(body.trim(), "body");
    }
}

#[cfg(test)]
mod task_link_tests {
    use super::*;
    #[test]
    fn selected_project_link_uses_exact_file_and_rejects_other_notes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("projects")).unwrap();
        fs::create_dir(root.path().join("tasks")).unwrap();
        fs::write(
            root.path().join("projects/a.md"),
            "---\ntitle: Project A\nstatus: Active\n---\n",
        )
        .unwrap();
        fs::write(
            root.path().join("tasks/other.md"),
            "---\ntitle: Not a project\n---\n",
        )
        .unwrap();
        assert_eq!(
            task_project_link(root.path(), "vault:projects/a.md").unwrap(),
            ("Project A".into(), "vault:projects/a.md".into())
        );
        assert!(task_project_link(root.path(), "vault:projects/../tasks/other.md").is_err());
        assert!(task_project_link(root.path(), "vault:tasks/other.md").is_err());
        assert!(task_project_link(root.path(), "https://example.com/project").is_err());
        assert!(task_project_link(root.path(), "vault:projects/missing.md").is_err());
    }
}
