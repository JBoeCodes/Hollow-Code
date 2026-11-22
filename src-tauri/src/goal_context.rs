//! Goal and project "why" for agent context (Paperclip PR 10).
//!
//! Hollow links projects to goals (`goal_urls`) but used to send neither to an
//! agent. This module resolves the project a workspace (or a task) belongs to,
//! reads its linked goals best-effort, and builds a small, capped block that
//! the three consumers inject as reference evidence:
//!
//! - `workspaces::context`: the `goalAlignment` key of the `<hollow-workspace>`
//!   envelope, capped at [`ALIGNMENT_CAP`] bytes after the envelope's own
//!   escaping and never counted toward the 48 KB file total;
//! - `commands::task_assist`: a `<project-alignment>` block for Break down and
//!   next steps on a task that has a stored `project_url`;
//! - `project_next_move`: the capped `goals` field (it reads through
//!   [`vault_goals::read_linked`] itself).
//!
//! Everything here is read-only and never fails a send or a run: a missing,
//! malformed or oversized goal or project only shrinks the block or omits it.

use crate::types::Project;
use crate::vault_goals::{self, Goal, GoalStatus, LinkedGoal};
use crate::vault_projects;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Byte cap of the alignment block, measured on the exact string the
/// surface delivers (after its escaping).
pub const ALIGNMENT_CAP: usize = 2_048;
/// At most this many linked goals are read; links beyond it are counted as
/// omitted.
pub const MAX_READ_GOALS: usize = 32;
/// Project notes larger than this are skipped by the bounded walk.
const MAX_PROJECT_FILE_BYTES: u64 = 1_000_000;
/// The bounded walk stops after this many project notes.
const MAX_PROJECT_FILES: usize = 2_000;
/// Unreadable goal links listed by URL; more than this become a count only.
const MAX_LISTED_UNREADABLE: usize = 4;

const TITLE_CAP: usize = 160;
const WHY_CAP: usize = 400;
const EVIDENCE_CAP: usize = 300;
const HORIZON_CAP: usize = 60;
const GOAL_URL_MAX: usize = 300;
const PROJECT_URL_CAP: usize = 300;
const DESIRED_CAP: usize = 300;
const REFERENCE_CAP: usize = 120;
/// Last resort for a single remaining goal: shorten why/successEvidence.
const SHORT_CAP: usize = 120;

/// The note carried by the flag-only block for a hand-set `project_url`
/// Hollow cannot resolve. No sync recreates a locally deleted note,
/// so the flag names only what the user can do.
const UNRESOLVED_NOTE: &str =
    "WORKSPACE.md project_url names a project note Hollow could not find; restore the note or correct project_url.";

/// How a workspace's project was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSource {
    TrackerProjectPath,
    NotionProjectPath,
    ProjectUrl,
    Title,
}

impl LinkSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrackerProjectPath => "trackerProjectPath",
            Self::NotionProjectPath => "notionProjectPath",
            Self::ProjectUrl => "projectUrl",
            Self::Title => "title",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProjectLink {
    /// `moved` is true when the reference named a stale path and the note
    /// was found by its filename elsewhere under projects/ (a status move).
    Resolved {
        project: Project,
        via: LinkSource,
        moved: bool,
    },
    /// Only for a hand-set `project_url` that resolves to nothing while no
    /// other rule matched. `reference` is capped at 120 bytes.
    Unresolved {
        reference: String,
    },
    Unlinked,
}

/// Trim `s` and cap it at `max` bytes on a char boundary. A cut value ends
/// with `…`, which is counted within `max`.
pub(crate) fn cap_bytes(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let ellipsis = '…'.len_utf8();
    let mut end = max.saturating_sub(ellipsis);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", s[..end].trim_end())
}

/// The escaping `project_next_move::build_model_prompt` and the task-assist
/// block apply after JSON serialization, so a value cannot close their tags.
pub(crate) fn escape_tag_delims(s: &str) -> String {
    s.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// Length of `v` as the `<hollow-workspace>` envelope carries it: JSON with
/// only `<` escaped.
pub(crate) fn envelope_len(v: &Value) -> usize {
    serde_json::to_string(v)
        .map(|s| s.replace('<', "\\u003c").len())
        .unwrap_or(usize::MAX)
}

/// Length of `v` as the task-assist block carries it (all three escapes).
fn escaped_len(v: &Value) -> usize {
    serde_json::to_string(v)
        .map(|s| escape_tag_delims(&s).len())
        .unwrap_or(usize::MAX)
}

/// Every project note (`vault_projects::project_files`: projects/ and each
/// workspace's PROJECT.md), read with bounds: no link following, notes over
/// 1 MB skipped by metadata, and at most 2,000 notes. `vault_projects::list`
/// reads every file unbounded, and this walk can run inside the run lease on
/// every request.
fn bounded_projects(root: &Path) -> Vec<Project> {
    let mut out = Vec::new();
    for (rel, path) in vault_projects::project_files(root)
        .into_iter()
        .take(MAX_PROJECT_FILES)
    {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_PROJECT_FILE_BYTES {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if content.len() as u64 > MAX_PROJECT_FILE_BYTES {
            continue;
        }
        if let Some(project) = vault_projects::parse(&rel, &content, root) {
            out.push(project);
        }
    }
    out
}

/// Lazily walked project list, so one resolution walks projects/ at most once.
struct ProjectIndex<'a> {
    root: &'a Path,
    projects: Option<Vec<Project>>,
}

impl<'a> ProjectIndex<'a> {
    fn new(root: &'a Path) -> Self {
        Self {
            root,
            projects: None,
        }
    }

    fn all(&mut self) -> &[Project] {
        let root = self.root;
        self.projects.get_or_insert_with(|| bounded_projects(root))
    }
}

/// The unique element of `items`, or None for zero or several.
fn unique<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
    let first = items.next()?;
    if items.next().is_some() {
        return None;
    }
    Some(first)
}

/// A project reference as a vault-relative path (`vault:` optional): a
/// workspace's `workspaces/<id>/PROJECT.md` (where a tracker or the retired
/// Notion sync keeps a project's mirror) or a note under `projects/`, including the legacy flat
/// `projects/notion-<id>.md`. Traversal, hidden components, backslashes, NUL
/// and line breaks are rejected (`vault_projects::is_project_rel`).
fn project_rel(url: &str) -> Option<String> {
    let url = url.trim();
    let rel = url.strip_prefix("vault:").unwrap_or(url);
    if rel.len() > 1024 || !vault_projects::is_project_rel(rel) {
        return None;
    }
    Some(rel.to_string())
}

fn resolve_project_url_with(index: &mut ProjectIndex<'_>, url: &str) -> Option<(Project, bool)> {
    let rel = project_rel(url)?;
    let direct = crate::workspaces::read_file(index.root, &rel)
        .ok()
        .and_then(|note| vault_projects::parse(&rel, &note.content, index.root));
    if let Some(project) = direct {
        return Some((project, false));
    }
    // A stale path after a status move: the same filename elsewhere under
    // projects/, when exactly one note has it.
    let filename = rel.rsplit('/').next()?;
    let suffix = format!("/{filename}");
    let found = unique(index.all().iter().filter(|project| {
        project
            .vault_path
            .as_deref()
            .is_some_and(|path| path != rel && path.ends_with(&suffix))
    }))
    .cloned()?;
    Some((found, true))
}

/// Tracker bindings require the exact path and identity. Their shared
/// PROJECT.md filename cannot prove a status move to another workspace.
pub(crate) fn resolve_tracker_project_url(
    root: &Path,
    brief: &BTreeMap<String, String>,
    url: &str,
) -> Option<Project> {
    let rel = project_rel(url)?;
    let note = crate::workspaces::read_file(root, &rel).ok()?;
    let fm = crate::frontmatter::parse(&note.content);
    let id = fm
        .get("tracker_project")
        .filter(|id| !id.trim().is_empty())?;
    if brief
        .get("tracker_project")
        .is_some_and(|expected| expected != id)
        || brief
            .get("tracker")
            .is_some_and(|expected| Some(expected.as_str()) != fm.get("tracker"))
    {
        return None;
    }
    vault_projects::parse(&rel, &note.content, root)
}

/// Resolve a project reference to its note. Returns the project and whether
/// it was found by filename after a move. Never guesses by display name.
pub fn resolve_project_url(root: &Path, url: &str) -> Option<(Project, bool)> {
    resolve_project_url_with(&mut ProjectIndex::new(root), url)
}

/// A non-blank, trimmed WORKSPACE.md frontmatter value.
fn brief_field<'a>(brief: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    brief
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// The project a workspace belongs to: (1) `notion_project_path`, which the
/// retired Notion sync wrote (a legacy key, still read); (2) a hand-set `project_url`; (3) a unique exact trimmed
/// title match. A stale `notion_project_path` is only logged: nothing
/// repairs a locally deleted note, so there is nothing for the user to fix.
pub fn resolve_workspace_project(
    root: &Path,
    brief: &BTreeMap<String, String>,
    title: &str,
) -> ProjectLink {
    let mut index = ProjectIndex::new(root);
    if let Some(path) = brief_field(brief, "tracker_project_path") {
        if let Some(project) = resolve_tracker_project_url(root, brief, path) {
            return ProjectLink::Resolved {
                project,
                via: LinkSource::TrackerProjectPath,
                moved: false,
            };
        }
        // An explicit tracker binding must not fall through to an unrelated
        // project's title. Keep the missing destination visible.
        return ProjectLink::Unresolved {
            reference: cap_bytes(path, REFERENCE_CAP),
        };
    }
    if let Some(path) = brief_field(brief, "notion_project_path") {
        match resolve_project_url_with(&mut index, path) {
            Some((project, moved)) => {
                return ProjectLink::Resolved {
                    project,
                    via: LinkSource::NotionProjectPath,
                    moved,
                }
            }
            None => eprintln!(
                "[goal_context] notion_project_path {:?} names no readable project note; trying project_url and the title",
                cap_bytes(path, REFERENCE_CAP)
            ),
        }
    }
    let mut unresolved = None;
    if let Some(url) = brief_field(brief, "project_url") {
        match resolve_project_url_with(&mut index, url) {
            Some((project, moved)) => {
                return ProjectLink::Resolved {
                    project,
                    via: LinkSource::ProjectUrl,
                    moved,
                }
            }
            None => unresolved = Some(cap_bytes(url, REFERENCE_CAP)),
        }
    }
    let title = title.trim();
    if !title.is_empty() {
        if let Some(project) = unique(
            index
                .all()
                .iter()
                .filter(|project| project.name.trim() == title),
        )
        .cloned()
        {
            return ProjectLink::Resolved {
                project,
                via: LinkSource::Title,
                moved: false,
            };
        }
    }
    match unresolved {
        Some(reference) => ProjectLink::Unresolved { reference },
        None => ProjectLink::Unlinked,
    }
}

fn status_rank(status: GoalStatus) -> u8 {
    match status {
        GoalStatus::Active => 0,
        GoalStatus::Paused => 1,
        GoalStatus::Achieved => 2,
        GoalStatus::Dropped => 3,
    }
}

fn goal_value(goal: &Goal) -> Value {
    let mut map = Map::new();
    map.insert("title".into(), json!(cap_bytes(&goal.title, TITLE_CAP)));
    if goal.url.len() <= GOAL_URL_MAX {
        map.insert("url".into(), json!(goal.url));
    }
    map.insert("status".into(), json!(goal.status.as_str()));
    let optional = [
        ("horizon", goal.horizon.as_deref(), HORIZON_CAP),
        ("targetDate", goal.target_date.as_deref(), HORIZON_CAP),
        ("why", goal.why.as_deref(), WHY_CAP),
        (
            "successEvidence",
            goal.success_evidence.as_deref(),
            EVIDENCE_CAP,
        ),
    ];
    for (key, value, cap) in optional {
        if let Some(value) = value.map(|value| cap_bytes(value, cap)) {
            if !value.is_empty() {
                map.insert(key.into(), json!(value));
            }
        }
    }
    Value::Object(map)
}

/// The block's parts, rendered and re-measured while [`fit`] sheds them.
struct Block {
    project: Map<String, Value>,
    goals: Vec<Value>,
    unreadable: Vec<String>,
    unreadable_listed: bool,
    omitted: usize,
}

impl Block {
    fn render(&self) -> Value {
        let mut map = Map::new();
        map.insert("project".into(), Value::Object(self.project.clone()));
        map.insert("goals".into(), Value::Array(self.goals.clone()));
        if !self.unreadable.is_empty() {
            if self.unreadable_listed {
                let listed: Vec<Value> = self
                    .unreadable
                    .iter()
                    .take(MAX_LISTED_UNREADABLE)
                    .map(|url| json!(url))
                    .collect();
                map.insert("unreadableGoals".into(), Value::Array(listed));
            }
            if !self.unreadable_listed || self.unreadable.len() > MAX_LISTED_UNREADABLE {
                map.insert("unreadableGoalCount".into(), json!(self.unreadable.len()));
            }
        }
        if self.omitted > 0 {
            map.insert(
                "truncated".into(),
                json!(format!("{} linked goal(s) omitted", self.omitted)),
            );
        }
        Value::Object(map)
    }
}

/// Shed parts until the block measures within `cap`, least valuable first:
/// (1) the unreadable URL list becomes a count; (2) the project's
/// desiredState goes; (3) goals are popped from the end (lowest status
/// first) while more than one remains; (4) the last goal's why and
/// successEvidence shrink to 120 bytes. Only then None.
fn fit(mut block: Block, cap: usize, measure: fn(&Value) -> usize) -> Option<Value> {
    let fits = |block: &Block| {
        let value = block.render();
        (measure(&value) <= cap).then_some(value)
    };
    if let Some(value) = fits(&block) {
        return Some(value);
    }
    if block.unreadable_listed && !block.unreadable.is_empty() {
        block.unreadable_listed = false;
        if let Some(value) = fits(&block) {
            return Some(value);
        }
    }
    if block.project.remove("desiredState").is_some() {
        if let Some(value) = fits(&block) {
            return Some(value);
        }
    }
    while block.goals.len() > 1 {
        block.goals.pop();
        block.omitted += 1;
        if let Some(value) = fits(&block) {
            return Some(value);
        }
    }
    if let Some(Value::Object(goal)) = block.goals.first_mut() {
        for key in ["why", "successEvidence"] {
            if let Some(Value::String(text)) = goal.get_mut(key) {
                *text = cap_bytes(text, SHORT_CAP);
            }
        }
    }
    fits(&block)
}

/// Build the block for a resolved project from its linked goals, or None
/// when no linked goal parses. `unread` counts links beyond [`MAX_READ_GOALS`].
fn build_block(
    project: &Project,
    via: Option<LinkSource>,
    moved: bool,
    linked: Vec<LinkedGoal>,
    unread: usize,
    measure: fn(&Value) -> usize,
) -> Option<Value> {
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    for item in linked {
        match item {
            LinkedGoal::Found(goal) => found.push(goal),
            LinkedGoal::Unreadable { url, .. } => unreadable.push(url),
        }
    }
    if found.is_empty() {
        return None;
    }
    // Stable: link order is kept within a status.
    found.sort_by_key(|goal| status_rank(goal.status));

    let mut summary = Map::new();
    summary.insert("title".into(), json!(cap_bytes(&project.name, TITLE_CAP)));
    summary.insert(
        "url".into(),
        json!(cap_bytes(&project.url, PROJECT_URL_CAP)),
    );
    summary.insert(
        "status".into(),
        json!(cap_bytes(&project.status, HORIZON_CAP)),
    );
    if let Some(via) = via {
        summary.insert("linkedBy".into(), json!(via.as_str()));
    }
    if moved {
        summary.insert("moved".into(), json!(true));
    }
    if let Some(desired) = project
        .desired_state
        .as_deref()
        .map(|value| cap_bytes(value, DESIRED_CAP))
        .filter(|value| !value.is_empty())
    {
        summary.insert("desiredState".into(), json!(desired));
    }
    let unreadable_listed = unreadable.len() <= MAX_LISTED_UNREADABLE;
    fit(
        Block {
            project: summary,
            goals: found.iter().map(goal_value).collect(),
            unreadable,
            unreadable_listed,
            omitted: unread,
        },
        ALIGNMENT_CAP,
        measure,
    )
}

/// Links a project has beyond the [`MAX_READ_GOALS`] that are read.
/// `goal_urls` is already trimmed and de-duplicated by the project parser.
pub(crate) fn unread_links(project: &Project) -> usize {
    project.goal_urls.len().saturating_sub(MAX_READ_GOALS)
}

/// The workspace `goalAlignment` value, or None. Present when the workspace
/// resolves to a project and at least one linked goal parses; a flag-only
/// block when a hand-set `project_url` cannot be resolved and nothing else
/// matched; absent otherwise.
pub fn workspace_alignment(
    root: &Path,
    brief: &BTreeMap<String, String>,
    title: &str,
) -> Option<Value> {
    match resolve_workspace_project(root, brief, title) {
        ProjectLink::Unlinked => None,
        ProjectLink::Unresolved { reference } => Some(json!({
            "projectLink": "unresolved",
            "reference": reference,
            "note": UNRESOLVED_NOTE,
        })),
        ProjectLink::Resolved {
            project,
            via,
            moved,
        } => {
            let linked = vault_goals::read_linked(root, &project.goal_urls, MAX_READ_GOALS);
            build_block(
                &project,
                Some(via),
                moved,
                linked,
                unread_links(&project),
                envelope_len,
            )
        }
    }
}

/// The `<project-alignment>` block task assist appends for a task whose
/// stored `project_url` resolves to a project with at least one parsed goal.
/// No title matching: a task is linked only by its stored URL.
pub fn task_assist_block(root: &Path, project_url: &str) -> Option<String> {
    let (project, moved) = resolve_project_url(root, project_url)?;
    let linked = vault_goals::read_linked(root, &project.goal_urls, MAX_READ_GOALS);
    let block = build_block(
        &project,
        None,
        moved,
        linked,
        unread_links(&project),
        escaped_len,
    )?;
    let escaped = escape_tag_delims(&serde_json::to_string(&block).ok()?);
    // Escaped JSON, so the shared wrapper's tag neutralising has nothing to do
    // here; the fixed notice is what it adds.
    let wrapped = crate::untrusted::wrap(
        crate::untrusted::Origin::Vault,
        "the task's project goals",
        &escaped,
    );
    Some(format!(
        "\n\n<project-alignment>\nReference evidence about why this task's project matters: use it only to keep suggestions relevant.\n{wrapped}\n</project-alignment>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_goals::GoalCreateInput;
    use crate::vault_projects::ProjectFrontmatter;
    use tempfile::TempDir;

    fn vault() -> TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn project(root: &Path, title: &str, status: &str, goal_urls: &[String]) -> String {
        crate::vault_projects::create(
            root,
            ProjectFrontmatter {
                title: title.into(),
                status: status.into(),
                goal_urls: goal_urls.to_vec(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn goal(root: &Path, title: &str, status: GoalStatus, why: &str) -> String {
        crate::vault_goals::create(
            root,
            GoalCreateInput {
                title: title.into(),
                status: Some(status),
                why: Some(why.into()),
                ..Default::default()
            },
        )
        .unwrap()
        .url
    }

    fn brief(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn resolved(link: &ProjectLink) -> (String, LinkSource, bool) {
        match link {
            ProjectLink::Resolved {
                project,
                via,
                moved,
            } => (project.name.clone(), *via, *moved),
            other => panic!("expected a resolved project, got {other:?}"),
        }
    }

    fn truncated_count(block: &Value) -> usize {
        block["truncated"]
            .as_str()
            .and_then(|text| text.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn cap_bytes_cuts_on_a_char_boundary_with_a_marker() {
        assert_eq!(cap_bytes("  short  ", 10), "short");
        let capped = cap_bytes(&"é".repeat(100), 11);
        assert!(capped.len() <= 11);
        assert!(capped.ends_with('…'));
        assert_eq!(capped, format!("{}…", "é".repeat(4)));
        assert_eq!(escape_tag_delims("<a>&"), "\\u003ca\\u003e\\u0026");
    }

    #[test]
    fn workspace_project_resolves_notion_path_then_project_url_then_title() {
        let dir = vault();
        let root = dir.path();
        let alpha = project(root, "Alpha", "active", &[]);
        let beta = project(root, "Beta", "active", &[]);
        project(root, "Gamma", "active", &[]);
        // (d) a status move leaves project_url stale; the filename finds it.
        let moved_rel = project(root, "Moved", "active", &[]);
        fs::create_dir_all(root.join("projects/done")).unwrap();
        fs::rename(root.join(&moved_rel), root.join("projects/done/Moved.md")).unwrap();
        // (e) Notion-created notes are flat.
        fs::write(
            root.join("projects/notion-abc.md"),
            "---\ntitle: Notion Abc\nstatus: Active\nnotion_id: abc\n---\n",
        )
        .unwrap();
        // (f) two same-titled projects.
        project(root, "Twin", "active", &[]);
        project(root, "Twin", "planning", &[]);

        // (a) a valid notion_project_path wins over project_url.
        let link = resolve_workspace_project(
            root,
            &brief(&[
                ("notion_project_path", &alpha),
                ("project_url", &format!("vault:{beta}")),
            ]),
            "Gamma",
        );
        assert_eq!(
            resolved(&link),
            ("Alpha".to_string(), LinkSource::NotionProjectPath, false)
        );
        // (b) a stale notion_project_path falls through to project_url.
        let link = resolve_workspace_project(
            root,
            &brief(&[
                ("notion_project_path", "projects/notion-gone.md"),
                ("project_url", &format!("vault:{beta}")),
            ]),
            "Gamma",
        );
        assert_eq!(
            resolved(&link),
            ("Beta".to_string(), LinkSource::ProjectUrl, false)
        );
        // (c) a unique exact title.
        let link = resolve_workspace_project(root, &brief(&[]), "  Gamma ");
        assert_eq!(
            resolved(&link),
            ("Gamma".to_string(), LinkSource::Title, false)
        );
        // (d)
        let link = resolve_workspace_project(
            root,
            &brief(&[("project_url", &format!("vault:{moved_rel}"))]),
            "Nothing",
        );
        assert_eq!(
            resolved(&link),
            ("Moved".to_string(), LinkSource::ProjectUrl, true)
        );
        match &link {
            ProjectLink::Resolved { project, .. } => {
                assert_eq!(project.url, "vault:projects/done/Moved.md")
            }
            _ => unreachable!(),
        }
        // (e)
        let link = resolve_workspace_project(
            root,
            &brief(&[("notion_project_path", "projects/notion-abc.md")]),
            "Nothing",
        );
        assert_eq!(
            resolved(&link),
            (
                "Notion Abc".to_string(),
                LinkSource::NotionProjectPath,
                false
            )
        );
        // (f) an ambiguous title is never guessed.
        assert!(matches!(
            resolve_workspace_project(root, &brief(&[]), "Twin"),
            ProjectLink::Unlinked
        ));
        // (g) a broken hand-set project_url with no title match is flagged.
        match resolve_workspace_project(
            root,
            &brief(&[("project_url", "vault:projects/active/Gone.md")]),
            "Nothing",
        ) {
            ProjectLink::Unresolved { reference } => {
                assert_eq!(reference, "vault:projects/active/Gone.md")
            }
            other => panic!("expected Unresolved, got {other:?}"),
        }
        // A broken project_url still yields to a unique title.
        let link = resolve_workspace_project(
            root,
            &brief(&[("project_url", "vault:projects/active/Gone.md")]),
            "Gamma",
        );
        assert_eq!(
            resolved(&link),
            ("Gamma".to_string(), LinkSource::Title, false)
        );
        // (h) a stale notion_project_path alone is silence, not a flag.
        assert!(matches!(
            resolve_workspace_project(
                root,
                &brief(&[("notion_project_path", "projects/notion-deleted.md")]),
                "Nothing",
            ),
            ProjectLink::Unlinked
        ));
        // (i) traversal and non-project references never resolve.
        fs::write(root.join("secret.md"), "---\ntitle: Secret\n---\n").unwrap();
        fs::create_dir_all(root.join("tasks")).unwrap();
        fs::write(root.join("tasks/a.md"), "---\ntitle: Task\n---\n").unwrap();
        for bad in [
            "vault:projects/../secret.md",
            "projects/active/../../secret.md",
            "vault:tasks/a.md",
            "vault:/etc/passwd",
            "vault:projects/.hidden.md",
            "vault:projects\\active\\Alpha.md",
        ] {
            assert!(resolve_project_url(root, bad).is_none(), "{bad} resolved");
        }
        assert!(
            resolve_project_url(root, &alpha).is_some(),
            "a bare path resolves"
        );
    }

    #[test]
    fn bounded_project_walk_skips_oversized_files() {
        let dir = vault();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/active")).unwrap();
        let mut huge = String::from("---\ntitle: Huge\nstatus: active\n---\n\n");
        huge.push_str(&"x".repeat(MAX_PROJECT_FILE_BYTES as usize + 1));
        fs::write(root.join("projects/active/huge.md"), huge).unwrap();
        assert!(matches!(
            resolve_workspace_project(root, &brief(&[]), "Huge"),
            ProjectLink::Unlinked
        ));
        project(root, "Normal", "active", &[]);
        let link = resolve_workspace_project(root, &brief(&[]), "Normal");
        assert_eq!(
            resolved(&link),
            ("Normal".to_string(), LinkSource::Title, false)
        );
    }

    #[test]
    fn alignment_block_is_capped_active_first_and_counts_unread_links() {
        let dir = vault();
        let root = dir.path();
        let why = "é".repeat(600);
        let mut urls = Vec::new();
        for n in 0..9 {
            let status = if n % 2 == 0 {
                GoalStatus::Achieved
            } else {
                GoalStatus::Dropped
            };
            urls.push(goal(root, &format!("Old goal {n:02}"), status, &why));
        }
        urls.push(goal(root, "Current goal", GoalStatus::Active, &why));
        project(root, "Capped", "active", &urls);
        let block = workspace_alignment(root, &brief(&[]), "Capped").expect("a block");
        assert!(envelope_len(&block) <= ALIGNMENT_CAP);
        let goals = block["goals"].as_array().unwrap();
        assert!(!goals.is_empty());
        assert_eq!(goals[0]["title"], "Current goal");
        assert_eq!(goals[0]["status"], "active");
        assert!(goals.len() < 10);
        assert_eq!(truncated_count(&block), 10 - goals.len());
        for goal in goals {
            let why = goal["why"].as_str().unwrap();
            assert!(why.len() <= WHY_CAP);
            assert!(why.ends_with('…'));
            assert!(why.trim_end_matches('…').chars().all(|c| c == 'é'));
        }
        assert_eq!(block["project"]["linkedBy"], "title");

        // 34 links: the two beyond MAX_READ_GOALS are never read, and counted.
        for n in 0..24 {
            urls.push(goal(
                root,
                &format!("Later goal {n:02}"),
                GoalStatus::Paused,
                "Later",
            ));
        }
        assert_eq!(urls.len(), 34);
        project(root, "Wide", "active", &urls);
        let block = workspace_alignment(root, &brief(&[]), "Wide").expect("a block");
        assert!(envelope_len(&block) <= ALIGNMENT_CAP);
        let goals = block["goals"].as_array().unwrap();
        assert_eq!(goals[0]["title"], "Current goal");
        assert_eq!(truncated_count(&block), 34 - goals.len());
    }

    #[test]
    fn one_valid_goal_survives_long_unreadable_links() {
        let dir = vault();
        let root = dir.path();
        let valid = goal(
            root,
            "The one that parses",
            GoalStatus::Active,
            "It matters",
        );
        let mut urls = vec![valid];
        for n in 0..4 {
            let name = format!("{n}{}", "m".repeat(179));
            fs::write(
                root.join(format!("goals/{name}.md")),
                "---\ntitle: Broken\nstatus: someday\n---\n",
            )
            .unwrap();
            urls.push(format!("vault:goals/{name}.md"));
        }
        urls.push("b".repeat(3 * 1024));
        project(root, "Messy", "active", &urls);
        let block = workspace_alignment(root, &brief(&[]), "Messy").expect("a block");
        assert!(envelope_len(&block) <= ALIGNMENT_CAP);
        assert_eq!(block["goals"][0]["title"], "The one that parses");
        assert_eq!(block["unreadableGoalCount"], 5);
        if let Some(listed) = block["unreadableGoals"].as_array() {
            assert!(listed.len() <= MAX_LISTED_UNREADABLE);
            assert!(listed
                .iter()
                .all(|url| url.as_str().unwrap().len() <= REFERENCE_CAP));
        }
    }

    #[test]
    fn fit_sheds_diagnostics_and_desired_state_before_any_goal() {
        let many = |n: usize| {
            (0..n)
                .map(|i| json!({"title": format!("g{i}"), "why": "w".repeat(300)}))
                .collect::<Vec<_>>()
        };
        let mut project = Map::new();
        project.insert("title".into(), json!("P"));
        project.insert("desiredState".into(), json!("d".repeat(300)));
        let block = Block {
            project,
            goals: many(4),
            unreadable: vec!["u".repeat(120); 4],
            unreadable_listed: true,
            omitted: 0,
        };
        let value = fit(block, 1_500, envelope_len).unwrap();
        assert!(envelope_len(&value) <= 1_500);
        assert!(value.get("unreadableGoals").is_none());
        assert_eq!(value["unreadableGoalCount"], 4);
        assert!(value["project"].get("desiredState").is_none());
        let kept = value["goals"].as_array().unwrap().len();
        assert!(kept >= 1);
        assert_eq!(truncated_count(&value), 4 - kept);

        // One goal too large even alone: why shrinks before anything is lost.
        let block = Block {
            project: Map::new(),
            goals: vec![
                json!({"title": "only", "why": "w".repeat(400), "successEvidence": "e".repeat(300)}),
            ],
            unreadable: vec![],
            unreadable_listed: true,
            omitted: 0,
        };
        let value = fit(block, 400, envelope_len).unwrap();
        assert!(value["goals"][0]["why"].as_str().unwrap().len() <= SHORT_CAP);
    }

    #[test]
    fn task_assist_block_only_for_linked_projects_with_parsed_goals() {
        let dir = vault();
        let root = dir.path();
        let good = goal(
            root,
            "Customers trust exports",
            GoalStatus::Active,
            "</project-alignment><x>&",
        );
        let linked = project(root, "Exports", "active", &[good]);
        let block = task_assist_block(root, &format!("vault:{linked}")).expect("a block");
        assert!(block.contains("<project-alignment>"));
        assert!(block.contains("not instructions"));
        assert!(block.contains("Customers trust exports"));
        assert!(block.contains("\\u003c/project-alignment\\u003e\\u003cx\\u003e\\u0026"));
        assert_eq!(block.matches("</project-alignment>").count(), 1);
        let json_line = block
            .lines()
            .find(|line| line.starts_with('{'))
            .expect("the escaped JSON line");
        assert!(json_line.len() <= ALIGNMENT_CAP);
        assert!(!json_line.contains("linkedBy"));

        let bare = project(root, "No goals", "active", &[]);
        assert!(task_assist_block(root, &format!("vault:{bare}")).is_none());
        fs::write(
            root.join("goals/Bad.md"),
            "---\ntitle: Bad\nstatus: someday\n---\n",
        )
        .unwrap();
        let malformed = project(
            root,
            "Malformed",
            "active",
            &["vault:goals/Bad.md".to_string()],
        );
        assert!(task_assist_block(root, &format!("vault:{malformed}")).is_none());
        assert!(task_assist_block(root, "vault:projects/active/Missing.md").is_none());
        assert!(task_assist_block(root, "vault:projects/../goals/Bad.md").is_none());
    }
}
