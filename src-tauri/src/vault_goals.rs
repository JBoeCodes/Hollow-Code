//! Vault-backed goals and deterministic project/goal alignment.
//!
//! Goals are first-class Markdown records under vault/goals/*.md. Their
//! stable identity is the vault: URL derived from the file path; editing a
//! title therefore never renames the file. All renderer-supplied paths are
//! constrained to the flat goals/ namespace before they reach vault I/O.

use crate::types::Project;
use crate::vault;
use chrono::{NaiveDate, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

const GOALS_DIR: &str = "goals";
const GOAL_TRASH_DIR: &str = ".trash/goals";

/// Serializes goal filename selection, moves, and mutations. mutate_note
/// supplies the vault-wide read-modify-write lock for an individual update;
/// this narrower lock also closes create/create and trash/restore races around
/// existence checks and path selection.
static GOALS_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    #[default]
    Active,
    Paused,
    Achieved,
    Dropped,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Achieved => "achieved",
            Self::Dropped => "dropped",
        }
    }
}

impl fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GoalStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "achieved" => Ok(Self::Achieved),
            "dropped" => Ok(Self::Dropped),
            other => Err(format!(
                "invalid goal status {other:?}; expected active, paused, achieved, or dropped"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    /// Stable identifier derived from vault_path, e.g.
    /// vault:goals/improve-reliability.md.
    pub url: String,
    pub title: String,
    pub status: GoalStatus,
    #[serde(default)]
    pub horizon: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    pub vault_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCreateInput {
    pub title: String,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub horizon: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
}

/// Patch semantics mirror the existing task/project inputs: None leaves a
/// value unchanged, while Some("") clears an optional scalar. An empty title
/// is rejected rather than silently ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalUpdateInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub horizon: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveGoalWithoutProjectIssue {
    pub goal_url: String,
    pub goal_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWithoutGoalIssue {
    pub project_url: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrokenGoalUrlReason {
    InvalidGoalUrl,
    GoalNotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenGoalUrlIssue {
    pub project_url: String,
    pub project_name: String,
    pub goal_url: String,
    pub reason: BrokenGoalUrlReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOnlyInactiveGoalsIssue {
    pub project_url: String,
    pub project_name: String,
    pub inactive_goal_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalAlignment {
    pub active_goal_count: usize,
    pub active_project_count: usize,
    pub healthy_goal_count: usize,
    pub healthy_project_count: usize,
    #[serde(default)]
    pub active_goals_without_active_projects: Vec<ActiveGoalWithoutProjectIssue>,
    #[serde(default)]
    pub active_projects_without_goals: Vec<ProjectWithoutGoalIssue>,
    #[serde(default)]
    pub broken_goal_urls: Vec<BrokenGoalUrlIssue>,
    #[serde(default)]
    pub projects_only_linked_to_inactive_goals: Vec<ProjectOnlyInactiveGoalsIssue>,
}

/// Typed seam between alignment and the evolving public Project schema.
/// It keeps the deterministic algorithm testable before Project.goal_urls
/// lands and lets other backend callers project any source into the same
/// shape without teaching alignment about Notion/vault details.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoalProjectProjection {
    pub url: String,
    pub name: String,
    pub status: String,
    pub goal_urls: Vec<String>,
}

#[derive(Debug, Clone)]
struct GoalDocument {
    goal: Goal,
    body: String,
    /// Preserve hand-authored scalar metadata this module does not own.
    extras: Vec<(String, String)>,
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn validate_title(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("goal title is empty".into());
    }
    if value.contains('\0') {
        return Err("goal title contains a NUL byte".into());
    }
    Ok(value.to_string())
}

fn validate_target_date(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("invalid goal target date {value:?}; expected YYYY-MM-DD"))?;
    Ok(Some(value.to_string()))
}

fn safe_filename(title: &str) -> Result<String, String> {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .take(180)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        Err("goal title does not produce a usable filename".into())
    } else {
        Ok(cleaned)
    }
}

fn validate_goal_rel(rel: &str) -> Result<String, String> {
    if rel != rel.trim()
        || rel.contains('\\')
        || rel.contains('\0')
        || rel.contains('\n')
        || rel.contains('\r')
    {
        return Err(format!("invalid goal path: {rel:?}"));
    }
    let mut parts = rel.split('/');
    let folder = parts.next().unwrap_or_default();
    let filename = parts.next().unwrap_or_default();
    if folder != GOALS_DIR || parts.next().is_some() {
        return Err(format!("path must be a direct child of goals/: {rel}"));
    }
    if filename.is_empty()
        || filename.starts_with('.')
        || !filename.ends_with(".md")
        || filename == ".md"
    {
        return Err(format!("invalid goal filename: {filename:?}"));
    }
    Ok(format!("{GOALS_DIR}/{filename}"))
}

fn rel_from_goal_url(vault_url: &str) -> Result<String, String> {
    let rel = vault_url
        .strip_prefix("vault:")
        .ok_or_else(|| format!("not a vault goal URL: {vault_url}"))?;
    validate_goal_rel(rel)
}

fn ensure_regular_file(vault_path: &Path, rel: &str) -> Result<(), String> {
    // safe_join validates containment and all existing symlink ancestors.
    vault::safe_join(vault_path, rel)?;
    let direct = vault_path.join(rel);
    let metadata = fs::symlink_metadata(&direct).map_err(|e| format!("read {rel}: {e}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("goal path is not a regular file: {rel}"));
    }
    Ok(())
}

fn trash_goal_rel(trash_id: &str) -> Result<String, String> {
    let original_rel = if let Some(rel) = trash_id.strip_prefix("vault:") {
        validate_goal_rel(rel)?
    } else if trash_id.starts_with(&format!("{GOALS_DIR}/")) {
        validate_goal_rel(trash_id)?
    } else {
        if trash_id != trash_id.trim()
            || trash_id.contains('/')
            || trash_id.contains('\\')
            || trash_id.contains('\0')
            || trash_id.contains('\n')
            || trash_id.contains('\r')
        {
            return Err(format!("invalid goal trash id: {trash_id:?}"));
        }
        validate_goal_rel(&format!("{GOALS_DIR}/{trash_id}"))?
    };
    let filename = original_rel
        .strip_prefix(&format!("{GOALS_DIR}/"))
        .ok_or_else(|| format!("invalid goal path: {original_rel}"))?;
    Ok(format!("{GOAL_TRASH_DIR}/{filename}"))
}

fn original_rel_from_trash_id(trash_id: &str) -> Result<String, String> {
    if let Some(rel) = trash_id.strip_prefix("vault:") {
        return validate_goal_rel(rel);
    }
    if trash_id.starts_with(&format!("{GOALS_DIR}/")) {
        return validate_goal_rel(trash_id);
    }
    validate_goal_rel(&format!("{GOALS_DIR}/{trash_id}"))
}

fn parse_document(rel: &str, content: &str) -> Result<GoalDocument, String> {
    let rel = validate_goal_rel(rel)?;
    let parsed = crate::frontmatter::parse(content);
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in &parsed.pairs {
        map.insert(key.clone(), value.clone());
    }

    let title = validate_title(
        map.get("title")
            .ok_or_else(|| format!("goal file has no title: {rel}"))?,
    )?;
    let status = map
        .get("status")
        .ok_or_else(|| format!("goal file has no status: {rel}"))?
        .parse::<GoalStatus>()?;
    let target_date = validate_target_date(map.get("target_date").cloned())?;

    const OWNED_KEYS: &[&str] = &[
        "type",
        "title",
        "status",
        "horizon",
        "target_date",
        "why",
        "success_evidence",
        "created",
        "tags",
    ];
    let extras = parsed
        .pairs
        .into_iter()
        .filter(|(key, _)| {
            !OWNED_KEYS.contains(&key.as_str())
                && !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        })
        .collect();

    Ok(GoalDocument {
        goal: Goal {
            url: format!("vault:{rel}"),
            title,
            status,
            horizon: map.get("horizon").and_then(|v| optional_text(v)),
            target_date,
            why: map.get("why").and_then(|v| optional_text(v)),
            success_evidence: map.get("success_evidence").and_then(|v| optional_text(v)),
            created: map.get("created").and_then(|v| optional_text(v)),
            vault_path: rel,
        },
        body: parsed.body,
        extras,
    })
}

fn render_document(document: &GoalDocument) -> String {
    let goal = &document.goal;
    let mut lines = vec!["---".to_string()];
    crate::frontmatter::push_kv(&mut lines, "type", "goal");
    crate::frontmatter::push_kv(&mut lines, "title", &goal.title);
    crate::frontmatter::push_kv(&mut lines, "status", goal.status.as_str());
    if let Some(value) = goal.horizon.as_deref() {
        crate::frontmatter::push_kv(&mut lines, "horizon", value);
    }
    if let Some(value) = goal.target_date.as_deref() {
        crate::frontmatter::push_kv(&mut lines, "target_date", value);
    }
    if let Some(value) = goal.why.as_deref() {
        crate::frontmatter::push_kv(&mut lines, "why", value);
    }
    if let Some(value) = goal.success_evidence.as_deref() {
        crate::frontmatter::push_kv(&mut lines, "success_evidence", value);
    }
    if let Some(value) = goal.created.as_deref() {
        crate::frontmatter::push_kv(&mut lines, "created", value);
    }
    for (key, value) in &document.extras {
        crate::frontmatter::push_kv(&mut lines, key, value);
    }
    lines.push("tags:".into());
    lines.push(format!("  - status/{}", goal.status.as_str()));
    lines.push("  - goals".into());
    lines.push("---".into());

    let mut output = lines.join("\n");
    output.push_str("\n\n");
    if !document.body.is_empty() {
        output.push_str(&document.body);
        if !document.body.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

/// List direct Markdown children of goals/. A missing folder is a valid,
/// empty legacy vault and is not created as a side effect.
pub fn list(vault_path: &Path) -> Result<Vec<Goal>, String> {
    let root = vault::safe_join(vault_path, GOALS_DIR)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err("goals exists but is not a directory".into());
    }

    let mut goals = Vec::new();
    let entries = fs::read_dir(&root).map_err(|e| format!("read goals: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read goals entry: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("read goals file type: {e}"))?;
        // No recursion and no symlink following: goals are deliberately flat.
        if !file_type.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if filename.starts_with('.') || !filename.ends_with(".md") {
            continue;
        }
        let rel = validate_goal_rel(&format!("{GOALS_DIR}/{filename}"))?;
        let content =
            fs::read_to_string(entry.path()).map_err(|e| format!("read goal {rel}: {e}"))?;
        let document =
            parse_document(&rel, &content).map_err(|e| format!("parse goal {rel}: {e}"))?;
        goals.push(document.goal);
    }
    goals.sort_by(|a, b| {
        a.title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
            .then_with(|| a.url.cmp(&b.url))
    });
    Ok(goals)
}

/// Largest goal file [`read_linked`] reads. Goal notes are a few hundred
/// bytes; anything bigger is reported as `too large` rather than read.
pub const MAX_GOAL_FILE_BYTES: u64 = 256 * 1024;

/// One linked goal as [`read_linked`] found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedGoal {
    Found(Goal),
    /// `url` is the link as written, capped at 120 bytes. `reason` is one of
    /// `invalid link`, `missing`, `too large`, `unreadable` or `malformed`.
    Unreadable {
        url: String,
        reason: &'static str,
    },
}

/// Best-effort read of the goals a project links (Paperclip PR 10), for
/// agent context. Unlike [`list`], which fails as a whole when any goal file
/// is malformed, this reads only the linked files, never errs and takes no
/// lock: each link becomes `Found` or `Unreadable` with a reason, so one bad
/// goal never hides the others. Links are trimmed, empty ones skipped,
/// duplicates dropped and link order kept; at most `limit` are read.
pub fn read_linked(vault_path: &Path, urls: &[String], limit: usize) -> Vec<LinkedGoal> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in urls {
        if out.len() >= limit {
            break;
        }
        let url = raw.trim();
        if url.is_empty() || !seen.insert(url.to_string()) {
            continue;
        }
        out.push(read_one_linked(vault_path, url));
    }
    out
}

fn read_one_linked(vault_path: &Path, url: &str) -> LinkedGoal {
    use std::io::Read;
    let unreadable = |reason: &'static str| LinkedGoal::Unreadable {
        url: crate::goal_context::cap_bytes(url, 120),
        reason,
    };
    let Ok(rel) = rel_from_goal_url(url) else {
        return unreadable("invalid link");
    };
    // safe_join validates containment and every existing symlink ancestor.
    if vault::safe_join(vault_path, &rel).is_err() {
        return unreadable("invalid link");
    }
    let direct = vault_path.join(&rel);
    // Size and kind come from metadata BEFORE reading, so an oversized file
    // is reported as such instead of failing mid-character.
    let metadata = match fs::symlink_metadata(&direct) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return unreadable("missing"),
        Err(_) => return unreadable("invalid link"),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return unreadable("invalid link");
    }
    if metadata.len() > MAX_GOAL_FILE_BYTES {
        return unreadable("too large");
    }
    let mut bytes = Vec::new();
    let read = fs::File::open(&direct)
        .and_then(|file| file.take(MAX_GOAL_FILE_BYTES + 1).read_to_end(&mut bytes));
    if read.is_err() {
        return unreadable("unreadable");
    }
    if bytes.len() as u64 > MAX_GOAL_FILE_BYTES {
        return unreadable("too large");
    }
    let Ok(content) = String::from_utf8(bytes) else {
        return unreadable("unreadable");
    };
    match parse_document(&rel, &content) {
        Ok(document) => LinkedGoal::Found(document.goal),
        Err(_) => unreadable("malformed"),
    }
}

/// Create a goal with a collision-safe human-readable filename.
pub fn create(vault_path: &Path, input: GoalCreateInput) -> Result<Goal, String> {
    let title = validate_title(&input.title)?;
    let target_date = validate_target_date(input.target_date)?;
    let filename = safe_filename(&title)?;
    let _guard = GOALS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut document = GoalDocument {
        goal: Goal {
            url: String::new(),
            title,
            status: input.status.unwrap_or_default(),
            horizon: input.horizon.as_deref().and_then(optional_text),
            target_date,
            why: input.why.as_deref().and_then(optional_text),
            success_evidence: input.success_evidence.as_deref().and_then(optional_text),
            created: Some(Utc::now().to_rfc3339()),
            vault_path: String::new(),
        },
        body: String::new(),
        extras: Vec::new(),
    };
    // Each candidate is published with `create_note`'s exclusive link, never
    // an `exists()` check and then a write: a goal file Obsidian or an agent
    // creates at the same moment is kept and the next suffix is tried
    // (merged plan slice 4, A·D8).
    for suffix in 1..1000 {
        let rel = if suffix == 1 {
            format!("{GOALS_DIR}/{filename}.md")
        } else {
            format!("{GOALS_DIR}/{filename} {suffix}.md")
        };
        document.goal.url = format!("vault:{rel}");
        document.goal.vault_path = rel.clone();
        match vault::create_note(vault_path, &rel, &render_document(&document)) {
            Ok(_) => return Ok(document.goal),
            Err(error) if error.starts_with("Note already exists:") => continue,
            Err(error) => return Err(error),
        }
    }
    Err("too many goal filename collisions".to_string())
}

/// Update structured goal fields without renaming the file, preserving both
/// stable URL and hand-authored body/unknown scalar metadata.
pub fn update(vault_path: &Path, vault_url: &str, input: GoalUpdateInput) -> Result<Goal, String> {
    let rel = rel_from_goal_url(vault_url)?;
    ensure_regular_file(vault_path, &rel)?;
    let _guard = GOALS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let note = vault::mutate_note(vault_path, &rel, |content| {
        let mut document = parse_document(&rel, content)?;
        if let Some(title) = input.title.as_deref() {
            document.goal.title = validate_title(title)?;
        }
        if let Some(status) = input.status {
            document.goal.status = status;
        }
        if let Some(value) = input.horizon.as_deref() {
            document.goal.horizon = optional_text(value);
        }
        if let Some(value) = input.target_date.as_deref() {
            document.goal.target_date = validate_target_date(Some(value.to_string()))?;
        }
        if let Some(value) = input.why.as_deref() {
            document.goal.why = optional_text(value);
        }
        if let Some(value) = input.success_evidence.as_deref() {
            document.goal.success_evidence = optional_text(value);
        }
        Ok(render_document(&document))
    })?;
    Ok(parse_document(&rel, &note.content)?.goal)
}

/// Move a goal into the goal-specific trash namespace. The caller may pass the
/// original stable URL to restore as its trash_id; a bare filename is accepted
/// too. Refusing a destination collision prevents an older trashed goal from
/// being overwritten by a newer goal that reused the same path.
pub fn trash(vault_path: &Path, vault_url: &str) -> Result<(), String> {
    let rel = rel_from_goal_url(vault_url)?;
    ensure_regular_file(vault_path, &rel)?;
    let trash_rel = trash_goal_rel(vault_url)?;
    let _guard = GOALS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if vault::safe_join(vault_path, &trash_rel)?.exists() {
        return Err(format!(
            "goal trash destination already exists: {trash_rel}"
        ));
    }
    vault::move_note(vault_path, &rel, &trash_rel)?;

    // move_note indexes its destination because normal moves remain visible.
    // Goal trash is hidden, so remove it from derived indexes and invalidate
    // the link index to match delete_note semantics.
    crate::vault_search_index::on_remove(vault_path, &trash_rel);
    crate::vault_embed_index::on_remove(vault_path, &trash_rel);
    vault::invalidate_index();
    Ok(())
}

/// Restore a goal from .trash/goals/ to its exact original stable path.
/// trash_id may be the original vault: URL, goals/<file>.md, or the bare
/// filename. Restore never overwrites a newly-created file at that path.
pub fn restore(vault_path: &Path, trash_id: &str) -> Result<Goal, String> {
    let original_rel = original_rel_from_trash_id(trash_id)?;
    let trash_rel = trash_goal_rel(trash_id)?;
    let _guard = GOALS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ensure_regular_file(vault_path, &trash_rel)?;
    if vault::safe_join(vault_path, &original_rel)?.exists() {
        return Err(format!(
            "goal restore destination already exists: {original_rel}"
        ));
    }

    // Validate the payload before moving it back into the live goals folder.
    let hidden_content = fs::read_to_string(vault::safe_join(vault_path, &trash_rel)?)
        .map_err(|e| format!("read trashed goal {trash_rel}: {e}"))?;
    parse_document(&original_rel, &hidden_content)?;

    let note = vault::move_note(vault_path, &trash_rel, &original_rel)?;
    Ok(parse_document(&original_rel, &note.content)?.goal)
}

fn serialized_goal_urls(project: &Project) -> Vec<String> {
    // The current Project model predates goal links. Reading the serialized
    // camelCase field keeps this module buildable now and automatically starts
    // consuming a later goal_urls field once the shared type adds it.
    let Ok(value) = serde_json::to_value(project) else {
        return Vec::new();
    };
    match value.get("goalUrls") {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Compute deterministic alignment for the public project type. Alignment is
/// intentionally non-LLM: it is a pure relation between project status and
/// stable goal URLs.
pub fn alignment(goals: &[Goal], projects: &[Project]) -> GoalAlignment {
    let projected: Vec<GoalProjectProjection> = projects
        .iter()
        .map(|project| GoalProjectProjection {
            url: project.url.clone(),
            name: project.name.clone(),
            status: project.status.clone(),
            goal_urls: serialized_goal_urls(project),
        })
        .collect();
    alignment_from_projections(goals, &projected)
}

fn is_active_project(status: &str) -> bool {
    matches!(
        status
            .trim()
            .to_ascii_lowercase()
            .replace(' ', "-")
            .as_str(),
        "active" | "planning"
    )
}

fn syntactically_valid_goal_url(url: &str) -> bool {
    url.strip_prefix("vault:")
        .and_then(|rel| validate_goal_rel(rel).ok())
        .is_some()
}

/// Projection-based implementation used by the public adapter and unit tests.
/// Keeping this public gives integration code a typed bridge while
/// Project.goal_urls is introduced in the shared wire model.
pub fn alignment_from_projections(
    goals: &[Goal],
    projects: &[GoalProjectProjection],
) -> GoalAlignment {
    let goals_by_url: HashMap<&str, &Goal> =
        goals.iter().map(|goal| (goal.url.as_str(), goal)).collect();
    let active_goals: Vec<&Goal> = goals
        .iter()
        .filter(|goal| goal.status == GoalStatus::Active)
        .collect();
    let active_projects: Vec<&GoalProjectProjection> = projects
        .iter()
        .filter(|project| is_active_project(&project.status))
        .collect();

    let mut supported_active_goals: HashSet<&str> = HashSet::new();
    let mut healthy_project_count = 0usize;
    let mut active_projects_without_goals = Vec::new();
    let mut broken_goal_urls = Vec::new();
    let mut projects_only_linked_to_inactive_goals = Vec::new();

    for project in &active_projects {
        let links: Vec<String> = project
            .goal_urls
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        if links.is_empty() {
            active_projects_without_goals.push(ProjectWithoutGoalIssue {
                project_url: project.url.clone(),
                project_name: project.name.clone(),
            });
            continue;
        }

        let mut active_links = Vec::new();
        let mut inactive_links = Vec::new();
        let mut broken_count = 0usize;
        for goal_url in links {
            if !syntactically_valid_goal_url(&goal_url) {
                broken_count += 1;
                broken_goal_urls.push(BrokenGoalUrlIssue {
                    project_url: project.url.clone(),
                    project_name: project.name.clone(),
                    goal_url,
                    reason: BrokenGoalUrlReason::InvalidGoalUrl,
                });
                continue;
            }
            let Some(goal) = goals_by_url.get(goal_url.as_str()) else {
                broken_count += 1;
                broken_goal_urls.push(BrokenGoalUrlIssue {
                    project_url: project.url.clone(),
                    project_name: project.name.clone(),
                    goal_url,
                    reason: BrokenGoalUrlReason::GoalNotFound,
                });
                continue;
            };
            if goal.status == GoalStatus::Active {
                supported_active_goals.insert(goal.url.as_str());
                active_links.push(goal.url.clone());
            } else {
                inactive_links.push(goal.url.clone());
            }
        }

        if !active_links.is_empty() {
            healthy_project_count += 1;
        } else if broken_count == 0 && !inactive_links.is_empty() {
            projects_only_linked_to_inactive_goals.push(ProjectOnlyInactiveGoalsIssue {
                project_url: project.url.clone(),
                project_name: project.name.clone(),
                inactive_goal_urls: inactive_links,
            });
        }
    }

    let mut active_goals_without_active_projects: Vec<ActiveGoalWithoutProjectIssue> = active_goals
        .iter()
        .filter(|goal| !supported_active_goals.contains(goal.url.as_str()))
        .map(|goal| ActiveGoalWithoutProjectIssue {
            goal_url: goal.url.clone(),
            goal_title: goal.title.clone(),
        })
        .collect();

    active_goals_without_active_projects.sort_by(|a, b| {
        a.goal_title
            .to_ascii_lowercase()
            .cmp(&b.goal_title.to_ascii_lowercase())
            .then_with(|| a.goal_url.cmp(&b.goal_url))
    });
    active_projects_without_goals.sort_by(|a, b| {
        a.project_name
            .to_ascii_lowercase()
            .cmp(&b.project_name.to_ascii_lowercase())
            .then_with(|| a.project_url.cmp(&b.project_url))
    });
    broken_goal_urls.sort_by(|a, b| {
        a.project_name
            .to_ascii_lowercase()
            .cmp(&b.project_name.to_ascii_lowercase())
            .then_with(|| a.goal_url.cmp(&b.goal_url))
    });
    projects_only_linked_to_inactive_goals.sort_by(|a, b| {
        a.project_name
            .to_ascii_lowercase()
            .cmp(&b.project_name.to_ascii_lowercase())
            .then_with(|| a.project_url.cmp(&b.project_url))
    });

    GoalAlignment {
        active_goal_count: active_goals.len(),
        active_project_count: active_projects.len(),
        healthy_goal_count: supported_active_goals.len(),
        healthy_project_count,
        active_goals_without_active_projects,
        active_projects_without_goals,
        broken_goal_urls,
        projects_only_linked_to_inactive_goals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn vault_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    fn input(title: &str) -> GoalCreateInput {
        GoalCreateInput {
            title: title.into(),
            status: Some(GoalStatus::Active),
            horizon: Some("2026".into()),
            target_date: Some("2026-12-31".into()),
            why: Some("Reduce manual recovery".into()),
            success_evidence: Some("Thirty intervention-free days".into()),
        }
    }

    fn synthetic_goal(url: &str, title: &str, status: GoalStatus) -> Goal {
        Goal {
            url: url.into(),
            title: title.into(),
            status,
            horizon: None,
            target_date: None,
            why: None,
            success_evidence: None,
            created: None,
            vault_path: url.strip_prefix("vault:").unwrap_or(url).into(),
        }
    }

    fn project(name: &str, status: &str, goal_urls: &[&str]) -> GoalProjectProjection {
        GoalProjectProjection {
            url: format!("vault:projects/{status}/{name}.md"),
            name: name.into(),
            status: status.into(),
            goal_urls: goal_urls.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn serde_uses_camel_case_fields_and_lowercase_statuses() {
        let value = serde_json::to_value(input("Reliability")).unwrap();
        assert_eq!(value["targetDate"], "2026-12-31");
        assert_eq!(value["successEvidence"], "Thirty intervention-free days");
        assert_eq!(value["status"], "active");
        assert!(value.get("target_date").is_none());

        let error = serde_json::from_value::<GoalCreateInput>(serde_json::json!({
            "title": "Bad",
            "status": "in-progress"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn missing_goals_folder_is_empty_and_not_created() {
        let dir = vault_dir();
        assert_eq!(list(dir.path()).unwrap(), Vec::<Goal>::new());
        assert!(!dir.path().join(GOALS_DIR).exists());
    }

    #[test]
    fn create_list_and_parse_round_trip() {
        let dir = vault_dir();
        let created = create(dir.path(), input("Improve operational reliability")).unwrap();
        assert_eq!(
            created.url,
            "vault:goals/Improve operational reliability.md"
        );
        assert_eq!(
            created.vault_path,
            "goals/Improve operational reliability.md"
        );
        assert!(created
            .created
            .as_deref()
            .is_some_and(|value| value.contains('T')));

        let goals = list(dir.path()).unwrap();
        assert_eq!(goals, vec![created]);
        let raw = fs::read_to_string(dir.path().join(&goals[0].vault_path)).unwrap();
        assert!(raw.contains("type: goal"));
        assert!(raw.contains("status: active"));
        assert!(raw.contains("  - goals"));
    }

    #[test]
    fn create_sanitizes_filename_and_uses_collision_suffixes() {
        let dir = vault_dir();
        let first = create(dir.path(), input("A/B: C")).unwrap();
        let second = create(dir.path(), input("A/B: C")).unwrap();
        assert_eq!(first.url, "vault:goals/A-B- C.md");
        assert_eq!(second.url, "vault:goals/A-B- C 2.md");
        assert_eq!(list(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn create_rejects_empty_title_and_bad_target_date_without_writing() {
        let dir = vault_dir();
        assert!(create(dir.path(), input("   "))
            .unwrap_err()
            .contains("empty"));
        let mut invalid = input("Invalid date");
        invalid.target_date = Some("12/31/2026".into());
        assert!(create(dir.path(), invalid)
            .unwrap_err()
            .contains("YYYY-MM-DD"));
        assert!(!dir.path().join(GOALS_DIR).exists());
    }

    #[test]
    fn list_is_flat_and_ignores_non_markdown_and_hidden_files() {
        let dir = vault_dir();
        fs::create_dir_all(dir.path().join("goals/nested")).unwrap();
        fs::write(dir.path().join("goals/readme.txt"), "ignore").unwrap();
        fs::write(dir.path().join("goals/.hidden.md"), "ignore").unwrap();
        fs::write(
            dir.path().join("goals/nested/Nested.md"),
            "---\ntitle: Nested\nstatus: active\n---\n",
        )
        .unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn list_and_update_do_not_follow_goal_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = vault_dir();
        fs::create_dir_all(dir.path().join("goals")).unwrap();
        fs::write(
            dir.path().join("outside.md"),
            "---\ntitle: Outside\nstatus: active\n---\n",
        )
        .unwrap();
        symlink(
            dir.path().join("outside.md"),
            dir.path().join("goals/Link.md"),
        )
        .unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
        assert!(update(
            dir.path(),
            "vault:goals/Link.md",
            GoalUpdateInput::default()
        )
        .unwrap_err()
        .contains("regular file"));
    }

    #[test]
    fn list_surfaces_malformed_status_instead_of_silently_dropping_goal() {
        let dir = vault_dir();
        fs::create_dir_all(dir.path().join("goals")).unwrap();
        fs::write(
            dir.path().join("goals/Bad.md"),
            "---\ntitle: Bad\nstatus: someday\n---\n",
        )
        .unwrap();
        let error = list(dir.path()).unwrap_err();
        assert!(error.contains("invalid goal status"));
        assert!(error.contains("goals/Bad.md"));
    }

    #[test]
    fn read_linked_reports_unreadable_goals_without_hiding_the_others() {
        let dir = vault_dir();
        let valid = create(dir.path(), input("Recoverable data")).unwrap();
        fs::write(
            dir.path().join("goals/Bad.md"),
            "---\ntitle: Bad\nstatus: someday\n---\n",
        )
        .unwrap();
        let bogus = "x".repeat(3 * 1024);
        let urls = vec![
            valid.url.clone(),
            "vault:goals/Bad.md".to_string(),
            "vault:goals/Missing.md".to_string(),
            "vault:projects/x.md".to_string(),
            format!("  {}  ", valid.url),
            bogus.clone(),
        ];
        let linked = read_linked(dir.path(), &urls, 32);
        assert_eq!(linked.len(), 5, "the duplicate is dropped: {linked:?}");
        assert_eq!(linked[0], LinkedGoal::Found(valid));
        let reasons: Vec<(&str, &str)> = linked[1..]
            .iter()
            .map(|item| match item {
                LinkedGoal::Unreadable { url, reason } => (url.as_str(), *reason),
                LinkedGoal::Found(goal) => panic!("unexpected goal {goal:?}"),
            })
            .collect();
        assert_eq!(reasons[0], ("vault:goals/Bad.md", "malformed"));
        assert_eq!(reasons[1], ("vault:goals/Missing.md", "missing"));
        assert_eq!(reasons[2], ("vault:projects/x.md", "invalid link"));
        assert_eq!(reasons[3].1, "invalid link");
        assert!(reasons[3].0.len() <= 120, "a bogus link is capped");
        assert!(reasons[3].0.ends_with('…'));
        // list() on the same vault still fails as a whole.
        assert!(list(dir.path())
            .unwrap_err()
            .contains("invalid goal status"));
    }

    #[cfg(unix)]
    #[test]
    fn read_linked_refuses_symlinks_flags_oversized_files_and_honours_the_limit() {
        use std::os::unix::fs::symlink;

        let dir = vault_dir();
        fs::create_dir_all(dir.path().join("goals")).unwrap();
        fs::write(
            dir.path().join("outside.md"),
            "---\ntitle: Outside\nstatus: active\n---\n",
        )
        .unwrap();
        symlink(
            dir.path().join("outside.md"),
            dir.path().join("goals/Link.md"),
        )
        .unwrap();
        // 300 KiB whose 256 KiB boundary falls inside a two-byte character.
        let mut big = String::from("---\ntitle: Big\nstatus: active\nwhy: \"");
        while big.len() < MAX_GOAL_FILE_BYTES as usize - 1 {
            big.push('a');
        }
        while big.len() < 300 * 1024 {
            big.push('é');
        }
        big.push_str("\"\n---\n");
        assert!(!big.is_char_boundary(MAX_GOAL_FILE_BYTES as usize));
        fs::write(dir.path().join("goals/Big.md"), &big).unwrap();
        let linked = read_linked(
            dir.path(),
            &["vault:goals/Link.md".into(), "vault:goals/Big.md".into()],
            32,
        );
        assert_eq!(
            linked,
            vec![
                LinkedGoal::Unreadable {
                    url: "vault:goals/Link.md".into(),
                    reason: "invalid link"
                },
                LinkedGoal::Unreadable {
                    url: "vault:goals/Big.md".into(),
                    reason: "too large"
                },
            ]
        );

        let urls: Vec<String> = (0..10)
            .map(|n| create(dir.path(), input(&format!("Goal {n}"))).unwrap().url)
            .collect();
        let limited = read_linked(dir.path(), &urls, 3);
        let found: Vec<&str> = limited
            .iter()
            .map(|item| match item {
                LinkedGoal::Found(goal) => goal.url.as_str(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            found,
            urls[..3].iter().map(String::as_str).collect::<Vec<_>>()
        );
    }

    #[test]
    fn update_preserves_stable_url_body_created_and_unknown_scalars() {
        let dir = vault_dir();
        let goal = create(dir.path(), input("Original title")).unwrap();
        let path = dir.path().join(&goal.vault_path);
        let existing = fs::read_to_string(&path).unwrap();
        let with_body = existing.replacen("tags:\n", "owner: Alex\ntags:\n", 1)
            + "Long-form context.\n- Keep this checklist.\n";
        fs::write(&path, with_body).unwrap();

        let updated = update(
            dir.path(),
            &goal.url,
            GoalUpdateInput {
                title: Some("Renamed display title".into()),
                status: Some(GoalStatus::Paused),
                horizon: Some("".into()),
                target_date: Some("2027-01-15".into()),
                why: Some("A clearer reason".into()),
                success_evidence: Some("".into()),
            },
        )
        .unwrap();

        assert_eq!(updated.url, goal.url);
        assert_eq!(updated.vault_path, goal.vault_path);
        assert_eq!(updated.title, "Renamed display title");
        assert_eq!(updated.status, GoalStatus::Paused);
        assert_eq!(updated.horizon, None);
        assert_eq!(updated.target_date.as_deref(), Some("2027-01-15"));
        assert_eq!(updated.success_evidence, None);
        assert_eq!(updated.created, goal.created);
        let raw = fs::read_to_string(path).unwrap();
        assert!(raw.contains("owner: Alex"));
        assert!(raw.contains("Long-form context."));
        assert!(raw.contains("- Keep this checklist."));
        assert!(raw.contains("  - status/paused"));
    }

    #[test]
    fn update_rejects_unsafe_or_non_goal_paths() {
        let dir = vault_dir();
        for url in [
            "goals/NoPrefix.md",
            "vault:../secret.md",
            "vault:goals/../../secret.md",
            "vault:projects/active/Project.md",
            "vault:goals/nested/Goal.md",
            "vault:goals/NotMarkdown.txt",
            "vault:goals\\Goal.md",
        ] {
            assert!(
                update(dir.path(), url, GoalUpdateInput::default()).is_err(),
                "unsafe URL should fail: {url}"
            );
        }
    }

    #[test]
    fn failed_update_does_not_modify_file() {
        let dir = vault_dir();
        let goal = create(dir.path(), input("Keep me")).unwrap();
        let path = dir.path().join(&goal.vault_path);
        let before = fs::read_to_string(&path).unwrap();
        let error = update(
            dir.path(),
            &goal.url,
            GoalUpdateInput {
                target_date: Some("not-a-date".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("YYYY-MM-DD"));
        assert_eq!(fs::read_to_string(path).unwrap(), before);
    }

    #[test]
    fn multiline_and_yaml_sensitive_values_round_trip() {
        let dir = vault_dir();
        let mut create_input = input("Quoted # goal");
        create_input.why = Some("Line one\nLine two: \"quoted\"".into());
        let goal = create(dir.path(), create_input).unwrap();
        let loaded = list(dir.path()).unwrap().remove(0);
        assert_eq!(loaded.why, goal.why);
        assert_eq!(loaded.title, "Quoted # goal");
    }

    #[test]
    fn trash_and_restore_preserve_bytes_and_stable_url() {
        let dir = vault_dir();
        let goal = create(dir.path(), input("Recoverable")).unwrap();
        let live = dir.path().join(&goal.vault_path);
        let before = fs::read_to_string(&live).unwrap();

        trash(dir.path(), &goal.url).unwrap();
        assert!(!live.exists());
        assert!(dir.path().join(".trash/goals/Recoverable.md").exists());
        assert!(list(dir.path()).unwrap().is_empty());

        let restored = restore(dir.path(), &goal.url).unwrap();
        assert_eq!(restored.url, goal.url);
        assert_eq!(fs::read_to_string(live).unwrap(), before);
    }

    #[test]
    fn restore_accepts_bare_filename_but_rejects_escape_and_collision() {
        let dir = vault_dir();
        let goal = create(dir.path(), input("Collision")).unwrap();
        trash(dir.path(), &goal.url).unwrap();
        fs::write(
            dir.path().join("goals/Collision.md"),
            "---\ntitle: New occupant\nstatus: active\n---\n",
        )
        .unwrap();
        assert!(restore(dir.path(), "Collision.md")
            .unwrap_err()
            .contains("destination already exists"));
        assert!(restore(dir.path(), "../Collision.md").is_err());
        assert!(restore(dir.path(), ".hidden.md").is_err());
    }

    #[test]
    fn second_trash_cannot_overwrite_an_older_trashed_goal() {
        let dir = vault_dir();
        let first = create(dir.path(), input("Same path")).unwrap();
        trash(dir.path(), &first.url).unwrap();
        let second = create(dir.path(), input("Same path")).unwrap();
        assert_eq!(second.url, first.url);
        assert!(trash(dir.path(), &second.url)
            .unwrap_err()
            .contains("trash destination already exists"));
        assert!(dir.path().join(&second.vault_path).exists());
    }

    #[test]
    fn concurrent_creates_choose_distinct_paths() {
        let dir = Arc::new(vault_dir());
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let dir = Arc::clone(&dir);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                create(dir.path(), input("Parallel"))
            }));
        }
        barrier.wait();
        let a = handles.remove(0).join().unwrap().unwrap();
        let b = handles.remove(0).join().unwrap().unwrap();
        assert_ne!(a.url, b.url);
        assert_eq!(list(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn concurrent_partial_updates_do_not_lose_fields() {
        let dir = Arc::new(vault_dir());
        let goal = create(dir.path(), input("Parallel update")).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for patch in [
            GoalUpdateInput {
                why: Some("Updated why".into()),
                ..Default::default()
            },
            GoalUpdateInput {
                success_evidence: Some("Updated evidence".into()),
                ..Default::default()
            },
        ] {
            let dir = Arc::clone(&dir);
            let barrier = Arc::clone(&barrier);
            let url = goal.url.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                update(dir.path(), &url, patch)
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let loaded = list(dir.path()).unwrap().remove(0);
        assert_eq!(loaded.why.as_deref(), Some("Updated why"));
        assert_eq!(loaded.success_evidence.as_deref(), Some("Updated evidence"));
    }

    #[test]
    fn alignment_covers_all_issue_classes_and_healthy_counts() {
        let active_supported =
            synthetic_goal("vault:goals/Supported.md", "Supported", GoalStatus::Active);
        let active_orphan = synthetic_goal("vault:goals/Orphan.md", "Orphan", GoalStatus::Active);
        let paused = synthetic_goal("vault:goals/Paused.md", "Paused", GoalStatus::Paused);
        let goals = vec![active_supported, active_orphan, paused];
        let projects = vec![
            project("Healthy", "Active", &["vault:goals/Supported.md"]),
            project(
                "Planning healthy",
                "Planning",
                &["vault:goals/Supported.md"],
            ),
            project("Missing", "Active", &[]),
            project("Inactive only", "Active", &["vault:goals/Paused.md"]),
            project("Broken missing", "Active", &["vault:goals/Gone.md"]),
            project("Broken syntax", "Active", &["https://example.com/goal"]),
            project("Not active", "Discovery", &[]),
        ];

        let result = alignment_from_projections(&goals, &projects);
        assert_eq!(result.active_goal_count, 2);
        assert_eq!(result.active_project_count, 6);
        assert_eq!(result.healthy_goal_count, 1);
        assert_eq!(result.healthy_project_count, 2);
        assert_eq!(result.active_goals_without_active_projects.len(), 1);
        assert_eq!(
            result.active_goals_without_active_projects[0].goal_title,
            "Orphan"
        );
        assert_eq!(result.active_projects_without_goals.len(), 1);
        assert_eq!(
            result.active_projects_without_goals[0].project_name,
            "Missing"
        );
        assert_eq!(result.broken_goal_urls.len(), 2);
        assert!(result.broken_goal_urls.iter().any(|issue| {
            issue.reason == BrokenGoalUrlReason::GoalNotFound
                && issue.goal_url == "vault:goals/Gone.md"
        }));
        assert!(result.broken_goal_urls.iter().any(|issue| {
            issue.reason == BrokenGoalUrlReason::InvalidGoalUrl
                && issue.goal_url == "https://example.com/goal"
        }));
        assert_eq!(result.projects_only_linked_to_inactive_goals.len(), 1);
        assert_eq!(
            result.projects_only_linked_to_inactive_goals[0].project_name,
            "Inactive only"
        );
    }

    #[test]
    fn alignment_deduplicates_links_and_keeps_extra_broken_link_visible() {
        let goal = synthetic_goal("vault:goals/A.md", "A", GoalStatus::Active);
        let projects = vec![project(
            "Mixed",
            "active",
            &[
                "vault:goals/A.md",
                "vault:goals/A.md",
                "vault:goals/Missing.md",
                "vault:goals/Missing.md",
            ],
        )];
        let result = alignment_from_projections(&[goal], &projects);
        assert_eq!(result.healthy_project_count, 1);
        assert_eq!(result.healthy_goal_count, 1);
        assert_eq!(result.broken_goal_urls.len(), 1);
    }

    #[test]
    fn public_alignment_adapter_handles_project_without_goal_urls() {
        let goal = synthetic_goal("vault:goals/A.md", "A", GoalStatus::Active);
        let existing_project = Project {
            url: "vault:projects/active/P.md".into(),
            name: "P".into(),
            status: "Active".into(),
            ..Default::default()
        };
        let result = alignment(&[goal], &[existing_project]);
        assert_eq!(result.active_project_count, 1);
        assert_eq!(result.active_projects_without_goals.len(), 1);
        assert_eq!(result.active_goals_without_active_projects.len(), 1);
    }

    #[test]
    fn alignment_issue_json_is_camel_case() {
        let value = serde_json::to_value(BrokenGoalUrlIssue {
            project_url: "vault:projects/active/P.md".into(),
            project_name: "P".into(),
            goal_url: "bad".into(),
            reason: BrokenGoalUrlReason::InvalidGoalUrl,
        })
        .unwrap();
        assert_eq!(value["projectUrl"], "vault:projects/active/P.md");
        assert_eq!(value["goalUrl"], "bad");
        assert_eq!(value["reason"], "invalidGoalUrl");
        assert!(value.get("project_url").is_none());
    }
}
