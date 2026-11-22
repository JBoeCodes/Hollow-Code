//! Workspace connections (five asks A2): why two workspaces belong together,
//! for the workspace map's edges and orbits and for agents
//! (`workspaces.workspace_connections`). Read-only, local, no model.
//!
//! Each edge joins two current workspaces (built-ins left out, archived ones
//! only when asked) and carries its **reasons**, each with its own weight:
//!
//! | kind | from | weight |
//! |---|---|---|
//! | `project` | both briefs name the same project (`brief_read::project_of`) | 0.6 |
//! | `people` | the same people notes' names or aliases, or the same email, in both briefs/plans (a one-word name only capitalised; an alias two notes share is ignored) | 0.2 each, ≤ 0.5 |
//! | `link` | a wiki-link or `workspaces/<id>` path from one's brief/plan into the other | 0.45 |
//! | `source` | the same SOURCES.md SHA-256 (0.45), else the same source file name (0.3) | ≤ 0.5 |
//! | `task` | tasks made from one's plan belong to a project the other names or has tasks in | 0.3 |
//! | `terms` | TF-IDF cosine of title + brief, at least [`TERMS_MIN`] | cosine × 0.8, ≤ 0.5 |
//! | `chat` | a conversation in one started one in the other (`session_starters`) | 0.25 + 0.05 each, ≤ 0.5 |
//!
//! An edge's weight is `1 − Π(1 − wᵢ)` over its reasons, so reasons add up
//! without passing 1; edges under [`EDGE_MIN`] are dropped. A person, email,
//! checksum or source name found in more than [`COMMON_SHARE`] of workspaces
//! (once there are five or more), or in more than [`GROUP_MAX`], says nothing
//! (the vault owner's own name would otherwise link everything); a term in
//! more than [`TERM_DF_MAX`] workspaces pairs nothing either.
//!
//! Cost: per workspace a 64 KB brief prefix, a 256 KB plan prefix, SOURCES.md
//! (64 KB) and a shallow `sources/` listing; the people notes and tasks once;
//! one session-folder query and one delegation query. Pairs come from
//! inverted indexes whose groups are capped ([`GROUP_MAX`], [`TERM_DF_MAX`]),
//! so the work is bounded by those caps, not n². The result is cached (one
//! slot with archived workspaces, one without) until the vault's walk
//! generation moves, at most [`CACHE_TTL`] (sessions live outside the vault).
//! Chat starts come from delegation records, kept 90 days.

use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

const PLAN_PREFIX: u64 = 256 * 1024;
const SOURCES_PREFIX: u64 = 64 * 1024;
/// Edges lighter than this are left out.
pub(crate) const EDGE_MIN: f64 = 0.12;
/// The TF-IDF cosine a pair needs before shared terms count.
pub(crate) const TERMS_MIN: f64 = 0.2;
/// A signal shared by more than this share of workspaces says nothing.
const COMMON_SHARE: f64 = 0.4;
/// A signal shared by more workspaces than this says nothing either.
pub(crate) const GROUP_MAX: usize = 25;
/// A term in more workspaces than this pairs nothing (it stays in the norm).
pub(crate) const TERM_DF_MAX: usize = 60;
/// An email whose last host label is one of these is a file name
/// (`icon@2x.png`), not an address.
const FILE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "webp", "pdf", "js", "ts", "tsx", "jsx", "json", "md",
    "txt", "csv", "zip", "rs", "py", "html", "css", "xlsx", "docx", "pptx", "mov", "mp4", "lock",
];
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

const W_PROJECT: f64 = 0.6;
const W_PERSON: f64 = 0.2;
const W_LINK: f64 = 0.45;
const W_SOURCE_HASH: f64 = 0.45;
const W_SOURCE_NAME: f64 = 0.3;
const W_TASK: f64 = 0.3;
const W_CAP: f64 = 0.5;

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "from",
    "into",
    "onto",
    "this",
    "that",
    "these",
    "those",
    "are",
    "was",
    "were",
    "been",
    "being",
    "have",
    "has",
    "had",
    "not",
    "but",
    "can",
    "will",
    "would",
    "should",
    "could",
    "may",
    "might",
    "must",
    "its",
    "his",
    "her",
    "their",
    "our",
    "your",
    "you",
    "they",
    "them",
    "who",
    "what",
    "when",
    "where",
    "which",
    "why",
    "how",
    "all",
    "any",
    "each",
    "every",
    "some",
    "more",
    "most",
    "other",
    "such",
    "only",
    "own",
    "same",
    "than",
    "too",
    "very",
    "just",
    "also",
    "about",
    "after",
    "before",
    "over",
    "under",
    "again",
    "then",
    "once",
    "here",
    "there",
    "out",
    "off",
    "via",
    "per",
    "one",
    "two",
    "new",
    "old",
    "now",
    "yes",
    "no",
    "workspace",
    "workspaces",
    "outcome",
    "context",
    "current",
    "next",
    "step",
    "decisions",
    "plan",
    "brief",
    "outputs",
    "sources",
    "notes",
    "http",
    "https",
    "www",
    "com",
    "md",
    "hollow_workspace",
    "status",
    "active",
    "created",
    "title",
    "todo",
    "done",
    "see",
];

/// One reason two workspaces are linked.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReason {
    /// `project`, `people`, `link`, `source`, `task`, `terms` or `chat`.
    #[cfg_attr(test, ts(as = "crate::ts_bindings::ConnectionKind"))]
    pub kind: String,
    /// In words, e.g. "same people: Pat Ruiz".
    pub detail: String,
    pub weight: f64,
}

/// Two linked workspaces (`a` < `b` by id).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEdge {
    pub a: String,
    pub b: String,
    /// 0–1: `1 − Π(1 − reason weight)`.
    pub weight: f64,
    /// Heaviest first.
    pub reasons: Vec<ConnectionReason>,
}

/// Every edge, heaviest first.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceConnections {
    pub edges: Vec<WorkspaceEdge>,
    pub generated_at: String,
    pub took_ms: u32,
    /// Served from the cache (the vault hasn't changed since).
    pub cached: bool,
}

/// What one workspace contributes.
#[derive(Default)]
struct Facts {
    id: String,
    title: String,
    project: Option<(String, String)>,
    task_projects: BTreeMap<String, String>,
    people: BTreeSet<String>,
    links: BTreeSet<String>,
    hashes: BTreeSet<String>,
    source_names: BTreeSet<String>,
    terms: HashMap<String, u32>,
}

fn read_prefix(path: &Path, cap: u64) -> String {
    crate::brief_read::read_prefix(path, cap).unwrap_or_default()
}

/// The body of a brief: its frontmatter dropped.
fn body(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            return &rest[end + 4..];
        }
    }
    text
}

/// Lowercase words for TF-IDF: alphanumeric, 3+ characters, not all digits,
/// not a stopword.
pub(crate) fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
}

/// Lowercase word tokens of `text` (split on anything not alphanumeric).
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A workspace's text as lookups for names: every run of 1–4 words
/// (lowercase, space-joined) and the words written capitalised.
pub(crate) struct Grams {
    runs: HashSet<String>,
    capitalised: HashSet<String>,
}

impl Grams {
    /// Whether `key` (1–4 lowercase space-joined words) is a run of the text.
    pub(crate) fn contains_run(&self, key: &str) -> bool {
        self.runs.contains(key)
    }
}

pub(crate) fn grams(text: &str) -> Grams {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut runs = HashSet::new();
    for i in 0..lower.len() {
        for n in 1..=4 {
            if i + n <= lower.len() {
                runs.insert(lower[i..i + n].join(" "));
            }
        }
    }
    let capitalised = words
        .iter()
        .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
        .map(|w| w.to_lowercase())
        .collect();
    Grams { runs, capitalised }
}

/// How a person is recognised: by words (a one-word name only when written
/// capitalised, so "Will" isn't the verb) or by an email.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Needle {
    Words(String),
    Email(String),
}

impl Needle {
    pub(crate) fn of(alias: &str) -> Option<Needle> {
        let a = alias.trim();
        if a.contains('@') {
            return emails(a).into_iter().next().map(Needle::Email);
        }
        let joined = tokens(a).join(" ");
        (joined.chars().count() >= 3).then_some(Needle::Words(joined))
    }

    pub(crate) fn found(&self, g: &Grams, mails: &BTreeSet<String>) -> bool {
        match self {
            Needle::Email(e) => mails.contains(e),
            Needle::Words(w) if w.contains(' ') => g.runs.contains(w),
            Needle::Words(w) => g.capitalised.contains(w),
        }
    }
}

/// Email addresses in `text`, lowercased.
pub(crate) fn emails(text: &str) -> BTreeSet<String> {
    text.split(|c: char| c.is_whitespace() || "<>()[]{},;:\"'`|".contains(c))
        .map(|t| {
            t.trim_matches(|c: char| c == '.' || c == '*')
                .to_lowercase()
        })
        .filter(|t| {
            let mut parts = t.splitn(2, '@');
            let (user, host) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            // The last label has a top-level domain's shape: letters, 2–24,
            // not a file extension (`icon@2x.png`, `react@18.2.0`).
            let tld = host.rsplit('.').next().unwrap_or("");
            !user.is_empty()
                && !matches!(user, "git" | "noreply" | "no-reply")
                && host.contains('.')
                && !host.starts_with('.')
                && !host.contains('@')
                && (2..=24).contains(&tld.len())
                && tld.chars().all(|c| c.is_ascii_alphabetic())
                && !FILE_EXTS.contains(&tld)
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
        })
        .collect()
}

/// SOURCES.md's checksums and entry names.
pub(crate) fn sources_md(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut hashes = BTreeSet::new();
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("## ") {
            let n = name.trim().to_lowercase();
            if !n.is_empty() {
                names.insert(n);
            }
        } else if let Some(rest) = t
            .strip_prefix("- SHA-256:")
            .or_else(|| t.strip_prefix("- **SHA-256:**"))
        {
            let h: String = rest
                .trim()
                .trim_matches('`')
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .collect::<String>()
                .to_lowercase();
            if h.len() == 64 {
                hashes.insert(h);
            }
        }
    }
    (hashes, names)
}

/// The workspace ids `text` points into: wiki-links and `workspaces/<id>`
/// paths (to the folder or a file in it), and wiki-links naming a workspace
/// by its title.
fn link_targets(
    text: &str,
    ids: &HashSet<String>,
    by_title: &HashMap<String, String>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let id_after = |s: &str| -> Option<String> {
        let id: String = s
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        ids.contains(&id).then_some(id)
    };
    for target in crate::vault::parse_wikilinks(text) {
        let t = target
            .trim()
            .trim_start_matches("vault:")
            .trim_start_matches('/');
        if let Some(rest) = t.strip_prefix("workspaces/") {
            if let Some(id) = id_after(rest) {
                out.insert(id);
            }
        } else if let Some(id) = by_title.get(&t.trim_end_matches(".md").to_lowercase()) {
            out.insert(id.clone());
        }
    }
    let mut from = 0;
    while let Some(i) = text[from..].find("workspaces/") {
        let at = from + i + "workspaces/".len();
        if let Some(id) = id_after(&text[at..]) {
            out.insert(id);
        }
        from = at;
    }
    out
}

/// A person as matched in briefs: the note's display name and what counts
/// as them.
pub(crate) struct PersonKey {
    pub(crate) name: String,
    pub(crate) needles: Vec<Needle>,
}

/// Every people note with its name and aliases as needles; a needle two
/// notes share (two Daves) is nobody's.
pub(crate) fn people_keys(vault: &Path) -> Vec<PersonKey> {
    let mut keys: Vec<PersonKey> = crate::vault_people::list(vault)
        .into_iter()
        .map(|p| {
            let mut needles: Vec<Needle> = std::iter::once(p.name.clone())
                .chain(crate::people_dossier::aliases(vault, &p.path))
                .filter_map(|n| Needle::of(&n))
                .collect();
            needles.sort();
            needles.dedup();
            PersonKey {
                name: p.name,
                needles,
            }
        })
        .collect();
    let mut uses: HashMap<Needle, usize> = HashMap::new();
    for k in &keys {
        for n in &k.needles {
            *uses.entry(n.clone()).or_default() += 1;
        }
    }
    for k in &mut keys {
        k.needles.retain(|n| uses[n] == 1);
    }
    keys.retain(|k| !k.needles.is_empty());
    keys
}

/// Combine reason weights: `1 − Π(1 − w)`.
pub(crate) fn combine(weights: impl IntoIterator<Item = f64>) -> f64 {
    1.0 - weights
        .into_iter()
        .fold(1.0, |acc, w| acc * (1.0 - w.clamp(0.0, 1.0)))
}

#[derive(Default)]
struct Pair {
    reasons: Vec<ConnectionReason>,
}

fn key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Every unordered pair among `members` (each group of workspaces sharing a
/// signal), skipped when the signal is too common to mean anything.
fn group_pairs(members: &[usize], n: usize) -> Vec<(usize, usize)> {
    if members.len() < 2
        || members.len() > GROUP_MAX
        || (n >= 5 && members.len() as f64 > COMMON_SHARE * n as f64)
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            out.push((*a, *b));
        }
    }
    out
}

fn names_list(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1..=3 => names.join(", "),
        n => format!("{} and {} more", names[..3].join(", "), n - 3),
    }
}

/// Build the edges at `now` (read-only).
pub fn build(vault: &Path, include_archived: bool) -> Result<WorkspaceConnections, String> {
    let started = std::time::Instant::now();
    let workspaces: Vec<crate::workspaces::Workspace> = crate::workspaces::list(vault)?
        .into_iter()
        .filter(|w| !crate::workspaces::is_builtin(&w.id))
        .filter(|w| include_archived || !w.status.trim().eq_ignore_ascii_case("archived"))
        .collect();
    let ids: HashSet<String> = workspaces.iter().map(|w| w.id.clone()).collect();
    let by_title: HashMap<String, String> = workspaces
        .iter()
        .map(|w| (w.title.trim().to_lowercase(), w.id.clone()))
        .collect();
    let people = people_keys(vault);
    // Tasks made from a workspace's plan, by project (resolved once per url).
    let mut project_cache: HashMap<String, Option<(String, String)>> = HashMap::new();
    let mut tasks_by_ws: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for t in crate::vault_tasks::list(vault) {
        let Some(origin) = t.origin.as_deref() else {
            continue;
        };
        let Some(rest) = origin.strip_prefix("plan:workspaces/") else {
            continue;
        };
        let ws: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let Some(url) = t.project_url.as_deref().filter(|u| !u.trim().is_empty()) else {
            continue;
        };
        let proj = project_cache
            .entry(url.to_string())
            .or_insert_with(|| {
                let (p, _) = crate::goal_context::resolve_project_url(vault, url)?;
                Some((format!("project:{}", p.vault_path.clone()?), p.name))
            })
            .clone();
        if let Some((k, label)) = proj {
            tasks_by_ws.entry(ws).or_default().insert(k, label);
        }
    }
    let mut facts: Vec<Facts> = Vec::with_capacity(workspaces.len());
    for ws in &workspaces {
        let Ok(dir) = crate::workspaces::folder(vault, &ws.id) else {
            continue;
        };
        let brief_text = read_prefix(&dir.join("WORKSPACE.md"), crate::brief_read::BRIEF_PREFIX);
        let plan_text = read_prefix(&dir.join("PLAN.md"), PLAN_PREFIX);
        let (brief, _) = crate::frontmatter::parse_map(&brief_text);
        let both = format!("{}\n{}", body(&brief_text), plan_text);
        let g = grams(&both);
        let mails = emails(&both);
        let mut f = Facts {
            id: ws.id.clone(),
            title: ws.title.clone(),
            project: crate::brief_read::project_of(vault, &brief),
            task_projects: tasks_by_ws.remove(&ws.id).unwrap_or_default(),
            ..Facts::default()
        };
        for p in &people {
            if p.needles.iter().any(|n| n.found(&g, &mails)) {
                f.people.insert(p.name.clone());
            }
        }
        // An email no person claims is a person of its own.
        for e in &mails {
            if !people
                .iter()
                .any(|p| p.needles.contains(&Needle::Email(e.clone())))
            {
                f.people.insert(e.clone());
            }
        }
        f.links = link_targets(&both, &ids, &by_title);
        f.links.remove(&ws.id);
        let (hashes, mut names) = sources_md(&read_prefix(&dir.join("SOURCES.md"), SOURCES_PREFIX));
        if let Ok(entries) = std::fs::read_dir(dir.join("sources")) {
            for e in entries.flatten().take(500) {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if !n.starts_with('.') {
                    names.insert(n);
                }
            }
        }
        f.hashes = hashes;
        f.source_names = names;
        let title_twice = format!("{} {}", ws.title, ws.title);
        for w in words(&title_twice).chain(words(body(&brief_text))) {
            *f.terms.entry(w).or_default() += 1;
        }
        facts.push(f);
    }
    let n = facts.len();
    let title_of = |i: usize| facts[i].title.clone();
    let mut pairs: HashMap<(String, String), Pair> = HashMap::new();
    let add = |pairs: &mut HashMap<(String, String), Pair>,
               a: usize,
               b: usize,
               kind: &str,
               detail: String,
               weight: f64| {
        pairs
            .entry(key(&facts[a].id, &facts[b].id))
            .or_default()
            .reasons
            .push(ConnectionReason {
                kind: kind.into(),
                detail,
                weight: (weight * 1000.0).round() / 1000.0,
            });
    };

    // Projects (named in both briefs).
    let mut by_project: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        if let Some((k, _)) = &f.project {
            by_project.entry(k.as_str()).or_default().push(i);
        }
    }
    let mut same_project: HashSet<(usize, usize)> = HashSet::new();
    for members in by_project.values() {
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                let label = facts[*a]
                    .project
                    .as_ref()
                    .map(|p| p.1.clone())
                    .unwrap_or_default();
                same_project.insert((*a, *b));
                add(
                    &mut pairs,
                    *a,
                    *b,
                    "project",
                    format!("same project: {label}"),
                    W_PROJECT,
                );
            }
        }
    }
    // Tasks: one's plan tasks in a project the other names or has tasks in.
    let mut task_index: BTreeMap<String, (String, BTreeSet<usize>)> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        for (k, label) in f
            .task_projects
            .iter()
            .chain(f.project.iter().map(|(k, l)| (k, l)))
        {
            task_index
                .entry(k.clone())
                .or_insert_with(|| (label.clone(), BTreeSet::new()))
                .1
                .insert(i);
        }
    }
    let mut task_pairs: BTreeMap<(usize, usize), String> = BTreeMap::new();
    for (k, (label, members)) in &task_index {
        let members: Vec<usize> = members.iter().copied().collect();
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                let has_tasks = |x: usize| facts[x].task_projects.contains_key(k);
                // Their shared brief project is already the `project` reason.
                let shared_brief = same_project.contains(&(*a, *b))
                    && facts[*a].project.as_ref().is_some_and(|p| &p.0 == k);
                if shared_brief || !(has_tasks(*a) || has_tasks(*b)) {
                    continue;
                }
                task_pairs.entry((*a, *b)).or_insert_with(|| label.clone());
            }
        }
    }
    for ((a, b), label) in task_pairs {
        add(
            &mut pairs,
            a,
            b,
            "task",
            format!("tasks in project {label}"),
            W_TASK,
        );
    }
    // People.
    let mut by_person: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        for p in &f.people {
            by_person.entry(p.as_str()).or_default().push(i);
        }
    }
    let mut shared_people: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
    for (person, members) in &by_person {
        for pair in group_pairs(members, n) {
            shared_people
                .entry(pair)
                .or_default()
                .push(person.to_string());
        }
    }
    for ((a, b), names) in shared_people {
        let w = (W_PERSON * names.len() as f64).min(W_CAP);
        add(
            &mut pairs,
            a,
            b,
            "people",
            format!("same people: {}", names_list(&names)),
            w,
        );
    }
    // Links (either direction).
    let index_of: HashMap<&str, usize> = facts
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.as_str(), i))
        .collect();
    let mut linked: BTreeMap<(usize, usize), (bool, bool)> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        for target in &f.links {
            if let Some(&j) = index_of.get(target.as_str()) {
                let (a, b) = if i < j { (i, j) } else { (j, i) };
                let e = linked.entry((a, b)).or_default();
                if i == a {
                    e.0 = true;
                } else {
                    e.1 = true;
                }
            }
        }
    }
    for ((a, b), (ab, ba)) in linked {
        let detail = match (ab, ba) {
            (true, true) => format!("“{}” and “{}” link to each other", title_of(a), title_of(b)),
            (true, false) => format!("“{}” links to “{}”", title_of(a), title_of(b)),
            _ => format!("“{}” links to “{}”", title_of(b), title_of(a)),
        };
        add(&mut pairs, a, b, "link", detail, W_LINK);
    }
    // Sources: the same checksum, else the same file name.
    let mut by_hash: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        for h in &f.hashes {
            by_hash.entry(h.as_str()).or_default().push(i);
        }
        for s in &f.source_names {
            by_name.entry(s.as_str()).or_default().push(i);
        }
    }
    let mut hash_pairs: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for members in by_hash.values() {
        for pair in group_pairs(members, n) {
            *hash_pairs.entry(pair).or_default() += 1;
        }
    }
    let mut name_pairs: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
    for (name, members) in &by_name {
        for pair in group_pairs(members, n) {
            name_pairs.entry(pair).or_default().push(name.to_string());
        }
    }
    let source_keys: BTreeSet<(usize, usize)> = hash_pairs
        .keys()
        .chain(name_pairs.keys())
        .copied()
        .collect();
    for (a, b) in source_keys {
        let hashes = hash_pairs.get(&(a, b)).copied().unwrap_or(0);
        let names = name_pairs.get(&(a, b)).cloned().unwrap_or_default();
        let w = if hashes > 0 {
            W_SOURCE_HASH
        } else {
            W_SOURCE_NAME
        };
        let detail = if names.is_empty() {
            format!(
                "both cite the same source file{}",
                if hashes == 1 { "" } else { "s" }
            )
        } else {
            format!("both cite {}", names_list(&names))
        };
        add(&mut pairs, a, b, "source", detail, w.min(W_CAP));
    }
    // Terms: TF-IDF cosine through an inverted index.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        for w in f.terms.keys() {
            *df.entry(w.as_str()).or_default() += 1;
        }
    }
    // Too common to tell workspaces apart. A word only one workspace has
    // stays in its vector: it pairs with nothing but keeps the norm honest.
    let common = |d: usize| n >= 4 && d as f64 > 0.5 * n as f64;
    let vectors: Vec<HashMap<&str, f64>> = facts
        .iter()
        .map(|f| {
            let mut v: HashMap<&str, f64> = f
                .terms
                .iter()
                .filter(|(w, _)| !common(df[w.as_str()]))
                .map(|(w, c)| {
                    let idf = (n as f64 / df[w.as_str()] as f64).ln();
                    (w.as_str(), (1.0 + (*c as f64).ln()) * idf)
                })
                .collect();
            let norm = v.values().map(|x| x * x).sum::<f64>().sqrt();
            if norm > 0.0 {
                v.values_mut().for_each(|x| *x /= norm);
            }
            v
        })
        .collect();
    let mut postings: HashMap<&str, Vec<(usize, f64)>> = HashMap::new();
    for (i, v) in vectors.iter().enumerate() {
        for (w, x) in v {
            if df[w] <= TERM_DF_MAX {
                postings.entry(w).or_default().push((i, *x));
            }
        }
    }
    let mut dots: HashMap<(usize, usize), f64> = HashMap::new();
    for list in postings.values() {
        for (k, (a, xa)) in list.iter().enumerate() {
            for (b, xb) in &list[k + 1..] {
                let pair = if a < b { (*a, *b) } else { (*b, *a) };
                *dots.entry(pair).or_default() += xa * xb;
            }
        }
    }
    let mut term_pairs: Vec<((usize, usize), f64)> =
        dots.into_iter().filter(|(_, c)| *c >= TERMS_MIN).collect();
    term_pairs.sort_by_key(|x| x.0);
    for ((a, b), cos) in term_pairs {
        let mut shared: Vec<(&str, f64)> = vectors[a]
            .iter()
            .filter_map(|(w, x)| vectors[b].get(w).map(|y| (*w, x * y)))
            .collect();
        shared.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(y.0)));
        let top: Vec<String> = shared.iter().take(3).map(|(w, _)| w.to_string()).collect();
        add(
            &mut pairs,
            a,
            b,
            "terms",
            format!("shared terms: {}", top.join(", ")),
            (cos * 0.8).min(W_CAP),
        );
    }
    // Chats that started chats across workspaces.
    let mut dir_to: HashMap<PathBuf, usize> = HashMap::new();
    for (i, f) in facts.iter().enumerate() {
        if let (Ok(dir), Some(ws)) = (
            crate::workspaces::folder(vault, &f.id),
            workspaces.iter().find(|w| w.id == f.id),
        ) {
            for d in crate::workspaces::session_dirs(&dir, ws) {
                dir_to.insert(d, i);
            }
        }
    }
    let sessions = crate::session_store::session_cwds(vault).unwrap_or_default();
    let mut dir_cache: HashMap<String, Option<usize>> = HashMap::new();
    let ws_of: HashMap<String, usize> = sessions
        .into_iter()
        .filter_map(|(key, cwd)| {
            let i = *dir_cache
                .entry(cwd.clone())
                .or_insert_with(|| dir_to.get(&crate::workspaces::session_dir(&cwd)).copied());
            i.map(|i| (key, i))
        })
        .collect();
    let keys: Vec<String> = ws_of.keys().cloned().collect();
    let starters = crate::session_store::session_starters(vault, &keys).unwrap_or_default();
    let mut starts: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for (child, parent) in &starters {
        let (Some(&c), Some(&p)) = (ws_of.get(child), ws_of.get(parent)) else {
            continue;
        };
        if c != p {
            *starts.entry((p, c)).or_default() += 1;
        }
    }
    let mut chat_pairs: BTreeMap<(usize, usize), Vec<(usize, usize, u32)>> = BTreeMap::new();
    for ((p, c), count) in starts {
        let pair = if p < c { (p, c) } else { (c, p) };
        chat_pairs.entry(pair).or_default().push((p, c, count));
    }
    for ((a, b), dirs) in chat_pairs {
        let total: u32 = dirs.iter().map(|d| d.2).sum();
        let detail = dirs
            .iter()
            .map(|(p, c, k)| {
                format!(
                    "“{}” started {k} chat{} in “{}”",
                    title_of(*p),
                    if *k == 1 { "" } else { "s" },
                    title_of(*c)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        add(
            &mut pairs,
            a,
            b,
            "chat",
            detail,
            (0.25 + 0.05 * total as f64).min(W_CAP),
        );
    }

    let mut edges: Vec<WorkspaceEdge> = pairs
        .into_iter()
        .map(|((a, b), mut p)| {
            p.reasons
                .sort_by(|x, y| y.weight.total_cmp(&x.weight).then(x.kind.cmp(&y.kind)));
            let weight = combine(p.reasons.iter().map(|r| r.weight));
            WorkspaceEdge {
                a,
                b,
                weight: (weight * 1000.0).round() / 1000.0,
                reasons: p.reasons,
            }
        })
        .filter(|e| e.weight >= EDGE_MIN)
        .collect();
    edges.sort_by(|x, y| {
        y.weight
            .total_cmp(&x.weight)
            .then(x.a.cmp(&y.a))
            .then(x.b.cmp(&y.b))
    });
    Ok(WorkspaceConnections {
        edges,
        generated_at: Utc::now().to_rfc3339(),
        took_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
        cached: false,
    })
}

type CacheEntry = (PathBuf, u64, std::time::Instant, WorkspaceConnections);
/// One slot without archived workspaces, one with.
static CACHE: std::sync::Mutex<[Option<CacheEntry>; 2]> = std::sync::Mutex::new([None, None]);

/// [`build`], from the cache while the vault's walk generation hasn't moved
/// and for at most [`CACHE_TTL`].
pub fn cached(vault: &Path, include_archived: bool) -> Result<WorkspaceConnections, String> {
    cached_at(vault, include_archived, crate::vault::walk_generation())
}

fn cached_at(
    vault: &Path,
    include_archived: bool,
    generation: u64,
) -> Result<WorkspaceConnections, String> {
    let slot = usize::from(include_archived);
    if let Ok(g) = CACHE.lock() {
        if let Some((root, gen, at, found)) = g[slot].as_ref() {
            if root == vault && *gen == generation && at.elapsed() < CACHE_TTL {
                let mut hit = found.clone();
                hit.cached = true;
                return Ok(hit);
            }
        }
    }
    let built = build(vault, include_archived)?;
    if let Ok(mut g) = CACHE.lock() {
        g[slot] = Some((
            vault.to_path_buf(),
            generation,
            std::time::Instant::now(),
            built.clone(),
        ));
    }
    Ok(built)
}

/// Refuse a workspace the connections don't consider, so "no edges" never
/// hides a wrong id.
fn check_considered(vault: &Path, id: &str, include_archived: bool) -> Result<(), String> {
    let ws = crate::workspaces::list(vault)?
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("No workspace “{id}”."))?;
    if crate::workspaces::is_builtin(&ws.id) {
        return Err(format!(
            "“{}” is built in and has no connections.",
            ws.title
        ));
    }
    if !include_archived && ws.status.trim().eq_ignore_ascii_case("archived") {
        return Err(format!("“{}” is archived: pass includeArchived.", ws.title));
    }
    Ok(())
}

/// Only the edges touching workspace `id`.
pub fn touching(mut c: WorkspaceConnections, id: &str) -> WorkspaceConnections {
    c.edges.retain(|e| e.a == id || e.b == id);
    c
}

/// The connections (read-only); `workspace` keeps only its own edges.
#[tauri::command]
pub async fn workspace_connections(
    include_archived: Option<bool>,
    workspace: Option<String>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<WorkspaceConnections, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let archived = include_archived.unwrap_or(false);
        let one = workspace
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty());
        if let Some(id) = one {
            check_considered(&vault, id, archived)?;
        }
        let all = cached(&vault, archived)?;
        Ok(match one {
            Some(id) => touching(all, id),
            None => all,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(root: &Path, id: &str, front: &str, body_text: &str) {
        let p = root.join("workspaces").join(id).join("WORKSPACE.md");
        let text = std::fs::read_to_string(&p).unwrap();
        let text = text.replacen("---\n", &format!("---\n{front}"), 1);
        std::fs::write(&p, format!("{text}\n{body_text}\n")).unwrap();
    }

    fn edge<'a>(c: &'a WorkspaceConnections, a: &str, b: &str) -> Option<&'a WorkspaceEdge> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        c.edges.iter().find(|e| e.a == a && e.b == b)
    }

    fn kinds(e: &WorkspaceEdge) -> Vec<&str> {
        e.reasons.iter().map(|r| r.kind.as_str()).collect()
    }

    #[test]
    fn helpers_find_words_emails_sources_and_combine() {
        let g = grams("Met Pat Ruiz today; will chase it. Jordan");
        let none = BTreeSet::new();
        assert!(
            Needle::of("Pat Ruiz").unwrap().found(&g, &none),
            "two words, any case"
        );
        assert!(
            Needle::of("jordan").unwrap().found(&g, &none),
            "capitalised in the text"
        );
        assert!(
            !Needle::of("Will").unwrap().found(&g, &none),
            "the verb is not Will"
        );
        assert!(!Needle::of("Chase").unwrap().found(&g, &none));
        assert!(!Needle::of("Jordanian").unwrap().found(&g, &none));
        assert_eq!(Needle::of("Al"), None, "too short");
        assert_eq!(
            Needle::of("PR@Example.com"),
            Some(Needle::Email("pr@example.com".into()))
        );
        let e = emails("Ask <Pat.Ruiz@example.com>, or x@y (no) and *ops@a.io*.");
        assert_eq!(
            e.into_iter().collect::<Vec<_>>(),
            ["ops@a.io", "pat.ruiz@example.com"]
        );
        // Not addresses: a package version, a retina image, a git remote.
        let fake =
            emails("react@18.2.0 icon@2x.png git@github.com:Org/repo noreply@github.com x@y.");
        assert!(fake.is_empty(), "{fake:?}");
        let (h, n) = sources_md(&format!(
            "# Sources\n\n## Push File.csv\n\n- SHA-256: `{}`\n## other\n- SHA-256: `short`\n",
            "A".repeat(64)
        ));
        assert_eq!(h.into_iter().collect::<Vec<_>>(), ["a".repeat(64)]);
        assert_eq!(
            n.into_iter().collect::<Vec<_>>(),
            ["other", "push file.csv"]
        );
        assert!((combine([0.5, 0.5]) - 0.75).abs() < 1e-9);
        assert_eq!(combine([]), 0.0);
        assert!(combine([0.6, 0.45, 0.5, 0.5, 0.5]) < 1.0);
    }

    #[test]
    fn each_reason_links_the_right_pair_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mk = |t: &str| crate::workspaces::create(root, t, "x").unwrap();
        let a = mk("TipFlow tips");
        let b = mk("Payroll exports");
        let c = mk("Garage pricing");
        let d = mk("Lot survey");
        let e = mk("Valet hiring");
        let f = mk("Unrelated thing");
        // People notes: Pat (with an email alias) and the owner everywhere.
        std::fs::create_dir_all(root.join("people")).unwrap();
        std::fs::write(
            root.join("people/Pat Ruiz.md"),
            "---\ntitle: Pat Ruiz\naliases: [Pat, dr@example.com]\n---\n",
        )
        .unwrap();
        std::fs::write(root.join("people/Alex.md"), "---\ntitle: Alex\n---\n").unwrap();
        for w in [&a, &b, &c, &d, &e, &f] {
            brief(root, &w.id, "", "Owner: Alex.");
        }
        brief(root, &a.id, "", "Ask Pat about the push file.");
        brief(root, &b.id, "", "Mail dr@example.com for exports.");
        // Same project.
        std::fs::create_dir_all(root.join("projects/active")).unwrap();
        std::fs::write(
            root.join("projects/active/Garages.md"),
            "---\ntitle: Garages 2027\n---\n",
        )
        .unwrap();
        brief(root, &c.id, "project_url: projects/active/Garages.md\n", "");
        brief(root, &d.id, "project_url: projects/active/Garages.md\n", "");
        // A link from e into c's outputs, by path.
        brief(
            root,
            &e.id,
            "",
            &format!("See [[workspaces/{}/outputs/report.md]].", c.id),
        );
        // A shared source by checksum.
        let hash = "b".repeat(64);
        for w in [&a, &f] {
            std::fs::write(
                root.join(&w.path).join("SOURCES.md"),
                format!("# Sources\n\n## vendor_report_hris_push.csv\n\n- SHA-256: `{hash}`\n"),
            )
            .unwrap();
        }
        let c_all = build(root, false).unwrap();
        let ab = edge(&c_all, &a.id, &b.id).expect("same person by name and by email alias");
        assert_eq!(kinds(ab), ["people"]);
        assert_eq!(ab.reasons[0].detail, "same people: Pat Ruiz");
        let cd = edge(&c_all, &c.id, &d.id).unwrap();
        assert_eq!(cd.reasons[0].detail, "same project: Garages 2027");
        assert!((cd.weight - 0.6).abs() < 1e-9);
        let ce = edge(&c_all, &c.id, &e.id).unwrap();
        assert_eq!(
            ce.reasons[0].detail,
            "“Valet hiring” links to “Garage pricing”"
        );
        let af = edge(&c_all, &a.id, &f.id).unwrap();
        assert_eq!(
            af.reasons[0].detail,
            "both cite vendor_report_hris_push.csv"
        );
        assert!((af.reasons[0].weight - W_SOURCE_HASH).abs() < 1e-9);
        // The owner is in every brief: too common to link anything.
        assert!(c_all
            .edges
            .iter()
            .all(|e| !e.reasons.iter().any(|r| r.detail.contains("Alex"))));
        // Heaviest first, nothing below the floor, a < b.
        assert!(c_all.edges.windows(2).all(|w| w[0].weight >= w[1].weight));
        assert!(c_all
            .edges
            .iter()
            .all(|e| e.weight >= EDGE_MIN && e.a < e.b));
        // `touching` keeps one workspace's edges.
        let only = touching(c_all.clone(), &c.id);
        assert!(!only.edges.is_empty() && only.edges.iter().all(|e| e.a == c.id || e.b == c.id));
    }

    #[test]
    fn brief_terms_and_plan_tasks_link_and_reasons_add_up() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mk = |t: &str| crate::workspaces::create(root, t, "x").unwrap();
        let a = mk("Tip reconciliation");
        let b = mk("Tip variance study");
        let c = mk("Board deck");
        let d = mk("Hiring plan");
        let e = mk("Office move");
        brief(
            root,
            &a.id,
            "",
            "Reconcile tipflow tips against hris counters by cost number.",
        );
        brief(
            root,
            &b.id,
            "",
            "Why tipflow tips differ from hris counters per cost number.",
        );
        brief(root, &c.id, "", "Quarterly slides for the board meeting.");
        brief(root, &d.id, "", "Recruiting pipeline and interview loops.");
        brief(root, &e.id, "", "Leases, movers and furniture.");
        // c's plan made a task in the Garages project, which e's brief names.
        std::fs::create_dir_all(root.join("projects/active")).unwrap();
        std::fs::write(
            root.join("projects/active/Garages.md"),
            "---\ntitle: Garages 2027\n---\n",
        )
        .unwrap();
        brief(root, &e.id, "project_url: projects/active/Garages.md\n", "");
        std::fs::create_dir_all(root.join("tasks")).unwrap();
        std::fs::write(
            root.join("tasks/Deck numbers.md"),
            format!(
                "---\ntitle: Deck numbers\nstatus: todo\nproject: \"[[projects/active/Garages]]\"\nproject_url: projects/active/Garages.md\norigin: plan:workspaces/{}/PLAN.md\n---\n",
                c.id
            ),
        )
        .unwrap();
        let all = build(root, false).unwrap();
        let ab = edge(&all, &a.id, &b.id).expect("brief terms");
        assert_eq!(kinds(ab), ["terms"]);
        assert!(ab.reasons[0].detail.starts_with("shared terms: "));
        assert!(
            ab.reasons[0].detail.contains("tipflow") || ab.reasons[0].detail.contains("counters")
        );
        let ce = edge(&all, &c.id, &e.id).expect("plan task in a project the other names");
        assert!(kinds(ce).contains(&"task"), "{ce:?}");
        assert_eq!(
            ce.reasons.iter().find(|r| r.kind == "task").unwrap().detail,
            "tasks in project Garages 2027"
        );
        assert!(
            edge(&all, &c.id, &d.id).is_none(),
            "unrelated briefs stay apart"
        );
    }

    #[test]
    fn an_alias_two_people_share_links_nobody_and_huge_groups_say_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("people")).unwrap();
        std::fs::write(
            root.join("people/Pat Ruiz.md"),
            "---\ntitle: Pat Ruiz\naliases: [Pat]\n---\n",
        )
        .unwrap();
        std::fs::write(
            root.join("people/Pat Lee.md"),
            "---\ntitle: Pat Lee\naliases: [Pat]\n---\n",
        )
        .unwrap();
        let a = crate::workspaces::create(root, "One", "x").unwrap();
        let b = crate::workspaces::create(root, "Two", "x").unwrap();
        brief(root, &a.id, "", "Ask Pat.");
        brief(root, &b.id, "", "Ask Pat.");
        let c = build(root, false).unwrap();
        assert!(
            c.edges
                .iter()
                .all(|e| e.reasons.iter().all(|r| r.kind != "people")),
            "{:?}",
            c.edges
        );
        // A group over GROUP_MAX pairs nothing, whatever its share.
        let members: Vec<usize> = (0..=GROUP_MAX).collect();
        assert!(group_pairs(&members, 10_000).is_empty());
        assert_eq!(group_pairs(&[1, 2, 3], 10_000).len(), 3);
    }

    #[test]
    fn a_chat_that_started_a_chat_in_another_workspace_links_them() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let root = dir.path();
        let a = crate::workspaces::create(root, "Launch prep", "x").unwrap();
        let b = crate::workspaces::create(root, "Budget model", "x").unwrap();
        let c = crate::workspaces::create(root, "Quiet corner", "x").unwrap();
        use crate::session_store::{record_delegation, start_turn};
        start_turn(root, "s-a2-a", "claude", &a.cwd, "run-a2-a", "A").unwrap();
        start_turn(root, "s-a2-b", "claude", &b.cwd, "run-a2-b", "B").unwrap();
        start_turn(root, "s-a2-b2", "claude", &b.cwd, "run-a2-b2", "B2").unwrap();
        start_turn(root, "s-a2-c", "claude", &c.cwd, "run-a2-c", "C").unwrap();
        let now = crate::session_store::db_time(Utc::now());
        let child = |key: &str| {
            format!(
                "{{\"kind\":\"childTask\",\"tool\":\"start_session\",\"childSession\":\"{key}\"}}"
            )
        };
        record_delegation(root, "run-a2-a", &now, &child("s-a2-b"), 10).unwrap();
        record_delegation(root, "run-a2-a", &now, &child("s-a2-b2"), 10).unwrap();
        // Within one workspace: no edge.
        start_turn(root, "s-a2-c2", "claude", &c.cwd, "run-a2-c2", "C2").unwrap();
        record_delegation(root, "run-a2-c", &now, &child("s-a2-c2"), 10).unwrap();
        let all = build(root, false).unwrap();
        let ab = edge(&all, &a.id, &b.id).expect("started across workspaces");
        let chat = ab.reasons.iter().find(|r| r.kind == "chat").unwrap();
        assert_eq!(
            chat.detail,
            "“Launch prep” started 2 chats in “Budget model”"
        );
        assert!((chat.weight - 0.35).abs() < 1e-9);
        assert!(all.edges.iter().all(|e| !(e.a == c.id && e.b == c.id)));
        assert!(edge(&all, &a.id, &c.id).is_none());
    }

    #[test]
    fn the_cache_serves_until_the_vault_changes_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = crate::workspaces::create(root, "Alpha one", "x").unwrap();
        let _b = crate::workspaces::create(root, "Beta two", "x").unwrap();
        let before = std::fs::read_to_string(root.join(&a.path).join("WORKSPACE.md")).unwrap();
        // A fixed generation (other tests bump the shared one as they write).
        let gen = u64::MAX - 7;
        let first = cached_at(root, false, gen).unwrap();
        assert!(!first.cached);
        let second = cached_at(root, false, gen).unwrap();
        assert!(second.cached, "same generation: served from the cache");
        assert_eq!(first.edges, second.edges);
        assert!(
            !cached_at(root, false, gen + 1).unwrap().cached,
            "a vault change rebuilds"
        );
        assert!(
            !cached_at(root, true, gen + 1).unwrap().cached,
            "archived on has its own slot"
        );
        assert!(
            cached_at(root, false, gen + 1).unwrap().cached,
            "and leaves the other's"
        );
        // A wrong, built-in or archived id is refused, not answered with nothing.
        assert!(check_considered(root, "nope", false)
            .unwrap_err()
            .contains("No workspace"));
        assert!(check_considered(root, &a.id, false).is_ok());
        let old = crate::workspaces::create(root, "Gone quiet", "x").unwrap();
        crate::workspaces::update_meta(root, &old.id, None, Some("archived")).unwrap();
        assert!(check_considered(root, &old.id, false)
            .unwrap_err()
            .contains("archived"));
        assert!(check_considered(root, &old.id, true).is_ok());
        assert_eq!(
            before,
            std::fs::read_to_string(root.join(&a.path).join("WORKSPACE.md")).unwrap()
        );
    }

    #[test]
    fn three_hundred_workspaces_connect_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let topics = [
            "payroll", "garage", "valet", "tips", "hiring", "audit", "pricing", "lease",
        ];
        for i in 0..300 {
            let w = crate::workspaces::create(root, &format!("Work {i}"), "x").unwrap();
            let t = topics[i % topics.len()];
            brief(
                root,
                &w.id,
                "",
                &format!(
                    "Notes about {t} and {} item{i}.",
                    topics[(i * 3) % topics.len()]
                ),
            );
        }
        let t0 = std::time::Instant::now();
        let c = build(root, false).unwrap();
        assert!(!c.edges.is_empty());
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(4),
            "{:?}",
            t0.elapsed()
        );
    }
}
