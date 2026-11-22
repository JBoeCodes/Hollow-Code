//! Companion backend v2 (overnight 10-12/13, lane H): what of the vault the
//! homelab backend may copy, beyond the task mirror of [`companion_bridge`].
//! Design: the `hollow-companion` repo's `docs/BACKEND_V2.md`.
//!
//! - **Scope.** Personal by default. `<support>/Companion/scope.json` holds
//!   "include work data" (off) and the personal-workspace list. Only the
//!   person's Settings changes it: front matter can opt a workspace *out*
//!   (`scope: work`) but never in, because agents edit `WORKSPACE.md` freely.
//!   The list is seeded once with Hollow's own and clearly personal
//!   workspaces; new workspaces stay off until added.
//! - **Rules.** The same path rules as the daemon's `server/src/docs.rs`
//!   (allowlist: notes, library notes, a personal workspace's brief, plan,
//!   sources, agents, outputs and source files). Work-only: learnings,
//!   `library/work`, `library/atlas`, `PROJECT.md`, `notion-*` workspaces.
//!   Never, in either scope: `me/`, sessions, dot paths, symlinks,
//!   `library/records/` and `linear-*` workspaces. Front matter `scope: work`
//!   still opts an otherwise-allowed file out of personal scope. Tasks go
//!   through the v1 task mirror, not here. The daemon checks every path again.
//! - **Walk.** Never follows a symlink (a linked folder can't pull
//!   `library/work` into a personal workspace), skips hidden names, hashes a
//!   file only when its size or mtime changed.

use crate::companion_inbox::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

const SCOPE_FILE: &str = "scope.json";
/// The daemon's limit; bigger files go as metadata only (`truncated`).
pub(crate) const MAX_DOC_BYTES: u64 = 256 * 1024;
const MAX_PATH_BYTES: usize = 512;
/// A runaway vault can't make one scan unbounded.
const MAX_DOCS: usize = 20_000;
const MAX_DEPTH: usize = 12;

// ── scope ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Scope {
    #[serde(default)]
    pub include_work_data: bool,
    #[serde(default)]
    pub personal_workspaces: Vec<String>,
    #[serde(default)]
    pub seeded_at: Option<String>,
}

impl Scope {
    pub(crate) fn name(&self) -> &'static str {
        if self.include_work_data {
            "work"
        } else {
            "personal"
        }
    }
}

/// Titles (lower case) of workspaces that default to personal scope (anything
/// uncertain stays off until enabled in Settings).
const SEED_TITLES: &[&str] = &[
    "hollow",
    "personal",
    "notes",
    "journal",
    "sandbox",
    "scratch",
    "second-brain",
];
/// Title fragments of Hollow's own build and release workspaces.
const SEED_TITLE_PARTS: &[&str] = &[
    "hollow",
    "canvas",
    "release",
    "light trails",
    "doorway",
    "agent effectiveness",
    "app tools test",
    "notion linked edits",
];
/// Any of these in a workspace's id or title marks it as work.
const WORK_MARKERS: &[&str] = &[
    "work",
    "client",
    "payroll",
    "claims",
    "sso",
    "culture",
    "performance-review",
    "finance",
    "hr",
    "ops-rollout",
];

/// One workspace as Settings lists it.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceEntry {
    pub id: String,
    pub title: String,
    pub status: String,
    /// Never personal: a `notion-*` or `linear-*` workspace, front matter
    /// `scope: work`, or a work marker in its id or title. `linear-*` also
    /// never mirrors, even when work data is on.
    pub work: bool,
    pub personal: bool,
}

/// Workspace `id`'s brief front matter; `None` when the brief is missing, is
/// no regular file, can't be read, or its front matter opens and doesn't
/// close. Only the head [`crate::workspaces::summary`] reads is read
/// (`MAX_CONTEXT`), so a brief of any size keeps its classification: until
/// the 2026-11-08 review a brief over [`MAX_DOC_BYTES`] lost it, and a
/// `scope: work` workspace with a long brief read as personal.
fn front(vault: &Path, id: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let path = vault.join("workspaces").join(id).join("WORKSPACE.md");
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let (text, _) =
        crate::workspaces::read_prefix(&path, crate::workspaces::MAX_CONTEXT as usize).ok()?;
    let parsed = crate::frontmatter::parse(&text);
    let opens = text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("---");
    // Unclosed, the parse hands the whole text back as the body.
    if opens && parsed.body.len() == text.len() {
        return None;
    }
    Some(parsed.pairs.into_iter().collect())
}

/// A brief's length, modification and change times (ns) and inode, as
/// `workspaces`' brief cache stamps it: an edit that restores the mtime still
/// moves the change time.
type BriefStamp = (u64, i128, i128, u64);
/// Briefs kept; past it the cache starts over.
const WORK_CACHE_MAX: usize = 4_096;
/// How long a cached work flag is trusted with its brief's stamp unchanged.
const WORK_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);
static WORK_CACHE: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<std::path::PathBuf, (std::time::Instant, BriefStamp, bool)>>,
> = once_cell::sync::Lazy::new(Default::default);

/// Whether listed folder `id` is a work workspace ([`describe`]), reused
/// while its brief's stamp holds, for up to [`WORK_MAX_AGE`]. A brief that
/// isn't a regular file is read as [`front`] reads it, each time.
fn work_cached(vault: &Path, id: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let work = || describe(id, front(vault, id).as_ref()).is_some_and(|(_, _, work)| work);
    let path = vault.join("workspaces").join(id).join("WORKSPACE.md");
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return work();
    };
    if !meta.file_type().is_file() {
        return work();
    }
    let stamp: BriefStamp = (
        meta.len(),
        meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
        meta.ctime() as i128 * 1_000_000_000 + meta.ctime_nsec() as i128,
        meta.ino(),
    );
    if let Some((at, seen, flag)) = WORK_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&path)
    {
        if *seen == stamp && at.elapsed() < WORK_MAX_AGE {
            return *flag;
        }
    }
    // Stamped before the read: a write in between leaves a stamp that no
    // longer matches, so the next call reads again.
    let flag = work();
    let mut cache = WORK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= WORK_CACHE_MAX {
        cache.clear();
    }
    cache.insert(path, (std::time::Instant::now(), stamp, flag));
    flag
}

fn is_work(
    id: &str,
    title: &str,
    front: Option<&std::collections::BTreeMap<String, String>>,
) -> bool {
    let id_l = id.to_ascii_lowercase();
    let title_l = title.to_lowercase();
    id_l.starts_with("notion-")
        || id_l.starts_with("linear-")
        || front
            .and_then(|f| f.get("scope"))
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("work"))
        || WORK_MARKERS
            .iter()
            .any(|m| id_l.contains(m) || title_l.contains(m))
}

/// Whether [`workspaces`] lists folder `id`, before its brief is read.
fn listed_folder(id: &str, is_dir: bool) -> bool {
    is_dir && !id.starts_with('.') && valid_workspace_id(id)
}

/// Workspace `id`'s title, status and work flag as [`workspaces`] lists it,
/// from its brief's front matter `fm`; `None` when it is merged into another.
fn describe(
    id: &str,
    fm: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<(String, String, bool)> {
    if fm.is_some_and(|f| f.contains_key("merged_into")) {
        return None;
    }
    // The built-in chats space reads as Chats whatever an older brief
    // says, as `workspaces::summary_in` has it (2026-11-04).
    let title = if crate::workspaces::is_quick_chats(id) {
        crate::workspaces::QUICK_CHATS_TITLE.to_string()
    } else {
        fm.and_then(|f| f.get("title").cloned())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| id.to_string())
    };
    let status = fm
        .and_then(|f| f.get("status").cloned())
        .unwrap_or_default();
    let work = is_work(id, &title, fm);
    Some((title, status, work))
}

/// Every workspace folder with its title, status and work flag (merged
/// workspaces left out).
pub(crate) fn workspaces(vault: &Path, scope: &Scope) -> Vec<WorkspaceEntry> {
    let Ok(dir) = std::fs::read_dir(vault.join("workspaces")) else {
        return Vec::new();
    };
    let mut out: Vec<WorkspaceEntry> = Vec::new();
    for entry in dir.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let Ok(ft) = entry.file_type() else { continue };
        if !listed_folder(&id, ft.is_dir()) {
            continue;
        }
        let Some((title, status, work)) = describe(&id, front(vault, &id).as_ref()) else {
            continue;
        };
        out.push(WorkspaceEntry {
            personal: !work && scope.personal_workspaces.contains(&id),
            id,
            title,
            status,
            work,
        });
    }
    out.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    out
}

/// The ids of [`workspaces`]' work workspaces, for the chat search's order
/// (work chats rank after the open workspace's): unsorted, and each flag
/// reused while its brief is unchanged ([`work_cached`]), since reading and
/// classifying every brief on each search took 4.7 ms of a 27 ms first page
/// (2026-11-08). Not for what leaves the Mac: that reads the briefs.
pub(crate) fn work_ids(vault: &Path) -> std::collections::HashSet<String> {
    let Ok(dir) = std::fs::read_dir(vault.join("workspaces")) else {
        return std::collections::HashSet::new();
    };
    dir.flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            let ft = entry.file_type().ok()?;
            (listed_folder(&id, ft.is_dir()) && work_cached(vault, &id)).then_some(id)
        })
        .collect()
}

/// The first personal list: Hollow's own workspaces and the clearly personal
/// projects, never anything marked as work.
pub(crate) fn seed(vault: &Path) -> Vec<String> {
    let empty = Scope {
        include_work_data: false,
        personal_workspaces: Vec::new(),
        seeded_at: None,
    };
    workspaces(vault, &empty)
        .into_iter()
        .filter(|w| {
            let t = w.title.to_lowercase();
            !w.work
                && (SEED_TITLES.contains(&t.as_str())
                    || SEED_TITLE_PARTS.iter().any(|p| t.contains(p)))
        })
        .map(|w| w.id)
        .collect()
}

fn scope_path(store: &Store) -> std::path::PathBuf {
    store.root().join(SCOPE_FILE)
}

/// The saved scope, seeding (and saving) it when the file is absent. A file
/// that exists but can't be read or parsed fails closed: personal scope with
/// no workspaces, the file left alone, and the reason returned for Settings
/// (re-seeding would widen what the person had narrowed).
pub(crate) fn load_scope_checked(store: &Store, vault: &Path) -> (Scope, Option<String>) {
    let closed = Scope {
        include_work_data: false,
        personal_workspaces: Vec::new(),
        seeded_at: None,
    };
    match std::fs::read_to_string(scope_path(store)) {
        Ok(text) => match serde_json::from_str::<Scope>(&text) {
            Ok(scope) => (scope, None),
            Err(e) => (
                closed,
                Some(format!(
                    "scope.json can't be read ({e}); only tasks are sent until it is saved again in Settings"
                )),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let scope = Scope {
                include_work_data: false,
                personal_workspaces: seed(vault),
                seeded_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            };
            let _ = save_scope(store, &scope);
            (scope, None)
        }
        Err(e) => (closed, Some(format!("scope.json can't be read ({e})"))),
    }
}

/// Whether the person turned "include work data" on, read from the scope
/// file in `dir` (`<support>/Companion`) without seeding or saving it (a
/// read-only caller such as the phone's search). Off when the file is
/// absent or can't be read, as [`load_scope_checked`] fails closed.
pub(crate) fn work_data_on(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(SCOPE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Scope>(&text).ok())
        .is_some_and(|scope| scope.include_work_data)
}

/// Whether workspace `id` never reaches a device, even with work data on: a
/// `linear-*` workspace (#1222, [`classify`]). Checked on each chat's own
/// workspace, not through a list that could miss it.
pub(crate) fn never_on_devices(id: &str) -> bool {
    id.to_ascii_lowercase().starts_with("linear-")
}

/// The workspaces whose chats a device's search may show: an allow-list, so
/// whatever isn't classified stays on the Mac (2026-11-08 review; the deny
/// list before it came from [`workspaces`], which skips merged folders, so an
/// interrupted merge showed a work or `linear-*` workspace's chats). Never a
/// `linear-*` workspace ([`never_on_devices`]). With `include_work_data` on,
/// every other workspace folder. With it off, only a workspace whose brief
/// was read ([`front`]), carries no `merged_into` (its chats may still be
/// moving to another workspace) and is not work ([`is_work`]): a missing,
/// unreadable or unclosed brief leaves the workspace out.
pub(crate) fn shown_on_devices(
    vault: &Path,
    include_work_data: bool,
) -> std::collections::HashSet<String> {
    let mut shown = std::collections::HashSet::new();
    let Ok(dir) = std::fs::read_dir(vault.join("workspaces")) else {
        return shown;
    };
    for entry in dir.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let Ok(ft) = entry.file_type() else { continue };
        // The workspace folder rule itself (ids up to 120 characters), not
        // the device scope's shorter one, so no valid workspace is left out.
        if !ft.is_dir() || !crate::workspaces::valid_id(&id) || never_on_devices(&id) {
            continue;
        }
        let allowed = include_work_data
            || front(vault, &id).is_some_and(|fm| {
                let title = fm
                    .get("title")
                    .filter(|t| !t.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| id.clone());
                !fm.contains_key("merged_into") && !is_work(&id, &title, Some(&fm))
            });
        if allowed {
            shown.insert(id);
        }
    }
    shown
}

#[cfg(test)]
pub(crate) fn load_scope(store: &Store, vault: &Path) -> Scope {
    load_scope_checked(store, vault).0
}

pub(crate) fn save_scope(store: &Store, scope: &Scope) -> Result<(), String> {
    std::fs::create_dir_all(store.root()).map_err(|e| format!("companion store: {e}"))?;
    let mut clean = scope.clone();
    clean.personal_workspaces.retain(|id| {
        let id_l = id.to_ascii_lowercase();
        valid_workspace_id(id) && !id_l.starts_with("notion-") && !id_l.starts_with("linear-")
    });
    clean.personal_workspaces.sort();
    clean.personal_workspaces.dedup();
    let text = serde_json::to_string_pretty(&clean).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&scope_path(store), text.as_bytes())
}

// ── rules (the daemon's `docs.rs`) ───────────────────────────────────────

fn valid_workspace_id(id: &str) -> bool {
    (1..=96).contains(&id.len())
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// (kind, workspace, work-only) for a vault path the backend may hold.
pub(crate) fn classify(path: &str) -> Option<(&'static str, Option<String>, bool)> {
    if path.len() > MAX_PATH_BYTES || path.contains('\\') || path.chars().any(char::is_control) {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.starts_with('.'))
    {
        return None;
    }
    let name = parts[parts.len() - 1];
    if !name.ends_with(".md") || name.len() <= 3 {
        return None;
    }
    // Work records and Linear project workspaces never leave the Mac, even
    // when work data is on. `notion-*` stays work-only (the flag below).
    if parts.len() >= 2 && parts[0] == "library" && parts[1] == "records" {
        return None;
    }
    if parts.len() >= 2
        && parts[0] == "workspaces"
        && parts[1].to_ascii_lowercase().starts_with("linear-")
    {
        return None;
    }
    let class = |kind: &'static str, ws: Option<&str>, work: bool| {
        let notion = ws.is_some_and(|w| w.to_ascii_lowercase().starts_with("notion-"));
        Some((kind, ws.map(String::from), work || notion))
    };
    match parts.as_slice() {
        ["tasks", .., _] => class("task", None, false),
        ["notes", .., _] => class("note", None, false),
        ["library", "notes", .., _] => class("note", None, false),
        ["library", "work" | "client", .., _] => class("library", None, true),
        ["learnings", _] => class("learning", None, true),
        ["workspaces", ws, file] if valid_workspace_id(ws) => match *file {
            "WORKSPACE.md" | "PLAN.md" | "SOURCES.md" | "AGENTS.md" => {
                class("workspace", Some(ws), false)
            }
            "PROJECT.md" => class("project", Some(ws), true),
            _ => None,
        },
        ["workspaces", ws, "outputs", .., _] if valid_workspace_id(ws) => {
            class("output", Some(ws), false)
        }
        ["workspaces", ws, "sources", rest @ ..] if valid_workspace_id(ws) && !rest.is_empty() => {
            if rest.len() == 1 && rest[0].starts_with("recording-") {
                class("recording", Some(ws), false)
            } else {
                class("source", Some(ws), false)
            }
        }
        _ => None,
    }
}

// ── the walk ─────────────────────────────────────────────────────────────

/// One document as it is on disk now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalDoc {
    pub path: String,
    pub kind: &'static str,
    pub hash: String,
    pub size: u64,
    pub mtime: Option<String>,
}

/// (size, mtime) → (hash, its front matter says `scope: work`), so a scan
/// reads only files that changed.
#[derive(Default)]
pub(crate) struct HashCache {
    seen: HashMap<String, (u64, Option<SystemTime>, String, bool)>,
}

/// The document's own front matter opts it out (`scope: work`).
fn says_work(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    crate::frontmatter::parse_map(text)
        .0
        .get("scope")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("work"))
}

/// A folder the walk may enter: its real path is its vault path (no
/// symlinked parent such as a linked `library/` or `workspaces/<id>`).
fn real_dir(vault_canon: &Path, vault: &Path, rel: &str) -> bool {
    std::fs::canonicalize(vault.join(rel)).is_ok_and(|real| real == vault_canon.join(rel))
}

fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn rfc3339(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Every document the scope allows, sorted by path, plus the personal list
/// it used. Never follows symlinks, not even a symlinked parent folder.
pub(crate) fn collect(
    vault: &Path,
    scope: &Scope,
    cache: &mut HashCache,
) -> (Vec<LocalDoc>, Vec<String>) {
    let work = scope.include_work_data;
    let Ok(canon) = std::fs::canonicalize(vault) else {
        return (Vec::new(), Vec::new());
    };
    let mut roots: Vec<String> = vec!["notes".into(), "library/notes".into()];
    if work {
        roots.extend([
            "library/work".into(),
            "library/client".into(),
            "learnings".into(),
        ]);
    }
    let entries = workspaces(vault, scope);
    let personal: Vec<String> = entries
        .iter()
        .filter(|w| w.personal)
        .map(|w| w.id.clone())
        .collect();
    let workspace_ids: Vec<String> = if work {
        entries.into_iter().map(|w| w.id).collect()
    } else {
        personal.clone()
    };
    let mut out: Vec<LocalDoc> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in &roots {
        if real_dir(&canon, vault, root) {
            walk(vault, root, 0, work, cache, &mut out, &mut seen);
        }
    }
    for id in &workspace_ids {
        let dir = format!("workspaces/{id}");
        if !real_dir(&canon, vault, &dir) {
            continue;
        }
        for file in [
            "WORKSPACE.md",
            "PLAN.md",
            "SOURCES.md",
            "AGENTS.md",
            "PROJECT.md",
        ] {
            visit(
                vault,
                &format!("{dir}/{file}"),
                work,
                cache,
                &mut out,
                &mut seen,
            );
        }
        for sub in ["outputs", "sources"] {
            walk(
                vault,
                &format!("{dir}/{sub}"),
                0,
                work,
                cache,
                &mut out,
                &mut seen,
            );
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    cache.seen.retain(|p, _| seen.contains(p));
    (out, personal)
}

fn walk(
    vault: &Path,
    rel: &str,
    depth: usize,
    work: bool,
    cache: &mut HashCache,
    out: &mut Vec<LocalDoc>,
    seen: &mut std::collections::HashSet<String>,
) {
    if depth > MAX_DEPTH || out.len() >= MAX_DOCS {
        return;
    }
    let full = vault.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return;
    };
    if !meta.file_type().is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&full) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    for name in names {
        let child = format!("{rel}/{name}");
        let Ok(meta) = std::fs::symlink_metadata(vault.join(&child)) else {
            continue;
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(vault, &child, depth + 1, work, cache, out, seen);
        } else if ft.is_file() && name.ends_with(".md") {
            visit(vault, &child, work, cache, out, seen);
        }
    }
}

fn visit(
    vault: &Path,
    rel: &str,
    work: bool,
    cache: &mut HashCache,
    out: &mut Vec<LocalDoc>,
    seen: &mut std::collections::HashSet<String>,
) {
    if out.len() >= MAX_DOCS {
        return;
    }
    let Some((kind, _, work_only)) = classify(rel) else {
        return;
    };
    if kind == "task" || (work_only && !work) {
        return;
    }
    let full = vault.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return;
    };
    if !meta.file_type().is_file() {
        return;
    }
    let size = meta.len();
    let mtime = meta.modified().ok();
    let (hash, opted_out) = match cache.seen.get(rel) {
        Some((s, m, h, w)) if *s == size && *m == mtime => (h.clone(), *w),
        _ => {
            let Ok(bytes) = std::fs::read(&full) else {
                return;
            };
            let h = sha256_hex(&bytes);
            let w = says_work(&bytes);
            cache
                .seen
                .insert(rel.to_string(), (size, mtime, h.clone(), w));
            (h, w)
        }
    };
    seen.insert(rel.to_string());
    if opted_out && !work {
        return;
    }
    out.push(LocalDoc {
        path: rel.to_string(),
        kind,
        hash,
        size,
        mtime: mtime.map(rfc3339),
    });
}

/// The document as the daemon takes it: content when it fits (and still
/// matches the scanned hash), else metadata only.
pub(crate) fn wire(vault: &Path, doc: &LocalDoc) -> Option<serde_json::Value> {
    if doc.size > MAX_DOC_BYTES {
        return Some(
            serde_json::json!({ "path": doc.path, "kind": doc.kind, "hash": doc.hash,
                                        "truncated": true, "size": doc.size, "mtime": doc.mtime }),
        );
    }
    let full = vault.join(&doc.path);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    // Unreadable or not UTF-8: listed as metadata only, rather than retried
    // on every scan.
    let Some(content) = std::fs::read_to_string(&full)
        .ok()
        .filter(|c| c.len() as u64 <= MAX_DOC_BYTES)
    else {
        return Some(
            serde_json::json!({ "path": doc.path, "kind": doc.kind, "hash": doc.hash,
                                        "truncated": true, "size": doc.size, "mtime": doc.mtime }),
        );
    };
    let hash = sha256_hex(content.as_bytes());
    Some(
        serde_json::json!({ "path": doc.path, "kind": doc.kind, "hash": hash,
                             "content": content, "mtime": doc.mtime }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn vault() -> tempfile::TempDir {
        let v = tempfile::tempdir().unwrap();
        let r = v.path();
        write(r, "tasks/a.md", "---\ntitle: A\n---\n");
        write(r, "notes/idea.md", "idea");
        write(r, "library/notes/daily.md", "daily");
        write(r, "library/work/README.md", "work wiki");
        write(r, "library/client/x.md", "client");
        write(r, "learnings/l.md", "learning");
        write(r, "me/Identity.md", "me");
        write(r, "drafts/heartbeat/d.md", "draft");
        write(
            r,
            "workspaces/hollow-1/WORKSPACE.md",
            "---\ntitle: hollow\nstatus: active\n---\n",
        );
        write(r, "workspaces/hollow-1/PLAN.md", "plan");
        write(r, "workspaces/hollow-1/outputs/r.md", "report");
        write(r, "workspaces/hollow-1/outputs/c.canvas.html", "<html>");
        write(
            r,
            "workspaces/hollow-1/sources/recording-1-x.md",
            "transcript",
        );
        write(r, "workspaces/hollow-1/sessions/s.md", "session");
        write(r, "workspaces/hollow-1/.hollow-attachments/a.md", "att");
        write(
            r,
            "workspaces/rel-2/WORKSPACE.md",
            "---\ntitle: Canvas C1 release\nstatus: archived\n---\n",
        );
        write(
            r,
            "workspaces/w-3/WORKSPACE.md",
            "---\ntitle: client2.0\nstatus: active\n---\n",
        );
        write(r, "workspaces/w-3/PLAN.md", "work plan");
        write(
            r,
            "workspaces/notion-34db/WORKSPACE.md",
            "---\ntitle: Claims AI System\n---\n",
        );
        write(
            r,
            "workspaces/x-4/WORKSPACE.md",
            "---\ntitle: hollow\nscope: work\n---\n",
        );
        write(
            r,
            "workspaces/m-5/WORKSPACE.md",
            "---\ntitle: hollow\nmerged_into: hollow-1\n---\n",
        );
        write(
            r,
            "workspaces/new-6/WORKSPACE.md",
            "---\ntitle: Something new\n---\n",
        );
        write(
            r,
            "library/records/people/ada.md",
            "---\nscope: work\n---\nAda",
        );
        write(r, "library/records/meetings/2026/note.md", "meeting notes");
        write(
            r,
            "workspaces/linear-acme-abcd1234/WORKSPACE.md",
            "---\ntitle: Acme\n---\n",
        );
        write(r, "workspaces/linear-acme-abcd1234/PLAN.md", "linear plan");
        write(
            r,
            "workspaces/linear-acme-abcd1234/PROJECT.md",
            "linear project",
        );
        write(
            r,
            "workspaces/linear-acme-abcd1234/outputs/r.md",
            "linear out",
        );
        v
    }

    /// 2026-11-04: the built-in chats space is listed as Chats, as everywhere
    /// else, even when an older brief still says Quick chats.
    #[test]
    fn the_built_in_chats_space_is_listed_as_chats() {
        let v = vault();
        write(
            v.path(),
            "workspaces/quick-chats/WORKSPACE.md",
            "---\nhollow_workspace: 1\nkind: quick\ntitle: \"Quick chats\"\n---\n\n# Quick chats\n",
        );
        let scope = Scope {
            include_work_data: false,
            personal_workspaces: vec![],
            seeded_at: None,
        };
        let listed = workspaces(v.path(), &scope);
        let quick = listed.iter().find(|w| w.id == "quick-chats").unwrap();
        assert_eq!(quick.title, "Quick chats");
        assert_eq!(
            listed.iter().find(|w| w.id == "new-6").unwrap().title,
            "Something new"
        );
    }

    #[test]
    fn the_seed_takes_hollow_and_personal_workspaces_never_work() {
        let v = vault();
        let seeded = seed(v.path());
        let set: std::collections::BTreeSet<&str> = seeded.iter().map(String::as_str).collect();
        assert_eq!(set, ["hollow-1", "rel-2"].into_iter().collect());
    }

    #[test]
    fn personal_scope_collects_only_allowed_paths() {
        let v = vault();
        let scope = Scope {
            include_work_data: false,
            personal_workspaces: vec![
                "hollow-1".into(),
                "w-3".into(),
                "notion-34db".into(),
                "x-4".into(),
            ],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let paths: Vec<String> = collect(v.path(), &scope, &mut cache)
            .0
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert_eq!(
            paths,
            vec![
                "library/notes/daily.md",
                "notes/idea.md",
                "workspaces/hollow-1/PLAN.md",
                "workspaces/hollow-1/WORKSPACE.md",
                "workspaces/hollow-1/outputs/r.md",
                "workspaces/hollow-1/sources/recording-1-x.md",
            ]
        );
    }

    #[test]
    fn work_scope_adds_the_work_library_learnings_and_every_workspace() {
        let v = vault();
        let scope = Scope {
            include_work_data: true,
            personal_workspaces: vec![],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let docs = collect(v.path(), &scope, &mut cache).0;
        let paths: Vec<&str> = docs.iter().map(|d| d.path.as_str()).collect();
        for p in [
            "library/work/README.md",
            "library/client/x.md",
            "learnings/l.md",
            "workspaces/w-3/PLAN.md",
            "workspaces/notion-34db/WORKSPACE.md",
        ] {
            assert!(paths.contains(&p), "{p}");
        }
        for p in [
            "me/Identity.md",
            "drafts/heartbeat/d.md",
            "tasks/a.md",
            "workspaces/hollow-1/sessions/s.md",
            "library/records/people/ada.md",
            "library/records/meetings/2026/note.md",
            "workspaces/linear-acme-abcd1234/WORKSPACE.md",
            "workspaces/linear-acme-abcd1234/PLAN.md",
            "workspaces/linear-acme-abcd1234/PROJECT.md",
            "workspaces/linear-acme-abcd1234/outputs/r.md",
        ] {
            assert!(!paths.contains(&p), "{p}");
        }
    }

    /// `library/records/` and `linear-*` never mirror, personal or work, and a
    /// `linear-*` id cannot be saved onto the personal list. `scope: work` on
    /// an ordinary note still opts it out of personal scope only.
    #[test]
    fn work_records_and_linear_workspaces_never_mirror() {
        let v = vault();
        assert!(classify("library/records/people/ada.md").is_none());
        assert!(classify("library/records/meetings/2026/note.md").is_none());
        assert!(classify("workspaces/linear-acme-abcd1234/WORKSPACE.md").is_none());
        assert!(classify("workspaces/linear-acme-abcd1234/PROJECT.md").is_none());
        assert!(classify("workspaces/Linear-Acme/PLAN.md").is_none());
        let notion = classify("workspaces/notion-34db/WORKSPACE.md").unwrap();
        assert!(notion.2, "a notion workspace stays work-only");
        for include_work_data in [false, true] {
            let scope = Scope {
                include_work_data,
                personal_workspaces: vec!["linear-acme-abcd1234".into(), "hollow-1".into()],
                seeded_at: None,
            };
            let mut cache = HashCache::default();
            let paths: Vec<String> = collect(v.path(), &scope, &mut cache)
                .0
                .into_iter()
                .map(|d| d.path)
                .collect();
            assert!(
                paths.iter().all(|p| {
                    !p.starts_with("library/records/")
                        && !p.to_ascii_lowercase().contains("/linear-")
                }),
                "work data on = {include_work_data}: {paths:?}"
            );
        }
        let listed = workspaces(
            v.path(),
            &Scope {
                include_work_data: false,
                personal_workspaces: vec!["linear-acme-abcd1234".into()],
                seeded_at: None,
            },
        );
        let linear = listed
            .iter()
            .find(|w| w.id == "linear-acme-abcd1234")
            .unwrap();
        assert!(linear.work && !linear.personal);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_followed() {
        let v = vault();
        let r = v.path();
        std::os::unix::fs::symlink(
            r.join("library/work"),
            r.join("workspaces/hollow-1/outputs/linked"),
        )
        .unwrap();
        std::os::unix::fs::symlink(r.join("me/Identity.md"), r.join("notes/me.md")).unwrap();
        let scope = Scope {
            include_work_data: false,
            personal_workspaces: vec!["hollow-1".into()],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let paths: Vec<String> = collect(r, &scope, &mut cache)
            .0
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert!(
            paths
                .iter()
                .all(|p| !p.contains("linked") && p != "notes/me.md"),
            "{paths:?}"
        );
    }

    #[test]
    fn hashes_are_cached_by_size_and_mtime_and_wire_carries_content() {
        let v = vault();
        let scope = Scope {
            include_work_data: false,
            personal_workspaces: vec![],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let docs = collect(v.path(), &scope, &mut cache).0;
        let idea = docs.iter().find(|d| d.path == "notes/idea.md").unwrap();
        assert_eq!(idea.hash, sha256_hex(b"idea"));
        let w = wire(v.path(), idea).unwrap();
        assert_eq!(w["content"], "idea");
        write(v.path(), "notes/big.md", &"x".repeat(300 * 1024));
        let docs = collect(v.path(), &scope, &mut cache).0;
        let big = docs.iter().find(|d| d.path == "notes/big.md").unwrap();
        let w = wire(v.path(), big).unwrap();
        assert_eq!(w["truncated"], true);
        assert!(w.get("content").is_none());
    }

    #[test]
    fn the_scope_file_is_seeded_once_and_never_holds_notion_ids() {
        let v = vault();
        let s = tempfile::tempdir().unwrap();
        let store = Store::at(s.path().to_path_buf());
        let first = load_scope(&store, v.path());
        assert!(first.seeded_at.is_some());
        assert!(!first.include_work_data);
        let mut changed = first.clone();
        changed.personal_workspaces = vec![
            "notion-34db".into(),
            "linear-acme-abcd1234".into(),
            "Linear-Other".into(),
            "new-6".into(),
            "../me".into(),
        ];
        save_scope(&store, &changed).unwrap();
        let again = load_scope(&store, v.path());
        assert_eq!(again.personal_workspaces, vec!["new-6".to_string()]);
        // The listing marks work workspaces even when listed.
        let list = workspaces(v.path(), &again);
        let w3 = list.iter().find(|w| w.id == "w-3").unwrap();
        assert!(w3.work && !w3.personal);
        assert!(
            list.iter().all(|w| w.id != "m-5"),
            "merged workspaces are left out"
        );
    }

    #[test]
    fn an_unreadable_scope_file_fails_closed_and_is_left_alone() {
        let v = vault();
        let st = tempfile::tempdir().unwrap();
        let store = Store::at(st.path().to_path_buf());
        std::fs::write(st.path().join(SCOPE_FILE), "{ not json").unwrap();
        let (scope, error) = load_scope_checked(&store, v.path());
        assert!(error.is_some());
        assert!(!scope.include_work_data && scope.personal_workspaces.is_empty());
        assert_eq!(
            std::fs::read_to_string(st.path().join(SCOPE_FILE)).unwrap(),
            "{ not json"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_parent_folder_is_not_walked() {
        let v = vault();
        let r = v.path();
        // A personal workspace whose folder is a link to a work workspace.
        std::os::unix::fs::symlink(r.join("workspaces/w-3"), r.join("workspaces/linked-7"))
            .unwrap();
        // library/notes replaced by a link to the work wiki.
        std::fs::remove_dir_all(r.join("library/notes")).unwrap();
        std::os::unix::fs::symlink(r.join("library/work"), r.join("library/notes")).unwrap();
        let scope = Scope {
            include_work_data: false,
            personal_workspaces: vec!["linked-7".into(), "hollow-1".into()],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let paths: Vec<String> = collect(r, &scope, &mut cache)
            .0
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert!(
            paths
                .iter()
                .all(|p| !p.starts_with("workspaces/linked-7") && !p.starts_with("library/notes")),
            "{paths:?}"
        );
        assert!(paths.contains(&"workspaces/hollow-1/PLAN.md".to_string()));
    }

    #[test]
    fn a_document_marked_scope_work_stays_off_in_personal_scope() {
        let v = vault();
        write(
            v.path(),
            "notes/meeting.md",
            "---\nscope: work\n---\nAcme notes",
        );
        let personal = Scope {
            include_work_data: false,
            personal_workspaces: vec![],
            seeded_at: None,
        };
        let mut cache = HashCache::default();
        let paths: Vec<String> = collect(v.path(), &personal, &mut cache)
            .0
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert!(!paths.contains(&"notes/meeting.md".to_string()));
        let work = Scope {
            include_work_data: true,
            ..personal
        };
        let paths: Vec<String> = collect(v.path(), &work, &mut cache)
            .0
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert!(paths.contains(&"notes/meeting.md".to_string()));
    }

    /// Second 2026-11-08 review: the devices' chat list is an allow-list.
    /// With work data off only workspaces read and classified as personal
    /// are on it; a merged one, a missing or unclosed brief and a work one
    /// are not, and a long brief keeps its `scope: work`. With it on, every
    /// folder but `linear-*` ones, merged or not.
    #[test]
    fn devices_show_only_workspaces_classified_as_allowed() {
        let v = vault();
        let r = v.path();
        let long = format!(
            "---\ntitle: Garden\nscope: work\n---\n{}",
            "x".repeat(300 * 1024)
        );
        write(r, "workspaces/big-7/WORKSPACE.md", &long);
        let unclosed = format!("---\ntitle: Garden\n{}", "y: z\n".repeat(20_000));
        write(r, "workspaces/open-8/WORKSPACE.md", &unclosed);
        write(r, "workspaces/nobrief-9/PLAN.md", "plan");
        write(
            r,
            "workspaces/linear-old-10/WORKSPACE.md",
            "---\ntitle: Old\nmerged_into: hollow-1\n---\n",
        );
        let sorted = |set: std::collections::HashSet<String>| {
            let mut ids: Vec<String> = set.into_iter().collect();
            ids.sort();
            ids
        };
        assert_eq!(
            sorted(shown_on_devices(r, false)),
            ["hollow-1", "new-6", "rel-2"]
        );
        assert_eq!(
            sorted(shown_on_devices(r, true)),
            [
                "big-7",
                "hollow-1",
                "m-5",
                "new-6",
                "nobrief-9",
                "notion-34db",
                "open-8",
                "rel-2",
                "w-3",
                "x-4"
            ]
        );
        assert!(never_on_devices("Linear-Acme") && !never_on_devices("hollow-1"));
        // A personal workspace id up to the folder rule's 120 characters is
        // shown too (delta review: the device scope's 96 left it out).
        let long_id = format!("personal-{}", "a".repeat(100));
        write(
            r,
            &format!("workspaces/{long_id}/WORKSPACE.md"),
            "---\ntitle: Garden\n---\n",
        );
        assert!(shown_on_devices(r, false).contains(&long_id));
        assert!(shown_on_devices(r, true).contains(&long_id));
        // Settings reads the long brief's front matter too.
        let listed = workspaces(
            r,
            &Scope {
                include_work_data: false,
                personal_workspaces: vec!["big-7".into()],
                seeded_at: None,
            },
        );
        let big = listed.iter().find(|w| w.id == "big-7").unwrap();
        assert!(big.work && !big.personal, "{big:?}");
        assert_eq!(big.title, "Garden");
    }
}
