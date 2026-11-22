//! Real-vault replay probes (overnight 10-17 OF17): the evening's merged
//! pure functions run over real files at $0, so an effect that was "by test
//! only; live after install" gets a number now. Baselines they reproduce:
//! HB03 (read-only task-state steps, Main records with a check-in goal),
//! HF03 (notes the embed index makes no chunk of), HF04 (requests read as
//! historical), HF05 (requests that get the oversized-note notice), HB02
//! (what a cut brief keeps), HB06/OB14 (handovers whose successor merges the
//! predecessor's task state) and OF24's $0 number (cut briefs that lose
//! their archive pointer).
//!
//! The real run is an `#[ignore]` test reading a vault copy and a Sessions
//! DB named by environment variables (see `docs/reference/build-and-ship.md`
//! for the exact command and its `trap`):
//!
//! ```text
//! TMPDIR=<per-run mktemp -d folder> HOLLOW_REPLAY_VAULT=<vault copy without restricted/> \
//! HOLLOW_REPLAY_DB=<Sessions DB> HOLLOW_REPLAY_OUT=<report.json> \
//!   cargo test --lib -- --ignored replay_probes
//! ```
//!
//! **Setup** ([`Replay::setup`], reused by S07, OB18 and OB25): a fresh
//! folder under the temp dir (`hollow-test-replay-<pid>-<random>`, mode
//! 0700) gets each workspace's context documents ([`WORKSPACE_DOCS`]), its
//! `PROJECT.md` and its attached `sources/`, and the vault's `projects/` and
//! `goals/` (what `context_with`'s attached sources and
//! `goal_context::workspace_alignment` read), plus, with
//! [`Replay::setup_with`], any other vault folders a probe needs; no path
//! with a `restricted` component is read or copied, and no link is
//! followed. The Sessions DB is snapshotted with `VACUUM INTO` from a
//! connection that opens it `?immutable=1&mode=ro` (the same read as
//! `sqlite3 -readonly 'file:<db>?immutable=1' '.backup …'`: no lock is
//! taken and no `-wal`/`-shm` is created beside the live file; frames the
//! app hasn't checkpointed yet aren't in it), into a file created empty
//! with mode 0600 first, so the snapshot is never readable by others, and
//! checked with `PRAGMA quick_check`. It goes where the test build's
//! session store looks for the copy's database
//! (`session_store::db_path_for`), so `task_turns` and the rest read it
//! unchanged. The snapshot holds every message body: dropping the
//! [`Replay`] deletes it (with its `-wal`/`-shm`) and the copied folder, the
//! shell `trap` deleting the run's own `TMPDIR` is the second guard, and
//! setup refuses a temp dir inside this checkout, under a `Worktrees`
//! folder or inside `~/Hollow` (and one it can't resolve), so it never
//! lives under a worktree. Nothing writes the source vault or the app
//! support dir.
//!
//! **Output** ([`report`]): aggregates only (counts, shares, byte sizes and
//! line numbers, and Hollow's own vocabulary as keys: note kinds, history
//! cue words). Never a request, a note's text, a path or a session key.
//! The report at `HOLLOW_REPLAY_OUT` is the only thing written outside the
//! temp dirs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

// context_compact's archive pointer line, as brief_selection matches it.
use crate::context_compact::POINTER;
use crate::session_store::TaskTurn;
use crate::task_state::Record;

/// The workspace documents a turn's envelope reads (`workspaces.rs`'s
/// `CONTEXT_DOCS` and, for the Main agent, its standing orders): what the
/// envelope probe needs of each workspace.
pub(crate) const WORKSPACE_DOCS: [&str; 6] = [
    "WORKSPACE.md",
    "PLAN.md",
    "SOURCES.md",
    "AGENTS.md",
    "NOTION.md",
    crate::workspaces::STANDING_ORDERS,
];
/// The vault folders the envelope reads beyond the workspace folders: the
/// project notes and goals `goal_context::workspace_alignment` resolves.
pub(crate) const ENVELOPE_FOLDERS: [&str; 2] = ["projects", "goals"];
/// A path component never read or copied.
pub(crate) const RESTRICTED: &str = "restricted";
/// The newest requests the history-cue and oversized-notice probes read
/// (HF04's and HF05's windows).
pub(crate) const REQUEST_WINDOW: usize = 500;
/// How `ContextDoc::value` marks a cut document.
const CUT_MARK: &str = "[Hollow: cut to fit.";
/// How `brief_selection` marks the Current context entries it left out.
const LEFT_OUT_MARK: &str = "older entries in Current context left out";
/// How a check-in's message opens (main_agent.rs's private `CHECKIN_OPEN`).
const CHECKIN_OPEN: &str = "⟲ Hollow: check-in at";

/// Whether vault-relative `rel` has a `restricted` component.
pub(crate) fn has_restricted(rel: &Path) -> bool {
    rel.components()
        .any(|c| c.as_os_str().to_str() == Some(RESTRICTED))
}

fn mode(path: &Path, bits: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits));
}

// ── Setup ─────────────────────────────────────────────────────────────────

/// A replay: copied workspace documents under a fresh temp folder and a
/// snapshot of the Sessions DB that the test build's session store reads
/// for that folder. Dropping it deletes both.
pub(crate) struct Replay {
    /// The copy's root: a vault holding only what was copied. Pass it to
    /// `session_store` and `workspaces` calls.
    pub vault: PathBuf,
    /// The vault copied from, read only (the embed and notice probes walk
    /// its notes in place).
    pub source: PathBuf,
    /// The snapshot (`session_store::db_path_for(&vault)`); empty until it
    /// is made.
    pub db: PathBuf,
    /// The ids of the workspaces whose documents were copied, sorted.
    pub workspaces: Vec<String>,
    /// The snapshot's `PRAGMA quick_check`: `ok`, or SQLite's messages.
    pub quick_check: String,
}

impl Replay {
    /// [`Replay::setup_with`] with the workspace documents alone.
    pub(crate) fn setup(source: &Path, db: &Path) -> Result<Replay, String> {
        Self::setup_with(source, db, &[])
    }

    /// Copy each workspace's [`WORKSPACE_DOCS`], `PROJECT.md` and `sources/`,
    /// the vault's [`ENVELOPE_FOLDERS`] and every file under the
    /// vault-relative `folders` (e.g. `learnings`) of vault `source`, skipping
    /// hidden and `restricted` paths and links, then snapshot Sessions DB
    /// `db`. On an error, what was made is deleted.
    pub(crate) fn setup_with(source: &Path, db: &Path, folders: &[&str]) -> Result<Replay, String> {
        if !source.is_dir() {
            return Err("the replay vault is not a folder".into());
        }
        if !db.is_file() {
            return Err("the Sessions DB is not a file".into());
        }
        let tmp = std::env::temp_dir();
        refuse_temp_dir(&tmp)?;
        let dir = tmp.join(format!(
            "hollow-test-replay-{}-{:08x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).map_err(|e| format!("replay folder: {e}"))?;
        mode(&dir, 0o700);
        // From here the guard owns the folder: an error below deletes it.
        let mut replay = Replay {
            vault: dir.clone(),
            source: source.to_path_buf(),
            db: PathBuf::new(),
            workspaces: Vec::new(),
            quick_check: String::new(),
        };
        replay.workspaces = copy_workspace_docs(source, &dir)?;
        for folder in ENVELOPE_FOLDERS.iter().chain(folders) {
            copy_folder(source, &dir, folder)?;
        }
        let snapshot = crate::session_store::db_path_for(&dir)
            .ok_or_else(|| "no session store path for the copy".to_string())?;
        if snapshot.exists() {
            return Err("the snapshot path is taken".into());
        }
        // Set before the file exists: a failed snapshot is still deleted.
        replay.db = snapshot;
        replay.quick_check = snapshot_db(db, &replay.db)?;
        Ok(replay)
    }

    /// A read-only connection to the snapshot.
    pub(crate) fn conn(&self) -> Result<Connection, String> {
        Connection::open_with_flags(
            &self.db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("replay snapshot open: {e}"))
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        if !self.db.as_os_str().is_empty() {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut path = self.db.clone().into_os_string();
                path.push(suffix);
                let _ = std::fs::remove_file(PathBuf::from(path));
            }
        }
        // Always the fresh temp folder `setup_with` made, never the source.
        let _ = std::fs::remove_dir_all(&self.vault);
    }
}

/// Refuse temp dir `tmp` for the snapshot when it is inside this checkout,
/// has a `Worktrees` component or is inside `~/Hollow`, and when it, the
/// checkout or `HOME` can't be resolved (fail closed). The retrieval eval's
/// snapshots (`retrieval_eval`, OF01b) check the same.
pub(crate) fn refuse_temp_dir(tmp: &Path) -> Result<(), String> {
    // The snapshot must never live under a worktree.
    const FIX: &str = "set TMPDIR to a fresh `mktemp -d /tmp/hollow-replay-tmp.XXXXXX`";
    let real = tmp
        .canonicalize()
        .map_err(|e| format!("TMPDIR can't be resolved ({e}): {FIX}"))?;
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
    let checkout = checkout
        .parent()
        .unwrap_or(checkout)
        .canonicalize()
        .map_err(|e| format!("this checkout can't be resolved ({e}): {FIX}"))?;
    if real.starts_with(&checkout) {
        return Err(format!("TMPDIR is inside this checkout: {FIX}"));
    }
    if tmp
        .components()
        .chain(real.components())
        .any(|c| c.as_os_str() == "Worktrees")
    {
        return Err(format!("TMPDIR is under a Worktrees folder: {FIX}"));
    }
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .ok_or_else(|| format!("HOME is unset: {FIX}"))?;
    let hollow = PathBuf::from(home).join("Hollow");
    // A missing ~/Hollow holds nothing: compare its path as written then.
    let hollow = hollow.canonicalize().unwrap_or(hollow);
    if real.starts_with(&hollow) {
        return Err(format!("TMPDIR is inside ~/Hollow: {FIX}"));
    }
    Ok(())
}

/// `path` for a `file:` URI: `%`, `?`, `#` and spaces escaped.
fn uri_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| "the source database path is not UTF-8".to_string())?;
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3f"),
            '#' => out.push_str("%23"),
            ' ' => out.push_str("%20"),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// Snapshot SQLite database `db` into `dest` (which must not exist): `dest`
/// is created empty with mode 0600 (and its folder 0700 when this makes
/// it), so the snapshot is never readable by others, then filled with
/// `VACUUM INTO` (which takes an existing empty file; a non-empty one is an
/// error) from an `immutable=1&mode=ro` connection, so the live file is
/// neither locked nor written. The snapshot's `PRAGMA quick_check`: `ok`,
/// or SQLite's messages. Errors name "the source database", not which one:
/// the retrieval eval snapshots the EmbedIndex DB with it too.
pub(crate) fn snapshot_db(db: &Path, dest: &Path) -> Result<String, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let db = db
        .canonicalize()
        .map_err(|e| format!("the source database: {e}"))?;
    if let Some(parent) = dest.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| format!("snapshot folder: {e}"))?;
            mode(parent, 0o700);
        }
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dest)
        .map_err(|e| format!("snapshot file: {e}"))?;
    let uri = format!("file:{}?immutable=1&mode=ro", uri_path(&db)?);
    let source = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("the source database, read only: {e}"))?;
    let target = dest
        .to_str()
        .ok_or_else(|| "the snapshot path is not UTF-8".to_string())?;
    source
        .execute_batch(&format!("VACUUM INTO '{}'", target.replace('\'', "''")))
        .map_err(|e| format!("snapshot: {e}"))?;
    drop(source);
    mode(dest, 0o600);
    quick_check(dest)
}

/// `PRAGMA quick_check` of the owned snapshot `path`: `ok`, or its
/// messages (page and index names, never row content) joined by `; `.
/// FTS5's integrity callback requires a write-capable connection. The source
/// database remains read-only; only the private snapshot is opened here.
fn quick_check(path: &Path) -> Result<String, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("snapshot check open: {e}"))?;
    let mut stmt = conn
        .prepare("PRAGMA quick_check")
        .map_err(|e| format!("snapshot check: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("snapshot check: {e}"))?;
    let messages: Vec<String> = rows
        .collect::<Result<_, _>>()
        .map_err(|e| format!("snapshot check: {e}"))?;
    Ok(messages.join("; "))
}

/// Copy each `workspaces/<id>/` document of [`WORKSPACE_DOCS`] that is a
/// regular file and, for a workspace that had one, its `PROJECT.md` and its
/// `sources/` with their `.provenance/` (as [`copy_folder`]); the ids that
/// had one, sorted.
fn copy_workspace_docs(source: &Path, dest: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let Ok(entries) = std::fs::read_dir(source.join("workspaces")) else {
        return Ok(ids);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str() else {
            continue;
        };
        if id.starts_with('.') || id == RESTRICTED {
            continue;
        }
        if !std::fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let to = dest.join("workspaces").join(id);
        let mut copied = false;
        for doc in WORKSPACE_DOCS {
            copied |= copy_file(&entry.path(), &to, doc)?;
        }
        if copied {
            let project = crate::vault_projects::WORKSPACE_PROJECT_FILE;
            copy_file(&entry.path(), &to, project)?;
            copy_folder(source, dest, &format!("workspaces/{id}/sources"))?;
            // The snapshots' provenance sidecars the envelope lists with them.
            copy_folder(
                source,
                dest,
                &format!("workspaces/{id}/sources/.provenance"),
            )?;
            ids.push(id.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// Copy `from/<name>` to `to/<name>` when it is a regular file (not a
/// link); whether it was.
fn copy_file(from: &Path, to: &Path, name: &str) -> Result<bool, String> {
    let from = from.join(name);
    if !std::fs::symlink_metadata(&from).is_ok_and(|m| m.is_file()) {
        return Ok(false);
    }
    std::fs::create_dir_all(to).map_err(|e| format!("copy folder: {e}"))?;
    std::fs::copy(from, to.join(name)).map_err(|e| format!("copy a workspace document: {e}"))?;
    Ok(true)
}

/// Copy every regular file under vault-relative `folder`, skipping hidden
/// and `restricted` paths; links are not followed, the folder itself
/// included (a linked or missing folder copies nothing).
fn copy_folder(source: &Path, dest: &Path, folder: &str) -> Result<(), String> {
    if has_restricted(Path::new(folder)) {
        return Err("a restricted folder is never copied".into());
    }
    if !std::fs::symlink_metadata(source.join(folder)).is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(source.join(folder))
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(name.starts_with('.') || name == RESTRICTED)
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(source) else {
            continue;
        };
        if has_restricted(rel) {
            continue;
        }
        let to = dest.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("copy folder: {e}"))?;
        }
        std::fs::copy(entry.path(), to).map_err(|e| format!("copy a note: {e}"))?;
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 / whole as f64 * 10_000.0).round() / 10_000.0
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let at = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

/// The workspace id of a session's `cwd` (`…/workspaces/<id>`), if it is a
/// workspace folder.
pub(crate) fn workspace_of(cwd: &str) -> Option<&str> {
    let mut parts = cwd.trim_end_matches('/').rsplit('/');
    let id = parts.next()?;
    (parts.next()? == "workspaces").then_some(id)
}

/// `turns` folded from an empty record, without receipts (the steps and the
/// goal read only the turns).
fn folded<'a>(turns: impl IntoIterator<Item = &'a TaskTurn>) -> Record {
    let receipts = crate::task_state::Receipts::none();
    let mut record = Record::default();
    for turn in turns {
        crate::task_state::fold(&mut record, turn, &receipts);
    }
    record
}

// ── Envelope (HB02, OF24) ─────────────────────────────────────────────────

/// What a brief's envelope copy (`included`, the envelope's `WORKSPACE.md`
/// value) kept of the brief (`source`). The options are `None` when the
/// brief has no such part.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BriefFacts {
    pub cut: bool,
    /// Its `## Decisions` heading is there.
    pub decisions: Option<bool>,
    /// The newest dated Current context entry's first line is there.
    pub newest_entry: Option<bool>,
    /// The "older entries … left out" marker is there.
    pub left_out: bool,
    /// The archive pointer line in Current context is there.
    pub pointer: Option<bool>,
}

/// [`BriefFacts`] for one brief, its Current context dated as of `today`
/// (as `brief_selection` dates it).
pub(crate) fn brief_facts(source: &str, included: &str, today: chrono::NaiveDate) -> BriefFacts {
    let kept: Vec<&str> = included.split('\n').map(str::trim_end).collect();
    let lines: Vec<&str> = source.split('\n').collect();
    let decisions = lines
        .iter()
        .find(|line| is_decisions_heading(line))
        .map(|line| holds(&kept, line));
    let (newest_entry, pointer) = match crate::context_compact::section(&lines) {
        Some((start, end)) => {
            let entries = crate::context_compact::entries(&lines, start, end, today);
            // Newest date; a later entry wins a tie (as brief_selection).
            let newest = entries
                .iter()
                .filter(|e| e.date.is_some())
                .max_by(|a, b| a.date.cmp(&b.date).then(a.start.cmp(&b.start)))
                .map(|e| holds(&kept, lines[e.start]));
            let pointer = lines[start..end]
                .iter()
                .find(|line| line.contains(POINTER))
                .map(|line| holds(&kept, line));
            (newest, pointer)
        }
        None => (None, None),
    };
    BriefFacts {
        cut: included.contains(CUT_MARK),
        decisions,
        newest_entry,
        left_out: included.contains(LEFT_OUT_MARK),
        pointer,
    }
}

/// Whether `kept` (lines, ends trimmed) has `line`.
fn holds(kept: &[&str], line: &str) -> bool {
    kept.contains(&line.trim_end())
}

fn is_decisions_heading(line: &str) -> bool {
    let name = line.trim_end().trim_start_matches('#').trim();
    // The slim brief's rule ("Decisions and constraints" counts, RF-806-F1).
    line.starts_with("## ")
        && !name.eq_ignore_ascii_case("outcome")
        && crate::workspaces::kept_brief_section(name)
}

/// Present / included counts of one brief part, over cut briefs.
#[derive(Default)]
struct Kept {
    present: usize,
    included: usize,
}

impl Kept {
    fn add(&mut self, part: Option<bool>) {
        if let Some(included) = part {
            self.present += 1;
            self.included += usize::from(included);
        }
    }
}

/// Each workspace's envelope through `workspaces::context_for_within` (the
/// full 128 KB envelope; no request, so attached sources go only when all
/// their text fits, or a one-chunk file whole, and are pointers otherwise):
/// how many briefs and plans were cut and what the cut briefs kept, the
/// envelope sizes, and the Main agent's brief.
pub(crate) fn envelopes(vault: &Path, ids: &[String]) -> Value {
    let request = crate::workspaces::SourceRequest {
        query: "",
        reader: crate::workspaces::SourceReader::ReadSource,
        child_packet: None,
    };
    let today = chrono::Local::now().date_naive();
    let (mut skipped, mut errors, mut briefs, mut cut, mut left_out, mut plans_cut) =
        (0, 0, 0, 0, 0, 0);
    let (mut decisions, mut newest, mut pointer) =
        (Kept::default(), Kept::default(), Kept::default());
    let mut sizes = Vec::new();
    let mut main = Value::Null;
    for id in ids {
        let cwd = vault.join("workspaces").join(id);
        let ctx = match crate::workspaces::context_for_within(vault, &cwd, &request, usize::MAX) {
            Ok(Some(ctx)) => ctx,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        // A quick chat's envelope is a fixed text.
        let Some(parts) = ctx.parts.as_ref() else {
            skipped += 1;
            continue;
        };
        sizes.push(ctx.text.len() as u64);
        let doc = |name: &str| parts.json.get(name).and_then(Value::as_str).unwrap_or("");
        plans_cut += usize::from(doc("PLAN.md").contains(CUT_MARK));
        let Ok(source) = std::fs::read_to_string(cwd.join("WORKSPACE.md")) else {
            continue;
        };
        briefs += 1;
        let facts = brief_facts(&source, doc("WORKSPACE.md"), today);
        if facts.cut {
            cut += 1;
            left_out += usize::from(facts.left_out);
            decisions.add(facts.decisions);
            newest.add(facts.newest_entry);
            pointer.add(facts.pointer);
        }
        if crate::workspaces::is_main_agent(id) {
            main = json!({
                "briefBytes": source.len(),
                "briefLines": source.lines().count(),
                "decisionsLine": source.lines().position(is_decisions_heading).map(|i| i + 1),
                "cut": facts.cut,
                "decisionsIncluded": facts.decisions,
                "newestEntryIncluded": facts.newest_entry,
                "archivePointerIncluded": facts.pointer,
            });
        }
    }
    sizes.sort_unstable();
    json!({
        "workspaces": ids.len(),
        "skipped": skipped,
        "errors": errors,
        "envelopeBytes": {
            "p50": percentile(&sizes, 0.5),
            "p95": percentile(&sizes, 0.95),
            "max": sizes.last().copied().unwrap_or(0),
        },
        "plansCut": plans_cut,
        "briefs": {
            "total": briefs,
            "cut": cut,
            "cutLeftOutMarked": left_out,
            "cutWithDecisions": decisions.present,
            "cutDecisionsIncluded": decisions.included,
            "cutWithDatedEntries": newest.present,
            "cutNewestEntryIncluded": newest.included,
            "cutWithArchivePointer": pointer.present,
            "cutArchivePointerLost": pointer.present - pointer.included,
        },
        "main": main,
    })
}

// ── Task state (HB03) ─────────────────────────────────────────────────────

/// How `task_state::fold` treats `turn`'s tool calls, by its own rule over
/// each call's full input: `(dropped, kept)`, the calls it drops as
/// read-only (a read tool by name, or a read-only shell call running no
/// check) and the ones it makes steps (before its step cap). A call still
/// running in a running turn is neither. HB03's "before" is the dropped
/// share; "after" is 0 by construction.
pub(crate) fn fold_split(turn: &TaskTurn) -> (usize, usize) {
    let running = turn.status.as_deref() == Some("running");
    let (mut dropped, mut kept) = (0, 0);
    for tool in &turn.tools {
        if crate::task_state::is_read_only(&tool.name)
            || crate::task_state::is_read_only_shell(tool)
        {
            dropped += 1;
        } else if !(running && tool.status == "running") {
            kept += 1;
        }
    }
    (dropped, kept)
}

fn is_checkin(text: &str) -> bool {
    text.trim_start().starts_with(CHECKIN_OPEN)
}

/// Every conversation's turns folded by `task_state::fold` (no receipts):
/// steps and their outcomes, the tool calls fold drops as read-only and the
/// ones it keeps ([`fold_split`], over the raw calls), and the records whose
/// goal is a host wake or check-in, overall and for the Main agent's
/// conversations (cwd `…/workspaces/main`).
pub(crate) fn task_states(vault: &Path, conn: &Connection) -> Result<Value, String> {
    let sessions: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT key, cwd FROM sessions ORDER BY key")
            .map_err(|e| format!("sessions: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("sessions: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("sessions: {e}"))?
    };
    let (mut errors, mut records, mut steps, mut host_goals) = (0, 0, 0, 0);
    let (mut dropped, mut kept) = (0, 0);
    let (mut main_records, mut main_host, mut main_checkin) = (0, 0, 0);
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    for (key, cwd) in &sessions {
        let Ok(turns) = crate::session_store::task_turns(vault, key, 0, i64::MAX) else {
            errors += 1;
            continue;
        };
        for turn in &turns {
            let (d, k) = fold_split(turn);
            dropped += d;
            kept += k;
        }
        let record = folded(&turns);
        if record.is_empty() {
            continue;
        }
        records += 1;
        steps += record.steps.len();
        for step in &record.steps {
            *outcomes.entry(step.outcome.clone()).or_default() += 1;
        }
        let goal = record.goal.as_ref().map_or("", |g| g.text.as_str());
        let host = crate::main_agent::is_wake_message(goal);
        host_goals += usize::from(host);
        if workspace_of(cwd).is_some_and(crate::workspaces::is_main_agent) {
            main_records += 1;
            main_host += usize::from(host);
            main_checkin += usize::from(is_checkin(goal));
        }
    }
    Ok(json!({
        "conversations": sessions.len(),
        "errors": errors,
        "records": records,
        "steps": steps,
        "readOnlyDropped": dropped,
        "stepsKept": kept,
        "readOnlyShareBefore": share(dropped, dropped + kept),
        "stepOutcomes": outcomes,
        "hostGoals": host_goals,
        "main": {
            "records": main_records,
            "hostGoals": main_host,
            "checkInGoals": main_checkin,
        },
    }))
}

// ── Handovers (HB06, OB14) ────────────────────────────────────────────────

/// When each turn of a conversation started and ended, by `ord`.
pub(crate) type TurnTimes = HashMap<i64, (DateTime<Utc>, Option<DateTime<Utc>>)>;

/// A conversation's `turns` as they stood at `at`: those started before it,
/// a turn that hadn't ended by then marked `running` (the status
/// `task_state::fold` and `refresh` read). A turn without a readable start
/// is left out.
pub(crate) fn turns_at(turns: &[TaskTurn], times: &TurnTimes, at: DateTime<Utc>) -> Vec<TaskTurn> {
    turns
        .iter()
        .filter_map(|turn| {
            let (started, ended) = times.get(&turn.ord)?;
            if *started >= at {
                return None;
            }
            let mut turn = turn.clone();
            if ended.is_none_or(|ended| ended > at) {
                turn.status = Some("running".into());
            }
            Some(turn)
        })
        .collect()
}

/// One recorded event of a turn: its kind, payload and when it was written.
pub(crate) type TurnEvent = (String, String, DateTime<Utc>);

/// Turn `turn` cut back to what its `events` held at `at`: calls begun
/// after it left out, calls whose result came after it `running` (no
/// result, no check outcome), and the reply the newest `assistant` event
/// by then held (empty before the first).
pub(crate) fn turn_as_of(turn: &mut TaskTurn, events: &[TurnEvent], at: DateTime<Utc>) {
    let id_of = |payload: &str| {
        serde_json::from_str::<Value>(payload)
            .ok()
            .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    };
    let (mut begun, mut answered) = (HashSet::new(), HashSet::new());
    let mut reply = String::new();
    for (kind, payload, when) in events {
        if *when > at {
            continue;
        }
        match kind.as_str() {
            "toolUse" => begun.extend(id_of(payload)),
            "toolResult" => answered.extend(id_of(payload)),
            "assistant" => reply = payload.clone(),
            _ => {}
        }
    }
    turn.tools.retain(|tool| begun.contains(&tool.id));
    for tool in &mut turn.tools {
        if !answered.contains(&tool.id) {
            tool.status = "running".into();
            tool.result = None;
            tool.is_error = false;
        }
    }
    turn.check_outcomes.retain(|id, _| answered.contains(id));
    turn.reply = reply;
}

/// Turn `ord` of conversation `key`'s recorded events, oldest first (none
/// once event retention dropped them; a row with an unreadable time is
/// left out).
fn turn_events(conn: &Connection, key: &str, ord: i64) -> Result<Vec<TurnEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, payload, at FROM events WHERE session_key = ?1 AND turn_ord = ?2 \
             ORDER BY id ASC",
        )
        .map_err(|e| format!("turn events: {e}"))?;
    let rows = stmt
        .query_map(params![key, ord], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("turn events: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, payload, at) = row.map_err(|e| format!("turn events: {e}"))?;
        if let Some(at) = crate::session_store::parse_iso(&at) {
            out.push((kind, payload.unwrap_or_default(), at));
        }
    }
    Ok(out)
}

fn turn_times(conn: &Connection, key: &str) -> Result<TurnTimes, String> {
    let mut stmt = conn
        .prepare("SELECT ord, started_at, ended_at FROM turns WHERE session_key = ?1")
        .map_err(|e| format!("turn times: {e}"))?;
    let rows = stmt
        .query_map(params![key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| format!("turn times: {e}"))?;
    let mut out = TurnTimes::new();
    for row in rows {
        let (ord, started, ended) = row.map_err(|e| format!("turn times: {e}"))?;
        if let Some(started) = crate::session_store::parse_iso(&started) {
            let ended = ended.as_deref().and_then(crate::session_store::parse_iso);
            out.insert(ord, (started, ended));
        }
    }
    Ok(out)
}

/// Each handover (a `childTask` delegation by tool `handover`, its run the
/// predecessor's turn): whether the successor's first turn started while
/// that turn ran, and whether the successor's first turn merges the
/// predecessor's task state. Today's rule (`task_state::peek`, OB14) folds
/// every turn started before the successor, the running one included; the
/// rule before OB14 (`refresh`) stopped at the first running turn.
///
/// A predecessor turn still in flight at the successor's start is cut back
/// to its events recorded by then ([`turn_as_of`]). Events are a
/// retention-limited cache (the newest 200 conversations, 90 days), so a
/// turn whose events are gone is folded from its final transcript instead:
/// `inFlightTurnsWithoutEvents` counts those, and `approximate` is true when
/// there is any.
pub(crate) fn handovers(vault: &Path, conn: &Connection) -> Result<Value, String> {
    let records: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT run_id, record_json FROM delegations ORDER BY id")
            .map_err(|e| format!("delegations: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("delegations: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("delegations: {e}"))?
    };
    let child_task = crate::delegation::Contract::ChildTask.as_str();
    let (mut count, mut with_pred, mut with_succ, mut overlapped, mut first) = (0, 0, 0, 0, 0);
    let (mut merged, mut merged_finished, mut errors) = (0, 0, 0);
    let (mut in_flight, mut without_events) = (0, 0);
    for (run_id, json) in &records {
        let Ok(record) = serde_json::from_str::<crate::run_manifest::DelegationRecord>(json) else {
            continue;
        };
        if record.kind != child_task || record.tool != crate::app_sessions::HANDOVER_TOOL {
            continue;
        }
        count += 1;
        let pred: Option<(String, i64)> = conn
            .query_row(
                "SELECT session_key, ord FROM turns WHERE run_id = ?1 ORDER BY ord DESC LIMIT 1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("handing-over turn: {e}"))?;
        let Some((pred_key, pred_ord)) = pred else {
            continue;
        };
        with_pred += 1;
        let times = turn_times(conn, &pred_key)?;
        first += usize::from(times.keys().min() == Some(&pred_ord));
        let succ_start: Option<String> = match record.child_session.as_deref() {
            Some(child) => conn
                .query_row(
                    "SELECT started_at FROM turns WHERE session_key = ?1 ORDER BY ord ASC LIMIT 1",
                    params![child],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("successor's first turn: {e}"))?,
            None => None,
        };
        let Some(at) = succ_start
            .as_deref()
            .and_then(crate::session_store::parse_iso)
        else {
            continue;
        };
        with_succ += 1;
        overlapped += usize::from(
            times
                .get(&pred_ord)
                .is_some_and(|(started, ended)| *started < at && ended.is_none_or(|e| e > at)),
        );
        let Ok(turns) = crate::session_store::task_turns(vault, &pred_key, 0, i64::MAX) else {
            errors += 1;
            continue;
        };
        let mut visible = turns_at(&turns, &times, at);
        for turn in &mut visible {
            if turn.status.as_deref() != Some("running") {
                continue;
            }
            in_flight += 1;
            let events = turn_events(conn, &pred_key, turn.ord)?;
            if events.is_empty() {
                without_events += 1;
            } else {
                turn_as_of(turn, &events, at);
            }
        }
        merged += usize::from(!folded(&visible).is_empty());
        let finished = visible
            .iter()
            .take_while(|turn| turn.status.as_deref() != Some("running"));
        merged_finished += usize::from(!folded(finished).is_empty());
    }
    Ok(json!({
        "handovers": count,
        "withPredecessorTurn": with_pred,
        "handingOverTurnWasFirst": first,
        "withSuccessorTurn": with_succ,
        "startedWhilePredecessorRan": overlapped,
        "mergedOnFirstTurn": merged,
        "mergedOnFirstTurnFinishedTurnsOnly": merged_finished,
        "inFlightTurns": in_flight,
        "inFlightTurnsWithoutEvents": without_events,
        "approximate": without_events > 0,
        "errors": errors,
    }))
}

// ── Notes (HF03, HF05) ────────────────────────────────────────────────────

/// The notes `vault::walk_notes` lists in `source`, without a `restricted`
/// component.
fn notes_of(source: &Path) -> Vec<crate::types::Note> {
    crate::vault::walk_notes(source)
        .into_iter()
        .filter(|note| !has_restricted(Path::new(&note.path)))
        .collect()
}

/// Every note through `vault_embed_index::prepare_chunks`: how many, and how
/// many with no chunk, by kind.
pub(crate) fn embed_chunks(source: &Path) -> Value {
    let mut by_kind: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut notes, mut zero, mut unreadable) = (0, 0, 0);
    for note in notes_of(source) {
        let Ok(content) = std::fs::read_to_string(source.join(&note.path)) else {
            unreadable += 1;
            continue;
        };
        let (kind, chunks) = crate::vault_embed_index::chunk_count(&note.path, &content);
        let entry = by_kind.entry(kind).or_default();
        entry.0 += 1;
        notes += 1;
        if chunks == 0 {
            entry.1 += 1;
            zero += 1;
        }
    }
    let by_kind: BTreeMap<String, Value> = by_kind
        .into_iter()
        .map(|(kind, (n, z))| (kind, json!({ "notes": n, "zeroChunks": z })))
        .collect();
    json!({
        "notes": notes,
        "unreadable": unreadable,
        "zeroChunks": zero,
        "byKind": by_kind,
    })
}

// ── Requests (HF04, HF05) ─────────────────────────────────────────────────

/// The newest [`REQUEST_WINDOW`] turn requests (next-step blocks stripped)
/// through `note_lifecycle::history_cue`, and through the oversized-notice
/// rule (`vault_search::oversized_notice_for`) against the retrieval-eligible
/// notes of `source` over the 1 MB scan limit. Before HF05 every request
/// with a search term got that notice whenever such a note existed.
///
/// TODO(OF17, after R1): once OB18's `main_agent::host_subject` and OB25's
/// `retrieval.queryBasis` merge, add the empty-host-subject count over the
/// wake and check-in requests (OB18's skipped retrievals and Jev calls) and
/// the follow-up basis counts (`request`, `followup+prior`,
/// `skipped-followup`; OB25) here.
pub(crate) fn requests(source: &Path, conn: &Connection) -> Result<Value, String> {
    let asked: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT asked FROM turns WHERE asked IS NOT NULL AND TRIM(asked) != '' \
                 ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| format!("requests: {e}"))?;
        let rows = stmt
            .query_map(params![REQUEST_WINDOW as i64], |row| row.get(0))
            .map_err(|e| format!("requests: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("requests: {e}"))?
    };
    let oversized: Vec<String> = notes_of(source)
        .into_iter()
        .filter(|note| {
            note.size > crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES
                && crate::retrieval_broker::vault_eligible(&note.path)
        })
        .map(|note| note.name)
        .collect();
    let (mut host, mut historical, mut with_terms, mut noticed) = (0, 0, 0, 0);
    let mut cues: BTreeMap<&'static str, usize> = BTreeMap::new();
    for request in &asked {
        let request = crate::next_step::strip(request);
        host += usize::from(crate::main_agent::is_wake_message(&request));
        if let Some(cue) = crate::note_lifecycle::history_cue(&request) {
            historical += 1;
            *cues.entry(cue).or_default() += 1;
        }
        let terms = crate::directory_connections::retrieval_terms(&request);
        if terms.is_empty() {
            continue;
        }
        with_terms += 1;
        noticed += usize::from(
            oversized
                .iter()
                .any(|name| crate::vault_search::oversized_notice_for(name, &terms)),
        );
    }
    let before_hf05 = if oversized.is_empty() { 0 } else { with_terms };
    Ok(json!({
        "window": REQUEST_WINDOW,
        "requests": asked.len(),
        "hostRequests": host,
        "historyCue": {
            "flagged": historical,
            "share": share(historical, asked.len()),
            "byCue": cues,
        },
        "oversizedNotice": {
            "oversizedNotes": oversized.len(),
            "requestsWithTerms": with_terms,
            "withNotice": noticed,
            "share": share(noticed, with_terms),
            "withNoticeBeforeHf05": before_hf05,
        },
    }))
}

// ── Report ────────────────────────────────────────────────────────────────

/// Every probe over `replay`, as one aggregates-only JSON object.
pub(crate) fn report(replay: &Replay) -> Result<Value, String> {
    let conn = replay.conn()?;
    let envelope = envelopes(&replay.vault, &replay.workspaces);
    let lost = envelope["briefs"]["cutArchivePointerLost"].clone();
    Ok(json!({
        "schema": 2,
        "snapshotQuickCheck": replay.quick_check,
        "envelope": envelope,
        "cutBriefsLosingArchivePointer": lost,
        "taskState": task_states(&replay.vault, &conn)?,
        "handover": handovers(&replay.vault, &conn)?,
        "embedChunks": embed_chunks(&replay.source),
        "requests": requests(&replay.source, &conn)?,
    }))
}

pub(crate) fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// The real-vault run. Skips (passes) when either variable is unset.
#[test]
#[ignore = "reads a real vault copy and Sessions DB: TMPDIR=<mktemp -d> HOLLOW_REPLAY_VAULT=<copy without restricted/> HOLLOW_REPLAY_DB=<db> cargo test --lib -- --ignored replay_probes"]
fn real_vault_replay_probes() {
    let (Some(vault), Some(db)) = (
        env_nonempty("HOLLOW_REPLAY_VAULT"),
        env_nonempty("HOLLOW_REPLAY_DB"),
    ) else {
        eprintln!("replay_probes: HOLLOW_REPLAY_VAULT or HOLLOW_REPLAY_DB is unset; skipped");
        return;
    };
    let vault = PathBuf::from(vault);
    assert!(
        !vault.join(RESTRICTED).exists(),
        "HOLLOW_REPLAY_VAULT has a restricted/ folder: run on a copy without it (rsync -a --exclude restricted/)"
    );
    let replay = Replay::setup(&vault, Path::new(&db)).expect("replay setup");
    let made = [replay.vault.clone(), replay.db.clone()];
    let result = report(&replay);
    drop(replay);
    let deleted = made.iter().all(|path| !path.exists());
    eprintln!("replay_probes: snapshot and copied docs deleted: {deleted}");
    let mut json = result.expect("replay probes");
    json["snapshotDeleted"] = deleted.into();
    let text = serde_json::to_string_pretty(&json).unwrap();
    match env_nonempty("HOLLOW_REPLAY_OUT") {
        Some(out) => std::fs::write(out, text).expect("write the replay report"),
        None => println!("{text}"),
    }
    assert!(deleted, "the snapshot or the copied docs were not deleted");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{append_event, finish_turn, start_turn, ConversationTool};

    const CHECKIN: &str = "⟲ Hollow: check-in at 10:00. Nothing new came up.";

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn db_of(vault: &Path) -> PathBuf {
        crate::session_store::db_path_for(vault).unwrap()
    }

    /// A brief with `n` dated Current context entries (oldest first), an
    /// archive pointer at the section's end when `pointer`, and Decisions.
    fn brief(title: &str, n: u32, pointer: bool) -> String {
        let mut blocks: Vec<String> = (0..n)
            .map(|i| {
                let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
                    + chrono::Duration::days(i64::from(i));
                format!(
                    "**{} 09:00 EDT, entry {i}.** {}",
                    day.format("%Y-%m-%d"),
                    "words ".repeat(60)
                )
            })
            .collect();
        if pointer {
            blocks.push(format!(
                "{POINTER} `outputs/workspace-archive/2026-02-28-current-context.md` on 2026-02-28 (through 2026-02-27)."
            ));
        }
        format!(
            "---\nhollow_workspace: 1\ntitle: {title}\n---\n# {title}\n\n## Outcome\n\nShip it.\n\n## Current context\n\n{}\n\n## Decisions\n\n- Keep the vault local.\n",
            blocks.join("\n\n")
        )
    }

    /// One tool call and its result in turn `ord` of `key`.
    fn tool(v: &Path, key: &str, ord: i64, id: &str, name: &str, input: Value) {
        let used = json!({ "id": id, "name": name, "input": input });
        append_event(v, key, ord, "toolUse", &used.to_string()).unwrap();
        let result = json!({ "id": id, "isError": false, "content": "ok" });
        append_event(v, key, ord, "toolResult", &result.to_string()).unwrap();
    }

    fn turn(v: &Path, key: &str, cwd: &str, run: &str, asked: &str) -> i64 {
        start_turn(v, key, "claude", cwd, run, asked).unwrap()
    }

    /// Setup copies each workspace's documents, project note and sources,
    /// the vault's projects and goals (and a named folder), never a
    /// restricted path, a link or anything else; the snapshot is what the
    /// session store reads for the copy, 0600, checked and frozen at setup;
    /// drop deletes both and leaves the source alone.
    #[test]
    fn setup_copies_docs_and_a_frozen_snapshot_and_drop_deletes_them() {
        use std::os::unix::fs::PermissionsExt;
        let dir = vault();
        let v = dir.path();
        let alpha = "---\nhollow_workspace: 1\n---\n# A\n";
        put(v, "workspaces/alpha/WORKSPACE.md", alpha);
        put(v, "workspaces/alpha/PLAN.md", "# Plan\n");
        put(v, "workspaces/alpha/PROJECT.md", "---\ntitle: A\n---\n");
        put(v, "workspaces/alpha/sources/brief.md", "a source\n");
        put(
            v,
            "workspaces/alpha/sources/.provenance/brief.md.json",
            "{}\n",
        );
        put(v, "workspaces/alpha/sources/restricted/s.md", "secret\n");
        put(v, "workspaces/alpha/outputs/report.md", "private output\n");
        put(v, "workspaces/alpha/restricted/WORKSPACE.md", "secret\n");
        put(v, "workspaces/restricted/WORKSPACE.md", "secret\n");
        put(v, "workspaces/empty/notes.md", "no context docs\n");
        put(v, "workspaces/empty/sources/x.md", "no context docs\n");
        put(v, "projects/p.md", "---\ntitle: P\n---\n");
        put(v, "projects/restricted/q.md", "secret\n");
        put(v, "goals/g.md", "---\ntitle: G\n---\n");
        put(v, "learnings/a.md", "a learning\n");
        put(v, "learnings/restricted/b.md", "secret\n");
        put(v, "learnings/.trash/c.md", "gone\n");
        put(v, "restricted/d.md", "secret\n");
        std::os::unix::fs::symlink(
            v.join("restricted/d.md"),
            v.join("workspaces/alpha/sources/link.md"),
        )
        .unwrap();
        let key = "s-of17-setup";
        turn(v, key, "/v/workspaces/alpha", "run-1", "First request.");

        let replay = Replay::setup_with(v, &db_of(v), &["learnings"]).unwrap();
        let copy = replay.vault.clone();
        let snapshot = replay.db.clone();
        assert!(copy.starts_with(std::env::temp_dir()));
        let name = copy.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(&format!("hollow-test-replay-{}-", std::process::id())));
        assert_eq!(replay.workspaces, vec!["alpha".to_string()]);
        assert_eq!(replay.quick_check, "ok");
        let bits = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(bits(&snapshot), 0o600);
        assert_eq!(bits(&copy), 0o700);
        // An existing snapshot file is never overwritten.
        assert!(snapshot_db(&db_of(v), &snapshot).is_err());
        let mut copied: Vec<String> = walkdir::WalkDir::new(&copy)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(&copy)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        copied.sort();
        assert_eq!(
            copied,
            vec![
                "goals/g.md".to_string(),
                "learnings/a.md".to_string(),
                "projects/p.md".to_string(),
                "workspaces/alpha/PLAN.md".to_string(),
                "workspaces/alpha/PROJECT.md".to_string(),
                "workspaces/alpha/WORKSPACE.md".to_string(),
                "workspaces/alpha/sources/.provenance/brief.md.json".to_string(),
                "workspaces/alpha/sources/brief.md".to_string(),
            ]
        );
        assert_eq!(snapshot, db_of(&copy));
        assert_ne!(snapshot, db_of(v));
        // A turn added to the source after setup isn't in the snapshot.
        turn(v, key, "/v/workspaces/alpha", "run-2", "Second request.");
        let turns = crate::session_store::task_turns(&replay.vault, key, 0, i64::MAX).unwrap();
        assert_eq!(turns.len(), 1);
        let n: i64 = replay
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(report(&replay).unwrap()["taskState"]["records"], 1);
        drop(replay);
        assert!(!copy.exists() && !snapshot.exists());
        for suffix in ["-wal", "-shm"] {
            let mut path = snapshot.clone().into_os_string();
            path.push(suffix);
            assert!(!PathBuf::from(path).exists());
        }
        assert!(v.join("workspaces/alpha/WORKSPACE.md").exists() && db_of(v).exists());
        assert!(Replay::setup(&v.join("missing"), &db_of(v)).is_err());
    }

    /// What a cut brief kept, read from its envelope copy: a section cut
    /// keeps Decisions and the newest entry and marks what it left out; a
    /// prefix cut keeps neither; a brief without the parts has none.
    #[test]
    fn brief_facts_read_what_a_cut_kept() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let source = format!(
            "---\nhollow_workspace: 1\n---\n# B\n\n## Current context\n\n**2026-07-01 entry 0.** old\n\n**2026-07-02 entry 1.** new\n\n{POINTER} `x.md` on 2026-07-03.\n\n## Decisions\n\n- d\n"
        );
        let sectioned = "---\nhollow_workspace: 1\n---\n# B\n\n## Current context\n\n[Hollow: 1 older entries in Current context left out; read WORKSPACE.md for them.]\n\n**2026-07-02 entry 1.** new\n\n## Decisions\n\n- d\n\n\n[Hollow: cut to fit. 120 of 200 bytes are included, by section.]";
        assert_eq!(
            brief_facts(&source, sectioned, today),
            BriefFacts {
                cut: true,
                decisions: Some(true),
                newest_entry: Some(true),
                left_out: true,
                pointer: Some(false),
            }
        );
        let prefix = "---\nhollow_workspace: 1\n---\n# B\n\n## Current context\n\n**2026-07-01 entry 0.** old\n\n[Hollow: cut to fit. Only the first 60 of 200 bytes are included.]";
        assert_eq!(
            brief_facts(&source, prefix, today),
            BriefFacts {
                cut: true,
                decisions: Some(false),
                newest_entry: Some(false),
                left_out: false,
                pointer: Some(false),
            }
        );
        assert_eq!(
            brief_facts(&source, &source, today),
            BriefFacts {
                cut: false,
                decisions: Some(true),
                newest_entry: Some(true),
                left_out: false,
                pointer: Some(true),
            }
        );
        let bare = "---\nhollow_workspace: 1\n---\n# B\n\nJust a note.\n";
        assert_eq!(brief_facts(bare, bare, today), BriefFacts::default());
    }

    /// The envelope probe through the real `context_for_within`: a brief
    /// past its 48 KB share is cut by section and keeps Decisions, its
    /// newest entry and its archive pointer (here at the newest end); a
    /// small one isn't cut; the Main agent's brief reports its size and
    /// where Decisions is.
    #[test]
    fn envelope_probe_counts_cut_briefs_and_what_they_kept() {
        let dir = vault();
        let v = dir.path();
        put(v, "workspaces/big/WORKSPACE.md", &brief("Big", 150, true));
        let small = brief("Small", 3, false);
        put(v, "workspaces/small/WORKSPACE.md", &small);
        let main = "---\nhollow_workspace: 1\ntitle: Main agent\n---\n# Main\n\n## Outcome\n\nHelp.\n\n## Decisions\n\n- One.\n";
        put(v, "workspaces/main/WORKSPACE.md", main);
        turn(v, "s-of17-env", "/v/workspaces/big", "run-1", "Hello.");
        let replay = Replay::setup(v, &db_of(v)).unwrap();
        let out = envelopes(&replay.vault, &replay.workspaces);
        assert_eq!(out["workspaces"], 3, "{out}");
        assert_eq!(out["errors"], 0, "{out}");
        let briefs = &out["briefs"];
        assert_eq!(briefs["total"], 3, "{out}");
        assert_eq!(briefs["cut"], 1, "{out}");
        assert_eq!(briefs["cutLeftOutMarked"], 1, "{out}");
        assert_eq!(briefs["cutWithDecisions"], 1, "{out}");
        assert_eq!(briefs["cutDecisionsIncluded"], 1, "{out}");
        assert_eq!(briefs["cutWithDatedEntries"], 1, "{out}");
        assert_eq!(briefs["cutNewestEntryIncluded"], 1, "{out}");
        assert_eq!(briefs["cutWithArchivePointer"], 1, "{out}");
        assert_eq!(briefs["cutArchivePointerLost"], 0, "{out}");
        assert_eq!(out["main"]["briefBytes"], main.len(), "{out}");
        assert_eq!(out["main"]["briefLines"], 13, "{out}");
        assert_eq!(out["main"]["decisionsLine"], 11, "{out}");
        assert_eq!(out["main"]["cut"], false, "{out}");
        let largest = out["envelopeBytes"]["max"].as_u64().unwrap();
        assert!(largest > 40_000, "{out}");
        // No text of a brief reaches the output.
        assert!(!out.to_string().contains("words"), "{out}");
    }

    fn call(id: &str, name: &str, input: Value, status: &str) -> ConversationTool {
        ConversationTool {
            id: id.into(),
            name: name.into(),
            input,
            result: (status != "running").then(|| "ok".to_string()),
            is_error: false,
            status: status.into(),
            parent_tool_use_id: None,
            result_chars: None,
            result_lines: None,
            result_turn: None,
        }
    }

    /// fold's split over the raw calls: a read tool and a read-only shell
    /// command (judged on the full input, past the 160-character step
    /// target) are dropped; an edit, a push and a check run are kept; a
    /// call still running in a running turn is neither, and fold agrees.
    #[test]
    fn fold_split_counts_what_fold_drops_and_keeps() {
        let long_grep = format!("grep -rn {} src", "x".repeat(300));
        let tools = vec![
            call("t1", "Read", json!({ "file_path": "src/lib.rs" }), "done"),
            call("t2", "Bash", json!({ "command": long_grep }), "done"),
            call(
                "t3",
                "Bash",
                json!({ "command": "git status && git log" }),
                "done",
            ),
            call(
                "t4",
                "Bash",
                json!({ "command": "git push origin main" }),
                "done",
            ),
            call(
                "t5",
                "Bash",
                json!({ "command": "cargo test --lib" }),
                "done",
            ),
            call(
                "t6",
                "Write",
                json!({ "file_path": "outputs/plan.md" }),
                "done",
            ),
            call("t7", "Edit", json!({ "file_path": "grep" }), "running"),
        ];
        let mut turn = TaskTurn {
            ord: 1,
            status: Some("success".into()),
            tools,
            ..TaskTurn::default()
        };
        assert_eq!(fold_split(&turn), (3, 4));
        assert_eq!(folded([&turn]).steps.len(), 4);
        turn.status = Some("running".into());
        assert_eq!(fold_split(&turn), (3, 3));
        assert_eq!(folded([&turn]).steps.len(), 3);
    }

    /// Folded conversations: a grep, a Read and a `git status` aren't steps
    /// (dropped as read-only: 3 of 5 calls), a push and a Write are; a Main
    /// conversation opened by a check-in has a host goal, one opened by a
    /// request doesn't.
    #[test]
    fn task_state_probe_counts_steps_and_host_goals() {
        let dir = vault();
        let v = dir.path();
        let a = "s-of17-ts-a";
        let asked = "Fix the sync. Never push to main.";
        let ord = turn(v, a, "/v/workspaces/alpha", "run-a", asked);
        let calls = [
            ("t1", "Bash", json!({ "command": "grep -rn sync src" })),
            ("t2", "Read", json!({ "file_path": "src/sync.rs" })),
            ("t3", "Bash", json!({ "command": "git push origin fix" })),
            ("t4", "Write", json!({ "file_path": "src/sync.rs" })),
            ("t5", "Bash", json!({ "command": "git status && git log" })),
        ];
        for (id, name, input) in calls {
            tool(v, a, ord, id, name, input);
        }
        finish_turn(v, a, ord, "success").unwrap();
        let conversations = [
            ("s-of17-ts-m1", "/v/workspaces/main", CHECKIN),
            ("s-of17-ts-m2", "/v/workspaces/main/", "Draft a note."),
        ];
        for (key, cwd, asked) in conversations {
            let ord = turn(v, key, cwd, "run-m", asked);
            finish_turn(v, key, ord, "success").unwrap();
        }
        let replay = Replay::setup(v, &db_of(v)).unwrap();
        let out = task_states(&replay.vault, &replay.conn().unwrap()).unwrap();
        assert_eq!(out["conversations"], 3, "{out}");
        assert_eq!(out["records"], 3, "{out}");
        assert_eq!(out["steps"], 2, "{out}");
        assert_eq!(out["readOnlyDropped"], 3, "{out}");
        assert_eq!(out["stepsKept"], 2, "{out}");
        assert_eq!(out["readOnlyShareBefore"], 0.6, "{out}");
        assert_eq!(out["stepOutcomes"]["done"], 2, "{out}");
        assert_eq!(out["hostGoals"], 1, "{out}");
        assert_eq!(out["main"]["records"], 2, "{out}");
        assert_eq!(out["main"]["hostGoals"], 1, "{out}");
        assert_eq!(out["main"]["checkInGoals"], 1, "{out}");
        assert!(!out.to_string().contains("sync"), "{out}");
    }

    /// Handovers: a successor that started while the handing-over turn ran
    /// merges its task state under today's rule (OB14's peek) but not under
    /// the finished-turns rule; one that started after it ended merges under
    /// both; one whose successor never ran counts only as a handover.
    #[test]
    fn handover_probe_treats_an_overlapping_predecessor_turn_as_running() {
        let dir = vault();
        let v = dir.path();
        let t0 = Utc::now() - chrono::Duration::hours(2);
        let at = |minutes: i64| (t0 + chrono::Duration::minutes(minutes)).to_rfc3339();
        let handover = |run: &str, child: &str| {
            let record = crate::run_manifest::DelegationRecord {
                kind: "childTask".into(),
                tool: crate::app_sessions::HANDOVER_TOOL.into(),
                agent: "claude".into(),
                child_session: Some(child.into()),
                at: at(0),
                ..Default::default()
            };
            let now = crate::session_store::db_time(Utc::now());
            let json = serde_json::to_string(&record).unwrap();
            assert!(crate::session_store::record_delegation(v, run, &now, &json, 8).unwrap());
        };
        let cwd = "/v/workspaces/alpha";
        // (session, run, asked, started, ended)
        let turns = [
            ("s-of17-p1", "run-p1", "Write the plan.", 0, Some(10)),
            ("s-of17-c1", "run-c1", "Pick up the plan.", 1, Some(20)),
            ("s-of17-p2", "run-p2", "Tidy the notes.", 30, Some(35)),
            ("s-of17-c2", "run-c2", "Carry on tidying.", 40, None),
            ("s-of17-p3", "run-p3", "Sort the inbox.", 50, None),
        ];
        for (key, run, asked, _, _) in turns {
            let ord = turn(v, key, cwd, run, asked);
            if key == "s-of17-p1" {
                // Written after c1 started (its events are re-dated below).
                tool(
                    v,
                    key,
                    ord,
                    "w1",
                    "Write",
                    json!({ "file_path": "plan.md" }),
                );
            }
            finish_turn(v, key, ord, "success").unwrap();
        }
        handover("run-p1", "s-of17-c1");
        handover("run-p2", "s-of17-c2");
        handover("run-p3", "s-of17-c3");
        // A start-session delegation is no handover.
        let start = crate::run_manifest::DelegationRecord {
            kind: "childTask".into(),
            tool: "start_session".into(),
            child_session: Some("s-of17-c1".into()),
            ..Default::default()
        };
        let now = crate::session_store::db_time(Utc::now());
        let start = serde_json::to_string(&start).unwrap();
        crate::session_store::record_delegation(v, "run-p2", &now, &start, 8).unwrap();
        {
            let conn = Connection::open(db_of(v)).unwrap();
            for (key, _, _, started, ended) in turns {
                conn.execute(
                    "UPDATE turns SET started_at = ?2, ended_at = ?3 WHERE session_key = ?1",
                    params![key, at(started), ended.map(at)],
                )
                .unwrap();
            }
            conn.execute(
                "UPDATE events SET at = ?2 WHERE session_key = ?1",
                params!["s-of17-p1", at(5)],
            )
            .unwrap();
        }
        let replay = Replay::setup(v, &db_of(v)).unwrap();
        let out = handovers(&replay.vault, &replay.conn().unwrap()).unwrap();
        assert_eq!(out["handovers"], 3, "{out}");
        assert_eq!(out["withPredecessorTurn"], 3, "{out}");
        assert_eq!(out["handingOverTurnWasFirst"], 3, "{out}");
        assert_eq!(out["withSuccessorTurn"], 2, "{out}");
        assert_eq!(out["startedWhilePredecessorRan"], 1, "{out}");
        assert_eq!(out["mergedOnFirstTurn"], 2, "{out}");
        assert_eq!(out["mergedOnFirstTurnFinishedTurnsOnly"], 1, "{out}");
        assert_eq!(out["inFlightTurns"], 1, "{out}");
        assert_eq!(out["inFlightTurnsWithoutEvents"], 0, "{out}");
        assert_eq!(out["approximate"], false, "{out}");
        assert_eq!(out["errors"], 0, "{out}");
    }

    /// A turn cut back to a moment: a call begun after it is gone, one whose
    /// result came after it is running, the reply is what had been said by
    /// then.
    #[test]
    fn turn_as_of_keeps_what_had_happened_by_then() {
        let t0 = Utc::now();
        let min = |m: i64| t0 + chrono::Duration::minutes(m);
        let event =
            |kind: &str, payload: &str, m: i64| (kind.to_string(), payload.to_string(), min(m));
        let mut turn = TaskTurn {
            ord: 1,
            status: Some("running".into()),
            reply: "Done. Pushed.".into(),
            tools: vec![
                call("a", "Write", json!({ "file_path": "a.md" }), "done"),
                call("b", "Bash", json!({ "command": "git push" }), "done"),
                call("c", "Edit", json!({ "file_path": "c.md" }), "done"),
            ],
            check_outcomes: [("b".to_string(), "pass".to_string())]
                .into_iter()
                .collect(),
            ..TaskTurn::default()
        };
        let events = [
            event("toolUse", r#"{"id":"a"}"#, 1),
            event("toolResult", r#"{"id":"a"}"#, 2),
            event("assistant", "Done.", 3),
            event("toolUse", r#"{"id":"b"}"#, 4),
            event("toolResult", r#"{"id":"b"}"#, 6),
            event("assistant", "Done. Pushed.", 7),
            event("toolUse", r#"{"id":"c"}"#, 8),
        ];
        turn_as_of(&mut turn, &events, min(5));
        let tools: Vec<(&str, &str, bool)> = turn
            .tools
            .iter()
            .map(|t| (t.id.as_str(), t.status.as_str(), t.result.is_some()))
            .collect();
        assert_eq!(tools, vec![("a", "done", true), ("b", "running", false)]);
        assert!(turn.check_outcomes.is_empty());
        assert_eq!(turn.reply, "Done.");
        // Fold sees one finished, unconfirmed step; the running call isn't one.
        let record = folded([&turn]);
        assert_eq!(record.steps.len(), 1);
        assert_eq!(record.steps[0].outcome, "unconfirmed");
    }

    /// TMPDIR is refused inside this checkout, under a `Worktrees` folder
    /// and when it can't be resolved; the test's own temp dir is fine.
    #[test]
    fn refuse_temp_dir_fails_closed() {
        assert!(refuse_temp_dir(&std::env::temp_dir()).is_ok());
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(refuse_temp_dir(checkout).is_err());
        let dir = vault();
        let worktrees = dir.path().join("Worktrees/lane");
        std::fs::create_dir_all(&worktrees).unwrap();
        assert!(refuse_temp_dir(&worktrees).is_err());
        assert!(refuse_temp_dir(&dir.path().join("missing")).is_err());
    }

    /// `turns_at`: a turn started after the moment is left out, one still
    /// going then is running, one already over keeps its status.
    #[test]
    fn turns_at_marks_what_was_still_running() {
        let t0 = Utc::now();
        let min = |m: i64| t0 + chrono::Duration::minutes(m);
        let turn = |ord: i64| TaskTurn {
            ord,
            status: Some("success".into()),
            ..TaskTurn::default()
        };
        let turns = [turn(1), turn(2), turn(3), turn(4)];
        let times: TurnTimes = [
            (1, (min(0), Some(min(5)))),
            (2, (min(6), Some(min(20)))),
            (3, (min(7), None)),
            (4, (min(11), Some(min(12)))),
        ]
        .into_iter()
        .collect();
        let seen = turns_at(&turns, &times, min(10));
        let status: Vec<(i64, &str)> = seen
            .iter()
            .map(|t| (t.ord, t.status.as_deref().unwrap()))
            .collect();
        assert_eq!(status, vec![(1, "success"), (2, "running"), (3, "running")]);
    }

    /// Requests: a past-pinned word is historical and an unpinned "before"
    /// isn't; only a request naming the oversized note's file hears about
    /// it, though before HF05 every request with a term did. A restricted
    /// oversized note is never listed.
    #[test]
    fn request_probe_counts_history_cues_and_oversized_notices() {
        let dir = vault();
        let v = dir.path();
        let filler = "lorem ipsum ".repeat(95_000);
        let big = format!("<html><body>\n<p>{filler}</p>\n</body></html>\n");
        put(v, "briefings/preview.html", &big);
        put(v, "restricted/huge.md", &big);
        put(v, "notes/owner.md", "Cobalt owner is Dana.\n");
        let key = "s-of17-req";
        for (run, asked) in [
            ("run-1", "What did we previously decide on Cobalt?"),
            ("run-2", "Run the tests before merging."),
            ("run-3", "Why is preview.html missing for Cobalt?"),
            ("run-4", CHECKIN),
        ] {
            let ord = turn(v, key, "/v/workspaces/alpha", run, asked);
            finish_turn(v, key, ord, "success").unwrap();
        }
        let replay = Replay::setup(v, &db_of(v)).unwrap();
        let out = requests(&replay.source, &replay.conn().unwrap()).unwrap();
        assert_eq!(out["requests"], 4, "{out}");
        assert_eq!(out["hostRequests"], 1, "{out}");
        assert_eq!(out["historyCue"]["flagged"], 1, "{out}");
        assert_eq!(out["historyCue"]["byCue"]["previously"], 1, "{out}");
        assert_eq!(out["historyCue"]["share"], 0.25, "{out}");
        let notice = &out["oversizedNotice"];
        assert_eq!(notice["oversizedNotes"], 1, "{out}");
        assert_eq!(notice["requestsWithTerms"], 4, "{out}");
        assert_eq!(notice["withNotice"], 1, "{out}");
        assert_eq!(notice["withNoticeBeforeHf05"], 4, "{out}");
        assert!(!out.to_string().contains("Cobalt"), "{out}");
    }

    /// prepare_chunks over the notes, by kind: a task with only a title
    /// gets its frontmatter chunk, an empty one gets none; restricted notes
    /// aren't read.
    #[test]
    fn embed_probe_counts_zero_chunk_notes_by_kind() {
        let dir = vault();
        let v = dir.path();
        let renew = "---\ntitle: Renew permit\ndescription: Elm St lot\n---\n";
        put(v, "tasks/renew.md", renew);
        put(v, "tasks/empty.md", "");
        put(v, "notes/owner.md", "Cobalt owner is Dana.\n");
        put(v, "restricted/empty.md", "");
        put(v, "notes/restricted/empty.md", "");
        let out = embed_chunks(v);
        assert_eq!(out["notes"], 3, "{out}");
        assert_eq!(out["zeroChunks"], 1, "{out}");
        let task = &out["byKind"][crate::vault_index::KIND_TASK];
        assert_eq!(task["notes"], 2, "{out}");
        assert_eq!(task["zeroChunks"], 1, "{out}");
        assert!(!out.to_string().contains("owner"), "{out}");
    }

    #[test]
    fn workspace_of_reads_a_workspace_folder_only() {
        assert_eq!(workspace_of("/v/workspaces/main"), Some("main"));
        assert_eq!(workspace_of("/v/workspaces/main/"), Some("main"));
        assert_eq!(workspace_of("/v/workspaces/main/outputs"), None);
        assert_eq!(workspace_of("/Users/x/Hollow/Worktrees/a"), None);
        assert!(has_restricted(Path::new("a/restricted/b.md")));
        assert!(!has_restricted(Path::new("a/restricted-ish/b.md")));
    }
}
