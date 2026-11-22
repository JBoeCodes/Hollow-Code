//! Session import, core (H5a, design `sources-and-import-2026-11-13` section
//! 4): Claude Code and Codex conversations from their folders into Hollow's
//! session store, read-only on the source side.
//!
//! **Flow.** [`inspect`] walks the folder the person chose ([`walker`]; the
//! root is a parameter, the default folders are resolved only by the command
//! layer, [`default_root`]), reads every transcript streamed line by line
//! ([`claude_code`], [`codex`]) and stages a manifest outside the vault
//! ([`staging`]): per conversation its title, dates, message count, size,
//! project folder, destination workspace and duplicate status. Nothing
//! enters the session database until [`commit`], which reads each picked
//! file again and writes it in ONE transaction per conversation, so a crash
//! between conversations leaves no half row. [`cancel`] discards a staged
//! or still-running import. Staging is deleted after a clean commit, on
//! cancel, or after `staging::KEEP_DAYS`.
//!
//! **What lands.** A `sessions` row (source title or first prompt cut to 80
//! characters, source dates, agent `claude` / `codex`, `adopted = 1`, cwd the
//! destination workspace's folder), one closed stint with no native id,
//! `turns` and `transcripts` in the normalized payload Hollow renders (tool
//! calls keep their name and 2 KB excerpts labelled `model::NOT_RERUN`;
//! attachments listed by name only, N28), and an `imports` row ([`SCHEMA`]).
//! Destination (N29): a session whose cwd is inside a connected project's
//! folder lands in that project's workspace when it has one, else Quick
//! chats; the commit takes a batch override.
//!
//! **Dedupe.** Same source id and content hash: "already imported" (a re-run
//! is a no-op). Same id, content that extends the old (its earlier turns
//! unchanged, the last one grown or more turns after it): the record is
//! updated in place, only its changed source turns rewritten, and its
//! provenance says "Updated from Claude Code <time>". Content that doesn't
//! extend: a new conversation marked "Version N", never merged.
//!
//! **Exclusions.** Hollow's own Claude and Codex sessions sit in the same
//! folders (`claude_accounts.rs`): any id in `stints.harness_session_id` is
//! skipped as Hollow's own.
//!
//! **Imported records are read-only.** `session_store::start_turn` refuses a
//! turn in one; [`continue_imported`] (N26) makes a new conversation linked
//! to it (`parent_key`, `fork_kind = 'import'`) whose fresh native session
//! gets the imported turns through the `<conversation-context>` handoff
//! (`context_compiler::compile_turn_within`), as quoted history, never rerun,
//! with no permissions carried over.
//!
//! **Agent search (N25).** Imported conversations are searchable by agents
//! like any chat, each result carrying its `imported` provenance; a
//! per-import "Keep out of agent search" flag ([`set_keep_out_of_search`])
//! hides a batch from the agent's search ([`hidden_from_agents`]).

pub(crate) mod archive;
pub(crate) mod chatgpt;
pub(crate) mod claude_code;
pub(crate) mod claudeai;
pub(crate) mod codex;
pub(crate) mod gemini;
mod lines;
pub(crate) mod model;
pub(crate) mod native_resume;
pub(crate) mod pi;
pub(crate) mod staging;
pub(crate) mod walker;

use crate::session_store::{db_time, with_read, with_write, ConversationTool};
use crate::types::ImportedProvenance;
use chrono::{DateTime, Utc};
use model::{Parsed, NOT_RERUN};
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use staging::Staging;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Bumped when a reader's output changes for the same file.
pub(crate) const PARSER_VERSION: i64 = 1;
/// One batch holds at most this many conversations.
pub(crate) const BATCH_MAX: usize = 5_000;
/// What a folder with transcripts in no layout a reader knows says.
pub(crate) const UNSUPPORTED: &str = "This export's format isn't supported yet.";
/// `sessions.fork_kind` of a conversation continued from an imported one.
pub(crate) const FORK_KIND: &str = "import";
/// The staged manifest's file name.
const MANIFEST: &str = "manifest.json";
/// A preview shows at most this many turns, each text cut to
/// [`PREVIEW_CHARS`].
const PREVIEW_TURNS: usize = 10;
const PREVIEW_CHARS: usize = 2_000;
/// The partial manifest is written every this many conversations.
const PARTIAL_EVERY: usize = 200;

/// The `imports` table (applied with the session schema). One row per
/// imported conversation version: `external_id` is `<source>:<id>` for the
/// first and `<source>:<id>#v<n>` for a later version, `base_id` the first
/// form. `turn_hashes` (a JSON array) is every source turn's hash, which
/// tells an extended source from a rewritten one. `keep_out_of_search` is
/// N25's switch.
pub(crate) const SCHEMA: &str = "\
 CREATE TABLE IF NOT EXISTS imports( \
   id INTEGER PRIMARY KEY AUTOINCREMENT, \
   import_id TEXT NOT NULL, \
   source TEXT NOT NULL, \
   root TEXT NOT NULL, \
   external_id TEXT NOT NULL, \
   base_id TEXT NOT NULL, \
   version INTEGER NOT NULL DEFAULT 1, \
   content_hash TEXT NOT NULL, \
   turn_hashes TEXT NOT NULL DEFAULT '[]', \
   shortened INTEGER NOT NULL DEFAULT 0, \
   parser_version INTEGER NOT NULL, \
   imported_at TEXT NOT NULL, \
   updated_at TEXT, \
   session_key TEXT NOT NULL, \
   source_cwd TEXT, \
   source_created_at TEXT, \
   source_last_at TEXT, \
   keep_out_of_search INTEGER NOT NULL DEFAULT 0, \
   UNIQUE(source, external_id) \
 ); \
 CREATE INDEX IF NOT EXISTS idx_imports_session ON imports(session_key); \
 CREATE INDEX IF NOT EXISTS idx_imports_base ON imports(source, base_id, version); \
 CREATE TABLE IF NOT EXISTS import_native( \
   session_key TEXT PRIMARY KEY, \
   source_session_id TEXT NOT NULL, \
   source_cwd TEXT NOT NULL, \
   cli_version TEXT, \
   created_at TEXT NOT NULL \
 );";

// ── sources ───────────────────────────────────────────────────────────

/// Where a conversation comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Source {
    Pi,
    ClaudeCode,
    Codex,
    /// An export file (H5b): a zip or folder the person picks.
    #[serde(rename = "chatgpt")]
    ChatGpt,
    /// Google Takeout "My Activity" for Gemini Apps (H5b).
    Gemini,
    /// claude.ai's export (H5b): a zip or folder the person picks.
    #[serde(rename = "claudeai")]
    ClaudeAi,
}

impl Source {
    pub fn parse(value: &str) -> Option<Source> {
        match value.trim() {
            "pi" => Some(Source::Pi),
            "claude-code" => Some(Source::ClaudeCode),
            "codex" => Some(Source::Codex),
            "chatgpt" => Some(Source::ChatGpt),
            "gemini" => Some(Source::Gemini),
            "claudeai" => Some(Source::ClaudeAi),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Pi => "pi",
            Source::ClaudeCode => "claude-code",
            Source::Codex => "codex",
            Source::ChatGpt => "chatgpt",
            Source::Gemini => "gemini",
            Source::ClaudeAi => "claudeai",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Pi => "Pi",
            Source::ClaudeCode => "Claude Code",
            Source::Codex => "Codex",
            Source::ChatGpt => "ChatGPT",
            Source::Gemini => "Gemini",
            Source::ClaudeAi => "claude.ai",
        }
    }

    /// An export file the person picks (no default folder, nothing to
    /// scan): ChatGPT, claude.ai and Gemini.
    pub fn is_export(self) -> bool {
        matches!(self, Source::ChatGpt | Source::Gemini | Source::ClaudeAi)
    }

    /// The Hollow agent the conversation is recorded under.
    pub fn agent(self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude",
            Source::Codex => "codex",
            // Hollow has no ChatGPT agent (design: `imported`).
            Source::Pi | Source::ChatGpt | Source::Gemini | Source::ClaudeAi => "imported",
        }
    }

    fn is_transcript(self, entry: &walker::Entry) -> bool {
        match self {
            Source::Pi => pi::is_transcript(entry),
            Source::ClaudeCode => claude_code::is_transcript(entry),
            Source::Codex => codex::is_transcript(entry),
            Source::ChatGpt => chatgpt::is_transcript(entry),
            Source::Gemini => gemini::is_transcript(entry),
            Source::ClaudeAi => claudeai::is_transcript(entry),
        }
    }

    fn read(self, path: &Path) -> Result<Parsed, String> {
        match self {
            Source::Pi => pi::read(path, model::CONVERSATION_MAX_BYTES),
            Source::ClaudeCode => claude_code::read(path, model::CONVERSATION_MAX_BYTES),
            Source::Codex => codex::read(path, model::CONVERSATION_MAX_BYTES),
            Source::ChatGpt => chatgpt::read(path, model::CONVERSATION_MAX_BYTES),
            Source::Gemini => gemini::read(path, model::CONVERSATION_MAX_BYTES),
            Source::ClaudeAi => claudeai::read(path, model::CONVERSATION_MAX_BYTES),
        }
    }
}

/// The folder a source's conversations live in by default: `~/.claude/
/// projects`; `$CODEX_HOME/sessions`, else `~/.codex/sessions`; an export
/// source has none (`None`: the person picks the file). Pure: the command
/// layer passes the home folder and `CODEX_HOME`.
pub(crate) fn default_root(
    source: Source,
    home: &Path,
    codex_home: Option<&Path>,
) -> Option<PathBuf> {
    match source {
        Source::Pi => Some(home.join(".pi").join("agent").join("sessions")),
        Source::ClaudeCode => Some(home.join(".claude").join("projects")),
        Source::Codex => Some(
            codex_home
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions"),
        ),
        Source::ChatGpt | Source::Gemini | Source::ClaudeAi => None,
    }
}

// ── manifest ──────────────────────────────────────────────────────────

/// A conversation's duplicate status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Status {
    /// Not imported yet.
    New,
    /// Imported before with the same content: skipped.
    AlreadyImported,
    /// Imported before; the source has grown since: updated in place.
    Updated,
    /// Imported before; the source no longer matches it: a second copy.
    NewVersion,
    /// One of Hollow's own sessions: never imported.
    HollowOwn,
    /// A Codex subagent's own thread: not picked by default.
    Subagent,
    /// Not in a layout the reader knows.
    Unsupported,
}

impl Status {
    /// Picked by default in the list.
    fn default_pick(self) -> bool {
        matches!(self, Status::New | Status::Updated | Status::NewVersion)
    }

    /// Can be committed at all.
    fn importable(self) -> bool {
        matches!(
            self,
            Status::New | Status::Updated | Status::NewVersion | Status::Subagent
        )
    }
}

/// One conversation in a staged import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestEntry {
    /// The file's path below the import's root: the id the list picks by.
    pub id: String,
    /// `<source>:<session id>`.
    pub external_id: String,
    pub title: String,
    pub created_at: Option<String>,
    pub last_at: Option<String>,
    pub message_count: usize,
    pub turn_count: usize,
    /// The file's size in bytes.
    pub size: u64,
    /// The session's working folder (`cwd`) in the source.
    pub project_folder: Option<String>,
    /// The workspace it lands in (N29) unless the commit overrides it.
    pub destination: String,
    pub status: Status,
    /// The version it would become (2 or more) for [`Status::NewVersion`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    /// The conversation it duplicates or updates, when imported before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_key: Option<String>,
    /// Picked by default.
    pub selected: bool,
    pub sidechain_lines: usize,
    pub malformed_lines: usize,
    pub truncated: bool,
    /// Oldest turns the 8 MB cap leaves out.
    pub shortened_turns: usize,
    pub content_hash: String,
    /// ChatGPT: branches of the conversation not imported (the transcript
    /// is the branch ending at `current_node`).
    #[serde(default)]
    pub other_branches: usize,
    /// Gemini: how the conversation was formed ("Grouped by time").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
}

/// Counts by status, for the list's header.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Counts {
    pub new: usize,
    pub already_imported: usize,
    pub updated: usize,
    pub new_version: usize,
    pub hollow_own: usize,
    pub subagent: usize,
    pub unsupported: usize,
}

impl Counts {
    fn add(&mut self, status: Status) {
        match status {
            Status::New => self.new += 1,
            Status::AlreadyImported => self.already_imported += 1,
            Status::Updated => self.updated += 1,
            Status::NewVersion => self.new_version += 1,
            Status::HollowOwn => self.hollow_own += 1,
            Status::Subagent => self.subagent += 1,
            Status::Unsupported => self.unsupported += 1,
        }
    }
}

/// A staged import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub import_id: String,
    pub source: Source,
    pub source_label: String,
    pub root: String,
    pub created_at: String,
    /// `inspecting` while the pass runs, then `ready`; `committed` once a
    /// commit left failures to retry.
    pub state: String,
    pub conversations: Vec<ManifestEntry>,
    pub counts: Counts,
    /// Transcript files the walk found (before the batch cap).
    pub found_files: usize,
    /// More than [`BATCH_MAX`] were found: the newest are listed.
    pub capped: bool,
    /// The walk stopped early, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    pub symlinks_skipped: usize,
    pub unreadable: usize,
    /// [`UNSUPPORTED`] when no transcript was in a known layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<String>,
    /// A sentence for the list ("No conversations in this folder.").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// An export source: the file the person picked and what it held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportInfo>,
}

/// What an export (H5b) held besides its conversations: `root` is then the
/// staged copy of them, and this is the person's own file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportInfo {
    /// The export's file or folder name.
    pub name: String,
    /// Stand-in for the file in `imports.root` (name, size, time).
    pub fingerprint: String,
    pub shards: usize,
    /// Array elements that weren't conversations.
    pub malformed: usize,
    /// Conversation ids that appeared in more than one shard.
    pub duplicates: usize,
    /// Shards that broke part way or couldn't be read.
    pub damaged: Vec<String>,
    /// Attachments and other files, by name only (N28), at most 500 names.
    pub other_files: Vec<String>,
    pub other_file_count: usize,
    pub nested_skipped: usize,
    /// A folder export: links skipped, never followed.
    pub links_skipped: usize,
}

// ── jobs (progress and cancel) ────────────────────────────────────────

/// A running or finished import in this process.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JobStatus {
    pub import_id: String,
    pub source: Source,
    /// `inspecting`, `ready`, `committing`, `committed`, `cancelled` or
    /// `failed`.
    pub state: String,
    /// Files read so far, and of how many.
    pub done: usize,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct Job {
    status: JobStatus,
    cancel: Arc<AtomicBool>,
}

static JOBS: Lazy<Mutex<HashMap<String, Job>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Job>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Register import `id` and hand back its cancel flag.
pub(crate) fn begin_job(id: &str, source: Source) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    jobs().insert(
        id.to_string(),
        Job {
            status: JobStatus {
                import_id: id.to_string(),
                source,
                state: "inspecting".into(),
                done: 0,
                total: 0,
                error: None,
            },
            cancel: cancel.clone(),
        },
    );
    cancel
}

fn set_job(id: &str, f: impl FnOnce(&mut JobStatus)) {
    if let Some(job) = jobs().get_mut(id) {
        f(&mut job.status);
    }
}

/// Record how inspect `id` ended.
pub(crate) fn end_job(id: &str, result: &Result<Manifest, String>) {
    set_job(id, |s| match result {
        Ok(_) => s.state = "ready".into(),
        Err(e) if e == CANCELLED => s.state = "cancelled".into(),
        Err(e) => {
            s.state = "failed".into();
            s.error = Some(e.clone());
        }
    });
}

/// What `cancel` and a cancelled pass answer.
pub(crate) const CANCELLED: &str = "The import was cancelled.";

/// The import's progress: this process's job when there is one, else the
/// staged manifest's state.
pub(crate) fn job_status(staging: &Staging, id: &str) -> Result<Option<JobStatus>, String> {
    if let Some(job) = jobs().get(id) {
        return Ok(Some(job.status.clone()));
    }
    Ok(staging
        .read_json::<Manifest>(id, MANIFEST)?
        .map(|m| JobStatus {
            import_id: m.import_id,
            source: m.source,
            state: m.state,
            done: m.conversations.len(),
            total: m.conversations.len(),
            error: None,
        }))
}

/// Cancel import `id`: a running pass stops at its next file and discards
/// what it staged; a staged one is deleted. `false` when there was nothing.
pub(crate) fn cancel(staging: &Staging, id: &str) -> Result<bool, String> {
    let dir = staging.dir(id)?;
    let running = {
        let mut jobs = jobs();
        match jobs.get_mut(id) {
            Some(job) if job.status.state == "inspecting" => {
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

// ── reads of the store ────────────────────────────────────────────────

/// Every native session id Hollow's own runs recorded (Claude and Codex):
/// what an import never takes.
pub(crate) fn own_ids(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT harness_session_id FROM stints \
             WHERE harness_session_id IS NOT NULL AND harness_session_id != ''",
        )
        .map_err(|e| format!("own sessions: {e}"))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("own sessions: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(ids)
}

/// The newest imported version of a source conversation.
#[derive(Debug, Clone)]
struct Existing {
    row: i64,
    session_key: String,
    version: i64,
    content_hash: String,
    turn_hashes: Vec<String>,
    shortened: i64,
}

fn latest(conn: &Connection, source: Source, base_id: &str) -> Result<Option<Existing>, String> {
    conn.query_row(
        "SELECT id, session_key, version, content_hash, turn_hashes, shortened FROM imports \
         WHERE source = ?1 AND base_id = ?2 ORDER BY version DESC LIMIT 1",
        params![source.as_str(), base_id],
        |row| {
            let hashes: String = row.get(4)?;
            Ok(Existing {
                row: row.get(0)?,
                session_key: row.get(1)?,
                version: row.get(2)?,
                content_hash: row.get(3)?,
                turn_hashes: serde_json::from_str(&hashes).unwrap_or_default(),
                shortened: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("import lookup: {e}"))
}

/// What a commit does with one parsed conversation.
#[derive(Debug, Clone)]
enum Decision {
    New,
    Already(String),
    Update(Existing),
    NewVersion(i64),
}

/// Whether `new` extends `old`: every old turn but the last unchanged, the
/// last one's prompt unchanged (its answer may have grown), and something
/// changed.
fn extends(old: &[String], new: &[String]) -> bool {
    let Some((last, before)) = old.split_last() else {
        return false;
    };
    new.len() >= old.len()
        && new[..before.len()] == *before
        && model::asked_half(&new[before.len()]) == model::asked_half(last)
        && old != new
}

fn decide(parsed: &Parsed, existing: Option<Existing>) -> Decision {
    match existing {
        None => Decision::New,
        Some(e) if e.content_hash == parsed.content_hash() => Decision::Already(e.session_key),
        Some(e) if extends(&e.turn_hashes, &parsed.turn_hashes) => Decision::Update(e),
        Some(e) => Decision::NewVersion(e.version + 1),
    }
}

fn base_id(source: Source, id: &str) -> String {
    format!("{}:{id}", source.as_str())
}

/// The status a parsed file gets against the store.
fn classify(
    conn: &Connection,
    source: Source,
    parsed: &Parsed,
    own: &HashSet<String>,
    stem: Option<&str>,
) -> Result<(Status, Option<Decision>), String> {
    if !parsed.recognized || parsed.id.is_none() {
        return Ok((Status::Unsupported, None));
    }
    let id = parsed.id.as_deref().unwrap_or("");
    if own.contains(id) || stem.is_some_and(|s| own.contains(s)) {
        return Ok((Status::HollowOwn, None));
    }
    let decision = decide(parsed, latest(conn, source, &base_id(source, id))?);
    let status = match (&decision, parsed.subagent) {
        (Decision::Already(_), _) => Status::AlreadyImported,
        (_, true) => Status::Subagent,
        (Decision::New, _) => Status::New,
        (Decision::Update(_), _) => Status::Updated,
        (Decision::NewVersion(_), _) => Status::NewVersion,
    };
    Ok((status, Some(decision)))
}

// ── destination ───────────────────────────────────────────────────────

/// The workspace a session in folder `cwd` lands in (N29): the connected
/// project (`projects`: workspace id and folder) whose folder holds `cwd`,
/// the deepest one, when it has a workspace (`has_workspace`); else Quick
/// chats.
pub(crate) fn destination_for(
    cwd: Option<&str>,
    projects: &[(String, PathBuf)],
    has_workspace: impl Fn(&str) -> bool,
) -> String {
    let quick = crate::workspaces::QUICK_CHATS_ID.to_string();
    let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) else {
        return quick;
    };
    let cwd = Path::new(cwd);
    // A transcript's cwd is untrusted text: a `..` part could name a folder
    // outside the project it starts with, so it lands in Quick chats.
    if cwd
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return quick;
    }
    projects
        .iter()
        .filter(|(_, folder)| !folder.as_os_str().is_empty() && cwd.starts_with(folder))
        .max_by_key(|(_, folder)| folder.components().count())
        .filter(|(id, _)| has_workspace(id))
        .map(|(id, _)| id.clone())
        .unwrap_or(quick)
}

fn workspace_exists(vault: &Path, id: &str) -> bool {
    vault
        .join("workspaces")
        .join(id)
        .join("WORKSPACE.md")
        .is_file()
}

// ── inspect ───────────────────────────────────────────────────────────

/// What inspect and commit work against.
pub(crate) struct Context<'a> {
    pub vault: &'a Path,
    pub staging: &'a Staging,
    /// Connected projects as (workspace id, folder), for N29.
    pub projects: &'a [(String, PathBuf)],
}

/// The connected projects as `destination_for` reads them.
pub(crate) fn connected_projects(vault: &Path) -> Vec<(String, PathBuf)> {
    crate::directory_connections::projects(vault)
        .into_iter()
        .map(|p| (p.id, PathBuf::from(p.path)))
        .collect()
}

/// Read `root` for `source` into staged import `id` (module docs). `cancel`
/// stops the pass between files and discards what it staged.
pub(crate) fn inspect(
    cx: &Context<'_>,
    id: &str,
    source: Source,
    root: &Path,
    budget: &walker::Budget,
    cancel: &AtomicBool,
) -> Result<Manifest, String> {
    staging::sweep(cx.staging, std::time::SystemTime::now());
    let result = if source.is_export() {
        inspect_export(cx, id, source, root, budget, cancel)
    } else {
        inspect_inner(cx, id, source, root, budget, cancel, None)
    };
    if result.is_err() {
        // A cancelled or failed pass leaves nothing staged.
        let _ = cx.staging.discard(id);
    }
    result
}

/// The folder inside a staged import that holds an export's conversations.
const STAGED_CONVERSATIONS: &str = "conversations";

/// An export source (H5b): open the zip or folder safely ([`archive`]),
/// split its conversations into the staging folder, then inspect that folder
/// like any other.
fn inspect_export(
    cx: &Context<'_>,
    id: &str,
    source: Source,
    input: &Path,
    budget: &walker::Budget,
    cancel: &AtomicBool,
) -> Result<Manifest, String> {
    let limits = archive::Limits {
        // Attachments that are archives are never opened; the export still is.
        nested: archive::Nested::Ignore,
        ..archive::Limits::default()
    };
    let mut export = archive::Export::open(input, limits)?;
    let dir = cx.staging.dir(id)?.join(STAGED_CONVERSATIONS);
    let staged = match source {
        Source::ChatGpt => chatgpt::stage(&mut export, &dir, cancel)?,
        Source::Gemini => gemini::stage(&mut export, &dir, cancel)?,
        Source::ClaudeAi => claudeai::stage(&mut export, &dir, cancel)?,
        _ => return Err(format!("{} isn't an export source.", source.label())),
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    let info = ExportInfo {
        name: input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        fingerprint: archive::fingerprint(input),
        shards: staged.shards,
        malformed: staged.malformed,
        duplicates: staged.duplicates,
        damaged: staged.damaged,
        other_files: staged.other_files,
        other_file_count: staged.other_file_count,
        nested_skipped: staged.nested_skipped,
        links_skipped: export.links_skipped,
    };
    let budget = walker::Budget {
        max_files: budget.max_files.max(archive::MAX_STAGED),
        ..*budget
    };
    let mut manifest = inspect_inner(cx, id, source, &dir, &budget, cancel, Some(info))?;
    if staged.capped {
        manifest.capped = true;
        cx.staging.write_json(id, MANIFEST, &manifest)?;
    }
    Ok(manifest)
}

fn inspect_inner(
    cx: &Context<'_>,
    id: &str,
    source: Source,
    root: &Path,
    budget: &walker::Budget,
    cancel: &AtomicBool,
    export: Option<ExportInfo>,
) -> Result<Manifest, String> {
    let walk = walker::walk(root, budget, cancel, |e| source.is_transcript(e))?;
    if walk.stopped == Some(walker::Stop::Cancelled) || cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    let mut entries = walk.entries;
    let found_files = entries.len();
    // Newest first, so a capped batch keeps the recent conversations.
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.rel.cmp(&b.rel)));
    let capped = entries.len() > BATCH_MAX;
    entries.truncate(BATCH_MAX);
    let own = with_read(cx.vault, own_ids)?;
    let mut manifest = Manifest {
        import_id: id.to_string(),
        source,
        source_label: source.label().to_string(),
        root: root.to_string_lossy().into_owned(),
        created_at: db_time(Utc::now()),
        state: "inspecting".into(),
        conversations: Vec::with_capacity(entries.len()),
        counts: Counts::default(),
        found_files,
        capped,
        stopped: walk.stopped.map(|s| s.sentence(budget)),
        symlinks_skipped: walk.symlinks,
        unreadable: walk.unreadable,
        unsupported: None,
        note: None,
        export,
    };
    cx.staging.write_json(id, MANIFEST, &manifest)?;
    set_job(id, |s| s.total = entries.len());
    for (n, entry) in entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let parsed = match source.read(&entry.path) {
            Ok(parsed) => parsed,
            Err(_) => {
                manifest.unreadable += 1;
                continue;
            }
        };
        let stem = entry.path.file_stem().and_then(|s| s.to_str());
        let (status, decision) =
            with_read(cx.vault, |conn| classify(conn, source, &parsed, &own, stem))?;
        manifest.counts.add(status);
        let external = parsed
            .id
            .as_deref()
            .map(|i| base_id(source, i))
            .unwrap_or_default();
        let (version, existing_key) = match &decision {
            Some(Decision::NewVersion(v)) => (Some(*v), None),
            Some(Decision::Already(key)) => (None, Some(key.clone())),
            Some(Decision::Update(e)) => (None, Some(e.session_key.clone())),
            _ => (None, None),
        };
        manifest.conversations.push(ManifestEntry {
            id: entry.rel_path(),
            external_id: external,
            title: parsed.display_title(),
            created_at: parsed.created_at.clone(),
            last_at: parsed.last_at.clone(),
            message_count: parsed.messages,
            turn_count: parsed.turn_hashes.len(),
            size: entry.size,
            project_folder: parsed.cwd.clone(),
            destination: destination_for(parsed.cwd.as_deref(), cx.projects, |w| {
                workspace_exists(cx.vault, w)
            }),
            status,
            version,
            existing_key,
            selected: status.default_pick(),
            sidechain_lines: parsed.sidechain,
            malformed_lines: parsed.malformed,
            truncated: parsed.truncated,
            shortened_turns: parsed.dropped_turns,
            content_hash: parsed.content_hash(),
            other_branches: parsed.other_branches,
            grouping: parsed.grouping.clone(),
        });
        set_job(id, |s| s.done = n + 1);
        if (n + 1) % PARTIAL_EVERY == 0 {
            cx.staging.write_json(id, MANIFEST, &manifest)?;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    if manifest.export.as_ref().is_some_and(|e| e.shards == 0) {
        // An export with no conversations file in a layout the reader knows.
        manifest.unsupported = Some(UNSUPPORTED.into());
    } else if manifest.conversations.is_empty() {
        manifest.note = Some(
            if manifest.export.is_some() {
                "No conversations in this export."
            } else {
                "No conversations in this folder."
            }
            .into(),
        );
    } else if manifest.counts.unsupported == manifest.conversations.len() {
        manifest.unsupported = Some(UNSUPPORTED.into());
    }
    manifest.state = "ready".into();
    cx.staging.write_json(id, MANIFEST, &manifest)?;
    Ok(manifest)
}

/// Staged import `id`'s manifest.
pub(crate) fn manifest(staging: &Staging, id: &str) -> Result<Manifest, String> {
    staging
        .read_json::<Manifest>(id, MANIFEST)?
        .ok_or_else(|| "That import isn't staged any more; inspect the folder again.".to_string())
}

/// The file behind manifest entry `entry`: its path under the root,
/// refused when a component is a link (the walk never followed one, and a
/// commit doesn't either).
fn entry_path(root: &Path, entry: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in entry.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("That conversation's path isn't under the import's folder.".into());
        }
        path.push(part);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|_| "That conversation's file is gone.".to_string())?;
        if meta.file_type().is_symlink() {
            return Err("That conversation's file is now a link; it was skipped.".into());
        }
    }
    Ok(path)
}

// ── preview ───────────────────────────────────────────────────────────

/// One turn of a preview.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewTurn {
    pub at: Option<String>,
    pub asked: String,
    pub reply: String,
    pub tools: Vec<String>,
}

/// A bounded preview of one staged conversation.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub id: String,
    pub title: String,
    pub turns: Vec<PreviewTurn>,
    /// Turns in the conversation (the preview shows the first and last).
    pub total_turns: usize,
    pub sidechain_lines: usize,
    /// ChatGPT: other branches of the conversation, not imported.
    pub other_branches: usize,
    /// Gemini: how the conversation was formed ("Grouped by time").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<String>,
}

fn cut(text: &str) -> String {
    if text.chars().count() <= PREVIEW_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(PREVIEW_CHARS).collect();
    format!("{kept}...")
}

/// The first and last turns of staged conversation `entry` (at most
/// [`PREVIEW_TURNS`], each text cut to 2,000 characters). Reads only.
pub(crate) fn preview(staging: &Staging, id: &str, entry: &str) -> Result<Preview, String> {
    let m = manifest(staging, id)?;
    let listed = m
        .conversations
        .iter()
        .find(|c| c.id == entry)
        .ok_or_else(|| "That conversation isn't in this import.".to_string())?;
    let parsed = m.source.read(&entry_path(Path::new(&m.root), entry)?)?;
    let total = parsed.turns.len();
    let head = PREVIEW_TURNS / 2;
    let picked: Vec<&model::ParsedTurn> = if total <= PREVIEW_TURNS {
        parsed.turns.iter().collect()
    } else {
        parsed.turns[..head]
            .iter()
            .chain(parsed.turns[total - (PREVIEW_TURNS - head)..].iter())
            .collect()
    };
    Ok(Preview {
        id: entry.to_string(),
        title: listed.title.clone(),
        turns: picked
            .into_iter()
            .map(|t| PreviewTurn {
                at: t.at.clone(),
                asked: cut(&t.asked_text()),
                reply: cut(&t.reply),
                tools: t.tools.iter().map(|tool| tool.name.clone()).collect(),
            })
            .collect(),
        total_turns: total + parsed.dropped_turns,
        sidechain_lines: parsed.sidechain,
        other_branches: parsed.other_branches,
        grouping: parsed.grouping.clone(),
    })
}

// ── commit ────────────────────────────────────────────────────────────

/// What the person picked.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CommitRequest {
    /// Manifest entry ids; `None` takes the default selection.
    pub ids: Option<Vec<String>>,
    /// A workspace for the whole batch instead of each one's own (N29).
    pub destination: Option<String>,
    /// N25: keep this batch out of agent search.
    pub keep_out_of_search: bool,
}

/// One conversation a commit could not write.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Failed {
    pub id: String,
    pub title: String,
    pub error: String,
}

/// A commit's result line.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitResult {
    pub import_id: String,
    pub imported: usize,
    pub updated: usize,
    pub new_versions: usize,
    pub already_imported: usize,
    /// Hollow's own, unsupported, or no longer there.
    pub skipped: usize,
    pub failed: Vec<Failed>,
    /// The conversations written (new, updated and new versions).
    pub session_keys: Vec<String>,
    /// Staging was deleted (nothing failed).
    pub staging_deleted: bool,
}

/// Commit staged import `id` (module docs).
pub(crate) fn commit(
    cx: &Context<'_>,
    id: &str,
    request: &CommitRequest,
    now: DateTime<Utc>,
) -> Result<CommitResult, String> {
    commit_with(cx, id, request, now, &|_, _| Ok(()))
}

/// A test's hook inside each conversation's transaction: `(stage, entry)`
/// where stage is `session` (row written) or `turns` (turns written); an
/// error there rolls that conversation back, as a crash would.
type Hook<'a> = &'a dyn Fn(&str, &str) -> Result<(), String>;

pub(crate) fn commit_with(
    cx: &Context<'_>,
    id: &str,
    request: &CommitRequest,
    now: DateTime<Utc>,
    hook: Hook<'_>,
) -> Result<CommitResult, String> {
    let mut m = manifest(cx.staging, id)?;
    if m.state == "inspecting" {
        return Err("That import is still being read; wait for its list.".into());
    }
    let picked: Vec<usize> = match &request.ids {
        Some(ids) => {
            let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
            (0..m.conversations.len())
                .filter(|&i| wanted.contains(m.conversations[i].id.as_str()))
                .collect()
        }
        None => (0..m.conversations.len())
            .filter(|&i| m.conversations[i].selected)
            .collect(),
    };
    if picked.len() > BATCH_MAX {
        return Err(format!(
            "Pick at most {BATCH_MAX} conversations at once; narrow by date or project."
        ));
    }
    let destination = match request.destination.as_deref().map(str::trim) {
        Some(w) if !w.is_empty() => {
            if crate::workspaces::is_main_agent(w) {
                return Err("Imported conversations can't go into the COO's workspace.".into());
            }
            crate::workspaces::session_dir_for(cx.vault, w)?;
            Some(w.to_string())
        }
        _ => None,
    };
    set_job(id, |s| {
        s.state = "committing".into();
        s.done = 0;
        s.total = picked.len();
    });
    let own = with_read(cx.vault, own_ids)?;
    let root = PathBuf::from(&m.root);
    let mut folders: HashMap<String, String> = HashMap::new();
    let mut out = CommitResult {
        import_id: id.to_string(),
        ..CommitResult::default()
    };
    for (n, &index) in picked.iter().enumerate() {
        let entry = m.conversations[index].clone();
        set_job(id, |s| s.done = n);
        if !entry.status.importable() {
            if entry.status == Status::AlreadyImported {
                out.already_imported += 1;
            } else {
                out.skipped += 1;
            }
            continue;
        }
        let workspace = destination
            .clone()
            .unwrap_or_else(|| entry.destination.clone());
        let written = (|| -> Result<(Status, Option<String>), String> {
            let path = entry_path(&root, &entry.id)?;
            let parsed = m.source.read(&path)?;
            let cwd = match folders.get(&workspace) {
                Some(cwd) => cwd.clone(),
                None => {
                    let dir = crate::workspaces::session_dir_for(cx.vault, &workspace)?;
                    let cwd = dir.to_string_lossy().into_owned();
                    folders.insert(workspace.clone(), cwd.clone());
                    cwd
                }
            };
            let stem = path.file_stem().and_then(|s| s.to_str());
            with_write(cx.vault, "session import", |tx| {
                // Decided again inside the write: another commit may have
                // landed since the list was made.
                let (status, decision) = classify(tx, m.source, &parsed, &own, stem)?;
                let Some(decision) = decision.filter(|_| status.importable()) else {
                    return Ok((status, None));
                };
                let key = write_one(
                    tx,
                    &Target {
                        import_id: id,
                        source: m.source,
                        // An export's `imports.root` is the file, not the
                        // staged copy that is deleted after the commit.
                        root: m
                            .export
                            .as_ref()
                            .map_or(m.root.as_str(), |e| e.fingerprint.as_str()),
                        parsed: &parsed,
                        cwd: &cwd,
                        keep_out: request.keep_out_of_search,
                        now,
                        entry: &entry.id,
                    },
                    decision,
                    hook,
                )?;
                Ok((status, Some(key)))
            })
        })();
        match written {
            Ok((status, Some(key))) => {
                match status {
                    Status::Updated => out.updated += 1,
                    Status::NewVersion => out.new_versions += 1,
                    _ => out.imported += 1,
                }
                out.session_keys.push(key.clone());
                m.conversations[index].status = Status::AlreadyImported;
                m.conversations[index].existing_key = Some(key);
                m.conversations[index].selected = false;
            }
            Ok((Status::AlreadyImported, None)) => out.already_imported += 1,
            Ok((_, None)) => out.skipped += 1,
            Err(error) => out.failed.push(Failed {
                id: entry.id.clone(),
                title: entry.title.clone(),
                error,
            }),
        }
    }
    if out.failed.is_empty() {
        cx.staging.discard(id)?;
        out.staging_deleted = true;
    } else {
        m.state = "committed".into();
        let failed: HashSet<&str> = out.failed.iter().map(|f| f.id.as_str()).collect();
        for c in &mut m.conversations {
            c.selected = failed.contains(c.id.as_str());
        }
        cx.staging.write_json(id, MANIFEST, &m)?;
    }
    set_job(id, |s| {
        s.state = "committed".into();
        s.done = picked.len();
    });
    Ok(out)
}

/// One conversation's write, inside its transaction.
struct Target<'a> {
    import_id: &'a str,
    source: Source,
    root: &'a str,
    parsed: &'a Parsed,
    cwd: &'a str,
    keep_out: bool,
    now: DateTime<Utc>,
    entry: &'a str,
}

/// An RFC 3339 time as the session store writes it, else `fallback`.
fn store_time(at: Option<&str>, fallback: &str) -> String {
    at.and_then(|a| DateTime::parse_from_rfc3339(a).ok())
        .map(|t| t.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|| fallback.to_string())
}

fn fresh_key(conn: &Connection) -> Result<String, String> {
    for _ in 0..8 {
        let key = crate::agent_sessions::mint_session_key();
        let used: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(|e| format!("import key: {e}"))?;
        if used == 0 {
            return Ok(key);
        }
    }
    Err("no free chat key".into())
}

/// One turn as stored.
struct StoredTurn {
    at: String,
    ended: String,
    asked: String,
    reply: String,
    tools: Vec<ConversationTool>,
}

/// The turns to store, in ord order from 1: a "Shortened" line first when
/// the cap left turns out, then the kept turns.
fn stored_turns(source: Source, parsed: &Parsed) -> Vec<StoredTurn> {
    let start = parsed.created_at.clone().unwrap_or_default();
    let mut out = Vec::with_capacity(parsed.turns.len() + 2);
    if let Some(grouping) = &parsed.grouping {
        out.push(StoredTurn {
            at: start.clone(),
            ended: start.clone(),
            asked: String::new(),
            reply: format!(
                "{grouping}: {} has no conversation ids, so these prompts were put in one chat because none came more than {} minutes after the one before.",
                source.label(),
                gemini::GAP_MINUTES
            ),
            tools: Vec::new(),
        });
    }
    if parsed.dropped_turns > 0 {
        let mb = (parsed.dropped_bytes as f64 / (1024.0 * 1024.0)).max(0.1);
        out.push(StoredTurn {
            at: start.clone(),
            ended: start.clone(),
            asked: String::new(),
            reply: format!(
                "Shortened: the oldest {} turns (about {mb:.1} MB of text) aren't in this copy; the {} file still has them.",
                parsed.dropped_turns,
                source.label()
            ),
            tools: Vec::new(),
        });
    }
    for turn in &parsed.turns {
        let at = turn.at.clone().unwrap_or_else(|| start.clone());
        let ended = turn.ended_at.clone().unwrap_or_else(|| at.clone());
        let tools = turn
            .tools
            .iter()
            .enumerate()
            .map(|(i, t)| ConversationTool {
                id: if t.id.is_empty() {
                    format!("imported-{i}")
                } else {
                    t.id.clone()
                },
                name: t.name.clone(),
                input: t.input.clone(),
                result: Some(match &t.output {
                    Some(output) => format!("{NOT_RERUN}\n\n{output}"),
                    None => NOT_RERUN.to_string(),
                }),
                is_error: t.is_error,
                status: if t.is_error { "error" } else { "done" }.to_string(),
                parent_tool_use_id: None,
                result_chars: None,
                result_lines: None,
                result_turn: None,
            })
            .collect();
        out.push(StoredTurn {
            at,
            ended,
            asked: turn.asked_text(),
            reply: turn.reply.clone(),
            tools,
        });
    }
    out
}

/// Write turns from `first` (1-based ord) on.
fn write_turns(
    tx: &Connection,
    key: &str,
    w: &Target<'_>,
    first: usize,
    fallback: &str,
) -> Result<usize, String> {
    let turns = stored_turns(w.source, w.parsed);
    for (i, turn) in turns.into_iter().enumerate().skip(first - 1) {
        crate::session_store::write_imported_turn(
            tx,
            key,
            i as i64 + 1,
            &crate::session_store::ImportedTurn {
                agent: w.source.agent(),
                started_at: &store_time(Some(&turn.at), fallback),
                ended_at: &store_time(Some(&turn.ended), fallback),
                asked: &turn.asked,
                reply: &turn.reply,
            },
            turn.tools,
        )?;
    }
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE session_key = ?1",
            params![key],
            |r| r.get(0),
        )
        .map_err(|e| format!("import turns: {e}"))?;
    Ok(count as usize)
}

fn write_one(
    tx: &Connection,
    w: &Target<'_>,
    decision: Decision,
    hook: Hook<'_>,
) -> Result<String, String> {
    let now = w.now.to_rfc3339();
    let parsed = w.parsed;
    let created = store_time(parsed.created_at.as_deref(), &now);
    let last = store_time(parsed.last_at.as_deref(), &created);
    let hashes = serde_json::to_string(&parsed.turn_hashes).map_err(|e| e.to_string())?;
    match decision {
        Decision::Already(key) => Ok(key),
        Decision::Update(old) => {
            // Rewrite from the first source turn that changed; everything
            // when either side was shortened (its ords don't line up).
            let first = if old.shortened > 0 || parsed.dropped_turns > 0 {
                1
            } else {
                old.turn_hashes
                    .iter()
                    .zip(&parsed.turn_hashes)
                    .take_while(|(a, b)| a == b)
                    .count()
                    + 1
                    // The "Grouped by time" note is stored turn 1.
                    + usize::from(parsed.grouping.is_some())
            };
            tx.execute(
                "DELETE FROM transcripts WHERE session_key = ?1 AND turn_ord >= ?2",
                params![old.session_key, first as i64],
            )
            .map_err(|e| format!("import update: {e}"))?;
            tx.execute(
                "DELETE FROM turns WHERE session_key = ?1 AND ord >= ?2",
                params![old.session_key, first as i64],
            )
            .map_err(|e| format!("import update: {e}"))?;
            hook("session", w.entry)?;
            let count = write_turns(tx, &old.session_key, w, first, &created)?;
            hook("turns", w.entry)?;
            tx.execute(
                "UPDATE sessions SET last_activity_at = ?2, ended_at = ?2, turn_count = ?3 \
                 WHERE key = ?1",
                params![old.session_key, last, count as i64],
            )
            .map_err(|e| format!("import update: {e}"))?;
            tx.execute(
                "UPDATE imports SET content_hash = ?2, turn_hashes = ?3, shortened = ?4, \
                 parser_version = ?5, updated_at = ?6, source_last_at = ?7, source_cwd = ?8 \
                 WHERE id = ?1",
                params![
                    old.row,
                    parsed.content_hash(),
                    hashes,
                    parsed.dropped_turns as i64,
                    PARSER_VERSION,
                    db_time(w.now),
                    parsed.last_at,
                    parsed.cwd,
                ],
            )
            .map_err(|e| format!("import update: {e}"))?;
            Ok(old.session_key)
        }
        Decision::New => insert_new(tx, w, 1, hook),
        Decision::NewVersion(version) => insert_new(tx, w, version, hook),
    }
}

/// A new imported conversation (version 1, or a later version beside the
/// earlier ones).
fn insert_new(
    tx: &Connection,
    w: &Target<'_>,
    version: i64,
    hook: Hook<'_>,
) -> Result<String, String> {
    let now = w.now.to_rfc3339();
    let parsed = w.parsed;
    let base = base_id(w.source, parsed.id.as_deref().unwrap_or(""));
    let created = store_time(parsed.created_at.as_deref(), &now);
    let last = store_time(parsed.last_at.as_deref(), &created);
    let hashes = serde_json::to_string(&parsed.turn_hashes).map_err(|e| e.to_string())?;
    let key = fresh_key(tx)?;
    let mut title = parsed.display_title();
    if version > 1 {
        title = format!("{title} (Version {version})");
    }
    tx.execute(
        "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, \
         ended_at, turn_count, adopted) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6, 0, 1)",
        params![key, title, w.source.agent(), w.cwd, created, last],
    )
    .map_err(|e| format!("import session: {e}"))?;
    tx.execute(
        "INSERT INTO stints(session_key, agent, started_at, ended_at, reason) \
         VALUES(?1, ?2, ?3, ?4, 'imported')",
        params![key, w.source.agent(), created, last],
    )
    .map_err(|e| format!("import stint: {e}"))?;
    hook("session", w.entry)?;
    let count = write_turns(tx, &key, w, 1, &created)?;
    hook("turns", w.entry)?;
    tx.execute(
        "UPDATE sessions SET turn_count = ?2 WHERE key = ?1",
        params![key, count as i64],
    )
    .map_err(|e| format!("import session: {e}"))?;
    let external = if version > 1 {
        format!("{base}#v{version}")
    } else {
        base.clone()
    };
    tx.execute(
        "INSERT INTO imports(import_id, source, root, external_id, base_id, version, \
         content_hash, turn_hashes, shortened, parser_version, imported_at, session_key, \
         source_cwd, source_created_at, source_last_at, keep_out_of_search) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            w.import_id,
            w.source.as_str(),
            w.root,
            external,
            base,
            version,
            parsed.content_hash(),
            hashes,
            parsed.dropped_turns as i64,
            PARSER_VERSION,
            db_time(w.now),
            key,
            parsed.cwd,
            parsed.created_at,
            parsed.last_at,
            w.keep_out,
        ],
    )
    .map_err(|e| format!("import record: {e}"))?;
    Ok(key)
}

// ── provenance, read-only, search ─────────────────────────────────────

/// Conversation `key`'s import provenance; `None` for a chat started in
/// Hollow (and when the table can't be read).
pub(crate) fn provenance(conn: &Connection, key: &str) -> Option<ImportedProvenance> {
    conn.query_row(
        "SELECT source, external_id, version, source_created_at, source_last_at, imported_at, \
         updated_at, keep_out_of_search, source_cwd, import_id FROM imports \
         WHERE session_key = ?1 ORDER BY id DESC LIMIT 1",
        params![key],
        |row| {
            let source: String = row.get(0)?;
            let label = Source::parse(&source).map_or("another app", Source::label);
            let updated: Option<String> = row.get(6)?;
            Ok(ImportedProvenance {
                note: updated.as_deref().map(|at| updated_note(label, at)),
                label: label.to_string(),
                source,
                external_id: row.get(1)?,
                version: row.get::<_, i64>(2)? as u32,
                original_created_at: row.get(3)?,
                original_last_at: row.get(4)?,
                imported_at: row.get(5)?,
                updated_at: updated,
                keep_out_of_search: row.get(7)?,
                source_cwd: row.get(8)?,
                import_id: row.get(9)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// "Updated from Claude Code 3:12 PM" (local time).
fn updated_note(label: &str, at: &str) -> String {
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => format!(
            "Updated from {label} {}",
            t.with_timezone(&chrono::Local).format("%-I:%M %p, %-d %b")
        ),
        Err(_) => format!("Updated from {label}"),
    }
}

/// Whether conversation `key` is an imported record (read-only).
pub(crate) fn is_imported(conn: &Connection, key: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM imports WHERE session_key = ?1 LIMIT 1",
        params![key],
        |_| Ok(()),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

/// What `start_turn` says for an imported record.
pub(crate) const READ_ONLY: &str =
    "This conversation was imported and is read-only. Use Continue in Hollow to carry it on.";

/// The imported conversations the person kept out of agent search (N25).
pub(crate) fn hidden_from_agents(vault: &Path) -> HashSet<String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT session_key FROM imports WHERE keep_out_of_search = 1")
            .map_err(|e| e.to_string())?;
        let keys = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(keys)
    })
    .unwrap_or_default()
}

/// N25's switch for every conversation of import `import_id`; the number
/// changed.
pub(crate) fn set_keep_out_of_search(
    vault: &Path,
    import_id: &str,
    keep_out: bool,
) -> Result<usize, String> {
    with_write(vault, "import search", |tx| {
        tx.execute(
            "UPDATE imports SET keep_out_of_search = ?2 WHERE import_id = ?1",
            params![import_id, keep_out],
        )
        .map_err(|e| format!("import search: {e}"))
    })
}

/// One committed import batch.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Batch {
    pub import_id: String,
    pub source: String,
    pub conversations: usize,
    pub imported_at: String,
    pub keep_out_of_search: bool,
}

/// Every committed batch, newest first.
pub(crate) fn batches(vault: &Path) -> Result<Vec<Batch>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT import_id, source, COUNT(*), MAX(imported_at), MAX(keep_out_of_search) \
                 FROM imports GROUP BY import_id, source ORDER BY MAX(imported_at) DESC",
            )
            .map_err(|e| format!("import batches: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Batch {
                    import_id: row.get(0)?,
                    source: row.get(1)?,
                    conversations: row.get::<_, i64>(2)? as usize,
                    imported_at: row.get(3)?,
                    keep_out_of_search: row.get(4)?,
                })
            })
            .map_err(|e| format!("import batches: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("import batches: {e}"))?;
        Ok(rows)
    })
}

// ── Continue in Hollow (N26) ──────────────────────────────────────────

/// What the person picked for the new conversation, as for any new chat.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ContinueRequest {
    pub agent: String,
    pub model: Option<String>,
    pub workspace_id: Option<String>,
    pub mode: Option<String>,
    pub effort: Option<String>,
}

/// The conversation Continue in Hollow made.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Continued {
    pub key: String,
    pub title: String,
    pub parent_key: String,
    pub agent: String,
    pub model: Option<String>,
    pub workspace_id: String,
    pub cwd: String,
    /// The permission mode the composer opens with (never the source's).
    pub mode: String,
    pub effort: Option<String>,
}

/// Continue imported conversation `key` in Hollow: a new, empty conversation
/// linked to it (`parent_key`, `fork_kind = 'import'`) in the workspace the
/// person picked (the imported one's own by default). Its first turn starts
/// a fresh native session that receives the imported turns as quoted
/// history ([`continued_import`]). The imported record is not written.
pub(crate) fn continue_imported(
    vault: &Path,
    key: &str,
    request: &ContinueRequest,
    now: DateTime<Utc>,
) -> Result<Continued, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err("That isn't a chat's key.".into());
    }
    let agent = crate::agent::AgentKind::from_opt(&request.agent)
        .ok_or_else(|| format!("Unknown agent {:?}.", request.agent))?;
    let mode = request
        .mode
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map(crate::app_mcp::normalize_perm)
        .unwrap_or("default")
        .to_string();
    let effort = request
        .effort
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string);
    if effort.as_deref().is_some_and(|e| e.len() > 32) {
        return Err("That effort level isn't one Hollow knows.".into());
    }
    let (title, source_cwd) = with_read(vault, |conn| {
        if !is_imported(conn, key) {
            return Err("That conversation wasn't imported.".to_string());
        }
        conn.query_row(
            "SELECT title, cwd FROM sessions WHERE key = ?1",
            params![key],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|e| format!("continue read: {e}"))
    })?;
    let workspace = match request
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
    {
        Some(w) => w.to_string(),
        None => crate::workspaces::workspace_for_cwd(vault, Path::new(&source_cwd))
            .ok()
            .flatten()
            .map(|w| w.id)
            .unwrap_or_else(|| crate::workspaces::QUICK_CHATS_ID.to_string()),
    };
    if crate::workspaces::is_main_agent(&workspace) {
        return Err("Continue in Hollow can't start in the COO's workspace.".into());
    }
    let cwd = crate::workspaces::session_dir_for(vault, &workspace)?
        .to_string_lossy()
        .into_owned();
    let title = format!(
        "{} (continued)",
        title
            .as_deref()
            .map(str::trim)
            .unwrap_or("Imported conversation")
    );
    let at = now.to_rfc3339();
    let new_key = with_write(vault, "continue import", |tx| {
        let new_key = fresh_key(tx)?;
        tx.execute(
            "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, \
             turn_count, parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?5, 0, ?6, ?7)",
            params![new_key, title, agent.as_str(), cwd, at, key, FORK_KIND],
        )
        .map_err(|e| format!("continue session: {e}"))?;
        if let Some(effort) = &effort {
            tx.execute(
                "INSERT INTO chat_efforts(key, effort, updated_at) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET effort = ?2, updated_at = ?3",
                params![new_key, effort, db_time(now)],
            )
            .map_err(|e| format!("continue effort: {e}"))?;
        }
        Ok(new_key)
    })?;
    Ok(Continued {
        key: new_key,
        title,
        parent_key: key.to_string(),
        agent: agent.as_str().to_string(),
        model: request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        workspace_id: workspace,
        cwd,
        mode,
        effort,
    })
}

/// For a conversation Continue in Hollow made: the imported conversation it
/// continues and its source's label. `None` for every other chat.
pub(crate) fn continued_import(vault: &Path, key: &str) -> Option<(String, String)> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return None;
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT s.parent_key, i.source FROM sessions s \
             JOIN imports i ON i.session_key = s.parent_key \
             WHERE s.key = ?1 AND s.fork_kind = ?2 ORDER BY i.id DESC LIMIT 1",
            params![key, FORK_KIND],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .map(|(parent, source)| {
        let label = Source::parse(&source).map_or("another app", Source::label);
        (parent, label.to_string())
    })
}

/// The agent name an imported entry carries in the handoff block: says it
/// is quoted history from the source, never rerun.
pub(crate) fn handoff_agent(label: &str) -> String {
    format!("imported from {label}: quoted history, never rerun")
}

#[cfg(test)]
mod native_resume_tests;
#[cfg(test)]
mod tests;
