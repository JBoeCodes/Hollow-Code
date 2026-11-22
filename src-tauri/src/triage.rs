//! The triage inbox (overnight 10-11 B6): what arrived, and where it
//! probably belongs. Read-mostly, local, no model.
//!
//! Three things live here; everything else the inbox shows comes from stores
//! that already exist (Waiting on you's `list_needs_you`, the Plaud reader in
//! `recordings.rs`):
//!
//! - **Arrivals** ([`arrivals`]): imports in `library/imports/<name>/` from
//!   the last [`IMPORT_DAYS`] days, and open tasks created in the last
//!   [`TASK_DAYS`] days by something other than the person (a Notion sync, a
//!   meeting, the heartbeat, an agent: the task carries `notion_id`, `source`
//!   or `origin`). A task the inbox itself made (`origin: triage:…`) is the
//!   person's own and never comes back as an arrival.
//! - **Suggested workspace** ([`suggest`]): for each item's text, the current
//!   workspace it most likely belongs to, with its reasons, from the same
//!   signals as the map's connections (`workspace_connections.rs`): the
//!   workspace's title or project named in the text ([`W_TITLE`] /
//!   [`W_PROJECT`]), the same people ([`W_PERSON`] each, at most [`W_CAP`]),
//!   and TF-IDF shared terms with the title and brief (cosine × 0.8, at most
//!   [`W_CAP`]); combined `1 − Π(1 − w)` and offered only at [`SUGGEST_MIN`].
//! - **Inbox state** ([`mark`]): `{vault}/.hollow/triage.json`, which items
//!   the person dismissed or filed (and where), so they leave the inbox. It
//!   holds ids only; dismissing never deletes, resolves or moves anything.
//!   Entries older than [`STATE_KEEP_DAYS`] are dropped on the next write
//!   (by then their items are too old to arrive anyway).

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::workspace_connections::{combine, emails, grams, people_keys, words, TERM_DF_MAX};

/// Imports newer than this many days arrive.
pub(crate) const IMPORT_DAYS: i64 = 14;
/// Tasks created in the last this many days arrive.
pub(crate) const TASK_DAYS: i64 = 3;
/// Most tasks one read returns (newest first): a first Notion sync can bring
/// hundreds at once.
const TASKS_MAX: usize = 60;
/// Most notes of one import listed (and attached when it is filed).
pub(crate) const IMPORT_FILES_MAX: usize = 25;
/// State entries older than this are dropped on write.
const STATE_KEEP_DAYS: i64 = 60;
/// A suggestion needs at least this combined weight.
pub(crate) const SUGGEST_MIN: f64 = 0.25;
/// Most items one suggest call scores, and the text it reads of each.
const SUGGEST_ITEMS_MAX: usize = 500;
const SUGGEST_TEXT_MAX: usize = 4096;
/// The shortest workspace title or project name that counts when named.
const NAME_MIN_CHARS: usize = 4;

const W_TITLE: f64 = 0.6;
const W_PROJECT: f64 = 0.6;
const W_PERSON: f64 = 0.2;
const W_CAP: f64 = 0.5;
/// The cosine shared terms need before they count.
const TERMS_MIN: f64 = 0.15;

static LOCK: Mutex<()> = Mutex::new(());

/// An import folder that arrived.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriageImport {
    /// `import:<folder name>`.
    pub id: String,
    pub title: String,
    /// `library/imports/<folder name>`.
    pub path: String,
    /// Vault-relative Markdown notes in it, at most [`IMPORT_FILES_MAX`].
    pub files: Vec<String>,
    /// Every note in it (may exceed `files`).
    pub file_count: usize,
    /// The folder's last change (RFC 3339).
    pub at: Option<String>,
    /// The first note's opening text, for the detail and the suggestion.
    pub excerpt: String,
}

/// A task that arrived from somewhere other than the person.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriageTask {
    /// `task:<vault path>`.
    pub id: String,
    /// Vault-relative path of the task note.
    pub path: String,
    pub title: String,
    /// Where it came from, in words: "Notion", "a meeting", "the heartbeat",
    /// "a plan", "an agent".
    pub from: String,
    pub at: Option<String>,
    pub project: Option<String>,
    pub due: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Filed {
    pub workspace_id: String,
    pub at: String,
}

/// Which items left the inbox, by id.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriageState {
    /// id → when it was dismissed (RFC 3339).
    #[serde(default)]
    pub dismissed: BTreeMap<String, String>,
    /// id → where and when it was filed.
    #[serde(default)]
    pub filed: BTreeMap<String, Filed>,
    /// id → the task made from it here and when (overnight S7: the row used
    /// to stay after its task was made, linked to nothing).
    #[serde(default)]
    pub tasked: BTreeMap<String, Tasked>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tasked {
    /// `vault:tasks/…`.
    pub task: String,
    pub at: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriageArrivals {
    pub imports: Vec<TriageImport>,
    pub tasks: Vec<TriageTask>,
    pub state: TriageState,
    /// Sources that couldn't be read, in words; their items are missing.
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestInput {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub workspace_id: String,
    pub title: String,
    /// 0–1.
    pub score: f64,
    /// Heaviest first, in words ("names the project TipFlow").
    pub reasons: Vec<String>,
}

/// Whether an inbox row stops arriving on its own (an import, a task or a
/// recording past its window), so its state entry may be pruned.
fn ages_out(id: &str) -> bool {
    ["import:", "task:", "recording:"]
        .iter()
        .any(|p| id.starts_with(p))
}

fn state_path(vault: &Path) -> PathBuf {
    vault.join(".hollow").join("triage.json")
}

fn rfc3339(t: std::time::SystemTime) -> String {
    DateTime::<Utc>::from(t).to_rfc3339()
}

/// A `created:` value as a time: RFC 3339, or a bare date (local midnight).
fn parse_when(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(raw.get(..10)?, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

/// "notes-on-tip-pooling" → "Notes on tip pooling".
fn humanize(slug: &str) -> String {
    let words: String = slug
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => slug.to_string(),
    }
}

/// A note's opening prose: frontmatter dropped, at most `max` characters.
fn excerpt_of(path: &Path, max: usize) -> (Option<String>, String) {
    let text = crate::brief_read::read_prefix(path, 16 * 1024).unwrap_or_default();
    let (fm, body) = crate::frontmatter::parse_map(&text);
    let title = fm
        .get("title")
        .map(|t| t.trim().trim_matches('"').to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| {
            body.lines()
                .find_map(|l| l.strip_prefix("# "))
                .map(|t| t.trim().to_string())
        });
    let prose: String = body
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (title, prose.chars().take(max).collect())
}

fn imports(vault: &Path, now: DateTime<Utc>) -> Result<Vec<TriageImport>, String> {
    let root = vault.join("library").join("imports");
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("imports ({e})")),
    };
    let cutoff = now - Duration::days(IMPORT_DAYS);
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        if DateTime::<Utc>::from(modified) < cutoff {
            continue;
        }
        let mut notes: Vec<String> = std::fs::read_dir(entry.path())
            .map(|d| {
                d.flatten()
                    .map(|f| f.file_name().to_string_lossy().to_string())
                    .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        if notes.is_empty() {
            continue;
        }
        notes.sort();
        // An index or overview note leads when there is one.
        if let Some(i) = notes.iter().position(|n| {
            matches!(
                n.as_str(),
                "index.md" | "README.md" | "overview.md" | "_index.md"
            )
        }) {
            let lead = notes.remove(i);
            notes.insert(0, lead);
        }
        let (title, excerpt) = excerpt_of(&entry.path().join(&notes[0]), 600);
        let rel = format!("library/imports/{name}");
        out.push(TriageImport {
            id: format!("import:{name}"),
            title: title.unwrap_or_else(|| humanize(&name)),
            path: rel.clone(),
            file_count: notes.len(),
            files: notes
                .iter()
                .take(IMPORT_FILES_MAX)
                .map(|n| format!("{rel}/{n}"))
                .collect(),
            at: Some(rfc3339(modified)),
            excerpt,
        });
    }
    out.sort_by(|a, b| b.at.cmp(&a.at).then(a.id.cmp(&b.id)));
    Ok(out)
}

/// Where a task came from, in words, or None when the person made it.
fn task_from(fm: &BTreeMap<String, String>) -> Option<String> {
    let get = |k: &str| fm.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let origin = get("origin").unwrap_or("");
    if origin.starts_with("triage:") {
        return None;
    }
    if get("notion_id").is_some() || get("notion_url").is_some() {
        return Some("Notion".into());
    }
    if !origin.is_empty() {
        let kind = origin.split(':').next().unwrap_or("");
        return Some(
            match kind {
                "meeting" => "a meeting",
                "heartbeat" => "the heartbeat",
                "plan" => "a plan",
                "project" => "a project",
                _ => "an agent",
            }
            .into(),
        );
    }
    get("source").map(|s| {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    })
}

fn tasks(vault: &Path, now: DateTime<Utc>) -> Vec<TriageTask> {
    let cutoff = now - Duration::days(TASK_DAYS);
    let mut out: Vec<(DateTime<Utc>, TriageTask)> = Vec::new();
    for t in crate::vault_tasks::list(vault) {
        if crate::vault_tasks::is_terminal_status(&t.status) {
            continue;
        }
        let Some(rel) = t.url.strip_prefix("vault:").map(str::to_string) else {
            continue;
        };
        let text = crate::brief_read::read_prefix(&vault.join(&rel), 8 * 1024).unwrap_or_default();
        let (fm, _) = crate::frontmatter::parse_map(&text);
        let Some(from) = task_from(&fm) else {
            continue;
        };
        let Some(at) = fm.get("created").and_then(|c| parse_when(c)) else {
            continue;
        };
        if at < cutoff || at > now + Duration::days(1) {
            continue;
        }
        out.push((
            at,
            TriageTask {
                id: format!("task:{rel}"),
                path: rel,
                title: t.title.clone(),
                from,
                at: Some(at.to_rfc3339()),
                project: t.project_name.clone().filter(|p| !p.is_empty()),
                due: t.due.clone().filter(|d| !d.is_empty()),
            },
        ));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    out.into_iter().take(TASKS_MAX).map(|(_, t)| t).collect()
}

/// The inbox state; a damaged file is set aside (and said so) by the reader.
pub fn read_state(vault: &Path) -> Result<TriageState, String> {
    Ok(
        crate::state_file::read_json::<TriageState>(&state_path(vault), "the inbox state")?
            .unwrap_or_default(),
    )
}

/// Everything that arrived, with the inbox state. A source that fails is
/// named in `unavailable`; the others still list.
pub fn arrivals(vault: &Path) -> TriageArrivals {
    arrivals_at(vault, Utc::now())
}

pub(crate) fn arrivals_at(vault: &Path, now: DateTime<Utc>) -> TriageArrivals {
    let mut unavailable = Vec::new();
    let imports = imports(vault, now).unwrap_or_else(|e| {
        unavailable.push(e);
        vec![]
    });
    let state = read_state(vault).unwrap_or_else(|e| {
        unavailable.push(e);
        TriageState::default()
    });
    TriageArrivals {
        imports,
        tasks: tasks(vault, now),
        state,
        unavailable,
    }
}

/// Record that `id` left the inbox (`dismiss`, `file` into `workspace_id`)
/// or came back (`restore`). Returns the new state.
#[cfg(test)]
pub fn mark(
    vault: &Path,
    id: &str,
    action: &str,
    workspace_id: Option<&str>,
) -> Result<TriageState, String> {
    mark_at(vault, id, action, workspace_id, Utc::now())
}

#[cfg(test)]
pub(crate) fn mark_at(
    vault: &Path,
    id: &str,
    action: &str,
    workspace_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<TriageState, String> {
    mark_inner(vault, id, action, workspace_id, None, now)
}

/// [`mark_at`], plus `task_ref` for the `task` action (the `vault:tasks/…`
/// ref of the task made from the row).
pub(crate) fn mark_inner(
    vault: &Path,
    id: &str,
    action: &str,
    workspace_id: Option<&str>,
    task_ref: Option<&str>,
    now: DateTime<Utc>,
) -> Result<TriageState, String> {
    let id = id.trim();
    if id.is_empty() || id.len() > 1024 || id.contains('\n') {
        return Err("Not an inbox item id.".into());
    }
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut state = read_state(vault)?;
    let at = now.to_rfc3339();
    match action {
        "dismiss" => {
            state.filed.remove(id);
            state.dismissed.insert(id.to_string(), at);
        }
        "file" => {
            let ws = workspace_id
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .ok_or("Filing needs a workspace.")?;
            crate::workspaces::summary(vault, ws).map_err(|e| format!("No workspace {ws}: {e}"))?;
            state.dismissed.remove(id);
            state.filed.insert(
                id.to_string(),
                Filed {
                    workspace_id: ws.to_string(),
                    at,
                },
            );
        }
        "task" => {
            // Only an arrival leaves with its task: a review, question,
            // Result or failure still waits until it is resolved.
            if !ages_out(id) {
                return Err(
                    "Only a recording, an import or a new task leaves the inbox with its task."
                        .into(),
                );
            }
            let task = task_ref
                .map(str::trim)
                .filter(|t| {
                    t.starts_with("vault:tasks/")
                        && t.ends_with(".md")
                        && t.len() <= 1024
                        && !t.contains(['\n', '\\'])
                        && !t.split('/').any(|part| part == "..")
                })
                .ok_or("Linking a task needs its vault:tasks/… ref.")?;
            state.tasked.insert(
                id.to_string(),
                Tasked {
                    task: task.to_string(),
                    at,
                },
            );
        }
        "restore" => {
            state.dismissed.remove(id);
            state.filed.remove(id);
            state.tasked.remove(id);
        }
        other => return Err(format!("Unknown inbox action {other:?}.")),
    }
    // Only arrivals age out of the inbox; a question, a Result or a review
    // item waits until it is resolved, so its dismissal is kept.
    let keep = now - Duration::days(STATE_KEEP_DAYS);
    let fresh = |id: &str, when: &str| !ages_out(id) || parse_when(when).is_none_or(|t| t >= keep);
    state.dismissed.retain(|id, when| fresh(id, when));
    state.filed.retain(|id, f| fresh(id, &f.at));
    state.tasked.retain(|id, t| fresh(id, &t.at));
    let path = state_path(vault);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, &bytes)?;
    Ok(state)
}

/// What one workspace offers to match against.
struct Profile {
    id: String,
    title: String,
    /// The title as lowercase space-joined words, when long enough to count.
    title_key: Option<String>,
    project: Option<(String, String)>,
    people: HashSet<String>,
    terms: HashMap<String, f64>,
}

fn name_key(name: &str) -> Option<String> {
    let joined = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let n = joined.split(' ').count();
    (joined.chars().count() >= NAME_MIN_CHARS && (1..=4).contains(&n)).then_some(joined)
}

fn brief_body(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            return &rest[end + 4..];
        }
    }
    text
}

fn profiles(
    vault: &Path,
    people: &[crate::workspace_connections::PersonKey],
) -> Result<(Vec<Profile>, HashMap<String, usize>), String> {
    let mut out = Vec::new();
    let mut df: HashMap<String, usize> = HashMap::new();
    for ws in crate::workspaces::list(vault)? {
        if crate::workspaces::is_builtin(&ws.id)
            || ws.status.trim().eq_ignore_ascii_case("archived")
        {
            continue;
        }
        let Ok(dir) = crate::workspaces::folder(vault, &ws.id) else {
            continue;
        };
        let brief_text = crate::brief_read::read_prefix(
            &dir.join("WORKSPACE.md"),
            crate::brief_read::BRIEF_PREFIX,
        )
        .unwrap_or_default();
        let (fm, _) = crate::frontmatter::parse_map(&brief_text);
        let body = brief_body(&brief_text);
        let g = grams(body);
        let mails = emails(body);
        let mut terms: HashMap<String, f64> = HashMap::new();
        let title_twice = format!("{} {}", ws.title, ws.title);
        for w in words(&title_twice).chain(words(body)) {
            *terms.entry(w).or_default() += 1.0;
        }
        for w in terms.keys() {
            *df.entry(w.clone()).or_default() += 1;
        }
        out.push(Profile {
            id: ws.id.clone(),
            title: ws.title.clone(),
            title_key: name_key(&ws.title),
            project: crate::brief_read::project_of(vault, &fm),
            people: people
                .iter()
                .filter(|p| p.needles.iter().any(|n| n.found(&g, &mails)))
                .map(|p| p.name.clone())
                .collect(),
            terms,
        });
    }
    Ok((out, df))
}

fn idf(df: &HashMap<String, usize>, n: usize, w: &str) -> Option<f64> {
    let d = *df.get(w)?;
    (d <= TERM_DF_MAX).then(|| (1.0 + n as f64 / d as f64).ln())
}

fn list_words(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        _ => format!(
            "{} and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

/// The best workspace for each item, or none below [`SUGGEST_MIN`].
pub fn suggest(
    vault: &Path,
    items: &[SuggestInput],
) -> Result<BTreeMap<String, Suggestion>, String> {
    let people = people_keys(vault);
    let (profiles, df) = profiles(vault, &people)?;
    let n = profiles.len();
    // Each workspace's TF-IDF vector and norm, once.
    let vectors: Vec<(HashMap<&str, f64>, f64)> = profiles
        .iter()
        .map(|p| {
            let v: HashMap<&str, f64> = p
                .terms
                .iter()
                .filter_map(|(w, tf)| idf(&df, n, w).map(|i| (w.as_str(), tf * i)))
                .collect();
            let norm = v.values().map(|x| x * x).sum::<f64>().sqrt();
            (v, norm)
        })
        .collect();
    let mut out = BTreeMap::new();
    for item in items.iter().take(SUGGEST_ITEMS_MAX) {
        let text: String = item.text.chars().take(SUGGEST_TEXT_MAX).collect();
        let g = grams(&text);
        let mails = emails(&text);
        let named = |key: &str| g.contains_run(key);
        let item_people: HashSet<String> = people
            .iter()
            .filter(|p| p.needles.iter().any(|nd| nd.found(&g, &mails)))
            .map(|p| p.name.clone())
            .collect();
        let mut tf: HashMap<String, f64> = HashMap::new();
        for w in words(&text) {
            *tf.entry(w).or_default() += 1.0;
        }
        let mut best: Option<Suggestion> = None;
        for (p, (vec, norm)) in profiles.iter().zip(&vectors) {
            let mut reasons: Vec<(f64, String)> = Vec::new();
            if let Some(key) = &p.title_key {
                if named(key) {
                    reasons.push((W_TITLE, format!("names the workspace {}", p.title)));
                }
            }
            if let Some((_, label)) = &p.project {
                // A project named like the workspace counts once, as the title.
                let key = name_key(label);
                if key.as_deref().is_some_and(named) && key != p.title_key {
                    reasons.push((W_PROJECT, format!("names the project {label}")));
                }
            }
            let mut shared: Vec<String> = item_people.intersection(&p.people).cloned().collect();
            if !shared.is_empty() {
                shared.sort();
                reasons.push((
                    (W_PERSON * shared.len() as f64).min(W_CAP),
                    format!("same people: {}", list_words(&shared)),
                ));
            }
            if *norm > 0.0 && !tf.is_empty() {
                let mut dot = 0.0;
                let mut inorm = 0.0;
                let mut parts: Vec<(f64, &str)> = Vec::new();
                for (w, f) in &tf {
                    // A word no workspace has still counts in the item's
                    // norm, at the rarest weight, or an item that shares one
                    // word with a brief would score a perfect cosine.
                    let i = match df.get(w.as_str()) {
                        None => (1.0 + n as f64).ln(),
                        Some(_) => match idf(&df, n, w) {
                            Some(i) => i,
                            None => continue,
                        },
                    };
                    let x = f * i;
                    inorm += x * x;
                    if let Some(y) = vec.get(w.as_str()) {
                        dot += x * y;
                        parts.push((x * y, w.as_str()));
                    }
                }
                let cos = if inorm > 0.0 {
                    dot / (inorm.sqrt() * norm)
                } else {
                    0.0
                };
                if cos >= TERMS_MIN {
                    parts.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
                    let top: Vec<String> =
                        parts.iter().take(3).map(|(_, w)| w.to_string()).collect();
                    reasons.push((
                        (cos * 0.8).min(W_CAP),
                        format!("shared terms: {}", top.join(", ")),
                    ));
                }
            }
            if reasons.is_empty() {
                continue;
            }
            let score = combine(reasons.iter().map(|r| r.0));
            if score < SUGGEST_MIN {
                continue;
            }
            reasons.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let better = best.as_ref().is_none_or(|b| {
                score > b.score + 1e-9 || ((score - b.score).abs() <= 1e-9 && p.id < b.workspace_id)
            });
            if better {
                best = Some(Suggestion {
                    workspace_id: p.id.clone(),
                    title: p.title.clone(),
                    score: (score * 1000.0).round() / 1000.0,
                    reasons: reasons.into_iter().map(|r| r.1).collect(),
                });
            }
        }
        if let Some(b) = best {
            out.insert(item.id.clone(), b);
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn triage_arrivals(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<TriageArrivals, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || arrivals(&vault))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn triage_suggest(
    state: tauri::State<'_, crate::commands::AppState>,
    items: Vec<SuggestInput>,
) -> Result<BTreeMap<String, Suggestion>, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || suggest(&vault, &items))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn triage_mark(
    state: tauri::State<'_, crate::commands::AppState>,
    id: String,
    action: String,
    workspace_id: Option<String>,
    task_ref: Option<String>,
) -> Result<TriageState, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        mark_inner(
            &vault,
            &id,
            &action,
            workspace_id.as_deref(),
            task_ref.as_deref(),
            Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn workspace(root: &Path, id: &str, title: &str, brief: &str) {
        write(
            root,
            &format!("workspaces/{id}/WORKSPACE.md"),
            &format!("---\nhollow_workspace: 1\ntitle: \"{title}\"\nstatus: active\n---\n\n# {title}\n\n{brief}\n"),
        );
    }

    fn input(id: &str, text: &str) -> SuggestInput {
        SuggestInput {
            id: id.into(),
            text: text.into(),
        }
    }

    #[test]
    fn suggests_the_workspace_an_item_names() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        workspace(
            v,
            "tipflow",
            "TipFlow tips",
            "Reconcile the TipFlow push file against HRIS each week.",
        );
        workspace(
            v,
            "atlas",
            "Atlas 2.0",
            "Revenue dashboards and pricing for parking locations.",
        );
        let got = suggest(v, &[input("a", "Meeting about TipFlow tips for last week")]).unwrap();
        let s = &got["a"];
        assert_eq!(s.workspace_id, "tipflow");
        assert!(
            s.reasons[0].starts_with("names the workspace"),
            "{:?}",
            s.reasons
        );
        assert!(s.score >= SUGGEST_MIN);
    }

    #[test]
    fn shared_terms_suggest_without_a_name() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        workspace(
            v,
            "pay",
            "Payroll",
            "Tip pooling reconciliation, payroll variances, timesheet imports, tip variances.",
        );
        workspace(
            v,
            "lot",
            "Lot design",
            "Parking stall layouts, striping, drive aisles and entrances.",
        );
        let got = suggest(
            v,
            &[input("a", "tip variances in the timesheet reconciliation")],
        )
        .unwrap();
        assert_eq!(got["a"].workspace_id, "pay");
        assert!(
            got["a"]
                .reasons
                .iter()
                .any(|r| r.starts_with("shared terms:")),
            "{:?}",
            got["a"].reasons
        );
    }

    #[test]
    fn one_shared_word_among_many_is_no_suggestion() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        workspace(
            v,
            "ops",
            "Operations",
            "Weekly staffing and payroll reviews for every garage.",
        );
        workspace(
            v,
            "lot",
            "Lot design",
            "Parking stall layouts, striping, drive aisles and entrances.",
        );
        let got = suggest(
            v,
            &[input(
                "a",
                "Weekly sync about the quarterly offsite catering budget",
            )],
        )
        .unwrap();
        assert!(!got.contains_key("a"), "{:?}", got.get("a"));
    }

    #[test]
    fn nothing_below_the_threshold_and_archived_or_builtin_never() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        workspace(v, "pay", "Payroll", "Tip pooling and payroll.");
        write(
            v,
            "workspaces/old/WORKSPACE.md",
            "---\ntitle: \"TipFlow archive\"\nstatus: archived\n---\n\nTipFlow TipFlow TipFlow\n",
        );
        let got = suggest(
            v,
            &[
                input("a", "TipFlow archive notes"),
                input("b", "lunch order for friday"),
            ],
        )
        .unwrap();
        assert!(
            !got.contains_key("b"),
            "an unrelated item gets no suggestion"
        );
        assert!(
            got.get("a").is_none_or(|s| s.workspace_id != "old"),
            "archived never suggested"
        );
    }

    #[test]
    fn people_in_both_count() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        write(
            v,
            "people/Pat Ruiz.md",
            "---\ntype: person\n---\n# Pat Ruiz\n",
        );
        workspace(
            v,
            "ops",
            "Operations review",
            "Weekly with Pat Ruiz about staffing.",
        );
        workspace(v, "lot", "Lot design", "Parking stall layouts.");
        let got = suggest(v, &[input("a", "Call with Pat Ruiz")]).unwrap();
        if let Some(s) = got.get("a") {
            assert_eq!(s.workspace_id, "ops");
            assert!(
                s.reasons.iter().any(|r| r.contains("Pat Ruiz")),
                "{:?}",
                s.reasons
            );
        }
    }

    #[test]
    fn imports_arrive_only_when_recent_and_with_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        write(
            v,
            "library/imports/tip-pooling/b.md",
            "---\ntitle: Tip pooling basics\n---\nPooling splits tips.\n",
        );
        write(v, "library/imports/tip-pooling/a.md", "Second note");
        std::fs::create_dir_all(v.join("library/imports/empty")).unwrap();
        let got = arrivals(v);
        assert_eq!(got.imports.len(), 1);
        let imp = &got.imports[0];
        assert_eq!(imp.id, "import:tip-pooling");
        assert_eq!(imp.file_count, 2);
        assert_eq!(imp.files[0], "library/imports/tip-pooling/a.md");
        // Fourteen days on, it no longer arrives.
        let later = arrivals_at(v, Utc::now() + Duration::days(IMPORT_DAYS + 1));
        assert!(later.imports.is_empty());
    }

    #[test]
    fn tasks_arrive_from_sources_other_than_the_person() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        let today = Local::now().format("%Y-%m-%d").to_string();
        write(v, "tasks/From Notion.md", &format!("---\ntitle: From Notion\nstatus: Not started\ncreated: {today}\nnotion_id: abc\n---\n"));
        write(
            v,
            "tasks/Mine.md",
            &format!("---\ntitle: Mine\nstatus: Not started\ncreated: {today}\n---\n"),
        );
        write(v, "tasks/Triage made.md", &format!("---\ntitle: Triage made\nstatus: Not started\ncreated: {today}\norigin: \"triage:import:x\"\n---\n"));
        write(v, "tasks/Meeting.md", &format!("---\ntitle: Meeting\nstatus: Not started\ncreated: {today}\norigin: \"meeting:notes/a.md\"\n---\n"));
        write(
            v,
            "tasks/Old.md",
            "---\ntitle: Old\nstatus: Not started\ncreated: 2020-01-01\nnotion_id: old\n---\n",
        );
        write(
            v,
            "tasks/Done.md",
            &format!("---\ntitle: Done\nstatus: Done\ncreated: {today}\nnotion_id: d\n---\n"),
        );
        let got = arrivals(v);
        let mut titles: Vec<(&str, &str)> = got
            .tasks
            .iter()
            .map(|t| (t.title.as_str(), t.from.as_str()))
            .collect();
        titles.sort();
        assert_eq!(
            titles,
            vec![("From Notion", "Notion"), ("Meeting", "a meeting")]
        );
    }

    #[test]
    fn a_task_made_from_a_row_is_linked_to_it_and_the_row_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        let s = mark_inner(
            v,
            "import:a",
            "task",
            None,
            Some("vault:tasks/call-pat.md"),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(s.tasked["import:a"].task, "vault:tasks/call-pat.md");
        assert!(!s.dismissed.contains_key("import:a") && !s.filed.contains_key("import:a"));
        assert!(
            mark_inner(v, "import:b", "task", None, None, Utc::now()).is_err(),
            "a task action names its task"
        );
        assert!(mark_inner(v, "import:b", "task", None, Some("not a ref"), Utc::now()).is_err());
        let traversal = Some("vault:tasks/../me/x.md");
        assert!(mark_inner(v, "import:b", "task", None, traversal, Utc::now()).is_err());
        let review = mark_inner(
            v,
            "review:r1",
            "task",
            None,
            Some("vault:tasks/x.md"),
            Utc::now(),
        );
        assert!(review.is_err(), "a review still waits");
        // Restore brings the row back; the task itself stays.
        let s = mark(v, "import:a", "restore", None).unwrap();
        assert!(s.tasked.is_empty());
        // An old arrival's link ages out like a filing.
        mark_inner(
            v,
            "import:old",
            "task",
            None,
            Some("vault:tasks/x.md"),
            Utc::now(),
        )
        .unwrap();
        let later = Utc::now() + Duration::days(STATE_KEEP_DAYS + 1);
        let s = mark_at(v, "import:new", "dismiss", None, later).unwrap();
        assert!(!s.tasked.contains_key("import:old"));
        // A state file from before has no `tasked` and still reads.
        std::fs::write(state_path(v), r#"{"dismissed":{},"filed":{}}"#).unwrap();
        assert!(read_state(v).unwrap().tasked.is_empty());
    }

    #[test]
    fn mark_dismisses_files_restores_and_prunes() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        workspace(v, "pay", "Payroll", "x");
        let s = mark(v, "import:a", "dismiss", None).unwrap();
        assert!(s.dismissed.contains_key("import:a"));
        let s = mark(v, "import:a", "file", Some("pay")).unwrap();
        assert!(!s.dismissed.contains_key("import:a"));
        assert_eq!(s.filed["import:a"].workspace_id, "pay");
        assert!(
            mark(v, "import:b", "file", Some("nope")).is_err(),
            "an unknown workspace is refused"
        );
        assert!(mark(v, "import:b", "file", None).is_err());
        assert!(mark(v, "import:b", "delete", None).is_err());
        let s = mark(v, "import:a", "restore", None).unwrap();
        assert!(s.filed.is_empty() && s.dismissed.is_empty());
        mark(v, "import:old", "dismiss", None).unwrap();
        mark(v, "question:q1", "dismiss", None).unwrap();
        let later = Utc::now() + Duration::days(STATE_KEEP_DAYS + 1);
        let s = mark_at(v, "import:new", "dismiss", None, later).unwrap();
        assert!(
            !s.dismissed.contains_key("import:old"),
            "old arrivals are pruned"
        );
        assert!(
            s.dismissed.contains_key("question:q1"),
            "a dismissed question still waits, so it is kept"
        );
        assert!(s.dismissed.contains_key("import:new"));
        assert_eq!(read_state(v).unwrap(), s);
    }

    #[test]
    fn parse_when_reads_dates_and_times() {
        assert!(parse_when("2026-10-10").is_some());
        assert!(parse_when("2026-10-10T08:00:00Z").is_some());
        assert!(parse_when("\"2026-10-10T08:00:00-04:00\"").is_some());
        assert!(parse_when("soon").is_none());
    }
}
