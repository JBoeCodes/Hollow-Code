//! Folder abstracts (CX23): a one-line abstract and a short overview per
//! folder of a project or an org section, built bottom up from its
//! children's abstracts, kept in a rebuildable derived index. The cheap
//! middle between CX4's short core and CX5b's retrieval.
//!
//! **Off by default.** `context.folderAbstracts` (`folder-abstracts.json` in
//! app data) gates both building and showing. Off, nothing is built,
//! [`refresh_now`] refuses, the read action says it is off, and
//! `workspaces.list_workspaces` answers byte for byte as before. It stays off
//! until a measured recall comparison says it helps (none is built here).
//!
//! **Roots.** Each active project workspace's vault folder (`workspaces/<id>`,
//! not build, Chats, COO or archived workspaces), root id `project:<id>`, and
//! each top-level folder of the org clone (hidden ones and `tools/` skipped,
//! as CX5b's retrieval does), root id `org:<folder>`. Under a root only
//! Markdown and text files count: hidden entries, symlinks and
//! [`SKIP_DIRS`] are skipped, at most [`MAX_DEPTH`] folders deep and
//! [`MAX_FILES`] files per root, each read through
//! `context_roots::read_in_root` (no symlink followed) within [`READ_MAX`].
//!
//! **Refresh, bottom up.** A root's tree is walked in post order. A file's
//! key is a hash of [`PROMPT_VERSION`] and its text; a folder's key is a
//! hash of the prompt version and its children's names and abstracts. A
//! node whose key matches the index keeps its entry and makes no model call;
//! otherwise the model writes it (a file gets an abstract, a folder an
//! abstract and an overview). So a changed file refreshes itself and each
//! folder above it once, only while the abstract it hands up changed, and an
//! unchanged tree makes no call. A folder with nothing summarized under it
//! gets no entry. A refresh makes at most `maxCalls` calls; one that stops
//! there (or on a model error) keeps what it wrote and the old entries it
//! didn't reach, and the next refresh goes on from there.
//!
//! **The index.** `FolderAbstracts/<vault stem>.json` in Hollow's support
//! folder (beside `EmbedIndex/`), never in a layer's own files: delete it
//! and the next refresh rebuilds it. Entries by root id, then by the path
//! inside the root (`""` for the root itself).
//!
//! **The model** is the [`AbstractModel`] seam: the app's agent with its
//! extraction model (Haiku on Claude unless chosen), tool-free, metered as
//! the `folder_abstract` one-shot ([`ConfiguredModel`]); tests count calls
//! with a fake.
//!
//! **Where a lead sees it.** With the setting on, `workspaces.list_workspaces`
//! rows that have one carry `abstract` (the root's line), so the COO or a
//! lead choosing between projects reads it before opening one;
//! `context.folder_abstracts` reads the overviews and an org section's;
//! `context.refresh_folder_abstracts` refreshes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The prompts' version: bump it when [`file_prompt`] or [`folder_prompt`]
/// changes, and every entry is written again on the next refresh.
pub(crate) const PROMPT_VERSION: u32 = 2;
/// Folders deep a root's walk goes, at most.
const MAX_DEPTH: usize = 3;
/// Files summarized per root, at most.
const MAX_FILES: usize = 80;
/// Largest file read; a bigger one is skipped.
const READ_MAX: u64 = 256 * 1024;
/// A file's text in its prompt, at most (characters).
const PROMPT_TEXT_MAX: usize = 6_000;
/// An abstract and an overview, at most (characters).
const ABSTRACT_MAX: usize = 200;
const OVERVIEW_MAX: usize = 800;
/// Model calls one refresh makes by default, and at most.
pub(crate) const DEFAULT_CALLS: usize = 100;
pub(crate) const MAX_CALLS: usize = 1_000;
/// Folders never walked.
const SKIP_DIRS: [&str; 6] = ["node_modules", "target", "dist", "build", "vendor", "log"];
/// The setting's file in app data.
const SETTINGS_FILE: &str = "folder-abstracts.json";

// ---------------------------------------------------------------------------
// The setting
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    enabled: bool,
}

/// Whether the setting saved at `path` is on (no file, or one that doesn't
/// read, is off).
pub(crate) fn enabled_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<SettingsFile>(&t).ok())
        .is_some_and(|s| s.enabled)
}

pub(crate) fn set_enabled_at(path: &Path, on: bool) -> Result<bool, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let text =
        serde_json::to_string_pretty(&SettingsFile { enabled: on }).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(path, text.as_bytes())?;
    Ok(on)
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(SETTINGS_FILE))
}

/// `context.folderAbstracts`.
pub(crate) fn enabled() -> bool {
    settings_path().map(|p| enabled_at(&p)).unwrap_or(false)
}

/// Save `context.folderAbstracts`.
pub(crate) fn set_enabled(on: bool) -> Result<bool, String> {
    set_enabled_at(&settings_path()?, on)
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// One node's summary and the key it was written for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub key: String,
    #[serde(rename = "abstract")]
    pub line: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub overview: String,
}

/// Entries by root id, then by the path inside the root.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Index {
    #[serde(default)]
    pub roots: BTreeMap<String, BTreeMap<String, Entry>>,
}

#[cfg(not(test))]
fn index_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("FolderAbstracts"))
}

#[cfg(test)]
fn index_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir(
        "hollow-test-folder-abstracts-",
    ))
}

/// The vault's index file.
pub(crate) fn index_path(vault: &Path) -> Option<PathBuf> {
    Some(index_root()?.join(format!(
        "{}.json",
        crate::session_store::vault_file_stem(vault)
    )))
}

/// The index at `path`; missing or unreadable reads as empty (it is
/// derived, so the next refresh rebuilds it).
pub(crate) fn load(path: &Path) -> Index {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save(path: &Path, index: &Index) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string(index).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(path, text.as_bytes())
}

// ---------------------------------------------------------------------------
// Roots and their trees
// ---------------------------------------------------------------------------

/// A folder summarized from its root down.
#[derive(Debug, Clone)]
pub(crate) struct Root {
    /// `project:<workspace id>` or `org:<folder>`.
    pub id: String,
    /// What the prompts call it (the workspace's title, `org/<folder>`).
    pub label: String,
    pub dir: PathBuf,
}

/// Every root: active project workspaces, then the org clone's sections.
pub(crate) fn roots(vault: &Path) -> Vec<Root> {
    let mut out = Vec::new();
    for w in crate::workspaces::list(vault).unwrap_or_default() {
        let builtin = matches!(w.kind.as_deref(), Some("quick") | Some("main"));
        if builtin || w.build.is_some() || crate::workspaces::is_archived_status(&w.status) {
            continue;
        }
        out.push(Root {
            id: format!("project:{}", w.id),
            label: w.title,
            dir: vault.join(&w.path),
        });
    }
    if let Some(org) = crate::context_layers::org_root() {
        let mut names: Vec<String> = std::fs::read_dir(&org)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| !n.starts_with('.') && n != "tools")
            .collect();
        names.sort();
        for name in names {
            out.push(Root {
                id: format!("org:{name}"),
                label: format!("org/{name}"),
                dir: org.join(&name),
            });
        }
    }
    out
}

/// One node to summarize, in post order (children before their folder).
#[derive(Debug, Clone)]
pub(crate) enum Item {
    File {
        rel: String,
        text: String,
    },
    /// A folder and its children's (name, rel) pairs, sorted by name.
    Dir {
        rel: String,
        children: Vec<(String, String)>,
    },
}

fn summarizable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")
}

/// `root`'s tree in post order, within the caps; empty when it isn't a
/// folder.
pub(crate) fn plan(root: &Path) -> Vec<Item> {
    let mut out = Vec::new();
    let mut files = 0usize;
    // The root's own path may pass a symlink (macOS's /var); below it none
    // is followed.
    if let Ok(root) = std::fs::canonicalize(root) {
        if root.is_dir() {
            walk(&root, "", 0, &mut files, &mut out);
        }
    }
    out
}

fn walk(root: &Path, rel: &str, depth: usize, files: &mut usize, out: &mut Vec<Item>) {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            // `file_type` doesn't follow a symlink, so one is neither.
            let t = e.file_type().ok()?;
            let name = e.file_name().to_str()?.to_string();
            // A hidden or secret-looking name (`credentials.md`, `secrets/`)
            // is never read: the rule `read_source` applies.
            (!crate::source_evidence::forbidden_component(&name) && (t.is_dir() || t.is_file()))
                .then_some((name, t.is_dir()))
        })
        .collect();
    entries.sort();
    let mut children = Vec::new();
    for (name, is_dir) in entries {
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if is_dir {
            if depth + 1 > MAX_DEPTH || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let before = out.len();
            walk(root, &child, depth + 1, files, out);
            if out.len() > before {
                children.push((name, child));
            }
        } else if summarizable(&name) && *files < MAX_FILES {
            let text = match crate::context_roots::read_in_root(root, &child, READ_MAX) {
                Ok(Some(t)) if !t.trim().is_empty() => t,
                _ => continue,
            };
            *files += 1;
            out.push(Item::File {
                rel: child.clone(),
                text,
            });
            children.push((name, child));
        }
    }
    if !children.is_empty() {
        out.push(Item::Dir {
            rel: rel.to_string(),
            children,
        });
    }
}

// ---------------------------------------------------------------------------
// The model seam and the refresh
// ---------------------------------------------------------------------------

/// The model a refresh asks.
pub(crate) trait AbstractModel {
    fn complete(&self, prompt: &str) -> impl Future<Output = Result<String, String>> + Send;
}

/// The app's agent with its extraction model, tool-free.
pub(crate) struct ConfiguredModel;

impl AbstractModel for ConfiguredModel {
    async fn complete(&self, prompt: &str) -> Result<String, String> {
        let cwd = std::env::temp_dir();
        let opts = crate::claude_runner::RunOpts {
            cwd: &cwd,
            timeout: std::time::Duration::from_secs(120),
            allowed_tools: &[],
            disallowed_tools: crate::claude_runner::NO_TOOLS,
            model: None,
            debug_log: None,
            meter: crate::usage_ledger::Meter::oneshot("folder_abstract"),
            self_hosted: None,
        };
        let (kind, saved) = crate::agent_settings::resolve(None, None)?;
        let model =
            crate::extract::model_for(kind, &crate::extract::extraction_settings()).or(saved);
        crate::claude_runner::run_oneshot_selected(kind, model, prompt, opts).await
    }
}

fn key(version: u32, kind: &str, payload: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("v{version}\0{kind}\0").as_bytes());
    h.update(payload.as_bytes());
    format!("{:x}", h.finalize())
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_string(),
    }
}

fn file_prompt(label: &str, rel: &str, text: &str) -> String {
    format!(
        "You write one line of an index a project lead reads before opening a project. \
         Reply with one plain line of at most 160 characters saying what the file is and what \
         it holds. No preamble, quotes or Markdown.\n\n\
         Project: {}\nFile: {}\n{}",
        crate::untrusted::inline(label, 200),
        crate::untrusted::inline(rel, 300),
        crate::untrusted::wrap(
            crate::untrusted::Origin::File,
            rel,
            cut(text, PROMPT_TEXT_MAX).as_str()
        )
    )
}

fn folder_prompt(label: &str, rel: &str, lines: &str) -> String {
    let at = if rel.is_empty() {
        "its top folder".to_string()
    } else {
        crate::untrusted::inline(rel, 300)
    };
    let label = crate::untrusted::inline(label, 200);
    format!(
        "You write an index a project lead reads before opening a project. Below are one-line \
         abstracts of the files and folders in {at} of {label}. \
         Reply with: first line, an abstract of at most 160 characters saying what this folder \
         holds; then a blank line; then an overview of at most 600 characters in plain sentences \
         (its main parts and where to look). No preamble, headings or quotes.\n\n\
         Project: {label}\nFolder: {at}\n{}",
        crate::untrusted::wrap(crate::untrusted::Origin::File, &at, lines)
    )
}

/// The abstract (first non-empty line) and the overview (the rest), cut to
/// their caps; `None` for an empty answer.
fn parse(answer: &str) -> Option<(String, String)> {
    let mut lines = answer.trim().lines();
    let first = lines.next()?.trim().trim_start_matches("Abstract:").trim();
    if first.is_empty() {
        return None;
    }
    let rest: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    let overview = rest.join(" ");
    let overview = overview.trim_start_matches("Overview:").trim();
    Some((cut(first, ABSTRACT_MAX), cut(overview, OVERVIEW_MAX)))
}

/// What a refresh did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub roots: usize,
    /// Model calls made.
    pub calls: usize,
    /// Nodes whose key matched: no call.
    pub reused: usize,
    /// It stopped at `maxCalls`; refresh again to go on.
    pub stopped_at_cap: bool,
    /// The model error it stopped on, if any.
    pub error: Option<String>,
    pub prompt_version: u32,
}

/// Refresh root `root_id` (`items` from [`plan`]) into `index` with `model`,
/// counting calls in `report`. Returns false once it stopped (the cap or an
/// error): what it wrote and the old entries it didn't reach are kept.
#[allow(clippy::too_many_arguments)] // the root, its tree, the model and the pass's limits
pub(crate) async fn refresh_root<M: AbstractModel>(
    index: &mut Index,
    root_id: &str,
    label: &str,
    items: &[Item],
    model: &M,
    version: u32,
    max_calls: usize,
    report: &mut Report,
) -> bool {
    let old = index.roots.remove(root_id).unwrap_or_default();
    let mut new: BTreeMap<String, Entry> = BTreeMap::new();
    let mut done = true;
    for item in items {
        let (rel, node_key, prompt, is_file) = match item {
            Item::File { rel, text } => (
                rel,
                key(version, "file", text),
                // The key hashes the raw text; the model sees it redacted.
                file_prompt(label, rel, &crate::claude_runner::redact(text)),
                true,
            ),
            Item::Dir { rel, children } => {
                let lines: Vec<String> = children
                    .iter()
                    .filter_map(|(name, child)| {
                        new.get(child).map(|e| format!("- {name}: {}", e.line))
                    })
                    .collect();
                if lines.is_empty() {
                    continue;
                }
                let lines = lines.join("\n");
                (
                    rel,
                    key(version, "folder", &lines),
                    folder_prompt(label, rel, &lines),
                    false,
                )
            }
        };
        if let Some(e) = old.get(rel).filter(|e| e.key == node_key) {
            new.insert(rel.clone(), e.clone());
            report.reused += 1;
            continue;
        }
        if report.calls >= max_calls {
            report.stopped_at_cap = true;
            done = false;
            break;
        }
        report.calls += 1;
        let parsed = match model.complete(&prompt).await {
            Ok(answer) => {
                parse(&answer).ok_or_else(|| "the model gave an empty answer".to_string())
            }
            Err(e) => Err(e),
        };
        match parsed {
            Ok((line, overview)) => {
                let overview = if is_file { String::new() } else { overview };
                new.insert(
                    rel.clone(),
                    Entry {
                        key: node_key,
                        line,
                        overview,
                    },
                );
            }
            Err(e) => {
                report.error = Some(format!("{root_id} {rel:?}: {e}"));
                done = false;
                break;
            }
        }
    }
    if !done {
        // Keep the old entries it didn't reach that are still in the tree.
        let live: std::collections::BTreeSet<&str> = items
            .iter()
            .map(|i| match i {
                Item::File { rel, .. } | Item::Dir { rel, .. } => rel.as_str(),
            })
            .collect();
        for (rel, e) in old {
            if live.contains(rel.as_str()) {
                new.entry(rel).or_insert(e);
            }
        }
    }
    if !new.is_empty() {
        index.roots.insert(root_id.to_string(), new);
    }
    done
}

/// One refresh at a time in this process.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// `context.refresh_folder_abstracts`: refresh every root (or only `only`,
/// a root id or a workspace id) with the configured model, at most
/// `max_calls` calls. Refused while the setting is off or another refresh
/// runs. A full refresh drops roots that are gone.
pub(crate) async fn refresh_now(
    vault: PathBuf,
    only: Option<String>,
    max_calls: usize,
) -> Result<Report, String> {
    if !enabled() {
        return Err(
            "Folder abstracts are off (context.folderAbstracts). Nothing was built.".into(),
        );
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err(
            "A folder abstracts refresh is already running. Nothing else was started.".into(),
        );
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    let path = index_path(&vault).ok_or("Hollow's support folder isn't available.")?;
    let only = only.map(|o| o.trim().to_string()).filter(|o| !o.is_empty());
    let (path_c, only_c) = (path.clone(), only.clone());
    let (mut index, planned) = crate::shared_mcp::blocking(move || {
        let mut all = roots(&vault);
        if let Some(o) = &only_c {
            all.retain(|r| r.id == *o || r.id == format!("project:{o}"));
        }
        let planned: Vec<(Root, Vec<Item>)> = all
            .into_iter()
            .map(|r| {
                let items = plan(&r.dir);
                (r, items)
            })
            .collect();
        (load(&path_c), planned)
    })
    .await
    .map_err(|e| format!("the refresh stopped: {e}"))?;
    if let Some(o) = &only {
        if planned.is_empty() {
            return Err(format!(
                "No folder abstracts root is named {o}. Nothing was built."
            ));
        }
    } else {
        let ids: std::collections::BTreeSet<&str> =
            planned.iter().map(|(r, _)| r.id.as_str()).collect();
        index.roots.retain(|id, _| ids.contains(id.as_str()));
    }
    let mut report = Report {
        roots: planned.len(),
        prompt_version: PROMPT_VERSION,
        ..Report::default()
    };
    let model = ConfiguredModel;
    for (root, items) in &planned {
        let before = report.calls;
        let go_on = refresh_root(
            &mut index,
            &root.id,
            &root.label,
            items,
            &model,
            PROMPT_VERSION,
            max_calls.clamp(1, MAX_CALLS),
            &mut report,
        )
        .await;
        // Saved after each root that made calls, so a crash loses no paid work.
        if report.calls > before {
            let (p, snapshot) = (path.clone(), index.clone());
            crate::shared_mcp::blocking(move || save(&p, &snapshot))
                .await
                .map_err(|e| format!("the refresh stopped: {e}"))??;
        }
        if !go_on {
            break;
        }
    }
    crate::shared_mcp::blocking(move || save(&path, &index))
        .await
        .map_err(|e| format!("the refresh stopped: {e}"))??;
    Ok(report)
}

// ---------------------------------------------------------------------------
// The read side
// ---------------------------------------------------------------------------

/// `context.folder_abstracts`: off, says so; on, each root's abstract and
/// overview, or with `root` every folder of that one root.
pub(crate) fn read(vault: &Path, root: Option<&str>) -> Value {
    if !enabled() {
        return serde_json::json!({
            "enabled": false,
            "note": "Folder abstracts are off (context.folderAbstracts, off until a measured recall comparison says they help).",
        });
    }
    let index = index_path(vault).map(|p| load(&p)).unwrap_or_default();
    read_index(&index, root)
}

fn read_index(index: &Index, root: Option<&str>) -> Value {
    let folders = |entries: &BTreeMap<String, Entry>, every: bool| -> Vec<Value> {
        entries
            .iter()
            .filter(|(rel, e)| if every { !e.overview.is_empty() || rel.is_empty() } else { rel.is_empty() })
            .map(|(rel, e)| serde_json::json!({ "path": rel, "abstract": e.line, "overview": e.overview }))
            .collect()
    };
    match root.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => {
            let id = if index.roots.contains_key(r) {
                r.to_string()
            } else {
                format!("project:{r}")
            };
            match index.roots.get(&id) {
                Some(entries) => {
                    serde_json::json!({ "enabled": true, "root": id, "folders": folders(entries, true) })
                }
                None => {
                    serde_json::json!({ "enabled": true, "root": id, "folders": [], "note": "Nothing built for this root yet: context.refresh_folder_abstracts builds it." })
                }
            }
        }
        None => {
            let roots: Vec<Value> = index
                .roots
                .iter()
                .filter_map(|(id, entries)| {
                    let top = entries.get("")?;
                    Some(serde_json::json!({ "root": id, "abstract": top.line, "overview": top.overview }))
                })
                .collect();
            serde_json::json!({ "enabled": true, "promptVersion": PROMPT_VERSION, "roots": roots })
        }
    }
}

/// `list_workspaces`' answer: while `on`, each row that has one carries its
/// root's `abstract`; off, `answer` comes back untouched and no index is read.
pub(crate) fn annotate(vault: &Path, mut answer: Value, on: bool) -> Value {
    if !on {
        return answer;
    }
    let index = index_path(vault).map(|p| load(&p)).unwrap_or_default();
    annotate_with(&index, &mut answer);
    answer
}

fn annotate_with(index: &Index, answer: &mut Value) {
    let Some(rows) = answer.get_mut("workspaces").and_then(Value::as_array_mut) else {
        return;
    };
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let line = index
            .roots
            .get(&format!("project:{id}"))
            .and_then(|e| e.get(""))
            .map(|e| e.line.clone());
        if let (Some(line), Some(obj)) = (line, row.as_object_mut()) {
            obj.insert("abstract".into(), Value::String(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Counts calls and remembers which node each was for; the answer is a
    /// hash of the prompt, so a changed input changes the abstract.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
        /// A node whose answer is fixed, whatever its input.
        fixed: Option<&'static str>,
    }

    impl AbstractModel for Fake {
        async fn complete(&self, prompt: &str) -> Result<String, String> {
            let node = prompt
                .lines()
                .find_map(|l| {
                    l.strip_prefix("File: ")
                        .or_else(|| l.strip_prefix("Folder: "))
                })
                .unwrap_or("")
                .to_string();
            self.calls.lock().unwrap().push(node.clone());
            if self.fixed == Some(node.as_str()) {
                return Ok("fixed abstract\n\nfixed overview".into());
            }
            Ok(format!(
                "abstract {}\n\noverview of {node}",
                &key(0, "fake", prompt)[..12]
            ))
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("a.md"), "alpha notes").unwrap();
        std::fs::write(p.join("b.txt"), "bravo text").unwrap();
        std::fs::write(p.join("skip.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join("sub/c.md"), "charlie plan").unwrap();
        std::fs::create_dir_all(p.join(".git")).unwrap();
        std::fs::write(p.join(".git/x.md"), "hidden").unwrap();
        std::fs::create_dir_all(p.join("node_modules")).unwrap();
        std::fs::write(p.join("node_modules/y.md"), "skipped").unwrap();
        dir
    }

    fn refresh(index: &mut Index, root: &Path, model: &Fake, version: u32, cap: usize) -> Report {
        let items = plan(root);
        let mut report = Report::default();
        block_on(refresh_root(
            index,
            "project:p",
            "P",
            &items,
            model,
            version,
            cap,
            &mut report,
        ));
        report
    }

    fn calls(model: &Fake) -> Vec<String> {
        std::mem::take(&mut *model.calls.lock().unwrap())
    }

    #[test]
    fn a_changed_child_refreshes_its_parent_once_and_an_unchanged_tree_calls_nothing() {
        let dir = tree();
        let model = Fake::default();
        let mut index = Index::default();
        let first = refresh(&mut index, dir.path(), &model, 1, 100);
        // a.md, b.txt, sub/c.md, sub, the root; nothing hidden, skipped or not text.
        assert_eq!(first.calls, 5);
        assert_eq!(
            calls(&model),
            ["a.md", "b.txt", "sub/c.md", "sub", "its top folder"]
        );
        let built = index.clone();
        assert!(!built.roots["project:p"][""].overview.is_empty());
        assert!(built.roots["project:p"]["a.md"].overview.is_empty());

        let again = refresh(&mut index, dir.path(), &model, 1, 100);
        assert_eq!((again.calls, again.reused), (0, 5));
        assert!(calls(&model).is_empty());
        assert_eq!(index, built);

        std::fs::write(dir.path().join("a.md"), "alpha notes, revised").unwrap();
        let changed = refresh(&mut index, dir.path(), &model, 1, 100);
        assert_eq!(calls(&model), ["a.md", "its top folder"]);
        assert_eq!((changed.calls, changed.reused), (2, 3));

        // Deeper: the file, its folder and the root, once each.
        std::fs::write(dir.path().join("sub/c.md"), "charlie plan v2").unwrap();
        refresh(&mut index, dir.path(), &model, 1, 100);
        assert_eq!(calls(&model), ["sub/c.md", "sub", "its top folder"]);
    }

    #[test]
    fn a_parent_stays_when_its_childs_abstract_did_not_change() {
        let dir = tree();
        let model = Fake {
            fixed: Some("b.txt"),
            ..Fake::default()
        };
        let mut index = Index::default();
        refresh(&mut index, dir.path(), &model, 1, 100);
        calls(&model);
        std::fs::write(dir.path().join("b.txt"), "bravo text, reworded").unwrap();
        refresh(&mut index, dir.path(), &model, 1, 100);
        assert_eq!(calls(&model), ["b.txt"]);
    }

    #[test]
    fn a_new_prompt_version_rewrites_every_entry() {
        let dir = tree();
        let model = Fake::default();
        let mut index = Index::default();
        refresh(&mut index, dir.path(), &model, 1, 100);
        calls(&model);
        let bumped = refresh(&mut index, dir.path(), &model, 2, 100);
        assert_eq!((bumped.calls, bumped.reused), (5, 0));
    }

    #[test]
    fn the_cap_stops_a_refresh_and_the_next_goes_on_from_there() {
        let dir = tree();
        let model = Fake::default();
        let mut index = Index::default();
        let part = refresh(&mut index, dir.path(), &model, 1, 2);
        assert!(part.stopped_at_cap);
        assert_eq!(index.roots["project:p"].len(), 2);
        calls(&model);
        let rest = refresh(&mut index, dir.path(), &model, 1, 100);
        assert_eq!((rest.calls, rest.reused), (3, 2));
        assert!(!rest.stopped_at_cap);
        // A removed file drops its entry on a full refresh.
        std::fs::remove_file(dir.path().join("b.txt")).unwrap();
        refresh(&mut index, dir.path(), &model, 1, 100);
        assert!(!index.roots["project:p"].contains_key("b.txt"));
    }

    #[test]
    fn a_model_error_stops_the_refresh_and_keeps_what_it_had() {
        struct Failing;
        impl AbstractModel for Failing {
            async fn complete(&self, _prompt: &str) -> Result<String, String> {
                Err("offline".into())
            }
        }
        let dir = tree();
        let model = Fake::default();
        let mut index = Index::default();
        refresh(&mut index, dir.path(), &model, 1, 100);
        let before = index.clone();
        std::fs::write(dir.path().join("a.md"), "alpha notes, revised").unwrap();
        let items = plan(dir.path());
        let mut report = Report::default();
        let done = block_on(refresh_root(
            &mut index,
            "project:p",
            "P",
            &items,
            &Failing,
            1,
            100,
            &mut report,
        ));
        assert!(!done);
        assert_eq!(report.calls, 1);
        assert!(report.error.as_deref().unwrap().contains("offline"));
        assert_eq!(index, before);
    }

    #[test]
    fn the_index_round_trips_and_annotates_only_rows_that_have_one() {
        let dir = tree();
        let model = Fake::default();
        let mut index = Index::default();
        refresh(&mut index, dir.path(), &model, 1, 100);
        let path = tempfile::tempdir().unwrap().path().join("deep/index.json");
        save(&path, &index).unwrap();
        assert_eq!(load(&path), index);
        assert_eq!(load(&path.with_file_name("missing.json")), Index::default());

        let mut answer = serde_json::json!({ "workspaces": [{ "id": "p" }, { "id": "q" }] });
        annotate_with(&index, &mut answer);
        let line = &index.roots["project:p"][""].line;
        assert_eq!(answer["workspaces"][0]["abstract"], serde_json::json!(line));
        assert!(answer["workspaces"][1].get("abstract").is_none());

        let all = read_index(&index, None);
        assert_eq!(all["roots"][0]["root"], "project:p");
        let one = read_index(&index, Some("p"));
        // The root and `sub`: folders only, never a file's line.
        let paths: Vec<&str> = one["folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["", "sub"]);
    }

    #[test]
    fn a_secret_named_file_or_folder_is_never_read_or_sent() {
        let dir = tree();
        std::fs::write(dir.path().join("credentials.md"), "user and password").unwrap();
        std::fs::create_dir_all(dir.path().join("secrets")).unwrap();
        std::fs::write(dir.path().join("secrets/notes.md"), "hidden").unwrap();
        // A key pasted into an ordinary note reaches the model redacted.
        let key_text = format!("token sk-ant-api03-{}", "a".repeat(40));
        std::fs::write(dir.path().join("a.md"), &key_text).unwrap();
        struct Seen(Mutex<Vec<String>>);
        impl AbstractModel for Seen {
            async fn complete(&self, prompt: &str) -> Result<String, String> {
                self.0.lock().unwrap().push(prompt.to_string());
                Ok("line".into())
            }
        }
        let model = Seen(Mutex::new(Vec::new()));
        let mut index = Index::default();
        let items = plan(dir.path());
        let mut report = Report::default();
        block_on(refresh_root(
            &mut index,
            "project:p",
            "P",
            &items,
            &model,
            1,
            100,
            &mut report,
        ));
        let entries = &index.roots["project:p"];
        assert!(!entries.contains_key("credentials.md"));
        assert!(!entries.keys().any(|k| k.starts_with("secrets")));
        assert_eq!(report.calls, 5);
        let prompts = model.0.lock().unwrap();
        assert!(prompts
            .iter()
            .all(|p| !p.contains("credentials") && !p.contains("secrets")));
        assert!(
            prompts.iter().all(|p| !p.contains(&"a".repeat(40))),
            "{prompts:?}"
        );
    }

    #[test]
    fn list_workspaces_is_unchanged_while_off() {
        let vault = tempfile::tempdir().unwrap();
        let mut index = Index::default();
        let dir = tree();
        refresh(&mut index, dir.path(), &Fake::default(), 1, 100);
        save(&index_path(vault.path()).unwrap(), &index).unwrap();
        let answer = serde_json::json!({ "total": 1, "workspaces": [{ "id": "p", "title": "P" }] });
        let off = annotate(vault.path(), answer.clone(), false);
        assert_eq!(
            serde_json::to_string(&off).unwrap(),
            serde_json::to_string(&answer).unwrap()
        );
        let on = annotate(vault.path(), answer, true);
        assert!(on["workspaces"][0]["abstract"].is_string());
    }

    #[test]
    fn the_setting_is_off_by_default_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        assert!(!enabled_at(&path));
        std::fs::write(&path, "not json").unwrap();
        assert!(!enabled_at(&path));
        assert!(set_enabled_at(&path, true).unwrap());
        assert!(enabled_at(&path));
        set_enabled_at(&path, false).unwrap();
        assert!(!enabled_at(&path));
    }

    #[test]
    fn parse_takes_the_first_line_and_cuts_both() {
        assert_eq!(parse("  \n"), None);
        let (line, overview) = parse("Abstract: Plans.\n\nOverview: Two parts.\nMore.").unwrap();
        assert_eq!(
            (line.as_str(), overview.as_str()),
            ("Plans.", "Two parts. More.")
        );
        let long = "x".repeat(ABSTRACT_MAX + 10);
        assert_eq!(parse(&long).unwrap().0.chars().count(), ABSTRACT_MAX + 1);
    }
}
