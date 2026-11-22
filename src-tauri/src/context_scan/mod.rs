//! Context scanners (CX19): find the instruction and memory files a person
//! already has (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, Cursor, Windsurf, Cline
//! and Copilot rules, saved commands, skills, memory folders) and propose
//! them for the review card (CX20). Read only.
//!
//! **Rules.**
//! - The roots are parameters. Nothing here knows `$HOME`; [`default_roots`]
//!   takes the home folder as an argument, and the command layer
//!   (`commands::context_scan_api`) is the only caller that resolves it. A
//!   test hands in a temp dir.
//! - One walker, one staging area: the walk is H5a's
//!   (`session_import::walker`, which refuses links and a link root and keeps
//!   to its budget) and a scan stages its manifest under
//!   `<app support>/Imports/<id>/` like an import. Nothing is written to the
//!   vault or to a scanned folder; no file is opened for writing.
//! - Budgets: files kept, bytes read in all, time, and a per-file cap
//!   ([`FILE_MAX`]); a cap that stops a walk is named in the manifest.
//! - Secrets: every file is checked with the shared rules (`scan_rules`).
//!   A finding names its rule and line, never the matched text, and a file
//!   with a secret-like or blocking finding (or one near the start of the
//!   excerpt) gets no excerpt at all. File contents are never copied: a
//!   proposal carries the path, kind, size, SHA-256 and at most
//!   [`EXCERPT_CHARS`] characters of a clean file.
//!
//! A proposal is only a proposal: accepting one (and where it lands) is the
//! person's click on CX20's card.

mod classify;
#[cfg(test)]
mod tests;

pub(crate) use classify::{Agent, FileKind};

use crate::scan_rules::{self, ScanOptions, Severity};
use crate::session_import::staging::Staging;
use crate::session_import::walker::{self, Budget, Stop};
use classify::{classify, skip_dir};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The staged document's name.
pub(crate) const MANIFEST: &str = "context-scan.json";
/// A file larger than this is listed (path, size) but not read.
pub(crate) const FILE_MAX: u64 = 256 * 1024;
/// The most characters of a clean file a proposal shows.
pub(crate) const EXCERPT_CHARS: usize = 240;
/// A finding this close to the start of a file withholds the excerpt.
const EXCERPT_GUARD_BYTES: usize = 600;
/// All the bytes one scan reads.
pub(crate) const READ_MAX: u64 = 32 * 1024 * 1024;
/// The cancel message.
pub(crate) const CANCELLED: &str = "The scan was cancelled.";

/// Whose files a root holds: the person's (home folders) or a project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Scope {
    User,
    Project,
}

impl Scope {
    /// The `scan_rules` layer whose severities apply.
    fn layer(self) -> &'static str {
        match self {
            Scope::User => "home",
            Scope::Project => "project",
        }
    }
}

/// One folder to scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanRoot {
    pub path: PathBuf,
    pub scope: Scope,
    /// Shown with each proposal ("Claude Code", the project's name).
    pub label: String,
    /// The tool this folder belongs to, for folders like `~/.cursor`.
    #[serde(default)]
    pub agent: Option<Agent>,
}

/// The agents' own folders under `home`, as user scope roots. Pure: the
/// caller resolves `home` and `codex_home`.
pub(crate) fn default_roots(home: &Path, codex_home: Option<&Path>) -> Vec<ScanRoot> {
    let r = |dir: PathBuf, label: &str, agent: Agent| ScanRoot {
        path: dir,
        scope: Scope::User,
        label: label.to_string(),
        agent: Some(agent),
    };
    vec![
        r(home.join(".claude"), "Claude Code", Agent::ClaudeCode),
        r(
            codex_home
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".codex")),
            "Codex",
            Agent::Codex,
        ),
        r(home.join(".cursor"), "Cursor", Agent::Cursor),
        r(home.join(".gemini"), "Gemini", Agent::Gemini),
        r(
            home.join(".codeium").join("windsurf"),
            "Windsurf",
            Agent::Windsurf,
        ),
    ]
}

/// A rule hit on a file: the rule, its group, how bad, and the line. Never
/// the matched text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Flag {
    pub rule: String,
    pub group: String,
    /// `warn` or `block`.
    pub severity: String,
    pub line: usize,
}

/// One instruction or memory file found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Proposal {
    /// Stable for the same root, path and content: `ctx-<12 hex>`.
    pub id: String,
    pub root_label: String,
    pub scope: Scope,
    pub agent: Agent,
    pub kind: FileKind,
    /// The absolute path (for the later read after a click).
    pub path: String,
    /// The path below the root, `/` separated.
    pub rel_path: String,
    pub size: u64,
    /// Modified time, seconds since the epoch.
    pub modified: Option<i64>,
    /// SHA-256 of the file; `None` when it was not read.
    pub hash: Option<String>,
    /// The first [`EXCERPT_CHARS`] of a clean file; `None` when withheld or unread.
    pub excerpt: Option<String>,
    /// Why there is no excerpt: `secret`, `finding`, `too-large`, `read-budget`, `unreadable`, `empty`.
    pub withheld: Option<String>,
    /// Looks like it holds a secret: never copy it as is.
    pub secret_like: bool,
    pub flags: Vec<Flag>,
    /// The id of an earlier proposal with the same hash.
    pub duplicate_of: Option<String>,
    /// For a Claude Code auto-memory entry (`projects/<slug>/memory/*.md`),
    /// the layer `memory_sort` proposes and why (PX6). Absent in a manifest
    /// staged before PX6 and for every other file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_sort: Option<crate::memory_sort::MemorySort>,
}

/// One root's outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootReport {
    pub label: String,
    pub path: String,
    pub scope: Scope,
    pub found: usize,
    pub visited: usize,
    pub symlinks_skipped: usize,
    pub unreadable: usize,
    /// A sentence when a budget stopped this root's walk or the root was refused.
    pub note: Option<String>,
}

/// A scan's result, staged under `<id>/context-scan.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanManifest {
    pub scan_id: String,
    pub state: String,
    pub roots: Vec<RootReport>,
    pub proposals: Vec<Proposal>,
    /// Files with a secret-like finding.
    pub secret_files: usize,
    pub bytes_read: u64,
}

/// A scan's progress.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanStatus {
    pub scan_id: String,
    /// `scanning`, `ready`, `cancelled` or `failed`.
    pub state: String,
    pub found: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The caps on one scan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanBudget {
    pub walk: Budget,
    pub read_bytes: u64,
}

impl Default for ScanBudget {
    fn default() -> Self {
        ScanBudget {
            walk: Budget {
                max_visited: 300_000,
                max_files: 2_000,
                max_bytes: 128 * 1024 * 1024,
                max_depth: 10,
                max_time: Duration::from_secs(30),
            },
            read_bytes: READ_MAX,
        }
    }
}

struct Job {
    status: ScanStatus,
    cancel: Arc<AtomicBool>,
}

static JOBS: Lazy<Mutex<HashMap<String, Job>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Job>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Register scan `id` and hand back its cancel flag.
pub(crate) fn begin_job(id: &str) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    jobs().insert(
        id.to_string(),
        Job {
            status: ScanStatus {
                scan_id: id.to_string(),
                state: "scanning".into(),
                found: 0,
                error: None,
            },
            cancel: cancel.clone(),
        },
    );
    cancel
}

/// Record how scan `id` ended.
pub(crate) fn end_job(id: &str, result: &Result<ScanManifest, String>) {
    if let Some(job) = jobs().get_mut(id) {
        match result {
            Ok(m) => {
                job.status.state = "ready".into();
                job.status.found = m.proposals.len();
            }
            Err(e) if e == CANCELLED => job.status.state = "cancelled".into(),
            Err(e) => {
                job.status.state = "failed".into();
                job.status.error = Some(e.clone());
            }
        }
    }
}

/// A scan's progress: this process's job, else the staged manifest's.
pub(crate) fn status(staging: &Staging, id: &str) -> Result<Option<ScanStatus>, String> {
    if let Some(job) = jobs().get(id) {
        return Ok(Some(job.status.clone()));
    }
    Ok(staging
        .read_json::<ScanManifest>(id, MANIFEST)?
        .map(|m| ScanStatus {
            scan_id: m.scan_id,
            state: m.state,
            found: m.proposals.len(),
            error: None,
        }))
}

/// The staged proposals of scan `id`.
pub(crate) fn manifest(staging: &Staging, id: &str) -> Result<ScanManifest, String> {
    staging
        .read_json::<ScanManifest>(id, MANIFEST)?
        .ok_or_else(|| "That scan is no longer here; run it again.".to_string())
}

/// Stop a running scan, or delete a staged one. `false` when there was nothing.
pub(crate) fn cancel(staging: &Staging, id: &str) -> Result<bool, String> {
    let dir = staging.dir(id)?;
    let running = {
        let mut jobs = jobs();
        match jobs.get_mut(id) {
            Some(job) if job.status.state == "scanning" => {
                job.cancel.store(true, Ordering::Relaxed);
                job.status.state = "cancelled".into();
                true
            }
            Some(job) => {
                job.status.state = "cancelled".into();
                false
            }
            None => false,
        }
    };
    let staged = dir.exists();
    staging.discard(id)?;
    Ok(running || staged)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn short_id(root: &str, rel: &str, hash: Option<&str>) -> String {
    let mut h = Sha256::new();
    h.update(root.as_bytes());
    h.update([0]);
    h.update(rel.as_bytes());
    h.update([0]);
    h.update(hash.unwrap_or("").as_bytes());
    format!("ctx-{}", &hex(&h.finalize())[..12])
}

fn excerpt_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(EXCERPT_CHARS).collect()
}

fn flags_of(text: &str, scope: Scope, rel: &str, bytes: &[u8]) -> (Vec<Flag>, usize) {
    // Path and content rules for the scope's layer, plus the secret rules
    // under the strictest layer, so a secret is flagged in a home file too.
    // A scanned file is someone else's text: it may waive nothing.
    let mut found = scan_rules::scan_bytes(
        bytes,
        rel,
        ScanOptions {
            honor_allows: false,
            ..ScanOptions::layer(scope.layer())
        },
    );
    let secrets = ["secret"];
    let strict = ScanOptions {
        layer: "project",
        groups: Some(&secrets),
        honor_allows: false,
    };
    found.extend(scan_rules::scan_text(text, strict));
    let mut seen = HashSet::new();
    let mut first_offset = usize::MAX;
    let mut flags = Vec::new();
    for f in found {
        if !seen.insert((f.rule, f.line)) {
            continue;
        }
        first_offset = first_offset.min(f.offset);
        flags.push(Flag {
            rule: f.rule.to_string(),
            group: f.group.to_string(),
            severity: match f.severity {
                Severity::Block => "block".into(),
                Severity::Warn => "warn".into(),
            },
            line: f.line,
        });
    }
    flags.sort_by(|a, b| a.line.cmp(&b.line).then(a.rule.cmp(&b.rule)));
    (flags, first_offset)
}

/// Check picked roots against the vault: no `..` component, each root
/// canonicalized (so an intermediate link is resolved), and none inside the
/// vault or an ancestor of it (a walk from `~` would otherwise reach the
/// vault). Returns the canonical roots and the canonical vault to avoid.
pub(crate) fn resolve_roots(
    roots: Vec<ScanRoot>,
    vault: &Path,
) -> Result<(Vec<ScanRoot>, Vec<PathBuf>), String> {
    let vault_real = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let mut out = Vec::new();
    for mut root in roots {
        if root
            .path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Pick a folder by its full path, without `..`.".into());
        }
        // A missing folder stays as it is: the scan reports it as a note.
        if let Ok(real) = std::fs::canonicalize(&root.path) {
            // A root that is itself a link is the walker's to refuse.
            let is_link = std::fs::symlink_metadata(&root.path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if real.starts_with(&vault_real) || vault_real.starts_with(&real) {
                return Err(
                    "The vault is Hollow's own folder; scan folders that don't contain it and aren't inside it."
                        .into(),
                );
            }
            if !is_link {
                root.path = real;
            }
        }
        out.push(root);
    }
    Ok((out, vec![vault_real, vault.to_path_buf()]))
}

/// Scan `roots` into staged scan `id`. Read only; `cancel` stops the pass
/// between files and discards what it staged.
pub(crate) fn scan(
    staging: &Staging,
    id: &str,
    roots: &[ScanRoot],
    avoid: &[PathBuf],
    budget: &ScanBudget,
    cancel: &AtomicBool,
) -> Result<ScanManifest, String> {
    crate::session_import::staging::sweep(staging, std::time::SystemTime::now());
    let result = scan_inner(staging, id, roots, avoid, budget, cancel);
    if result.is_err() {
        let _ = staging.discard(id);
    }
    result
}

fn scan_inner(
    staging: &Staging,
    id: &str,
    roots: &[ScanRoot],
    avoid: &[PathBuf],
    budget: &ScanBudget,
    cancel: &AtomicBool,
) -> Result<ScanManifest, String> {
    if roots.is_empty() {
        return Err("There is no folder to scan.".into());
    }
    let started = Instant::now();
    let mut manifest = ScanManifest {
        scan_id: id.to_string(),
        state: "ready".into(),
        roots: Vec::new(),
        proposals: Vec::new(),
        secret_files: 0,
        bytes_read: 0,
    };
    let mut by_hash: HashMap<String, String> = HashMap::new();
    // The date an auto-memory entry's age is measured against (PX6).
    let today = chrono::Local::now().date_naive();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        let mut report = RootReport {
            label: root.label.clone(),
            path: root.path.to_string_lossy().into_owned(),
            scope: root.scope,
            found: 0,
            visited: 0,
            symlinks_skipped: 0,
            unreadable: 0,
            note: None,
        };
        let mut walk_budget = budget.walk;
        walk_budget.max_time = budget
            .walk
            .max_time
            .saturating_sub(started.elapsed())
            .max(Duration::from_millis(1));
        let hint = root.agent;
        let walked = walker::walk_pruned(
            &root.path,
            &walk_budget,
            cancel,
            |e| classify(&e.rel, hint).is_some() && !avoid.iter().any(|a| e.path.starts_with(a)),
            skip_dir,
        );
        let walk = match walked {
            Ok(w) => w,
            Err(e) => {
                // A missing, link or non-folder root is a note, not a failure.
                report.note = Some(e);
                manifest.roots.push(report);
                continue;
            }
        };
        if walk.stopped == Some(Stop::Cancelled) || cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        report.visited = walk.visited;
        report.symlinks_skipped = walk.symlinks;
        report.unreadable = walk.unreadable;
        report.note = walk.stopped.map(|s| s.sentence(&walk_budget));
        for entry in walk.entries {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            // The same file reached through two roots is one proposal.
            if !seen_paths.insert(entry.path.clone()) {
                continue;
            }
            let Some((agent, kind)) = classify(&entry.rel, hint) else {
                continue;
            };
            let rel = entry.rel_path();
            let modified = entry.modified.and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs() as i64)
            });
            let mut p = Proposal {
                id: String::new(),
                root_label: root.label.clone(),
                scope: root.scope,
                agent,
                kind,
                path: entry.path.to_string_lossy().into_owned(),
                rel_path: rel.clone(),
                size: entry.size,
                modified,
                hash: None,
                excerpt: None,
                withheld: None,
                secret_like: false,
                flags: Vec::new(),
                duplicate_of: None,
                memory_sort: None,
            };
            // PX6: Claude Code's auto-memory entries get a proposed layer
            // from the text read below (pure; nothing more is read).
            let auto_memory = agent == Agent::ClaudeCode
                && root.scope == Scope::User
                && kind == FileKind::Memory
                && crate::memory_sort::auto_memory_slug(&rel).is_some();
            let mut read_text: Option<String> = None;
            if entry.size > FILE_MAX {
                p.withheld = Some("too-large".into());
            } else if manifest.bytes_read.saturating_add(entry.size) > budget.read_bytes {
                p.withheld = Some("read-budget".into());
            } else {
                match std::fs::read(&entry.path) {
                    Err(_) => p.withheld = Some("unreadable".into()),
                    Ok(bytes) => {
                        manifest.bytes_read += bytes.len() as u64;
                        p.hash = Some(hex(&Sha256::digest(&bytes)));
                        let mut clean: Vec<u8> = bytes.clone();
                        clean.retain(|&b| b != 0);
                        let text = String::from_utf8_lossy(&clean).into_owned();
                        let (flags, first_offset) = flags_of(&text, root.scope, &rel, &bytes);
                        p.secret_like = flags.iter().any(|f| f.group == "secret");
                        let blocking = flags.iter().any(|f| f.severity == "block");
                        if p.secret_like {
                            p.withheld = Some("secret".into());
                        } else if blocking || first_offset < EXCERPT_GUARD_BYTES {
                            p.withheld = Some("finding".into());
                        } else if text.trim().is_empty() {
                            p.withheld = Some("empty".into());
                        } else {
                            p.excerpt = Some(excerpt_of(&text));
                        }
                        p.flags = flags;
                        if auto_memory {
                            read_text = Some(text);
                        }
                    }
                }
            }
            if auto_memory {
                p.memory_sort = Some(crate::memory_sort::classify(
                    &rel,
                    read_text.as_deref(),
                    modified,
                    today,
                ));
            }
            p.id = short_id(&report.path, &rel, p.hash.as_deref());
            if let Some(hash) = &p.hash {
                match by_hash.get(hash) {
                    Some(first) => p.duplicate_of = Some(first.clone()),
                    None => {
                        by_hash.insert(hash.clone(), p.id.clone());
                    }
                }
            }
            if p.secret_like {
                manifest.secret_files += 1;
            }
            report.found += 1;
            manifest.proposals.push(p);
        }
        manifest.roots.push(report);
    }
    manifest.proposals.sort_by(|a, b| {
        (a.scope as u8, a.root_label.as_str(), a.rel_path.as_str()).cmp(&(
            b.scope as u8,
            b.root_label.as_str(),
            b.rel_path.as_str(),
        ))
    });
    staging.write_json(id, MANIFEST, &manifest)?;
    Ok(manifest)
}
